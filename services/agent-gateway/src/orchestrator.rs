use std::{collections::BTreeMap, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    adapters::{ModelAdapters, build_multimodal_user_message, tool_result_message},
    audio::encode_le_f32,
    config::Settings,
    context::ContextStore,
    protocol::VideoFrame,
    tools::{ToolExecutor, schemas, select_forced_tool},
};

const SYSTEM_PROMPT: &str = "你是 Ripple Live，一个运行在用户自有服务器上的中文多模态语音 Agent。
你可以理解当前语音转写和随附的视频画面。回答应自然、简洁、适合直接朗读。
需要外部动作或精确结果时必须使用提供的工具，不要假装已经调用。
只有用户明确要求记住某件事时才调用 remember；需要历史记忆时调用 recall。
工具失败时如实说明。不要在朗读内容中输出工具调用 JSON。";

#[derive(Clone)]
pub struct AgentOrchestrator {
    settings: Arc<Settings>,
    context: ContextStore,
    adapters: ModelAdapters,
    tools: ToolExecutor,
}

impl AgentOrchestrator {
    pub fn new(settings: Arc<Settings>, context: ContextStore, adapters: ModelAdapters) -> Self {
        Self {
            settings,
            tools: ToolExecutor::new(context.clone()),
            context,
            adapters,
        }
    }

    pub async fn run_turn(
        &self,
        session_id: &str,
        send: &mpsc::Sender<Value>,
        audio: Vec<f32>,
        frames: Vec<VideoFrame>,
        transcript_override: Option<String>,
        response_id: &str,
    ) -> anyhow::Result<()> {
        emit_response(send, response_id, json!({"type": "response.created"})).await?;
        let transcript = match transcript_override
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => text.to_owned(),
            None if audio.is_empty() => anyhow::bail!("没有可处理的音频或文本"),
            None => self.adapters.transcribe(&audio).await?,
        };
        if transcript.trim().is_empty() {
            anyhow::bail!("ASR 未识别出文本");
        }

        emit(
            send,
            json!({"type": "input.transcript.final", "text": transcript}),
        )
        .await?;
        self.context
            .add_turn(
                session_id,
                "user",
                &transcript,
                Some(&json!({"frames": frames.len()})),
            )
            .await?;

        let mut history = self
            .context
            .recent_messages(session_id, self.settings.max_recent_turns)
            .await?;
        history.pop();
        let mut messages = vec![json!({"role": "system", "content": SYSTEM_PROMPT})];
        messages.extend(history);
        messages.push(build_multimodal_user_message(&transcript, &frames));

        let audio_chunk_size =
            (self.settings.sample_rate_out as usize * self.settings.audio_chunk_ms.max(20) / 1_000)
                .max(1);
        let (speech_sender, speech_receiver) = mpsc::channel::<String>(4);
        let speech = stream_speech(
            self.adapters.clone(),
            send.clone(),
            response_id.to_owned(),
            speech_receiver,
            audio_chunk_size,
            self.settings.sample_rate_out,
        );
        let generation = async move {
            let available_tools = schemas();
            let forced_tool = select_forced_tool(&transcript);
            let mut final_text = String::new();
            for round in 0..self.settings.max_tool_rounds {
                let tool_choice = if round == 0 {
                    forced_tool
                        .map(|name| json!({"type": "function", "function": {"name": name}}))
                        .unwrap_or_else(|| json!("auto"))
                } else {
                    json!("auto")
                };
                let mut stream = self
                    .adapters
                    .complete_stream(&messages, &available_tools, tool_choice)
                    .await?;
                let mut content = String::new();
                let mut tool_calls = BTreeMap::new();
                let mut segmenter = SpeechSegmenter::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    if let Some(delta) = chunk
                        .pointer("/choices/0/delta/content")
                        .and_then(Value::as_str)
                        .filter(|delta| !delta.is_empty())
                    {
                        content.push_str(delta);
                        emit_response(
                            send,
                            response_id,
                            json!({"type": "response.text.delta", "delta": delta}),
                        )
                        .await?;
                        for phrase in segmenter.push(delta) {
                            speech_sender.send(phrase).await.map_err(|_| {
                                anyhow::anyhow!("speech pipeline stopped unexpectedly")
                            })?;
                        }
                    }
                    merge_tool_call_deltas(&mut tool_calls, &chunk);
                }
                let calls = tool_calls.into_values().collect::<Vec<_>>();
                let raw_message = streamed_raw_message(&content, &calls);
                messages.push(raw_message);
                if calls.is_empty() {
                    for phrase in segmenter.finish() {
                        speech_sender
                            .send(phrase)
                            .await
                            .map_err(|_| anyhow::anyhow!("speech pipeline stopped unexpectedly"))?;
                    }
                    final_text = content.trim().to_owned();
                    break;
                }
                if !content.trim().is_empty() {
                    anyhow::bail!("模型同时返回了朗读文本和工具调用");
                }
                for call in calls {
                    emit_response(
                        send,
                        response_id,
                        json!({
                            "type": "response.tool.started",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments
                        }),
                    )
                    .await?;
                    let result = self
                        .tools
                        .execute(session_id, &call.name, &call.arguments)
                        .await;
                    self.context
                        .record_event(
                            session_id,
                            "tool.result",
                            &json!({
                                "name": call.name,
                                "arguments": call.arguments,
                                "result": result
                            }),
                        )
                        .await?;
                    emit_response(
                        send,
                        response_id,
                        json!({
                            "type": "response.tool.completed",
                            "call_id": call.id,
                            "name": call.name,
                            "result": result
                        }),
                    )
                    .await?;
                    messages.push(tool_result_message(&call, &result));
                }
            }
            drop(speech_sender);
            if final_text.is_empty() {
                anyhow::bail!("工具调用轮次超过限制或模型没有生成回复");
            }
            anyhow::Ok(final_text)
        };
        let (generation_result, speech_result) = tokio::join!(generation, speech);
        let final_text = generation_result?;
        speech_result?;
        self.context
            .add_turn(session_id, "assistant", &final_text, None)
            .await?;
        emit_response(
            send,
            response_id,
            json!({"type": "response.done", "text": final_text}),
        )
        .await
    }
}

async fn stream_speech(
    adapters: ModelAdapters,
    send: mpsc::Sender<Value>,
    response_id: String,
    mut sentences: mpsc::Receiver<String>,
    audio_chunk_size: usize,
    sample_rate: u32,
) -> anyhow::Result<()> {
    while let Some(sentence) = sentences.recv().await {
        let mut stream = adapters.synthesize_stream(&sentence).await?;
        let mut buffered = Vec::new();
        while let Some(output) = stream.next().await {
            buffered.extend(output?);
            while buffered.len() >= audio_chunk_size {
                let remainder = buffered.split_off(audio_chunk_size);
                let chunk = std::mem::replace(&mut buffered, remainder);
                emit_audio(&send, &response_id, &chunk, sample_rate).await?;
            }
        }
        if !buffered.is_empty() {
            emit_audio(&send, &response_id, &buffered, sample_rate).await?;
        }
    }
    Ok(())
}

fn merge_tool_call_deltas(output: &mut BTreeMap<usize, crate::adapters::ToolCall>, chunk: &Value) {
    let Some(deltas) = chunk
        .pointer("/choices/0/delta/tool_calls")
        .and_then(Value::as_array)
    else {
        return;
    };
    for delta in deltas {
        let index = delta.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let call = output
            .entry(index)
            .or_insert_with(|| crate::adapters::ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        if let Some(value) = delta.get("id").and_then(Value::as_str) {
            call.id.push_str(value);
        }
        if let Some(value) = delta.pointer("/function/name").and_then(Value::as_str) {
            call.name.push_str(value);
        }
        if let Some(value) = delta.pointer("/function/arguments").and_then(Value::as_str) {
            call.arguments.push_str(value);
        }
    }
}

fn streamed_raw_message(content: &str, calls: &[crate::adapters::ToolCall]) -> Value {
    if calls.is_empty() {
        json!({"role": "assistant", "content": content})
    } else {
        let tool_calls = calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments}
                })
            })
            .collect::<Vec<_>>();
        json!({"role": "assistant", "content": null, "tool_calls": tool_calls})
    }
}

struct SpeechSegmenter {
    pending: String,
}

impl SpeechSegmenter {
    fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    fn push(&mut self, delta: &str) -> Vec<String> {
        let mut ready = Vec::new();
        for character in delta.chars() {
            self.pending.push(character);
            let length = self.pending.chars().count();
            let sentence_end = matches!(character, '。' | '！' | '？' | '!' | '?' | '；' | ';');
            let clause_end = length >= 12 && matches!(character, '，' | ',' | '：' | ':');
            if (sentence_end || clause_end || length >= 36)
                && let Some(phrase) = self.take_pending()
            {
                ready.push(phrase);
            }
        }
        ready
    }

    fn finish(mut self) -> Vec<String> {
        self.take_pending().into_iter().collect()
    }

    fn take_pending(&mut self) -> Option<String> {
        let phrase = self.pending.trim().to_owned();
        self.pending.clear();
        (!phrase.is_empty()).then_some(phrase)
    }
}

async fn emit(send: &mpsc::Sender<Value>, event: Value) -> anyhow::Result<()> {
    send.send(event)
        .await
        .map_err(|_| anyhow::anyhow!("client disconnected"))
}

async fn emit_response(
    send: &mpsc::Sender<Value>,
    response_id: &str,
    mut event: Value,
) -> anyhow::Result<()> {
    if let Some(object) = event.as_object_mut() {
        object.insert("response_id".to_owned(), json!(response_id));
    }
    emit(send, event).await
}

async fn emit_audio(
    send: &mpsc::Sender<Value>,
    response_id: &str,
    samples: &[f32],
    sample_rate: u32,
) -> anyhow::Result<()> {
    emit_response(
        send,
        response_id,
        json!({
            "type": "response.audio.delta",
            "audio": STANDARD.encode(encode_le_f32(samples)),
            "sample_rate": sample_rate
        }),
    )
    .await?;
    tokio::task::yield_now().await;
    Ok(())
}

pub fn chunks_by_chars(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

pub fn split_for_speech(text: &str, max_chars: usize) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        current.push(character);
        if matches!(character, '。' | '！' | '？' | '!' | '?' | '；' | ';') {
            push_bounded(&mut sentences, current.trim(), max_chars);
            current.clear();
        }
    }
    push_bounded(&mut sentences, current.trim(), max_chars);
    if sentences.is_empty() && !text.is_empty() {
        push_bounded(&mut sentences, text, max_chars);
    }
    sentences
}

fn push_bounded(output: &mut Vec<String>, text: &str, max_chars: usize) {
    if text.is_empty() {
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    output.extend(
        chars
            .chunks(max_chars)
            .map(|chunk| chunk.iter().collect::<String>()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_unicode_without_breaking_characters() {
        assert_eq!(
            split_for_speech("你好。第二句！", 80),
            vec!["你好。", "第二句！"]
        );
        assert_eq!(chunks_by_chars("一二三四五", 2), vec!["一二", "三四", "五"]);
    }

    #[test]
    fn segments_streaming_text_at_natural_boundaries() {
        let mut segmenter = SpeechSegmenter::new();
        assert!(segmenter.push("先说结论，").is_empty());
        assert_eq!(
            segmenter.push("这项优化可以明显降低延迟，"),
            vec!["先说结论，这项优化可以明显降低延迟，"]
        );
        assert_eq!(segmenter.push("并保持自然。"), vec!["并保持自然。"]);
        assert!(segmenter.finish().is_empty());
    }

    #[test]
    fn merges_streamed_tool_call_arguments() {
        let mut calls = BTreeMap::new();
        merge_tool_call_deltas(
            &mut calls,
            &json!({"choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call-1",
                "function": {"name": "calculate", "arguments": "{\"expression\":"}
            }]}}]}),
        );
        merge_tool_call_deltas(
            &mut calls,
            &json!({"choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "function": {"arguments": "\"1+2\"}"}
            }]}}]}),
        );
        let call = calls.get(&0).unwrap();
        assert_eq!(call.name, "calculate");
        assert_eq!(call.arguments, "{\"expression\":\"1+2\"}");
    }
}
