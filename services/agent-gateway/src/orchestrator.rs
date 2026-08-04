use std::{collections::BTreeMap, sync::Arc, time::Instant};

use anyhow::Context as _;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use chrono_tz::Asia::Shanghai;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    adapters::{
        FunctionCall, ModelAdapters, build_responses_user_input, function_call_output,
        parse_responses_output, reject_legacy_tool_markup, responses_tool_schema,
    },
    audio::encode_le_f32,
    config::Settings,
    context::ContextStore,
    context_compiler::ContextCompiler,
    endpointing::{
        EndpointDecision, EndpointEvaluation, deterministic_decision, parse_classifier_decision,
    },
    memory::{MemoryArtifact, MemoryService},
    protocol::VideoFrame,
    response_gate::{
        GATE_INSTRUCTIONS, GateOutcome, build_gate_input, gate_tool_schema, parse_gate_response,
    },
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

fn forced_tool_instructions(name: &str) -> String {
    format!(
        "{}\nYou MUST use the {name} tool in this turn. Do not answer before the tool result is available.",
        system_prompt()
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
        let classifier = self.adapters.classify_turn_end(&transcript).await;
        let classifier_latency_ms = Some(classifier_started.elapsed().as_millis());
        let (decision, reason) = match classifier {
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

    pub async fn gate_transcript(&self, session_id: &str, transcript: &str) -> GateOutcome {
        let started = Instant::now();
        let history = match self.context.recent_messages(session_id, 2).await {
            Ok(history) => history,
            Err(error) => {
                warn!(%session_id, %error, "Gate context loading failed; failing open");
                return GateOutcome::fallback("context_error", started.elapsed().as_millis());
            }
        };
        let assistant_just_replied = history
            .last()
            .and_then(|item| item.get("role"))
            .and_then(Value::as_str)
            == Some("assistant");
        let input = build_gate_input(&history, transcript, assistant_just_replied);
        let tools = [gate_tool_schema()];
        let request = self
            .adapters
            .respond(&input, &tools, json!("auto"), GATE_INSTRUCTIONS);
        match tokio::time::timeout(self.settings.gate_timeout, request).await {
            Ok(Ok(output)) => parse_gate_response(&output, started.elapsed().as_millis())
                .unwrap_or_else(|error| {
                    warn!(%session_id, %error, "Gate output invalid; failing open");
                    GateOutcome::fallback("invalid_output", started.elapsed().as_millis())
                }),
            Ok(Err(error)) => {
                warn!(%session_id, %error, "Gate model call failed; failing open");
                GateOutcome::fallback("model_error", started.elapsed().as_millis())
            }
            Err(_) => {
                warn!(%session_id, "Gate model call timed out; failing open");
                GateOutcome::fallback("timeout", started.elapsed().as_millis())
            }
        }
    }
    pub async fn run_text_response(
        &self,
        user_id: &str,
        conversation_id: &str,
        input: &str,
        response_id: &str,
    ) -> anyhow::Result<String> {
        let user_input = input.trim();
        if user_input.is_empty() {
            anyhow::bail!("input 不能为空");
        }
        let user_turn_id = self
            .context
            .add_turn(conversation_id, "user", user_input, None)
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
        let mut response_input = compiled.messages;
        let tools = self
            .tools
            .schemas()
            .iter()
            .map(responses_tool_schema)
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mut response_artifacts = Vec::<MemoryArtifact>::new();
        for round in 0..self.settings.max_tool_rounds {
            let forced_name = (round == 0)
                .then(|| forced_route.as_ref().map(|route| route.name.as_str()))
                .flatten();
            let round_tools = if let Some(name) = forced_name {
                tools
                    .iter()
                    .filter(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                tools.clone()
            };
            let instructions = forced_name.map_or_else(system_prompt, forced_tool_instructions);
            let reply = self
                .adapters
                .respond(&response_input, &round_tools, json!("auto"), &instructions)
                .await?;
            response_input.extend(reply.output_items.clone());
            if reply.function_calls.is_empty() {
                let output = reply.text.trim().to_owned();
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
            if !reply.text.trim().is_empty() {
                info!(
                    %conversation_id,
                    %response_id,
                    round,
                    text_chars = reply.text.chars().count(),
                    tool_calls = reply.function_calls.len(),
                    "ignoring tool-call round text and continuing with tools"
                );
            }
            for call in reply.function_calls {
                let outcome = self
                    .tools
                    .execute(
                        &ToolExecutionContext {
                            user_id: user_id.to_owned(),
                            conversation_id: conversation_id.to_owned(),
                            user_turn_id,
                            response_id: response_id.to_owned(),
                            tool_call_id: call.call_id.clone(),
                            transcript: user_input.to_owned(),
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
                            "call_id": call.call_id,
                            "name": call.name,
                            "arguments": call.arguments,
                            "result": result
                        }),
                    )
                    .await?;
                response_input.push(function_call_output(&call.call_id, &result));
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
            None => self
                .adapters
                .transcribe(&audio)
                .await
                .context("ASR_FAILED")?,
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
        let mut input = compiled.messages;
        input.push(build_responses_user_input(&transcript, &frames));

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
            let available_tools = self
                .tools
                .schemas()
                .iter()
                .map(responses_tool_schema)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let forced_tool = forced_route.map(|route| route.name);
            let mut final_text = String::new();
            let mut response_artifacts = Vec::<MemoryArtifact>::new();
            let mut agent_first_delta_recorded = false;
            for round in 0..self.settings.max_tool_rounds {
                let forced_name = (round == 0).then_some(forced_tool.as_deref()).flatten();
                let round_tools = if let Some(name) = forced_name {
                    available_tools
                        .iter()
                        .filter(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    available_tools.clone()
                };
                let instructions = forced_name.map_or_else(system_prompt, forced_tool_instructions);
                let agent_started = Instant::now();
                info!(%session_id, %response_id, round, "agent generation started");
                self.record_flow_event(
                    session_id,
                    "server.agent.started",
                    json!({
                        "response_id": response_id,
                        "round": round,
                        "tool_choice": "auto",
                        "forced_tool": forced_name
                    }),
                )
                .await;
                let mut stream = self
                    .adapters
                    .respond_stream(&input, &round_tools, json!("auto"), &instructions)
                    .await
                    .context("AGENT_FAILED")?;
                let mut content = String::new();
                let mut tool_calls = BTreeMap::<usize, FunctionCall>::new();
                let mut completed_items = BTreeMap::<usize, Value>::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.context("AGENT_FAILED")?;
                    if !agent_first_delta_recorded && let Some(kind) = useful_delta_kind(&chunk) {
                        agent_first_delta_recorded = true;
                        self.record_flow_event(
                            session_id,
                            "server.agent.first_delta",
                            json!({
                                "response_id": response_id,
                                "round": round,
                                "kind": kind,
                                "elapsed_ms": agent_started.elapsed().as_millis()
                            }),
                        )
                        .await;
                    }
                    if let Some(delta) = responses_text_delta(&chunk) {
                        content.push_str(delta);
                    }
                    merge_responses_function_event(&mut tool_calls, &chunk)?;
                    merge_completed_response(&mut tool_calls, &mut completed_items, &chunk)?;
                    if chunk.get("type").and_then(Value::as_str)
                        == Some("response.output_item.done")
                        && let Some(index) = chunk.get("output_index").and_then(Value::as_u64)
                        && let Some(item) = chunk.get("item")
                    {
                        completed_items.insert(index as usize, item.clone());
                    }
                    match chunk.get("type").and_then(Value::as_str) {
                        Some("response.failed") | Some("response.incomplete") => {
                            anyhow::bail!("AGENT_FAILED: Responses upstream did not complete")
                        }
                        _ => {}
                    }
                }
                let calls = tool_calls.into_values().collect::<Vec<_>>();
                reject_legacy_tool_markup(&content).context("AGENT_INVALID_TOOL_OUTPUT")?;
                for call in &calls {
                    serde_json::from_str::<Value>(&call.arguments)
                        .context("AGENT_FAILED: function_call arguments were not valid JSON")?;
                }
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
                input.extend(completed_items.into_values());
                if calls.is_empty() {
                    let mut segmenter = SpeechSegmenter::new();
                    let mut phrases = segmenter.push(&content);
                    phrases.extend(segmenter.finish());
                    for phrase in phrases {
                        emit_response(
                            send,
                            response_id,
                            json!({"type": "response.text.delta", "delta": &phrase}),
                        )
                        .await?;
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
                        call_id = %call.call_id,
                        tool = %call.name,
                        "tool execution started"
                    );
                    self.record_flow_event(
                        session_id,
                        "server.tool.started",
                        json!({
                            "response_id": response_id,
                            "call_id": call.call_id,
                            "name": call.name
                        }),
                    )
                    .await;
                    emit_response(
                        send,
                        response_id,
                        json!({
                            "type": "response.tool.started",
                            "call_id": call.call_id,
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
                                tool_call_id: call.call_id.clone(),
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
                                "call_id": call.call_id,
                                "name": call.name,
                                "arguments": call.arguments,
                                "result": result
                            }),
                        )
                        .await?;
                    info!(
                        %session_id,
                        %response_id,
                        call_id = %call.call_id,
                        tool = %call.name,
                        "tool execution completed"
                    );
                    emit_response(
                        send,
                        response_id,
                        json!({
                            "type": "response.tool.completed",
                            "call_id": call.call_id,
                            "name": call.name,
                            "result": result
                        }),
                    )
                    .await?;
                    input.push(function_call_output(&call.call_id, &result));
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
        speech_result.context("TTS_FAILED")?;
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
        let mut first_audio_recorded = false;
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
        let mut stream = adapters
            .synthesize_stream(&sentence)
            .await
            .context("TTS_FAILED")?;
        let mut buffered = Vec::new();
        let mut audio_samples = 0usize;
        while let Some(output) = stream.next().await {
            let output = output.context("TTS_FAILED")?;
            if !first_audio_recorded && !output.is_empty() {
                first_audio_recorded = true;
                record_flow_event(
                    &context,
                    &session_id,
                    "server.tts.first_audio",
                    json!({
                        "response_id": response_id,
                        "segment_index": segment_index,
                        "elapsed_ms": segment_started.elapsed().as_millis(),
                        "audio_samples": output.len()
                    }),
                )
                .await;
            }
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

fn useful_delta_kind(chunk: &Value) -> Option<&'static str> {
    if responses_text_delta(chunk).is_some() {
        return Some("text");
    }
    if chunk.get("type").and_then(Value::as_str) == Some("response.output_item.added")
        && chunk.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
    {
        return Some("tool_call");
    }
    None
}

fn responses_text_delta(event: &Value) -> Option<&str> {
    (event.get("type").and_then(Value::as_str) == Some("response.output_text.delta"))
        .then(|| event.get("delta").and_then(Value::as_str))
        .flatten()
        .filter(|delta| !delta.is_empty())
}

fn merge_responses_function_event(
    output: &mut BTreeMap<usize, FunctionCall>,
    event: &Value,
) -> anyhow::Result<()> {
    if let Some(delta) = responses_text_delta(event) {
        reject_legacy_tool_markup(delta)?;
    }
    let index = event
        .get("output_index")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    match event.get("type").and_then(Value::as_str) {
        Some("response.output_item.added")
            if event.pointer("/item/type").and_then(Value::as_str) == Some("function_call") =>
        {
            let index = index.context("function_call item did not include output_index")?;
            output.insert(
                index,
                FunctionCall {
                    call_id: event
                        .pointer("/item/call_id")
                        .and_then(Value::as_str)
                        .context("function_call item did not include call_id")?
                        .to_owned(),
                    name: event
                        .pointer("/item/name")
                        .and_then(Value::as_str)
                        .context("function_call item did not include name")?
                        .to_owned(),
                    arguments: event
                        .pointer("/item/arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
            );
        }
        Some("response.function_call_arguments.delta") => {
            let index = index.context("function_call delta did not include output_index")?;
            let call = output
                .get_mut(&index)
                .context("function_call delta arrived before item")?;
            call.arguments.push_str(
                event
                    .get("delta")
                    .and_then(Value::as_str)
                    .context("function_call delta did not include delta")?,
            );
        }
        _ => {}
    }
    Ok(())
}

fn merge_completed_response(
    calls: &mut BTreeMap<usize, FunctionCall>,
    completed_items: &mut BTreeMap<usize, Value>,
    event: &Value,
) -> anyhow::Result<()> {
    if event.get("type").and_then(Value::as_str) != Some("response.completed") {
        return Ok(());
    }
    let Some(response) = event.get("response") else {
        return Ok(());
    };
    if !response.get("output").is_some_and(Value::is_array) {
        return Ok(());
    }
    let parsed = parse_responses_output(response)?;
    calls.clear();
    calls.extend(parsed.function_calls.into_iter().enumerate());
    completed_items.clear();
    completed_items.extend(parsed.output_items.into_iter().enumerate());
    Ok(())
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
    use crate::{endpointing::EndpointDecision, response_gate::GateDecision};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn endpointing_orchestrator(
        transcript: Result<&str, &str>,
        classifier: Result<&str, &str>,
    ) -> (tempfile::TempDir, AgentOrchestrator, Arc<AtomicUsize>) {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open(&settings.database_path()).await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();
        let (adapters, classifier_calls) = ModelAdapters::with_endpointing_test_results(
            (*settings).clone(),
            transcript.map(str::to_owned).map_err(str::to_owned),
            classifier.map(str::to_owned).map_err(str::to_owned),
        )
        .unwrap();
        let orchestrator =
            AgentOrchestrator::new(Arc::clone(&settings), context, adapters, memories).unwrap();
        (directory, orchestrator, classifier_calls)
    }

    #[tokio::test]
    async fn endpoint_evaluation_accepts_complete_at_exact_confidence_boundary() {
        let (_directory, orchestrator, calls) = endpointing_orchestrator(
            Ok("今天天气"),
            Ok(r#"{"decision":"complete","confidence":0.75}"#),
        )
        .await;

        let evaluation = orchestrator.evaluate_turn_end(&[0.1; 1600]).await;

        assert_eq!(evaluation.decision, EndpointDecision::Complete);
        assert_eq!(evaluation.reason, "classifier");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(evaluation.classifier_latency_ms.is_some());
    }

    #[tokio::test]
    async fn endpoint_evaluation_accepts_confident_continue() {
        let (_directory, orchestrator, _) = endpointing_orchestrator(
            Ok("今天天气"),
            Ok(r#"{"decision":"continue","confidence":0.9}"#),
        )
        .await;

        let evaluation = orchestrator.evaluate_turn_end(&[0.1; 1600]).await;

        assert_eq!(evaluation.decision, EndpointDecision::Continue);
        assert_eq!(evaluation.reason, "classifier");
    }

    #[tokio::test]
    async fn endpoint_evaluation_rejects_below_threshold_classifier_result() {
        let (_directory, orchestrator, _) = endpointing_orchestrator(
            Ok("今天天气"),
            Ok(r#"{"decision":"complete","confidence":0.7499999999999999}"#),
        )
        .await;

        let evaluation = orchestrator.evaluate_turn_end(&[0.1; 1600]).await;

        assert_eq!(evaluation.decision, EndpointDecision::Uncertain);
        assert_eq!(evaluation.reason, "classifier_low_confidence");
    }

    #[tokio::test]
    async fn endpoint_evaluation_maps_malformed_and_transport_failures_to_uncertain() {
        let (_directory, malformed, _) =
            endpointing_orchestrator(Ok("今天天气"), Ok("not json")).await;
        let malformed = malformed.evaluate_turn_end(&[0.1; 1600]).await;
        assert_eq!(malformed.decision, EndpointDecision::Uncertain);
        assert_eq!(malformed.reason, "classifier_malformed");
        assert!(!malformed.transcript.is_empty());

        let (_directory, transport, _) =
            endpointing_orchestrator(Ok("今天天气"), Err("transport")).await;
        let transport = transport.evaluate_turn_end(&[0.1; 1600]).await;
        assert_eq!(transport.decision, EndpointDecision::Uncertain);
        assert_eq!(transport.reason, "classifier_error");
        assert!(!transport.transcript.is_empty());
    }

    #[tokio::test]
    async fn endpoint_evaluation_bypasses_classifier_for_deterministic_decision() {
        let (_directory, orchestrator, calls) =
            endpointing_orchestrator(Ok("因为"), Err("must not run")).await;

        let evaluation = orchestrator.evaluate_turn_end(&[0.1; 1600]).await;

        assert_eq!(evaluation.decision, EndpointDecision::Continue);
        assert_eq!(evaluation.reason, "deterministic");
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(evaluation.classifier_latency_ms, None);
    }

    #[tokio::test]
    async fn endpoint_evaluation_maps_asr_failure_to_uncertain() {
        let (_directory, orchestrator, calls) =
            endpointing_orchestrator(Err("asr"), Err("must not run")).await;

        let evaluation = orchestrator.evaluate_turn_end(&[0.1; 1600]).await;

        assert_eq!(evaluation.decision, EndpointDecision::Uncertain);
        assert_eq!(evaluation.reason, "asr_error");
        assert!(evaluation.transcript.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(evaluation.classifier_latency_ms, None);
    }

    #[tokio::test]
    async fn gate_model_failure_fails_open() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.agent_backend = "mock".to_owned();
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open(&settings.database_path()).await.unwrap();
        context.touch_session("gate-session").await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();
        let orchestrator = AgentOrchestrator::new(
            Arc::clone(&settings),
            context,
            ModelAdapters::new((*settings).clone()).unwrap(),
            memories,
        )
        .unwrap();

        let outcome = orchestrator.gate_transcript("gate-session", "你好").await;

        assert_eq!(outcome.decision, GateDecision::Respond);
        assert!(outcome.fallback);
    }

    #[test]
    fn reads_responses_text_delta() {
        assert_eq!(
            responses_text_delta(&json!({
                "type":"response.output_text.delta",
                "delta":"你好"
            })),
            Some("你好")
        );
        assert_eq!(
            responses_text_delta(&json!({
                "type":"response.output_item.added",
                "item":{"type":"function_call"}
            })),
            None
        );
    }

    #[test]
    fn forced_tool_instruction_avoids_protocol_markup() {
        let instructions = forced_tool_instructions("calculate");
        assert!(instructions.contains("You MUST use the calculate tool"));
        assert!(instructions.contains("Do not answer before the tool result is available"));
        assert!(!instructions.contains("function_call"));
    }

    #[test]
    fn merges_only_structured_responses_function_events() {
        let mut calls = BTreeMap::new();
        merge_responses_function_event(
            &mut calls,
            &json!({
                "type":"response.output_item.added",
                "output_index":0,
                "item":{
                    "type":"function_call",
                    "call_id":"call_1",
                    "name":"calculate",
                    "arguments":""
                }
            }),
        )
        .unwrap();
        merge_responses_function_event(
            &mut calls,
            &json!({
                "type":"response.function_call_arguments.delta",
                "output_index":0,
                "delta":"{\"expression\":\"7*8\"}"
            }),
        )
        .unwrap();
        let error = merge_responses_function_event(
            &mut calls,
            &json!({
                "type":"response.output_text.delta",
                "delta":"<tool_call>{\"name\":\"remember\"}</tool_call>"
            }),
        )
        .unwrap_err();

        assert!(error.to_string().contains("legacy tool tag"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[&0].call_id, "call_1");
        assert_eq!(calls[&0].arguments, "{\"expression\":\"7*8\"}");
    }

    #[test]
    fn uses_canonical_function_calls_from_completed_response() {
        let mut calls = BTreeMap::new();
        let mut items = BTreeMap::new();
        merge_completed_response(
            &mut calls,
            &mut items,
            &json!({
                "type":"response.completed",
                "response":{
                    "output":[{
                        "type":"function_call",
                        "call_id":"call_final",
                        "name":"calculate",
                        "arguments":"{\"expression\":\"7 * 8\"}"
                    }]
                }
            }),
        )
        .unwrap();

        assert_eq!(calls[&0].call_id, "call_final");
        assert_eq!(items[&0]["type"], "function_call");
    }

    #[test]
    fn first_useful_delta_distinguishes_text_and_tool_calls() {
        assert_eq!(
            useful_delta_kind(&json!({"type":"response.output_text.delta","delta":"你"})),
            Some("text")
        );
        assert_eq!(
            useful_delta_kind(&json!({
                "type":"response.output_item.added",
                "item":{"type":"function_call"}
            })),
            Some("tool_call")
        );
        assert_eq!(
            useful_delta_kind(&json!({"type":"response.in_progress"})),
            None
        );
    }

    #[tokio::test]
    async fn records_first_result_milestones_once_for_a_mock_turn() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.asr_backend = "mock".to_owned();
        settings.agent_backend = "mock".to_owned();
        settings.tts_backend = "mock".to_owned();
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open(&settings.database_path()).await.unwrap();
        context.touch_session("metrics-session").await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();
        let orchestrator = AgentOrchestrator::new(
            Arc::clone(&settings),
            context.clone(),
            ModelAdapters::new((*settings).clone()).unwrap(),
            memories,
        )
        .unwrap();
        let (sender, mut receiver) = mpsc::channel(128);
        let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });

        orchestrator
            .run_turn(
                "metrics-user",
                "metrics-session",
                &sender,
                Vec::new(),
                Vec::new(),
                Some("你好".to_owned()),
                "metrics-response",
            )
            .await
            .unwrap();
        drop(sender);
        drain.await.unwrap();

        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT kind, COUNT(*) FROM events
             WHERE json_extract(payload, '$.response_id') = ?
               AND kind IN ('server.agent.first_delta', 'server.tts.first_audio')
             GROUP BY kind ORDER BY kind",
        )
        .bind("metrics-response")
        .fetch_all(context.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("server.agent.first_delta".to_owned(), 1),
                ("server.tts.first_audio".to_owned(), 1),
            ]
        );
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
}
