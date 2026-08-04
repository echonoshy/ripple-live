use std::{collections::BTreeMap, sync::Arc, time::Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use chrono_tz::Asia::Shanghai;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    adapters::{ModelAdapters, build_multimodal_user_message, tool_result_message},
    audio::encode_le_f32,
    config::Settings,
    context::ContextStore,
    context_compiler::ContextCompiler,
    endpointing::{
        EndpointDecision, EndpointEvaluation, deterministic_decision, parse_classifier_decision,
    },
    memory::{MemoryArtifact, MemoryService},
    protocol::VideoFrame,
    tools::{ToolExecutionContext, ToolExecutor},
};

const SYSTEM_PROMPT: &str = "你是 Ripple Live，一个运行在用户自有服务器上的中文多模态语音 Agent。
你可以理解当前语音转写和随附的视频画面。回答应自然、简洁、适合直接朗读。
需要外部动作或精确结果时必须使用提供的工具，不要假装已经调用。
只有用户明确要求联网、网上查询、最新信息或外部资料时才调用 web_search，并根据返回来源作答；“搜索一下”本身不表示可以联网。若用户说记忆、图库、保存的图片、历史照片等，即使带有“搜索”一词，也必须调用 recall，绝不能改为联网搜索。若工具返回 result_count 为 0，不得把自身已有知识说成搜索结果，必须明确说明本次搜索没有找到结果。
只有用户明确要求记住或记录某件事时才调用 remember；“记录一下”“保存一下”也属于明确记录请求。有当前画面时，visual_summary 必须客观描述画面中有助于以后检索的物品、位置和文字。需要查找用户过去保存的信息时调用 recall，并把 query 提炼为简洁关键词。本地记忆没有结果时，只能说明本地未找到并询问是否需要联网，不得自动调用 web_search。
用户明确要求把当前信息做成待办或提醒时调用 create_todo；它会同时保存画面证据。未指定时间就不要设置提醒；“明天”“下周”等相对时间先用 get_current_time 获取 Asia/Shanghai 当前时间，再换算成带时区的 RFC3339 due_at。用户只要求总结时，直接给出简短摘要和不超过三条重点；只有明确说要保存时才写入记忆。
工具返回 ok=false 时绝对不能声称操作成功，必须说明工具返回的错误；需要修正参数时，应再次调用工具，只有收到 ok=true 后才能确认已创建或已保存。不要在朗读内容中输出工具调用 JSON。";

fn system_prompt() -> String {
    let now = Utc::now().with_timezone(&Shanghai);
    format!(
        "{SYSTEM_PROMPT}\n当前 Asia/Shanghai 日期时间：{}。处理今天、明天、下班等时间表达时必须以此为准。",
        now.to_rfc3339_opts(SecondsFormat::Secs, true)
    )
}

const MIN_SPEECH_CHUNK_CHARS: usize = 40;
const SOFT_SPEECH_CHUNK_CHARS: usize = 60;
const MAX_SPEECH_CHUNK_CHARS: usize = 80;
const MAX_TTS_UTTERANCE_CHARS: usize = 320;

#[derive(Clone)]
pub struct AgentOrchestrator {
    settings: Arc<Settings>,
    context: ContextStore,
    context_compiler: ContextCompiler,
    memories: MemoryService,
    adapters: ModelAdapters,
    tools: ToolExecutor,
}

impl AgentOrchestrator {
    pub fn new(
        settings: Arc<Settings>,
        context: ContextStore,
        adapters: ModelAdapters,
        memories: MemoryService,
    ) -> anyhow::Result<Self> {
        let context_compiler = ContextCompiler::new(
            context.clone(),
            settings.max_recent_turns,
            settings.context_max_chars,
        );
        let tools = ToolExecutor::new(
            memories.clone(),
            &settings.skills_dir,
            settings.cli_max_output_bytes,
            &settings.search_proxy,
        )?;
        info!(external_tools = tools.external_tool_count(), skills_dir = %settings.skills_dir.display(), "skills loaded");
        Ok(Self {
            settings,
            tools,
            context,
            context_compiler,
            memories,
            adapters,
        })
    }

    pub fn external_tool_count(&self) -> usize {
        self.tools.external_tool_count()
    }

    pub async fn transcribe_candidate(&self, audio: &[f32]) -> anyhow::Result<String> {
        if audio.is_empty() {
            anyhow::bail!("没有可处理的音频");
        }
        let transcript = self.adapters.transcribe(audio).await?;
        let transcript = transcript.trim().to_owned();
        if transcript.is_empty() {
            anyhow::bail!("ASR 未识别出文本");
        }
        Ok(transcript)
    }

    pub async fn evaluate_turn_end(&self, audio: &[f32]) -> EndpointEvaluation {
        let transcript = match self.transcribe_candidate(audio).await {
            Ok(transcript) => transcript,
            Err(_) => {
                return EndpointEvaluation {
                    transcript: String::new(),
                    decision: EndpointDecision::Uncertain,
                    reason: "asr_error",
                    classifier_latency_ms: None,
                };
            }
        };

        if let Some(decision) = deterministic_decision(&transcript) {
            return EndpointEvaluation {
                transcript,
                decision,
                reason: "deterministic",
                classifier_latency_ms: None,
            };
        }

        let classifier_started = Instant::now();
        let classifier_result = self.adapters.classify_turn_end(&transcript).await;
        let classifier_latency_ms = Some(classifier_started.elapsed().as_millis());
        let (decision, reason) = match classifier_result {
            Err(_) => (EndpointDecision::Uncertain, "classifier_error"),
            Ok(reply) => match parse_classifier_decision(&reply) {
                Some((decision, confidence)) if confidence >= 0.75 => (decision, "classifier"),
                Some(_) => (EndpointDecision::Uncertain, "classifier_low_confidence"),
                None => (EndpointDecision::Uncertain, "classifier_malformed"),
            },
        };

        EndpointEvaluation {
            transcript,
            decision,
            reason,
            classifier_latency_ms,
        }
    }

    pub async fn run_text_response(
        &self,
        user_id: &str,
        conversation_id: &str,
        input: &str,
        response_id: &str,
    ) -> anyhow::Result<String> {
        let input = input.trim();
        if input.is_empty() {
            anyhow::bail!("input 不能为空");
        }
        let user_turn_id = self
            .context
            .add_turn(conversation_id, "user", input, None)
            .await?;
        let (routing_input, routing_turns) =
            self.context.trailing_user_input(conversation_id, 4).await?;
        let forced_route = self.tools.forced_route(&routing_input);
        self.context
            .record_event(
                conversation_id,
                "server.tool.routed",
                &json!({
                    "response_id": response_id,
                    "tool": forced_route.as_ref().map(|route| &route.name),
                    "reason": forced_route.as_ref().map(|route| route.reason).unwrap_or("model_auto"),
                    "routing_turns": routing_turns,
                    "routing_input_chars": routing_input.chars().count()
                }),
            )
            .await?;
        let compiled = self.context_compiler.compile(conversation_id).await?;
        let mut messages = vec![json!({"role": "system", "content": system_prompt()})];
        messages.extend(compiled.messages);
        let tools = self.tools.schemas();

        let mut response_artifacts = Vec::<MemoryArtifact>::new();
        for round in 0..self.settings.max_tool_rounds {
            let tool_choice = if round == 0 {
                forced_route
                    .as_ref()
                    .map(|route| route.name.clone())
                    .map(|name| json!({"type": "function", "function": {"name": name}}))
                    .unwrap_or_else(|| json!("auto"))
            } else {
                json!("auto")
            };
            let reply = self
                .adapters
                .complete(&messages, &tools, tool_choice)
                .await?;
            messages.push(reply.raw_message);
            if reply.tool_calls.is_empty() {
                let output = reply.content.trim().to_owned();
                if output.is_empty() {
                    anyhow::bail!("模型没有生成回复");
                }
                let assistant_turn_id = self
                    .context
                    .add_turn(conversation_id, "assistant", &output, None)
                    .await?;
                self.memories
                    .attach_to_turn(assistant_turn_id, &response_artifacts)
                    .await?;
                return Ok(output);
            }
            if !reply.content.trim().is_empty() {
                info!(
                    %conversation_id,
                    %response_id,
                    round,
                    text_chars = reply.content.chars().count(),
                    tool_calls = reply.tool_calls.len(),
                    "ignoring tool-call round text and continuing with tools"
                );
            }
            for call in reply.tool_calls {
                let outcome = self
                    .tools
                    .execute(
                        &ToolExecutionContext {
                            user_id: user_id.to_owned(),
                            conversation_id: conversation_id.to_owned(),
                            user_turn_id,
                            response_id: response_id.to_owned(),
                            tool_call_id: call.id.clone(),
                            transcript: input.to_owned(),
                            frames: Vec::new(),
                        },
                        &call.name,
                        &call.arguments,
                    )
                    .await;
                let result = outcome.value;
                response_artifacts.extend(outcome.artifacts);
                self.context
                    .record_event(
                        conversation_id,
                        "tool.result",
                        &json!({
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                            "result": result
                        }),
                    )
                    .await?;
                messages.push(tool_result_message(&call, &result));
            }
        }
        anyhow::bail!("工具调用轮次超过限制")
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn(
        &self,
        user_id: &str,
        session_id: &str,
        send: &mpsc::Sender<Value>,
        audio: Vec<f32>,
        frames: Vec<VideoFrame>,
        transcript_override: Option<String>,
        response_id: &str,
    ) -> anyhow::Result<()> {
        let turn_started = Instant::now();
        let input_kind = if transcript_override.is_some() {
            "text"
        } else {
            "audio"
        };
        info!(
            %session_id,
            %response_id,
            input = input_kind,
            audio_samples = audio.len(),
            frames = frames.len(),
            "turn started"
        );
        self.record_flow_event(
            session_id,
            "server.turn.started",
            json!({
                "response_id": response_id,
                "input": input_kind,
                "audio_samples": audio.len(),
                "frames": frames.len()
            }),
        )
        .await;
        emit_response(send, response_id, json!({"type": "response.created"})).await?;
        let transcription_started = Instant::now();
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
        let transcription_ms = transcription_started.elapsed().as_millis();
        info!(
            %session_id,
            %response_id,
            source = input_kind,
            text_chars = transcript.chars().count(),
            elapsed_ms = transcription_ms,
            "input transcript ready"
        );
        self.record_flow_event(
            session_id,
            "server.transcript.completed",
            json!({
                "response_id": response_id,
                "source": input_kind,
                "text_chars": transcript.chars().count(),
                "elapsed_ms": transcription_ms
            }),
        )
        .await;

        emit(
            send,
            json!({"type": "input.transcript.final", "text": transcript}),
        )
        .await?;
        let user_turn_id = self
            .context
            .add_turn(
                session_id,
                "user",
                &transcript,
                Some(&json!({"frames": frames.len()})),
            )
            .await?;
        let (routing_input, routing_turns) =
            self.context.trailing_user_input(session_id, 4).await?;
        let forced_route = self.tools.forced_route(&routing_input);
        self.record_flow_event(
            session_id,
            "server.tool.routed",
            json!({
                "response_id": response_id,
                "tool": forced_route.as_ref().map(|route| &route.name),
                "reason": forced_route.as_ref().map(|route| route.reason).unwrap_or("model_auto"),
                "routing_turns": routing_turns,
                "routing_input_chars": routing_input.chars().count()
            }),
        )
        .await;

        let mut compiled = self.context_compiler.compile(session_id).await?;
        compiled.messages.pop();
        let history_messages = compiled.history_messages.saturating_sub(1);
        info!(
            %session_id,
            %response_id,
            history_messages,
            memories = compiled.memories,
            estimated_chars = compiled.estimated_chars,
            "context loaded"
        );
        self.record_flow_event(
            session_id,
            "server.context.loaded",
            json!({
                "response_id": response_id,
                "history_messages": history_messages,
                "memories": compiled.memories,
                "estimated_chars": compiled.estimated_chars,
                "frames": frames.len()
            }),
        )
        .await;
        let mut messages = vec![json!({"role": "system", "content": system_prompt()})];
        messages.extend(compiled.messages);
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
            self.context.clone(),
            session_id.to_owned(),
        );
        let generation = async move {
            let available_tools = self.tools.schemas();
            let forced_tool = forced_route.map(|route| route.name);
            let mut final_text = String::new();
            let mut response_artifacts = Vec::<MemoryArtifact>::new();
            for round in 0..self.settings.max_tool_rounds {
                let tool_choice = if round == 0 {
                    forced_tool
                        .as_deref()
                        .map(|name| json!({"type": "function", "function": {"name": name}}))
                        .unwrap_or_else(|| json!("auto"))
                } else {
                    json!("auto")
                };
                let agent_started = Instant::now();
                info!(%session_id, %response_id, round, "agent generation started");
                self.record_flow_event(
                    session_id,
                    "server.agent.started",
                    json!({
                        "response_id": response_id,
                        "round": round,
                        "tool_choice": tool_choice
                    }),
                )
                .await;
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
                let agent_ms = agent_started.elapsed().as_millis();
                info!(
                    %session_id,
                    %response_id,
                    round,
                    elapsed_ms = agent_ms,
                    text_chars = content.chars().count(),
                    tool_calls = calls.len(),
                    "agent generation completed"
                );
                self.record_flow_event(
                    session_id,
                    "server.agent.completed",
                    json!({
                        "response_id": response_id,
                        "round": round,
                        "elapsed_ms": agent_ms,
                        "text_chars": content.chars().count(),
                        "tool_calls": calls.len()
                    }),
                )
                .await;
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
                    info!(
                        %session_id,
                        %response_id,
                        round,
                        text_chars = content.chars().count(),
                        tool_calls = calls.len(),
                        "excluding tool-call round text from speech and continuing with tools"
                    );
                }
                for call in calls {
                    info!(
                        %session_id,
                        %response_id,
                        call_id = %call.id,
                        tool = %call.name,
                        "tool execution started"
                    );
                    self.record_flow_event(
                        session_id,
                        "server.tool.started",
                        json!({
                            "response_id": response_id,
                            "call_id": call.id,
                            "name": call.name
                        }),
                    )
                    .await;
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
                    let outcome = self
                        .tools
                        .execute(
                            &ToolExecutionContext {
                                user_id: user_id.to_owned(),
                                conversation_id: session_id.to_owned(),
                                user_turn_id,
                                response_id: response_id.to_owned(),
                                tool_call_id: call.id.clone(),
                                transcript: transcript.clone(),
                                frames: frames.clone(),
                            },
                            &call.name,
                            &call.arguments,
                        )
                        .await;
                    let result = outcome.value;
                    for artifact in outcome.artifacts {
                        emit_response(
                            send,
                            response_id,
                            json!({
                                "type": "ripple.response.artifact.added",
                                "artifact": artifact
                            }),
                        )
                        .await?;
                        response_artifacts.push(artifact);
                    }
                    self.context
                        .record_event(
                            session_id,
                            "tool.result",
                            &json!({
                                "response_id": response_id,
                                "call_id": call.id,
                                "name": call.name,
                                "arguments": call.arguments,
                                "result": result
                            }),
                        )
                        .await?;
                    info!(
                        %session_id,
                        %response_id,
                        call_id = %call.id,
                        tool = %call.name,
                        "tool execution completed"
                    );
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
            anyhow::Ok((final_text, response_artifacts))
        };
        let (generation_result, speech_result) = tokio::join!(generation, speech);
        let (final_text, response_artifacts) = generation_result?;
        speech_result?;
        let assistant_turn_id = self
            .context
            .add_turn(session_id, "assistant", &final_text, None)
            .await?;
        self.memories
            .attach_to_turn(assistant_turn_id, &response_artifacts)
            .await?;
        let total_ms = turn_started.elapsed().as_millis();
        info!(
            %session_id,
            %response_id,
            elapsed_ms = total_ms,
            text_chars = final_text.chars().count(),
            "turn completed"
        );
        self.record_flow_event(
            session_id,
            "server.turn.completed",
            json!({
                "response_id": response_id,
                "elapsed_ms": total_ms,
                "text_chars": final_text.chars().count()
            }),
        )
        .await;
        emit_response(
            send,
            response_id,
            json!({"type": "response.done", "text": final_text}),
        )
        .await
    }

    async fn record_flow_event(&self, session_id: &str, kind: &str, payload: Value) {
        if let Err(error) = self.context.record_event(session_id, kind, &payload).await {
            warn!(%session_id, %kind, %error, "failed to record turn event");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_speech(
    adapters: ModelAdapters,
    send: mpsc::Sender<Value>,
    response_id: String,
    mut sentences: mpsc::Receiver<String>,
    audio_chunk_size: usize,
    sample_rate: u32,
    context: ContextStore,
    session_id: String,
) -> anyhow::Result<()> {
    let mut segment_index = 0usize;
    while let Some(sentence) = sentences.recv().await {
        let segment_started = Instant::now();
        info!(
            %session_id,
            %response_id,
            segment_index,
            text_chars = sentence.chars().count(),
            "tts segment started"
        );
        record_flow_event(
            &context,
            &session_id,
            "server.tts.started",
            json!({
                "response_id": response_id,
                "segment_index": segment_index,
                "text_chars": sentence.chars().count()
            }),
        )
        .await;
        let mut stream = adapters.synthesize_stream(&sentence).await?;
        let mut buffered = Vec::new();
        let mut audio_samples = 0usize;
        while let Some(output) = stream.next().await {
            let output = output?;
            audio_samples += output.len();
            buffered.extend(output);
            while buffered.len() >= audio_chunk_size {
                let remainder = buffered.split_off(audio_chunk_size);
                let chunk = std::mem::replace(&mut buffered, remainder);
                emit_audio(&send, &response_id, &chunk, sample_rate).await?;
            }
        }
        if !buffered.is_empty() {
            emit_audio(&send, &response_id, &buffered, sample_rate).await?;
        }
        let elapsed_ms = segment_started.elapsed().as_millis();
        info!(
            %session_id,
            %response_id,
            segment_index,
            elapsed_ms,
            audio_samples,
            "tts segment completed"
        );
        record_flow_event(
            &context,
            &session_id,
            "server.tts.completed",
            json!({
                "response_id": response_id,
                "segment_index": segment_index,
                "elapsed_ms": elapsed_ms,
                "audio_samples": audio_samples
            }),
        )
        .await;
        segment_index += 1;
    }
    Ok(())
}

async fn record_flow_event(context: &ContextStore, session_id: &str, kind: &str, payload: Value) {
    if let Err(error) = context.record_event(session_id, kind, &payload).await {
        warn!(%session_id, %kind, %error, "failed to record speech event");
    }
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
    ready: Vec<String>,
}

impl SpeechSegmenter {
    fn new() -> Self {
        Self {
            pending: String::new(),
            ready: Vec::new(),
        }
    }

    fn push(&mut self, delta: &str) -> Vec<String> {
        for character in delta.chars() {
            self.pending.push(character);
            let length = self.pending.chars().count();
            let sentence_end = length >= MIN_SPEECH_CHUNK_CHARS
                && matches!(character, '。' | '！' | '？' | '!' | '?' | '；' | ';');
            let clause_end =
                length >= SOFT_SPEECH_CHUNK_CHARS && matches!(character, '，' | ',' | '：' | ':');
            if (sentence_end || clause_end || length >= MAX_SPEECH_CHUNK_CHARS)
                && let Some(phrase) = self.take_pending()
            {
                self.ready.push(phrase);
            }
        }
        Vec::new()
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(phrase) = self.take_pending() {
            self.ready.push(phrase);
        }
        coalesce_speech_segments(self.ready, MAX_TTS_UTTERANCE_CHARS)
    }

    fn take_pending(&mut self) -> Option<String> {
        let phrase = self.pending.trim().to_owned();
        self.pending.clear();
        (!phrase.is_empty()).then_some(phrase)
    }
}

fn coalesce_speech_segments(segments: Vec<String>, max_chars: usize) -> Vec<String> {
    let mut utterances = Vec::new();
    let mut current = String::new();

    for segment in segments {
        if !current.is_empty() && current.chars().count() + segment.chars().count() > max_chars {
            utterances.push(current);
            current = String::new();
        }
        current.push_str(&segment);
    }
    if !current.is_empty() {
        utterances.push(current);
    }
    utterances
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
    use crate::endpointing::EndpointDecision;

    async fn orchestrator_with_failing_asr() -> AgentOrchestrator {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.asr_backend = "openai".to_owned();
        settings.asr_url = "http://127.0.0.1:1/v1/audio/transcriptions".to_owned();
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open(&settings.database_path()).await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();

        AgentOrchestrator::new(
            Arc::clone(&settings),
            context,
            ModelAdapters::new((*settings).clone()).unwrap(),
            memories,
        )
        .unwrap()
    }

    async fn orchestrator_with_mock_models() -> AgentOrchestrator {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.asr_backend = "mock".to_owned();
        settings.agent_backend = "mock".to_owned();
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open(&settings.database_path()).await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();

        AgentOrchestrator::new(
            Arc::clone(&settings),
            context,
            ModelAdapters::new((*settings).clone()).unwrap(),
            memories,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn endpoint_evaluation_marks_asr_failure_uncertain() {
        let evaluation = orchestrator_with_failing_asr()
            .await
            .evaluate_turn_end(&[0.1; 1600])
            .await;

        assert_eq!(evaluation.decision, EndpointDecision::Uncertain);
        assert_eq!(evaluation.reason, "asr_error");
        assert!(evaluation.transcript.is_empty());
        assert_eq!(evaluation.classifier_latency_ms, None);
    }

    #[tokio::test]
    async fn endpoint_evaluation_maps_malformed_classifier_output_to_uncertain() {
        let evaluation = orchestrator_with_mock_models()
            .await
            .evaluate_turn_end(&[0.1; 1600])
            .await;

        assert_eq!(evaluation.decision, EndpointDecision::Uncertain);
        assert_eq!(evaluation.reason, "classifier_malformed");
        assert!(!evaluation.transcript.is_empty());
        assert!(evaluation.classifier_latency_ms.is_some());
    }

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
        assert!(segmenter.push("这项优化可以明显降低延迟，").is_empty());
        assert!(segmenter.push("并保持自然。").is_empty());
        assert_eq!(
            segmenter.finish(),
            vec!["先说结论，这项优化可以明显降低延迟，并保持自然。"]
        );
    }

    #[test]
    fn segments_long_streaming_text_between_forty_and_eighty_characters() {
        let mut at_sentence = SpeechSegmenter::new();
        let sentence = format!("{}。", "连".repeat(MIN_SPEECH_CHUNK_CHARS - 1));
        assert!(at_sentence.push(&sentence).is_empty());
        assert_eq!(at_sentence.finish(), vec![sentence]);

        let mut at_clause = SpeechSegmenter::new();
        let clause = format!("{}，", "连".repeat(SOFT_SPEECH_CHUNK_CHARS - 1));
        assert!(at_clause.push(&clause).is_empty());
        assert_eq!(at_clause.finish(), vec![clause]);

        let mut at_limit = SpeechSegmenter::new();
        let bounded = "连".repeat(MAX_SPEECH_CHUNK_CHARS);
        assert!(at_limit.push(&bounded).is_empty());
        assert_eq!(at_limit.finish(), vec![bounded]);
    }

    #[test]
    fn buffers_streaming_speech_until_the_round_outcome_is_known() {
        let mut segmenter = SpeechSegmenter::new();
        let sentence = format!("{}。", "工".repeat(MIN_SPEECH_CHUNK_CHARS - 1));

        assert!(segmenter.push(&sentence).is_empty());
        assert_eq!(segmenter.finish(), vec![sentence]);
    }

    #[test]
    fn combines_adjacent_sentences_into_one_tts_utterance() {
        let mut segmenter = SpeechSegmenter::new();
        let first = format!("{}。", "第".repeat(40));
        let second = format!("{}。", "二".repeat(40));

        assert!(segmenter.push(&first).is_empty());
        assert!(segmenter.push(&second).is_empty());
        assert_eq!(segmenter.finish(), vec![format!("{first}{second}")]);
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
