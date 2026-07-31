use std::sync::Arc;

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
            let reply = self
                .adapters
                .complete(&messages, &available_tools, tool_choice)
                .await?;
            messages.push(reply.raw_message);
            if reply.tool_calls.is_empty() {
                final_text = reply.content.trim().to_owned();
                break;
            }
            for call in reply.tool_calls {
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
        if final_text.is_empty() {
            anyhow::bail!("工具调用轮次超过限制或模型没有生成回复");
        }

        self.context
            .add_turn(session_id, "assistant", &final_text, None)
            .await?;
        for delta in chunks_by_chars(&final_text, 24) {
            emit_response(
                send,
                response_id,
                json!({"type": "response.text.delta", "delta": delta}),
            )
            .await?;
            tokio::task::yield_now().await;
        }

        let audio_chunk_size =
            (self.settings.sample_rate_out as usize * self.settings.audio_chunk_ms.max(20) / 1_000)
                .max(1);
        for sentence in split_for_speech(&final_text, 80) {
            let mut stream = self.adapters.synthesize_stream(&sentence).await?;
            let mut buffered = Vec::new();
            while let Some(output) = stream.next().await {
                buffered.extend(output?);
                while buffered.len() >= audio_chunk_size {
                    let remainder = buffered.split_off(audio_chunk_size);
                    let chunk = std::mem::replace(&mut buffered, remainder);
                    emit_audio(send, response_id, &chunk, self.settings.sample_rate_out).await?;
                }
            }
            if !buffered.is_empty() {
                emit_audio(send, response_id, &buffered, self.settings.sample_rate_out).await?;
            }
        }
        emit_response(
            send,
            response_id,
            json!({"type": "response.done", "text": final_text}),
        )
        .await
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
}
