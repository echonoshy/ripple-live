use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, Stream, StreamExt};
use ripple_agent_gateway::{
    activation::{ActivationMode, evaluate as evaluate_activation},
    adapters::ModelAdapters,
    audio::decode_le_f32,
    config::Settings,
    context::{ContextStore, LibraryAction, LibraryScope},
    endpointing::{EndpointEvaluation, is_stop_command},
    memory::{MemoryService, TodoUpdate},
    orchestrator::AgentOrchestrator,
    protocol::{ClientEvent, SessionMode, VideoFrame},
    response_gate::{GateDecision, GateOutcome},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle};
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{Level, error, info, warn};
use uuid::Uuid;

const REALTIME_PROTOCOL_MIN: u32 = 4;
const REALTIME_PROTOCOL_VERSION: u32 = 5;

#[derive(Clone)]
struct AppState {
    settings: Arc<Settings>,
    context: ContextStore,
    memories: MemoryService,
    orchestrator: AgentOrchestrator,
}

struct ActiveResponse {
    id: String,
    handle: JoinHandle<()>,
}

struct PendingTurn {
    response_id: String,
    transcript: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ModeChangePlan {
    Unchanged,
    ChangeOnly,
    ReleasePendingAudioTurn,
}

fn plan_mode_change(
    current: SessionMode,
    requested: SessionMode,
    pending: Option<&PendingTurn>,
) -> ModeChangePlan {
    if current == requested {
        return ModeChangePlan::Unchanged;
    }
    if current == SessionMode::Video && requested == SessionMode::Audio && pending.is_some() {
        return ModeChangePlan::ReleasePendingAudioTurn;
    }
    ModeChangePlan::ChangeOnly
}

fn mode_snapshot_after_change(
    snapshot: SessionMode,
    current: SessionMode,
    requested: SessionMode,
) -> SessionMode {
    if current != requested && requested == SessionMode::Audio {
        SessionMode::Audio
    } else {
        snapshot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndpointPhase {
    Speaking,
    Paused,
}

#[derive(Debug, Default)]
struct EndpointState {
    active_turn_id: Option<String>,
    phase: Option<EndpointPhase>,
    audio: Vec<f32>,
    evaluation: Option<EndpointEvaluation>,
}

impl EndpointState {
    #[cfg(test)]
    fn speaking(turn_id: &str) -> Self {
        Self {
            active_turn_id: Some(turn_id.to_owned()),
            phase: Some(EndpointPhase::Speaking),
            audio: Vec::new(),
            evaluation: None,
        }
    }

    fn start(&mut self, turn_id: &str, pre_roll_limit: usize) {
        keep_pre_roll(&mut self.audio, pre_roll_limit);
        self.active_turn_id = Some(turn_id.to_owned());
        self.phase = Some(EndpointPhase::Speaking);
        self.evaluation = None;
    }

    fn append_audio(&mut self, encoded: Option<&str>, settings: &Settings) -> anyhow::Result<()> {
        append_audio(
            &mut self.audio,
            encoded,
            self.active_turn_id.is_some(),
            settings,
        )
    }

    fn pause(&mut self, turn_id: &str) -> bool {
        if self.active_turn_id.as_deref() != Some(turn_id) {
            return false;
        }
        self.phase = Some(EndpointPhase::Paused);
        self.evaluation = None;
        true
    }

    fn resume(&mut self, turn_id: &str) -> bool {
        if self.active_turn_id.as_deref() != Some(turn_id) {
            return false;
        }
        self.phase = Some(EndpointPhase::Speaking);
        self.evaluation = None;
        true
    }

    fn accepts_result(&self, turn_id: &str) -> bool {
        self.active_turn_id.as_deref() == Some(turn_id) && self.phase == Some(EndpointPhase::Paused)
    }

    fn store_evaluation(&mut self, turn_id: &str, evaluation: EndpointEvaluation) -> bool {
        if !self.accepts_result(turn_id) {
            return false;
        }
        self.evaluation = Some(evaluation);
        true
    }

    fn take_commit(&mut self, turn_id: &str) -> Option<(Vec<f32>, Option<EndpointEvaluation>)> {
        if self.active_turn_id.as_deref() != Some(turn_id) {
            return None;
        }
        self.active_turn_id = None;
        self.phase = None;
        Some((std::mem::take(&mut self.audio), self.evaluation.take()))
    }

    fn consume_stop(&mut self, turn_id: &str, transcript: &str) -> bool {
        if self.active_turn_id.as_deref() != Some(turn_id) || !is_stop_command(transcript) {
            return false;
        }
        self.clear();
        true
    }

    fn clear(&mut self) {
        self.active_turn_id = None;
        self.phase = None;
        self.audio.clear();
        self.evaluation = None;
    }
}

struct PendingEndpoint {
    turn_id: String,
    generation: u64,
    handle: JoinHandle<()>,
}

struct EndpointTaskResult {
    turn_id: String,
    generation: u64,
    evaluation: EndpointEvaluation,
    audio_duration_ms: u128,
}

struct PendingTranscription {
    turn_id: String,
    generation: u64,
    response_id: String,
    mode: SessionMode,
    handle: JoinHandle<()>,
}

struct TranscriptionTaskResult {
    turn_id: String,
    generation: u64,
    response_id: String,
    transcript: Result<String, String>,
}

struct PendingGate {
    turn_id: String,
    generation: u64,
    response_id: String,
    mode: SessionMode,
    handle: JoinHandle<()>,
}

struct GateTaskResult {
    turn_id: String,
    generation: u64,
    response_id: String,
    transcript: String,
    gate: GateOutcome,
}

enum RealtimeInput<T> {
    Endpoint(EndpointTaskResult),
    Transcription(TranscriptionTaskResult),
    Gate(GateTaskResult),
    Socket(Option<T>),
}

async fn next_realtime_input<S>(
    socket: &mut S,
    endpoint_results: &mut mpsc::Receiver<EndpointTaskResult>,
    transcription_results: &mut mpsc::Receiver<TranscriptionTaskResult>,
    gate_results: &mut mpsc::Receiver<GateTaskResult>,
) -> RealtimeInput<S::Item>
where
    S: Stream + Unpin,
{
    tokio::select! {
        Some(result) = endpoint_results.recv() => RealtimeInput::Endpoint(result),
        Some(result) = transcription_results.recv() => RealtimeInput::Transcription(result),
        Some(result) = gate_results.recv() => RealtimeInput::Gate(result),
        message = socket.next() => RealtimeInput::Socket(message),
    }
}

fn spawn_transcription_task<F>(
    turn_id: String,
    generation: u64,
    response_id: String,
    mode: SessionMode,
    results: mpsc::Sender<TranscriptionTaskResult>,
    transcription: F,
) -> PendingTranscription
where
    F: Future<Output = anyhow::Result<String>> + Send + 'static,
{
    let task_turn_id = turn_id.clone();
    let task_response_id = response_id.clone();
    let handle = tokio::spawn(async move {
        let transcript = transcription.await.map_err(|error| error.to_string());
        let _ = results
            .send(TranscriptionTaskResult {
                turn_id: task_turn_id,
                generation,
                response_id: task_response_id,
                transcript,
            })
            .await;
    });
    PendingTranscription {
        turn_id,
        generation,
        response_id,
        mode,
        handle,
    }
}

fn spawn_final_transcription(
    orchestrator: AgentOrchestrator,
    audio: Vec<f32>,
    turn_id: String,
    generation: u64,
    response_id: String,
    mode: SessionMode,
    results: mpsc::Sender<TranscriptionTaskResult>,
) -> PendingTranscription {
    spawn_transcription_task(
        turn_id,
        generation,
        response_id,
        mode,
        results,
        async move { orchestrator.transcribe_candidate(&audio).await },
    )
}

fn cancel_transcription(pending: &mut Option<PendingTranscription>) {
    if let Some(pending) = pending.take() {
        pending.handle.abort();
    }
}

fn spawn_gate_task<F>(
    turn_id: String,
    generation: u64,
    response_id: String,
    transcript: String,
    mode: SessionMode,
    results: mpsc::Sender<GateTaskResult>,
    gate: F,
) -> PendingGate
where
    F: Future<Output = GateOutcome> + Send + 'static,
{
    let task_turn_id = turn_id.clone();
    let task_response_id = response_id.clone();
    let handle = tokio::spawn(async move {
        let gate = gate.await;
        let _ = results
            .send(GateTaskResult {
                turn_id: task_turn_id,
                generation,
                response_id: task_response_id,
                transcript,
                gate,
            })
            .await;
    });
    PendingGate {
        turn_id,
        generation,
        response_id,
        mode,
        handle,
    }
}

fn spawn_voice_gate(
    orchestrator: AgentOrchestrator,
    session_id: String,
    turn_id: String,
    generation: u64,
    response_id: String,
    transcript: String,
    mode: SessionMode,
    results: mpsc::Sender<GateTaskResult>,
) -> PendingGate {
    let gate_transcript = transcript.clone();
    let gate = async move {
        orchestrator
            .gate_transcript(&session_id, &gate_transcript)
            .await
    };
    spawn_gate_task(
        turn_id,
        generation,
        response_id,
        transcript,
        mode,
        results,
        gate,
    )
}

fn cancel_gate(pending: &mut Option<PendingGate>) {
    if let Some(pending) = pending.take() {
        pending.handle.abort();
    }
}

fn gate_result_matches(pending: &PendingGate, result: &GateTaskResult) -> bool {
    pending.turn_id == result.turn_id
        && pending.generation == result.generation
        && pending.response_id == result.response_id
}

fn transcription_result_matches(
    pending_turn_id: &str,
    pending_generation: u64,
    pending_response_id: &str,
    result_turn_id: &str,
    result_generation: u64,
    result_response_id: &str,
) -> bool {
    pending_turn_id == result_turn_id
        && pending_generation == result_generation
        && pending_response_id == result_response_id
}

fn endpoint_result_matches(
    pending_turn_id: &str,
    pending_generation: u64,
    result_turn_id: &str,
    result_generation: u64,
    state_accepts: bool,
) -> bool {
    pending_turn_id == result_turn_id && pending_generation == result_generation && state_accepts
}

fn cancel_endpoint(pending: &mut Option<PendingEndpoint>) {
    if let Some(pending) = pending.take() {
        pending.handle.abort();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FrameCorrelation {
    Matched(String),
    Stale,
}

fn correlate_pending_frame(
    pending_response_id: &str,
    event_response_id: Option<&str>,
) -> FrameCorrelation {
    match event_response_id {
        Some(response_id) if response_id == pending_response_id => {
            FrameCorrelation::Matched(response_id.to_owned())
        }
        _ => FrameCorrelation::Stale,
    }
}

fn client_protocol_version(event: &ClientEvent) -> Option<u32> {
    event
        .extra
        .get("protocol_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
}

fn validate_protocol_version(version: Option<u32>) -> Result<u32, ()> {
    version
        .filter(|version| (REALTIME_PROTOCOL_MIN..=REALTIME_PROTOCOL_VERSION).contains(version))
        .ok_or(())
}

fn unsupported_mode_change_event(
    negotiated_protocol_version: u32,
    event: &ClientEvent,
) -> Option<Value> {
    (negotiated_protocol_version < 5).then(|| {
        json!({
            "type": "error",
            "code": "unsupported_protocol",
            "message": "当前会话协议不支持切换模式",
            "response_id": event.response_id,
            "mode": event.extra.get("mode").and_then(Value::as_str)
        })
    })
}

fn validate_mode_set(
    negotiated_protocol_version: u32,
    event: &ClientEvent,
) -> Result<SessionMode, Value> {
    if let Some(error) = unsupported_mode_change_event(negotiated_protocol_version, event) {
        return Err(error);
    }
    let requested_mode = event.extra.get("mode").and_then(Value::as_str);
    SessionMode::parse(requested_mode).map_err(|_| {
        json!({
            "type": "error",
            "code": "invalid_mode",
            "message": "会话模式只支持 audio 或 video",
            "response_id": event.response_id,
            "mode": requested_mode
        })
    })
}

fn turn_needs_frame(mode: SessionMode) -> bool {
    mode == SessionMode::Video
}

fn requested_frame_event(mode: SessionMode, response_id: &str) -> Option<Value> {
    turn_needs_frame(mode).then(|| {
        json!({
            "type": "input.frame.requested",
            "response_id": response_id
        })
    })
}

enum ModeChangeEffect {
    PreserveActiveResponse,
    SpawnAudioTurn {
        response_id: String,
        transcript: String,
        frames: Vec<VideoFrame>,
    },
    Send(Value),
}

fn apply_mode_set_transaction(
    negotiated_protocol_version: u32,
    event: &ClientEvent,
    session_mode: &mut SessionMode,
    pending_transcription_mode: Option<&mut SessionMode>,
    pending_gate_mode: Option<&mut SessionMode>,
    pending_turn: &mut Option<PendingTurn>,
    frames: &mut VecDeque<VideoFrame>,
) -> Vec<ModeChangeEffect> {
    let mut effects = vec![ModeChangeEffect::PreserveActiveResponse];
    let accepted_mode = match validate_mode_set(negotiated_protocol_version, event) {
        Ok(mode) => mode,
        Err(error) => {
            effects.push(ModeChangeEffect::Send(error));
            return effects;
        }
    };
    let current_mode = *session_mode;
    if let Some(snapshot) = pending_transcription_mode {
        *snapshot = mode_snapshot_after_change(*snapshot, current_mode, accepted_mode);
    }
    if let Some(snapshot) = pending_gate_mode {
        *snapshot = mode_snapshot_after_change(*snapshot, current_mode, accepted_mode);
    }
    let plan = plan_mode_change(current_mode, accepted_mode, pending_turn.as_ref());
    let released_turn = if plan == ModeChangePlan::ReleasePendingAudioTurn {
        take_pending_video_turn(pending_turn, frames)
    } else {
        None
    };
    if accepted_mode == SessionMode::Audio {
        frames.clear();
    }
    *session_mode = accepted_mode;
    if let Some(pending) = released_turn {
        effects.push(ModeChangeEffect::SpawnAudioTurn {
            response_id: pending.response_id,
            transcript: pending.transcript,
            frames: Vec::new(),
        });
    }
    effects.push(ModeChangeEffect::Send(json!({
        "type": "session.mode.changed",
        "mode": session_mode.as_str()
    })));
    effects
}

enum AcceptedTurnAction {
    RequestFrame {
        pending: PendingTurn,
        event: Value,
    },
    SpawnAudioTurn {
        response_id: String,
        transcript: String,
        frames: Vec<VideoFrame>,
    },
}

fn plan_accepted_turn(
    mode: SessionMode,
    response_id: &str,
    transcript: &str,
) -> AcceptedTurnAction {
    match requested_frame_event(mode, response_id) {
        Some(event) => AcceptedTurnAction::RequestFrame {
            pending: PendingTurn {
                response_id: response_id.to_owned(),
                transcript: transcript.to_owned(),
            },
            event,
        },
        None => AcceptedTurnAction::SpawnAudioTurn {
            response_id: response_id.to_owned(),
            transcript: transcript.to_owned(),
            frames: Vec::new(),
        },
    }
}

fn audio_duration_ms(sample_count: usize, sample_rate: u32) -> u128 {
    if sample_rate == 0 {
        return 0;
    }
    sample_count as u128 * 1_000 / sample_rate as u128
}

fn endpoint_evaluated_payload(
    turn_id: &str,
    audio_duration_ms: u128,
    evaluation: &EndpointEvaluation,
) -> Value {
    json!({
        "turn_id": turn_id,
        "audio_duration_ms": audio_duration_ms,
        "transcript_chars": evaluation.transcript.chars().count(),
        "decision": evaluation.decision.as_str(),
        "reason": evaluation.reason,
        "classifier_latency_ms": evaluation.classifier_latency_ms
    })
}

fn endpoint_committed_payload(
    turn_id: &str,
    audio_duration_ms: u128,
    evaluation: Option<&EndpointEvaluation>,
    client_fallback_finalized: bool,
) -> Value {
    json!({
        "turn_id": turn_id,
        "audio_duration_ms": audio_duration_ms,
        "transcript_chars": evaluation.map(|item| item.transcript.chars().count()),
        "decision": evaluation.map(|item| item.decision.as_str()),
        "reason": evaluation.map(|item| item.reason),
        "classifier_latency_ms": evaluation.and_then(|item| item.classifier_latency_ms),
        "client_fallback_finalized": client_fallback_finalized
    })
}

fn stop_command_payload(turn_id: &str, transcript: &str) -> Value {
    json!({
        "turn_id": turn_id,
        "command": "stop",
        "transcript_chars": transcript.chars().count()
    })
}

fn client_endpoint_fallback(event: &ClientEvent) -> bool {
    event
        .extra
        .get("endpoint_fallback")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn client_cancel_clears_input(event: &ClientEvent) -> bool {
    event
        .extra
        .get("clear_input")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn clear_input_state(
    endpoint_state: &mut EndpointState,
    pending_endpoint: &mut Option<PendingEndpoint>,
    pending_transcription: &mut Option<PendingTranscription>,
    pending_gate: &mut Option<PendingGate>,
    frames: &mut VecDeque<VideoFrame>,
    pending_turn: &mut Option<PendingTurn>,
) {
    cancel_endpoint(pending_endpoint);
    cancel_transcription(pending_transcription);
    cancel_gate(pending_gate);
    endpoint_state.clear();
    frames.clear();
    *pending_turn = None;
}

fn take_pending_video_turn(
    pending_turn: &mut Option<PendingTurn>,
    frames: &mut VecDeque<VideoFrame>,
) -> Option<PendingTurn> {
    let pending = pending_turn.take();
    if pending.is_some() {
        frames.clear();
    }
    pending
}

async fn cancel_pending_video_turn(
    pending_turn: &mut Option<PendingTurn>,
    frames: &mut VecDeque<VideoFrame>,
    sender: &mpsc::Sender<Value>,
    context: &ContextStore,
    session_id: &str,
    reason: &str,
) {
    let Some(pending) = take_pending_video_turn(pending_turn, frames) else {
        return;
    };
    info!(%session_id, response_id = %pending.response_id, %reason, "pending video response cancelled");
    record_event_best_effort(
        context,
        session_id,
        "server.response.cancelled",
        &json!({"response_id": pending.response_id, "reason": reason}),
    )
    .await;
    let _ = send_event(
        sender,
        json!({
            "type": "response.cancelled",
            "response_id": pending.response_id,
            "reason": reason
        }),
    )
    .await;
}

fn normalize_playback_started(
    active_response_id: Option<&str>,
    event_response_id: Option<&str>,
    buffered_ms: Option<u64>,
) -> Option<(String, u64)> {
    let active = active_response_id?;
    let response_id = event_response_id?;
    if active != response_id {
        return None;
    }
    Some((
        response_id.to_owned(),
        buffered_ms.unwrap_or_default().min(10_000),
    ))
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ripple_agent_gateway=info,tower_http=info".into()),
        )
        .init();
    let settings = Arc::new(Settings::from_env()?);
    let context =
        ContextStore::connect(&settings.database_url, settings.database_max_connections).await?;
    context
        .seed_invitation_codes(
            &settings.invite_codes,
            settings.invite_max_uses,
            settings.invite_ttl_hours,
        )
        .await?;
    if settings.invite_codes.is_empty() {
        warn!("registration is disabled because RIPPLE_INVITE_CODES is empty");
    }
    let adapters = ModelAdapters::new((*settings).clone())?;
    let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets")).await?;
    let orchestrator = AgentOrchestrator::new(
        Arc::clone(&settings),
        context.clone(),
        adapters,
        memories.clone(),
    )?;
    let state = AppState {
        settings: Arc::clone(&settings),
        context,
        memories,
        orchestrator,
    };
    let app = app(state);

    let listener = tokio::net::TcpListener::bind(settings.address).await?;
    info!(address = %settings.address, "Ripple Rust Agent Gateway started");
    axum::serve(listener, app).await?;
    Ok(())
}

fn app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static("*"))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers(tower_http::cors::Any);
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/me", get(me))
        .route(
            "/v1/auth/me/avatar",
            axum::routing::put(upload_avatar).delete(clear_avatar),
        )
        .route("/v1/auth/logout", post(logout))
        .route(
            "/v1/profile",
            get(get_user_profile).put(update_user_profile),
        )
        .route("/v1/projects", get(list_projects).post(create_project))
        .route(
            "/v1/projects/{project_id}",
            get(get_project)
                .patch(update_project)
                .delete(archive_project),
        )
        .route(
            "/v1/projects/{project_id}/conversations",
            get(list_project_conversations).post(create_project_conversation),
        )
        .route("/v1/conversations", get(list_conversations))
        .route("/v1/conversations/batch", post(batch_conversations))
        .route(
            "/v1/conversations/{conversation_id}",
            get(get_conversation)
                .patch(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/messages",
            get(conversation_messages),
        )
        .route("/v1/memories", get(list_memories))
        .route("/v1/memories/batch", post(batch_memories))
        .route(
            "/v1/memories/{memory_id}",
            get(get_memory).patch(update_memory).delete(delete_memory),
        )
        .route("/v1/todos", get(list_todos).post(create_todo))
        .route(
            "/v1/todos/{todo_id}",
            axum::routing::patch(update_todo).delete(delete_todo),
        )
        .route("/v1/assets/{asset_id}/content", get(asset_content))
        .route("/v1/responses", post(create_response))
        .route("/v1/agent/realtime", get(realtime))
        .layer(cors)
        .layer(
            TraceLayer::new_for_http().make_span_with(
                DefaultMakeSpan::new()
                    .include_headers(false)
                    .level(Level::INFO),
            ),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "gateway": "rust",
        "backends": {
            "asr": state.settings.asr_backend,
            "agent": state.settings.agent_backend,
            "tts": state.settings.tts_backend
        },
        "external_tools": state.orchestrator.external_tool_count()
    }))
}

async fn ready(State(state): State<AppState>) -> Response {
    let report = ripple_agent_gateway::readiness::check(&state.settings, &state.context).await;
    let status = if report.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(report)).into_response()
}

#[derive(Deserialize)]
struct TodoQuery {
    limit: Option<i64>,
    completed: Option<bool>,
}

#[derive(Deserialize)]
struct TodoPatch {
    title: Option<String>,
    due_at: Option<f64>,
    clear_due_at: Option<bool>,
    completed: Option<bool>,
}

#[derive(Deserialize)]
struct CreateTodoRequest {
    title: String,
    due_at: Option<f64>,
}

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    invitation_code: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LibraryListQuery {
    #[serde(default)]
    scope: LibraryScope,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    query: String,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LibraryPatch {
    title: Option<String>,
    is_pinned: Option<bool>,
    archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BatchMutation {
    ids: Vec<String>,
    action: LibraryAction,
}

#[derive(Debug, Deserialize)]
struct MemoryPatch {
    user_note: Option<String>,
    is_pinned: Option<bool>,
    archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ProjectCreate {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
}

#[derive(Debug, Deserialize)]
struct ProjectPatch {
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UserProfileUpdate {
    #[serde(default)]
    ai_identity: String,
    #[serde(default)]
    user_identity: String,
    #[serde(default)]
    preferred_name: String,
    #[serde(default)]
    basic_memory: String,
}

#[derive(Deserialize)]
struct ResponsesRequest {
    input: Value,
    #[serde(default)]
    conversation: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stream: bool,
}

async fn register(State(state): State<AppState>, Json(request): Json<RegisterRequest>) -> Response {
    match state
        .context
        .register_user(
            &request.email,
            &request.password,
            &request.invitation_code,
            state.settings.auth_token_ttl_hours,
        )
        .await
    {
        Ok((user, token)) => (
            StatusCode::CREATED,
            Json(json!({"access_token": token, "token_type": "bearer", "user": user})),
        )
            .into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn login(State(state): State<AppState>, Json(request): Json<LoginRequest>) -> Response {
    match state
        .context
        .login_user(
            &request.email,
            &request.password,
            state.settings.auth_token_ttl_hours,
        )
        .await
    {
        Ok((user, token)) => {
            Json(json!({"access_token": token, "token_type": "bearer", "user": user}))
                .into_response()
        }
        Err(error) => api_error(StatusCode::UNAUTHORIZED, &error.to_string()),
    }
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match authenticated_user(&state, &headers).await {
        Ok(user) => Json(json!({"user": user})).into_response(),
        Err(response) => response,
    }
}

async fn upload_avatar(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let mut user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some("image/jpeg")
    {
        return api_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "头像必须是 JPEG 图片");
    }
    match state.memories.set_avatar(&user.id, &body).await {
        Ok(avatar_url) => {
            user.avatar_url = Some(avatar_url);
            Json(json!({"user": user})).into_response()
        }
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn clear_avatar(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.memories.clear_avatar(&user.id).await {
        Ok(_) => {
            user.avatar_url = None;
            Json(json!({"user": user})).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return api_error(StatusCode::UNAUTHORIZED, "需要登录");
    };
    if let Err(error) = state.context.revoke_token(token).await {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn get_user_profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.context.user_profile(&user.id).await {
        Ok(profile) => Json(json!({"data": profile})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn update_user_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UserProfileUpdate>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .update_user_profile(
            &user.id,
            &request.ai_identity,
            &request.user_identity,
            &request.preferred_name,
            &request.basic_memory,
        )
        .await
    {
        Ok(profile) => Json(json!({"data": profile})).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ProjectCreate>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .create_project(
            &user.id,
            &request.name,
            &request.description,
            &request.instructions,
        )
        .await
    {
        Ok(project) => (StatusCode::CREATED, Json(json!({"data": project}))).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LibraryListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .list_projects(&user.id, query.scope, query.limit.unwrap_or(50))
        .await
    {
        Ok(projects) => Json(json!({"data": projects})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.context.project(&user.id, &project_id).await {
        Ok(Some(project)) => Json(json!({"data": project})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "项目不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn update_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<ProjectPatch>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .update_project(
            &user.id,
            &project_id,
            request.name.as_deref(),
            request.description.as_deref(),
            request.instructions.as_deref(),
            request.archived,
        )
        .await
    {
        Ok(project) => Json(json!({"data": project})).into_response(),
        Err(error) if error.to_string().contains("不存在") => {
            api_error(StatusCode::NOT_FOUND, &error.to_string())
        }
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn archive_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .update_project(&user.id, &project_id, None, None, None, Some(true))
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) if error.to_string().contains("不存在") => {
            api_error(StatusCode::NOT_FOUND, &error.to_string())
        }
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn create_project_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .create_project_conversation(&user.id, &project_id)
        .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({"id": id, "project_id": project_id})),
        )
            .into_response(),
        Err(error) if error.to_string().contains("不存在") => {
            api_error(StatusCode::NOT_FOUND, &error.to_string())
        }
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn list_project_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Query(query): Query<LibraryListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .list_project_conversations(
            &user.id,
            &project_id,
            query.scope,
            query.limit.unwrap_or(50),
        )
        .await
    {
        Ok(conversations) => Json(json!({"data": conversations})).into_response(),
        Err(error) if error.to_string().contains("不存在") => {
            api_error(StatusCode::NOT_FOUND, &error.to_string())
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LibraryListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .list_conversations(
            &user.id,
            query.scope,
            query.pinned,
            &query.query,
            query.limit.unwrap_or(50),
        )
        .await
    {
        Ok(conversations) => Json(json!({"data": conversations})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn get_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .conversation_summary(&user.id, &conversation_id)
        .await
    {
        Ok(Some(conversation)) => Json(json!({"data": conversation})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "对话不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn update_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(request): Json<LibraryPatch>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if request.title.is_none() && request.is_pinned.is_none() && request.archived.is_none() {
        return api_error(StatusCode::BAD_REQUEST, "至少需要提供一个修改字段");
    }
    if let Some(title) = request.title
        && let Err(error) = state
            .context
            .rename_conversation(&user.id, &conversation_id, &title)
            .await
    {
        return mutation_error(error);
    }
    if let Some(is_pinned) = request.is_pinned
        && let Err(error) = state
            .context
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation_id),
                if is_pinned {
                    LibraryAction::Pin
                } else {
                    LibraryAction::Unpin
                },
            )
            .await
    {
        return mutation_error(error);
    }
    if let Some(archived) = request.archived
        && let Err(error) = state
            .context
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation_id),
                if archived {
                    LibraryAction::Archive
                } else {
                    LibraryAction::Unarchive
                },
            )
            .await
    {
        return mutation_error(error);
    }
    match state
        .context
        .conversation_summary(&user.id, &conversation_id)
        .await
    {
        Ok(Some(conversation)) => Json(json!({"data": conversation})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "对话不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn delete_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .mutate_conversations(
            &user.id,
            std::slice::from_ref(&conversation_id),
            LibraryAction::Delete,
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => mutation_error(error),
    }
}

async fn batch_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BatchMutation>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .mutate_conversations(&user.id, &request.ids, request.action)
        .await
    {
        Ok(updated) => Json(json!({"updated": updated})).into_response(),
        Err(error) => mutation_error(error),
    }
}

async fn conversation_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .conversation_messages(&user.id, &conversation_id, query.limit.unwrap_or(500))
        .await
    {
        Ok(Some(messages)) => Json(json!({"data": messages})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "对话不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn list_memories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LibraryListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .memories
        .list(
            &user.id,
            query.scope,
            query.pinned,
            &query.query,
            query.limit.unwrap_or(50).clamp(1, 100) as usize,
        )
        .await
    {
        Ok(memories) => Json(json!({"data": memories})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn get_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(memory_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.memories.get(&user.id, &memory_id).await {
        Ok(Some(memory)) => {
            Json(json!({"data": memory.memory, "assets": memory.assets})).into_response()
        }
        Ok(None) => api_error(StatusCode::NOT_FOUND, "记忆不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn update_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(memory_id): Path<String>,
    Json(request): Json<MemoryPatch>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if request.user_note.is_none() && request.is_pinned.is_none() && request.archived.is_none() {
        return api_error(StatusCode::BAD_REQUEST, "至少需要提供一个修改字段");
    }
    match state.memories.get(&user.id, &memory_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "记忆不存在"),
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
    if let Some(note) = request.user_note
        && let Err(error) = state.memories.update(&user.id, &memory_id, &note).await
    {
        return api_error(StatusCode::BAD_REQUEST, &error.to_string());
    }
    if let Some(is_pinned) = request.is_pinned
        && let Err(error) = state
            .memories
            .mutate(
                &user.id,
                std::slice::from_ref(&memory_id),
                if is_pinned {
                    LibraryAction::Pin
                } else {
                    LibraryAction::Unpin
                },
            )
            .await
    {
        return mutation_error(error);
    }
    if let Some(archived) = request.archived
        && let Err(error) = state
            .memories
            .mutate(
                &user.id,
                std::slice::from_ref(&memory_id),
                if archived {
                    LibraryAction::Archive
                } else {
                    LibraryAction::Unarchive
                },
            )
            .await
    {
        return mutation_error(error);
    }
    match state.memories.get(&user.id, &memory_id).await {
        Ok(Some(memory)) => Json(json!({"data": memory.memory})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "记忆不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn batch_memories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BatchMutation>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .memories
        .mutate(&user.id, &request.ids, request.action)
        .await
    {
        Ok(updated) => Json(json!({"updated": updated})).into_response(),
        Err(error) => mutation_error(error),
    }
}

async fn delete_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(memory_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.memories.delete(&user.id, &memory_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => api_error(StatusCode::NOT_FOUND, "记忆不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn list_todos(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TodoQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .memories
        .list_todos(
            &user.id,
            query.completed,
            query.limit.unwrap_or(100).clamp(1, 100) as usize,
        )
        .await
    {
        Ok(todos) => Json(json!({"data": todos})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn update_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(todo_id): Path<String>,
    Json(request): Json<TodoPatch>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .memories
        .update_todo(
            &user.id,
            &todo_id,
            TodoUpdate {
                title: request.title,
                due_at: if request.clear_due_at.unwrap_or(false) {
                    Some(None)
                } else {
                    request.due_at.map(Some)
                },
                completed: request.completed,
            },
        )
        .await
    {
        Ok(Some(todo)) => Json(json!({"data": todo})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "待办不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn create_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTodoRequest>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .memories
        .create_manual_todo(&user.id, &request.title, request.due_at)
        .await
    {
        Ok(todo) => (StatusCode::CREATED, Json(json!({ "data": todo }))).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn delete_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(todo_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.memories.delete_todo(&user.id, &todo_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => api_error(StatusCode::NOT_FOUND, "待办不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn asset_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let content = match state.memories.asset_content(&user.id, &asset_id).await {
        Ok(Some(content)) => content,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "图片不存在"),
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    };
    match tokio::fs::read(content.path).await {
        Ok(bytes) => match HeaderValue::from_str(&content.mime_type) {
            Ok(content_type) => (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, content_type),
                    (
                        axum::http::header::CACHE_CONTROL,
                        HeaderValue::from_static("private, max-age=3600"),
                    ),
                ],
                bytes,
            )
                .into_response(),
            Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            api_error(StatusCode::NOT_FOUND, "图片不存在")
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn create_response(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ResponsesRequest>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if request.stream {
        return api_error(
            StatusCode::BAD_REQUEST,
            "当前版本暂不支持 Responses API 流式输出",
        );
    }
    if request
        .model
        .as_deref()
        .is_some_and(|model| model.trim().is_empty())
    {
        return api_error(StatusCode::BAD_REQUEST, "model 不能为空");
    }
    let input = match responses_input_text(&request.input) {
        Some(input) if !input.trim().is_empty() => input,
        _ => return api_error(StatusCode::BAD_REQUEST, "当前仅支持文本 input"),
    };
    let requested_conversation = request.conversation.as_ref().and_then(|value| {
        value
            .as_str()
            .or_else(|| value.get("id").and_then(Value::as_str))
    });
    let conversation_id = match requested_conversation {
        Some(id) => match state.context.conversation_belongs_to(id, &user.id).await {
            Ok(true) => id.to_owned(),
            Ok(false) => return api_error(StatusCode::NOT_FOUND, "对话不存在"),
            Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        None => match state.context.create_conversation(&user.id).await {
            Ok(id) => id,
            Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
    };
    let response_id = format!("resp_{}", Uuid::new_v4().simple());
    match state
        .orchestrator
        .run_text_response(&user.id, &conversation_id, &input, &response_id)
        .await
    {
        Ok(output) => {
            let message_id = format!("msg_{}", Uuid::new_v4().simple());
            let completed_at = unix_timestamp();
            Json(json!({
                "id": response_id,
                "object": "response",
                "created_at": completed_at,
                "status": "completed",
                "completed_at": completed_at,
                "error": null,
                "incomplete_details": null,
                "instructions": null,
                "max_output_tokens": null,
                "model": state.settings.agent_model,
                "conversation": {"id": conversation_id},
                "output": [{
                    "id": message_id,
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": output, "annotations": []}]
                }],
                "parallel_tool_calls": false,
                "previous_response_id": null,
                "reasoning": {"effort": null, "summary": null},
                "store": true,
                "temperature": state.settings.agent_temperature,
                "text": {"format": {"type": "text"}},
                "tool_choice": "auto",
                "tools": [],
                "top_p": 1.0,
                "truncation": "disabled",
                "usage": null,
                "metadata": {}
            }))
            .into_response()
        }
        Err(error) => api_error(StatusCode::BAD_GATEWAY, &error.to_string()),
    }
}

async fn authenticated_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ripple_agent_gateway::auth::AuthUser, Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(api_error(StatusCode::UNAUTHORIZED, "需要登录"));
    };
    match state.context.authenticate(token).await {
        Ok(Some(user)) => Ok(user),
        Ok(None) => Err(api_error(
            StatusCode::UNAUTHORIZED,
            "登录已失效，请重新登录",
        )),
        Err(error) => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &error.to_string(),
        )),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

fn responses_input_text(input: &Value) -> Option<String> {
    if let Some(text) = input.as_str() {
        return Some(text.to_owned());
    }
    let items = input.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        if item.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        match item.get("content") {
            Some(Value::String(text)) => parts.push(text.clone()),
            Some(Value::Array(content)) => {
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("input_text" | "text")
                    ) && let Some(text) = part.get("text").and_then(Value::as_str)
                    {
                        parts.push(text.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn api_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {"message": message, "type": "invalid_request_error", "param": null, "code": null}
        })),
    )
        .into_response()
}

fn mutation_error(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("不存在") {
        StatusCode::NOT_FOUND
    } else if message.starts_with("ids ") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    api_error(status, &message)
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn unix_timestamp_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

async fn realtime(
    websocket: WebSocketUpgrade,
    Query(query): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Response {
    let user = match query.get("access_token").filter(|token| !token.is_empty()) {
        Some(token) => match state.context.authenticate(token).await {
            Ok(Some(user)) => user,
            Ok(None) => return api_error(StatusCode::UNAUTHORIZED, "登录已失效，请重新登录"),
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
            }
        },
        None if !state.settings.anonymous_realtime_user_id.is_empty() => {
            warn!(
                user_id = %state.settings.anonymous_realtime_user_id,
                "accepting anonymous realtime device connection"
            );
            ripple_agent_gateway::auth::AuthUser {
                id: state.settings.anonymous_realtime_user_id.clone(),
                email: "passport-device@local".to_owned(),
                avatar_url: None,
            }
        }
        None => return api_error(StatusCode::UNAUTHORIZED, "需要登录"),
    };
    let conversation_id = match query.get("conversation_id").filter(|id| !id.is_empty()) {
        Some(id) => match state.context.conversation_belongs_to(id, &user.id).await {
            Ok(true) => id.clone(),
            Ok(false) => return api_error(StatusCode::NOT_FOUND, "对话不存在"),
            Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        None => match state.context.create_conversation(&user.id).await {
            Ok(id) => id,
            Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
    };
    websocket
        .on_upgrade(move |socket| async move {
            if let Err(error) = handle_socket(socket, state, user.id, conversation_id).await {
                warn!(%error, "realtime session ended with an error");
            }
        })
        .into_response()
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user_id: String,
    session_id: String,
) -> anyhow::Result<()> {
    if let Err(error) = state.context.touch_session(&session_id).await {
        error!(%session_id, %error, "failed to initialize session");
        return Ok(());
    }
    info!(%session_id, "session connected");
    record_event_best_effort(
        &state.context,
        &session_id,
        "server.session.connected",
        &json!({}),
    )
    .await;
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (event_sender, mut event_receiver) = mpsc::channel::<Value>(64);
    let writer = tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            let Ok(encoded) = serde_json::to_string(&event) else {
                continue;
            };
            if ws_sender.send(Message::Text(encoded.into())).await.is_err() {
                break;
            }
        }
    });

    if send_event(
        &event_sender,
        json!({
            "type": "session.created",
            "session_id": session_id,
            "conversation_id": session_id,
            "sample_rate_in": state.settings.sample_rate_in,
            "sample_rate_out": state.settings.sample_rate_out
        }),
    )
    .await
    .is_err()
    {
        writer.abort();
        return Ok(());
    }

    let mut endpoint_state = EndpointState::default();
    let mut frames = VecDeque::<VideoFrame>::with_capacity(state.settings.max_frames);
    let mut active_response: Option<ActiveResponse> = None;
    let mut activation_mode = ActivationMode::Continuous;
    let mut awake_until: Option<Instant> = None;
    let mut session_mode = SessionMode::Audio;
    let mut negotiated_protocol_version: Option<u32> = None;
    let mut session_ready = false;
    let mut pending_turn: Option<PendingTurn> = None;
    let mut pending_endpoint: Option<PendingEndpoint> = None;
    let mut pending_transcription: Option<PendingTranscription> = None;
    let mut pending_gate: Option<PendingGate> = None;
    let mut endpoint_generation = 0_u64;
    let mut gate_generation = 0_u64;
    let (endpoint_results, mut endpoint_results_rx) = mpsc::channel::<EndpointTaskResult>(4);
    let (transcription_results, mut transcription_results_rx) =
        mpsc::channel::<TranscriptionTaskResult>(4);
    let (gate_results, mut gate_results_rx) = mpsc::channel::<GateTaskResult>(4);

    loop {
        let message = match next_realtime_input(
            &mut ws_receiver,
            &mut endpoint_results_rx,
            &mut transcription_results_rx,
            &mut gate_results_rx,
        )
        .await
        {
            RealtimeInput::Endpoint(result) => {
                let is_current = pending_endpoint.as_ref().is_some_and(|pending| {
                    endpoint_result_matches(
                        &pending.turn_id,
                        pending.generation,
                        &result.turn_id,
                        result.generation,
                        endpoint_state.accepts_result(&result.turn_id),
                    )
                });
                if !is_current {
                    warn!(
                        %session_id,
                        turn_id = %result.turn_id,
                        "stale endpoint evaluation ignored"
                    );
                    continue;
                }
                pending_endpoint.take();
                let EndpointTaskResult {
                    turn_id,
                    generation: _,
                    evaluation,
                    audio_duration_ms,
                } = result;
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.endpoint_evaluated",
                    &endpoint_evaluated_payload(&turn_id, audio_duration_ms, &evaluation),
                )
                .await;
                if endpoint_state.consume_stop(&turn_id, &evaluation.transcript) {
                    frames.clear();
                    pending_turn = None;
                    cancel_response(
                        &mut active_response,
                        &event_sender,
                        &state.context,
                        &session_id,
                        "stop_command",
                    )
                    .await;
                    record_event_best_effort(
                        &state.context,
                        &session_id,
                        "server.input.stop_command_handled",
                        &stop_command_payload(&turn_id, &evaluation.transcript),
                    )
                    .await;
                    send_event(
                        &event_sender,
                        json!({
                            "type": "input.command.handled",
                            "turn_id": turn_id,
                            "command": "stop"
                        }),
                    )
                    .await?;
                    continue;
                }
                let decision = evaluation.decision;
                let reason = evaluation.reason;
                let classifier_latency_ms = evaluation.classifier_latency_ms;
                endpoint_state.store_evaluation(&turn_id, evaluation);
                send_event(
                    &event_sender,
                    json!({
                        "type": "input.turn.decision",
                        "turn_id": turn_id,
                        "decision": decision.as_str(),
                        "reason": reason,
                        "classifier_latency_ms": classifier_latency_ms
                    }),
                )
                .await?;
                continue;
            }
            RealtimeInput::Transcription(result) => {
                let is_current = pending_transcription.as_ref().is_some_and(|pending| {
                    transcription_result_matches(
                        &pending.turn_id,
                        pending.generation,
                        &pending.response_id,
                        &result.turn_id,
                        result.generation,
                        &result.response_id,
                    )
                });
                if !is_current {
                    warn!(
                        %session_id,
                        turn_id = %result.turn_id,
                        response_id = %result.response_id,
                        "stale final transcription ignored"
                    );
                    continue;
                }
                let accepted_mode = pending_transcription
                    .take()
                    .expect("current transcription has matching pending state")
                    .mode;
                let TranscriptionTaskResult {
                    turn_id,
                    generation: _,
                    response_id,
                    transcript,
                } = result;
                match transcript {
                    Ok(transcript) => {
                        queue_voice_transcript(
                            &state,
                            &session_id,
                            &event_sender,
                            &mut active_response,
                            activation_mode,
                            &mut awake_until,
                            accepted_mode,
                            &mut frames,
                            &mut pending_turn,
                            &turn_id,
                            response_id,
                            transcript,
                            &mut pending_gate,
                            &mut gate_generation,
                            &gate_results,
                        )
                        .await?;
                    }
                    Err(error) => {
                        warn!(%session_id, %response_id, %error, "audio transcription failed");
                        record_event_best_effort(
                            &state.context,
                            &session_id,
                            "server.transcript.failed",
                            &json!({"response_id": response_id, "reason": "asr_error"}),
                        )
                        .await;
                        send_event(
                            &event_sender,
                            failed_response_event(
                                &response_id,
                                &anyhow::anyhow!("ASR_FAILED: {error}"),
                            ),
                        )
                        .await?;
                    }
                }
                continue;
            }
            RealtimeInput::Gate(result) => {
                let is_current = pending_gate
                    .as_ref()
                    .is_some_and(|pending| gate_result_matches(pending, &result));
                if !is_current {
                    warn!(
                        %session_id,
                        turn_id = %result.turn_id,
                        response_id = %result.response_id,
                        "stale response gate result ignored"
                    );
                    continue;
                }
                let accepted_mode = pending_gate
                    .take()
                    .expect("current gate result has matching pending state")
                    .mode;
                let GateTaskResult {
                    turn_id: _,
                    generation: _,
                    response_id,
                    transcript,
                    gate,
                } = result;
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.gate.completed",
                    &json!({
                        "response_id": response_id,
                        "transcript": transcript,
                        "gate_decision": gate.decision.as_str(),
                        "gate_reason": gate.reason,
                        "gate_latency_ms": gate.latency_ms,
                        "gate_fallback": gate.fallback
                    }),
                )
                .await;
                if gate.decision == GateDecision::Ignore {
                    frames.clear();
                    continue;
                }
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "gate_respond",
                )
                .await;
                match plan_accepted_turn(accepted_mode, &response_id, &transcript) {
                    AcceptedTurnAction::RequestFrame { pending, event } => {
                        if let Some(superseded) = pending_turn.take() {
                            record_event_best_effort(
                                &state.context,
                                &session_id,
                                "server.response.cancelled",
                                &json!({
                                    "response_id": superseded.response_id,
                                    "reason": "superseded_before_frame"
                                }),
                            )
                            .await;
                            send_event(
                                &event_sender,
                                json!({
                                    "type": "response.cancelled",
                                    "response_id": superseded.response_id,
                                    "reason": "superseded_before_frame"
                                }),
                            )
                            .await?;
                        }
                        frames.clear();
                        pending_turn = Some(pending);
                        send_event(&event_sender, event).await?;
                    }
                    AcceptedTurnAction::SpawnAudioTurn {
                        response_id,
                        transcript,
                        frames: accepted_frames,
                    } => {
                        active_response = Some(spawn_turn(
                            state.orchestrator.clone(),
                            state.context.clone(),
                            user_id.clone(),
                            session_id.clone(),
                            event_sender.clone(),
                            Vec::new(),
                            accepted_frames,
                            Some(transcript),
                            response_id,
                        ));
                    }
                }
                continue;
            }
            RealtimeInput::Socket(Some(message)) => message,
            RealtimeInput::Socket(None) => break,
        };
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Binary(_)) => {
                let _ = send_event(
                    &event_sender,
                    json!({"type": "error", "message": "当前协议只接受 JSON 文本帧"}),
                )
                .await;
                continue;
            }
        };
        let raw: Value = match serde_json::from_str(text.as_str()) {
            Ok(value) => value,
            Err(error) => {
                let _ = send_event(
                    &event_sender,
                    json!({"type": "error", "message": format!("无效 JSON: {error}")}),
                )
                .await;
                continue;
            }
        };
        let event: ClientEvent = match serde_json::from_value(raw.clone()) {
            Ok(event) => event,
            Err(error) => {
                let _ = send_event(
                    &event_sender,
                    json!({"type": "error", "message": format!("无效事件: {error}")}),
                )
                .await;
                continue;
            }
        };
        if event.kind != "input.audio.append" {
            let mut logged = raw;
            if let Some(object) = logged.as_object_mut()
                && let Some(image) = object.remove("image")
            {
                object.insert(
                    "image_bytes_base64".to_owned(),
                    json!(image.as_str().map(str::len).unwrap_or_default()),
                );
            }
            if let Err(error) = state
                .context
                .record_event(&session_id, &format!("client.{}", event.kind), &logged)
                .await
            {
                warn!(%session_id, %error, "failed to record client event");
            }
        }

        if event.kind != "session.start" && !session_ready {
            let _ = send_event(
                &event_sender,
                json!({
                    "type": "error",
                    "code": "unsupported_protocol",
                    "message": "必须先使用协议 v4 或 v5 初始化会话"
                }),
            )
            .await;
            break;
        }

        let result = match event.kind.as_str() {
            "session.start" => {
                if session_ready {
                    send_event(
                        &event_sender,
                        json!({
                            "type": "error",
                            "code": "session_already_started",
                            "message": "会话已经初始化"
                        }),
                    )
                    .await?;
                    continue;
                }
                activation_mode = ActivationMode::parse(
                    event.extra.get("activation_mode").and_then(Value::as_str),
                );
                let protocol_version = client_protocol_version(&event);
                let Ok(accepted_protocol_version) = validate_protocol_version(protocol_version)
                else {
                    send_event(
                        &event_sender,
                        json!({
                            "type": "error",
                            "code": "unsupported_protocol",
                            "message": "客户端与服务端协议版本不一致，需要使用协议 v4 或 v5"
                        }),
                    )
                    .await?;
                    break;
                };
                let initial_mode = event.extra.get("mode").and_then(Value::as_str);
                let Ok(accepted_mode) = SessionMode::parse_initial(initial_mode) else {
                    send_event(
                        &event_sender,
                        json!({
                            "type": "error",
                            "code": "invalid_mode",
                            "message": "会话模式只支持 audio 或 video",
                            "mode": initial_mode
                        }),
                    )
                    .await?;
                    break;
                };
                negotiated_protocol_version = Some(accepted_protocol_version);
                session_mode = accepted_mode;
                session_ready = true;
                let result = send_event(
                    &event_sender,
                    json!({
                        "type": "session.ready",
                        "session_id": session_id,
                        "activation_mode": match activation_mode {
                            ActivationMode::Wake => "wake",
                            ActivationMode::Continuous => "continuous",
                        },
                        "protocol_version": accepted_protocol_version,
                        "sample_rate_in": state.settings.sample_rate_in,
                        "sample_rate_out": state.settings.sample_rate_out,
                        "mode": session_mode.as_str()
                    }),
                )
                .await;
                if result.is_ok() {
                    info!(%session_id, "session ready");
                    record_event_best_effort(
                        &state.context,
                        &session_id,
                        "server.session.ready",
                        &json!({
                            "mode": session_mode.as_str(),
                            "protocol_version": accepted_protocol_version
                        }),
                    )
                    .await;
                }
                result
            }
            "session.mode.set" => {
                let negotiated_protocol_version = negotiated_protocol_version
                    .expect("a ready session always has a negotiated protocol version");
                let effects = apply_mode_set_transaction(
                    negotiated_protocol_version,
                    &event,
                    &mut session_mode,
                    pending_transcription
                        .as_mut()
                        .map(|pending| &mut pending.mode),
                    pending_gate.as_mut().map(|pending| &mut pending.mode),
                    &mut pending_turn,
                    &mut frames,
                );
                let mut outcome = Ok(());
                for effect in effects {
                    match effect {
                        ModeChangeEffect::PreserveActiveResponse => {}
                        ModeChangeEffect::SpawnAudioTurn {
                            response_id,
                            transcript,
                            frames: accepted_frames,
                        } => {
                            active_response = Some(spawn_turn(
                                state.orchestrator.clone(),
                                state.context.clone(),
                                user_id.clone(),
                                session_id.clone(),
                                event_sender.clone(),
                                Vec::new(),
                                accepted_frames,
                                Some(transcript),
                                response_id,
                            ));
                        }
                        ModeChangeEffect::Send(event) => {
                            if let Err(error) = send_event(&event_sender, event).await {
                                outcome = Err(error);
                                break;
                            }
                        }
                    }
                }
                outcome
            }
            "input.audio.append" => {
                let result = endpoint_state.append_audio(event.audio.as_deref(), &state.settings);
                if let Err(error) = result {
                    send_event(
                        &event_sender,
                        json!({"type": "error", "message": error.to_string()}),
                    )
                    .await
                } else {
                    Ok(())
                }
            }
            "input.video.frame" => {
                let correlation = pending_turn
                    .as_ref()
                    .map(|pending| {
                        correlate_pending_frame(&pending.response_id, event.response_id.as_deref())
                    })
                    .unwrap_or(FrameCorrelation::Stale);
                if correlation == FrameCorrelation::Stale {
                    warn!(
                        %session_id,
                        response_id = event.response_id.as_deref().unwrap_or("missing"),
                        "stale video frame ignored"
                    );
                    continue;
                }
                match event
                    .image
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("视频帧缺少 image"))
                    .and_then(|encoded| STANDARD.decode(encoded).map_err(Into::into))
                {
                    Ok(bytes) if bytes.len() <= 2 * 1024 * 1024 => {
                        let mime_type = event.mime_type.unwrap_or_else(|| "image/jpeg".to_owned());
                        if mime_type != "image/jpeg" {
                            send_event(
                                &event_sender,
                                json!({"type": "error", "message": "当前只支持 JPEG 视频帧"}),
                            )
                            .await
                        } else {
                            if frames.len() == state.settings.max_frames {
                                frames.pop_front();
                            }
                            frames.push_back(VideoFrame {
                                bytes,
                                mime_type,
                                captured_at_ms: event
                                    .extra
                                    .get("captured_at")
                                    .and_then(Value::as_i64),
                                received_at_ms: unix_timestamp_millis(),
                            });
                            Ok(())
                        }
                    }
                    Ok(_) => {
                        send_event(
                            &event_sender,
                            json!({"type": "error", "message": "视频帧不能超过 2MB"}),
                        )
                        .await
                    }
                    Err(error) => {
                        send_event(
                            &event_sender,
                            json!({"type": "error", "message": error.to_string()}),
                        )
                        .await
                    }
                }
            }
            "input.speech_started" => {
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "speech_started",
                )
                .await;
                cancel_pending_video_turn(
                    &mut pending_turn,
                    &mut frames,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "speech_started",
                )
                .await;
                cancel_endpoint(&mut pending_endpoint);
                cancel_transcription(&mut pending_transcription);
                cancel_gate(&mut pending_gate);
                let Some(turn_id) = event
                    .turn_id
                    .as_deref()
                    .filter(|turn_id| !turn_id.is_empty())
                else {
                    warn!(%session_id, "speech start without turn id ignored");
                    continue;
                };
                endpoint_state.start(turn_id, state.settings.sample_rate_in as usize / 2);
                info!(%session_id, %turn_id, pre_roll_samples = endpoint_state.audio.len(), "speech started");
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.speech_started",
                    &json!({
                        "turn_id": turn_id,
                        "pre_roll_samples": endpoint_state.audio.len()
                    }),
                )
                .await;
                send_event(
                    &event_sender,
                    json!({"type": "input.speech_started", "turn_id": turn_id}),
                )
                .await
            }
            "input.speech_resumed" => {
                let Some(turn_id) = event
                    .turn_id
                    .as_deref()
                    .filter(|turn_id| !turn_id.is_empty())
                else {
                    warn!(%session_id, "speech resume without turn id ignored");
                    continue;
                };
                if !endpoint_state.resume(turn_id) {
                    warn!(%session_id, %turn_id, "stale speech resume ignored");
                    continue;
                }
                cancel_endpoint(&mut pending_endpoint);
                send_event(
                    &event_sender,
                    json!({"type": "input.speech_resumed", "turn_id": turn_id}),
                )
                .await
            }
            "input.turn.pause" => {
                let Some(turn_id) = event
                    .turn_id
                    .as_deref()
                    .filter(|turn_id| !turn_id.is_empty())
                else {
                    warn!(%session_id, "turn pause without turn id ignored");
                    continue;
                };
                if !endpoint_state.pause(turn_id) {
                    warn!(%session_id, %turn_id, "stale turn pause ignored");
                    continue;
                }
                cancel_endpoint(&mut pending_endpoint);
                let captured_audio = endpoint_state.audio.clone();
                let endpoint_orchestrator = state.orchestrator.clone();
                let endpoint_results = endpoint_results.clone();
                let task_turn_id = turn_id.to_owned();
                let task_audio_duration_ms =
                    audio_duration_ms(captured_audio.len(), state.settings.sample_rate_in);
                endpoint_generation += 1;
                let task_generation = endpoint_generation;
                let handle = tokio::spawn(async move {
                    let evaluation = endpoint_orchestrator
                        .evaluate_turn_end(&captured_audio)
                        .await;
                    let _ = endpoint_results
                        .send(EndpointTaskResult {
                            turn_id: task_turn_id,
                            generation: task_generation,
                            evaluation,
                            audio_duration_ms: task_audio_duration_ms,
                        })
                        .await;
                });
                pending_endpoint = Some(PendingEndpoint {
                    turn_id: turn_id.to_owned(),
                    generation: task_generation,
                    handle,
                });
                Ok(())
            }
            "input.commit" => {
                let Some(turn_id) = event
                    .turn_id
                    .as_deref()
                    .filter(|turn_id| !turn_id.is_empty())
                else {
                    warn!(%session_id, "audio commit without turn id ignored");
                    continue;
                };
                let Some((captured_audio, reusable_evaluation)) =
                    endpoint_state.take_commit(turn_id)
                else {
                    warn!(%session_id, %turn_id, "stale audio commit ignored");
                    continue;
                };
                let client_fallback_finalized = client_endpoint_fallback(&event);
                let committed_audio_duration_ms =
                    audio_duration_ms(captured_audio.len(), state.settings.sample_rate_in);
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.endpoint_committed",
                    &endpoint_committed_payload(
                        turn_id,
                        committed_audio_duration_ms,
                        reusable_evaluation.as_ref(),
                        client_fallback_finalized,
                    ),
                )
                .await;
                cancel_endpoint(&mut pending_endpoint);
                cancel_gate(&mut pending_gate);
                let response_id = Uuid::new_v4().to_string();
                if let Some(evaluation) =
                    reusable_evaluation.filter(|item| !item.transcript.is_empty())
                {
                    queue_voice_transcript(
                        &state,
                        &session_id,
                        &event_sender,
                        &mut active_response,
                        activation_mode,
                        &mut awake_until,
                        session_mode,
                        &mut frames,
                        &mut pending_turn,
                        turn_id,
                        response_id,
                        evaluation.transcript,
                        &mut pending_gate,
                        &mut gate_generation,
                        &gate_results,
                    )
                    .await?;
                } else {
                    endpoint_generation += 1;
                    let generation = endpoint_generation;
                    let results = transcription_results.clone();
                    pending_transcription = Some(spawn_final_transcription(
                        state.orchestrator.clone(),
                        captured_audio,
                        turn_id.to_owned(),
                        generation,
                        response_id,
                        session_mode,
                        results,
                    ));
                }
                continue;
            }
            "input.clear" => {
                clear_input_state(
                    &mut endpoint_state,
                    &mut pending_endpoint,
                    &mut pending_transcription,
                    &mut pending_gate,
                    &mut frames,
                    &mut pending_turn,
                );
                Ok(())
            }
            "input.video.commit" => {
                let Some(pending) = pending_turn.as_ref() else {
                    warn!(%session_id, "video commit received without a pending activated turn");
                    continue;
                };
                match correlate_pending_frame(&pending.response_id, event.response_id.as_deref()) {
                    FrameCorrelation::Matched(_) => {}
                    FrameCorrelation::Stale => {
                        warn!(
                            %session_id,
                            response_id = event.response_id.as_deref().unwrap_or("missing"),
                            expected_response_id = %pending.response_id,
                            "stale video commit ignored"
                        );
                        continue;
                    }
                }
                let pending = pending_turn
                    .take()
                    .expect("pending turn was checked immediately before take");
                let captured_frames: Vec<_> = frames.drain(..).collect();
                active_response = Some(spawn_turn(
                    state.orchestrator.clone(),
                    state.context.clone(),
                    user_id.clone(),
                    session_id.clone(),
                    event_sender.clone(),
                    Vec::new(),
                    captured_frames,
                    Some(pending.transcript),
                    pending.response_id,
                ));
                Ok(())
            }
            "input.text.commit" => {
                cancel_endpoint(&mut pending_endpoint);
                cancel_transcription(&mut pending_transcription);
                cancel_gate(&mut pending_gate);
                endpoint_state.clear();
                cancel_pending_video_turn(
                    &mut pending_turn,
                    &mut frames,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "new_text_commit",
                )
                .await;
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "new_text_commit",
                )
                .await;
                let captured_frames: Vec<_> = frames.drain(..).collect();
                let text = event.text.unwrap_or_default();
                let response_id = Uuid::new_v4().to_string();
                info!(%session_id, %response_id, text_chars = text.chars().count(), frames = captured_frames.len(), "text input committed");
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.committed",
                    &json!({
                        "response_id": response_id,
                        "input": "text",
                        "text_chars": text.chars().count(),
                        "frames": captured_frames.len()
                    }),
                )
                .await;
                active_response = Some(spawn_turn(
                    state.orchestrator.clone(),
                    state.context.clone(),
                    user_id.clone(),
                    session_id.clone(),
                    event_sender.clone(),
                    Vec::new(),
                    captured_frames,
                    Some(text),
                    response_id,
                ));
                Ok(())
            }
            "output.playback.started" => {
                if let Some((response_id, buffered_ms)) = normalize_playback_started(
                    active_response
                        .as_ref()
                        .map(|response| response.id.as_str()),
                    event.response_id.as_deref(),
                    event.extra.get("buffered_ms").and_then(Value::as_u64),
                ) {
                    record_event_best_effort(
                        &state.context,
                        &session_id,
                        "server.output.playback.started",
                        &json!({
                            "response_id": response_id,
                            "buffered_ms": buffered_ms
                        }),
                    )
                    .await;
                }
                Ok(())
            }
            "response.cancel" => {
                cancel_transcription(&mut pending_transcription);
                cancel_gate(&mut pending_gate);
                cancel_pending_video_turn(
                    &mut pending_turn,
                    &mut frames,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "client_request",
                )
                .await;
                if client_cancel_clears_input(&event) {
                    clear_input_state(
                        &mut endpoint_state,
                        &mut pending_endpoint,
                        &mut pending_transcription,
                        &mut pending_gate,
                        &mut frames,
                        &mut pending_turn,
                    );
                }
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "client_request",
                )
                .await;
                Ok(())
            }
            "session.wake" => {
                awake_until = Some(Instant::now() + Duration::from_secs(30));
                send_event(
                    &event_sender,
                    json!({"type": "session.awake", "expires_in_seconds": 30}),
                )
                .await
            }
            "session.close" => {
                cancel_endpoint(&mut pending_endpoint);
                cancel_transcription(&mut pending_transcription);
                cancel_gate(&mut pending_gate);
                break;
            }
            _ => {
                warn!(kind = %event.kind, extra = ?event.extra, "unknown client event");
                Ok(())
            }
        };
        if result.is_err() {
            break;
        }
    }

    cancel_endpoint(&mut pending_endpoint);
    cancel_transcription(&mut pending_transcription);
    cancel_gate(&mut pending_gate);
    cancel_response(
        &mut active_response,
        &event_sender,
        &state.context,
        &session_id,
        "session_disconnected",
    )
    .await;
    info!(%session_id, "session disconnected");
    record_event_best_effort(
        &state.context,
        &session_id,
        "server.session.disconnected",
        &json!({}),
    )
    .await;
    drop(event_sender);
    let _ = writer.await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn queue_voice_transcript(
    state: &AppState,
    session_id: &str,
    event_sender: &mpsc::Sender<Value>,
    active_response: &mut Option<ActiveResponse>,
    activation_mode: ActivationMode,
    awake_until: &mut Option<Instant>,
    session_mode: SessionMode,
    frames: &mut VecDeque<VideoFrame>,
    pending_turn: &mut Option<PendingTurn>,
    turn_id: &str,
    response_id: String,
    transcript: String,
    pending_gate: &mut Option<PendingGate>,
    gate_generation: &mut u64,
    gate_results: &mpsc::Sender<GateTaskResult>,
) -> anyhow::Result<()> {
    if is_stop_command(&transcript) {
        frames.clear();
        *pending_turn = None;
        cancel_response(
            active_response,
            event_sender,
            &state.context,
            session_id,
            "stop_command",
        )
        .await;
        record_event_best_effort(
            &state.context,
            session_id,
            "server.input.stop_command_handled",
            &stop_command_payload(turn_id, &transcript),
        )
        .await;
        send_event(
            event_sender,
            json!({
                "type": "input.command.handled",
                "turn_id": turn_id,
                "command": "stop"
            }),
        )
        .await?;
        return Ok(());
    }
    if activation_mode == ActivationMode::Wake {
        send_event(
            event_sender,
            json!({"type": "input.activation.checking", "response_id": response_id}),
        )
        .await?;
        let follow_up_window_open = awake_until.is_some_and(|deadline| deadline > Instant::now());
        let decision = evaluate_activation(&transcript, follow_up_window_open);
        if decision.reason == "sleep_command" {
            *awake_until = None;
        }
        record_event_best_effort(
            &state.context,
            session_id,
            if decision.accepted {
                "server.activation.accepted"
            } else {
                "server.activation.rejected"
            },
            &json!({
                "response_id": response_id,
                "reason": decision.reason,
                "text_chars": transcript.chars().count()
            }),
        )
        .await;
        if !decision.accepted {
            frames.clear();
            send_event(
                event_sender,
                json!({
                    "type": "input.activation.rejected",
                    "response_id": response_id,
                    "reason": decision.reason
                }),
            )
            .await?;
            return Ok(());
        }
        *awake_until = Some(Instant::now() + Duration::from_secs(30));
        send_event(
            event_sender,
            json!({
                "type": "input.activation.accepted",
                "response_id": response_id,
                "text": transcript,
                "reason": decision.reason,
                "needs_frame": turn_needs_frame(session_mode)
            }),
        )
        .await?;
    }
    cancel_gate(pending_gate);
    *gate_generation += 1;
    *pending_gate = Some(spawn_voice_gate(
        state.orchestrator.clone(),
        session_id.to_owned(),
        turn_id.to_owned(),
        *gate_generation,
        response_id,
        transcript,
        session_mode,
        gate_results.clone(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header::AUTHORIZATION},
    };
    use ripple_agent_gateway::{
        adapters::ModelAdapters,
        memory::{CreateMemoryRequest, CreateTodoRequest as MemoryCreateTodoRequest},
        readiness::check as check_readiness,
    };
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn late_evaluation_result_cannot_finalize_a_resumed_turn() {
        let mut state = EndpointState::speaking("turn-1");
        state.pause("turn-1");
        state.resume("turn-1");

        assert!(!state.accepts_result("turn-1"));
    }

    #[test]
    fn tentative_pause_keeps_the_complete_turn_audio_when_more_audio_arrives() {
        let mut settings = Settings::from_env().unwrap();
        settings.sample_rate_in = 100;
        settings.max_audio_seconds = 30;
        let mut state = EndpointState::speaking("turn-1");
        state.audio = vec![0.1; 60];
        state.pause("turn-1");
        let encoded = STANDARD.encode(
            [0.2_f32; 10]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );

        state.append_audio(Some(&encoded), &settings).unwrap();
        state.resume("turn-1");

        assert_eq!(state.audio.len(), 70);
    }

    #[test]
    fn older_pause_result_is_stale_even_for_same_turn_id() {
        assert!(!endpoint_result_matches("turn-1", 2, "turn-1", 1, true));
        assert!(endpoint_result_matches("turn-1", 2, "turn-1", 2, true));
    }

    #[tokio::test]
    async fn blocking_fallback_asr_does_not_block_client_input() {
        let (_endpoint_sender, mut endpoint_results) = mpsc::channel(1);
        let (transcription_sender, mut transcription_results) = mpsc::channel(1);
        let (_gate_sender, mut gate_results) = mpsc::channel(1);
        let mut pending = Some(spawn_transcription_task(
            "turn-2".to_owned(),
            7,
            "response-7".to_owned(),
            SessionMode::Audio,
            transcription_sender,
            std::future::pending::<anyhow::Result<String>>(),
        ));
        let mut client_events = futures_util::stream::iter(["input.clear"]);

        let event = tokio::time::timeout(
            Duration::from_millis(50),
            next_realtime_input(
                &mut client_events,
                &mut endpoint_results,
                &mut transcription_results,
                &mut gate_results,
            ),
        )
        .await
        .expect("client input must remain responsive while ASR is pending");

        assert!(matches!(event, RealtimeInput::Socket(Some("input.clear"))));
        assert!(!pending.as_ref().unwrap().handle.is_finished());
        cancel_transcription(&mut pending);
    }

    #[tokio::test]
    async fn blocking_gate_does_not_block_cancel_or_clear_input() {
        let (_endpoint_sender, mut endpoint_results) = mpsc::channel(1);
        let (_transcription_sender, mut transcription_results) = mpsc::channel(1);
        let (gate_sender, mut gate_results) = mpsc::channel(1);
        let mut pending = Some(spawn_gate_task(
            "turn-gate".to_owned(),
            9,
            "response-gate".to_owned(),
            "请继续".to_owned(),
            SessionMode::Audio,
            gate_sender,
            std::future::pending(),
        ));
        let mut client_events = futures_util::stream::iter(["response.cancel", "input.clear"]);

        for expected in ["response.cancel", "input.clear"] {
            let event = tokio::time::timeout(
                Duration::from_millis(50),
                next_realtime_input(
                    &mut client_events,
                    &mut endpoint_results,
                    &mut transcription_results,
                    &mut gate_results,
                ),
            )
            .await
            .expect("client input must remain responsive while the gate is pending");
            assert!(matches!(event, RealtimeInput::Socket(Some(kind)) if kind == expected));
        }

        assert!(!pending.as_ref().unwrap().handle.is_finished());
        cancel_gate(&mut pending);
    }

    #[tokio::test]
    async fn clear_input_aborts_speaking_turn_fallback_and_clears_captured_media() {
        let mut endpoint_state = EndpointState::speaking("turn-speaking");
        endpoint_state.audio = vec![0.1; 3200];
        let fallback_handle = tokio::spawn(std::future::pending::<()>());
        let fallback_abort = fallback_handle.abort_handle();
        let mut pending_transcription = Some(PendingTranscription {
            turn_id: "turn-speaking".to_owned(),
            generation: 4,
            response_id: "response-speaking".to_owned(),
            mode: SessionMode::Audio,
            handle: fallback_handle,
        });
        let mut pending_endpoint = None;
        let gate_handle = tokio::spawn(std::future::pending::<()>());
        let gate_abort = gate_handle.abort_handle();
        let mut pending_gate = Some(PendingGate {
            turn_id: "turn-speaking".to_owned(),
            generation: 5,
            response_id: "response-speaking".to_owned(),
            mode: SessionMode::Audio,
            handle: gate_handle,
        });
        let mut frames = VecDeque::from([VideoFrame {
            bytes: vec![1, 2, 3],
            mime_type: "image/jpeg".to_owned(),
            captured_at_ms: None,
            received_at_ms: 0,
        }]);
        let mut pending_turn = Some(PendingTurn {
            response_id: "response-speaking".to_owned(),
            transcript: "旧请求".to_owned(),
        });

        clear_input_state(
            &mut endpoint_state,
            &mut pending_endpoint,
            &mut pending_transcription,
            &mut pending_gate,
            &mut frames,
            &mut pending_turn,
        );
        tokio::task::yield_now().await;

        assert!(endpoint_state.audio.is_empty());
        assert!(endpoint_state.active_turn_id.is_none());
        assert!(pending_transcription.is_none());
        assert!(fallback_abort.is_finished());
        assert!(pending_gate.is_none());
        assert!(gate_abort.is_finished());
        assert!(frames.is_empty());
        assert!(pending_turn.is_none());
    }

    #[tokio::test]
    async fn clear_input_aborts_pause_evaluation_and_clears_captured_media() {
        let mut endpoint_state = EndpointState::speaking("turn-paused");
        endpoint_state.audio = vec![0.1; 2400];
        assert!(endpoint_state.pause("turn-paused"));
        let endpoint_handle = tokio::spawn(std::future::pending::<()>());
        let endpoint_abort = endpoint_handle.abort_handle();
        let mut pending_endpoint = Some(PendingEndpoint {
            turn_id: "turn-paused".to_owned(),
            generation: 5,
            handle: endpoint_handle,
        });
        let mut pending_transcription = None;
        let mut pending_gate = None;
        let mut frames = VecDeque::new();
        let mut pending_turn = None;

        clear_input_state(
            &mut endpoint_state,
            &mut pending_endpoint,
            &mut pending_transcription,
            &mut pending_gate,
            &mut frames,
            &mut pending_turn,
        );
        tokio::task::yield_now().await;

        assert!(endpoint_state.audio.is_empty());
        assert!(endpoint_state.active_turn_id.is_none());
        assert!(pending_endpoint.is_none());
        assert!(endpoint_abort.is_finished());
    }

    #[test]
    fn fallback_transcription_requires_matching_turn_generation_and_response() {
        assert!(transcription_result_matches(
            "turn-2",
            7,
            "response-7",
            "turn-2",
            7,
            "response-7",
        ));
        assert!(!transcription_result_matches(
            "turn-2",
            7,
            "response-7",
            "turn-2",
            6,
            "response-7",
        ));
        assert!(!transcription_result_matches(
            "turn-2",
            7,
            "response-7",
            "turn-stale",
            7,
            "response-7",
        ));
        assert!(!transcription_result_matches(
            "turn-2",
            7,
            "response-7",
            "turn-2",
            7,
            "response-stale",
        ));
    }

    #[test]
    fn stop_command_clears_audio_without_spawning_agent_turn() {
        let mut state = EndpointState::speaking("turn-2");
        state.audio = vec![0.1; 1600];

        assert!(state.consume_stop("turn-2", "不要说了"));
        assert!(state.audio.is_empty());
        assert!(state.evaluation.is_none());
    }

    #[test]
    fn endpoint_observability_records_audio_decision_latency_and_client_fallback() {
        let evaluation = EndpointEvaluation {
            transcript: "请继续".to_owned(),
            decision: ripple_agent_gateway::endpointing::EndpointDecision::Complete,
            reason: "classifier",
            classifier_latency_ms: Some(12),
        };
        let audio_ms = audio_duration_ms(2400, 16_000);

        let evaluated = endpoint_evaluated_payload("turn-observed", audio_ms, &evaluation);
        assert_eq!(evaluated["audio_duration_ms"], 150);
        assert_eq!(evaluated["decision"], "complete");
        assert_eq!(evaluated["reason"], "classifier");
        assert_eq!(evaluated["classifier_latency_ms"], 12);

        let committed =
            endpoint_committed_payload("turn-observed", audio_ms, Some(&evaluation), true);
        assert_eq!(committed["audio_duration_ms"], 150);
        assert_eq!(committed["decision"], "complete");
        assert_eq!(committed["reason"], "classifier");
        assert_eq!(committed["classifier_latency_ms"], 12);
        assert_eq!(committed["client_fallback_finalized"], true);
    }

    #[test]
    fn client_commit_fallback_marker_and_stop_command_kind_are_explicit() {
        let fallback_commit: ClientEvent = serde_json::from_value(json!({
            "type": "input.commit",
            "turn_id": "turn-fallback",
            "endpoint_fallback": true
        }))
        .unwrap();
        assert!(client_endpoint_fallback(&fallback_commit));

        let stop = stop_command_payload("turn-stop", "停一下");
        assert_eq!(stop["turn_id"], "turn-stop");
        assert_eq!(stop["command"], "stop");
        assert_eq!(stop["transcript_chars"], 3);
    }

    #[test]
    fn force_cancel_invalidates_input_before_a_queued_commit_arrives() {
        let force_cancel: ClientEvent = serde_json::from_value(json!({
            "type": "response.cancel",
            "clear_input": true
        }))
        .unwrap();
        let mut endpoint_state = EndpointState::speaking("turn-queued");
        endpoint_state.audio = vec![0.1; 1600];
        let mut pending_endpoint = None;
        let mut pending_transcription = None;
        let mut pending_gate = None;
        let mut frames = VecDeque::new();
        let mut pending_turn = None;

        assert!(client_cancel_clears_input(&force_cancel));
        clear_input_state(
            &mut endpoint_state,
            &mut pending_endpoint,
            &mut pending_transcription,
            &mut pending_gate,
            &mut frames,
            &mut pending_turn,
        );
        assert!(endpoint_state.take_commit("turn-queued").is_none());
    }

    #[test]
    fn cancelling_pending_video_turn_clears_frames_and_correlation() {
        let mut pending_turn = Some(PendingTurn {
            response_id: "response-video-old".to_owned(),
            transcript: "看一下这个".to_owned(),
        });
        let mut frames = VecDeque::from([VideoFrame {
            bytes: vec![1, 2, 3],
            mime_type: "image/jpeg".to_owned(),
            captured_at_ms: None,
            received_at_ms: 0,
        }]);

        let cancelled = take_pending_video_turn(&mut pending_turn, &mut frames);

        assert_eq!(
            cancelled.map(|turn| turn.response_id),
            Some("response-video-old".to_owned())
        );
        assert!(pending_turn.is_none());
        assert!(frames.is_empty());
    }

    #[test]
    fn failed_response_is_public_and_correlated() {
        let event = failed_response_event(
            "response-9",
            &anyhow::anyhow!("AGENT_FAILED: upstream included a private body"),
        );

        assert_eq!(event["type"], "response.failed");
        assert_eq!(event["response_id"], "response-9");
        assert_eq!(event["code"], "agent_unavailable");
        assert_eq!(event["message"], "Agent 服务暂时不可用");
        assert!(!event.to_string().contains("private body"));
    }

    #[test]
    fn failed_response_reports_recoverable_empty_agent_output() {
        let event = failed_response_event(
            "response-empty",
            &anyhow::anyhow!("AGENT_EMPTY_RESPONSE: upstream returned no text after retry"),
        );

        assert_eq!(event["code"], "agent_empty_response");
        assert_eq!(event["message"], "模型暂未生成回复，请重试");
    }

    #[test]
    fn failed_response_distinguishes_an_agent_request_rejection() {
        let event = failed_response_event(
            "response-10",
            &anyhow::anyhow!("AGENT_REQUEST_REJECTED status=400 summary=249 validation errors:"),
        );

        assert_eq!(event["code"], "agent_request_rejected");
        assert_eq!(event["message"], "Agent 请求格式不兼容");
        assert!(!event.to_string().contains("validation errors"));
    }

    #[test]
    fn playback_start_accepts_only_the_active_response_and_clamps_buffering() {
        assert_eq!(
            normalize_playback_started(Some("response-1"), Some("response-1"), Some(50_000)),
            Some(("response-1".to_owned(), 10_000))
        );
        assert_eq!(
            normalize_playback_started(Some("response-1"), Some("stale"), Some(450)),
            None
        );
        assert_eq!(
            normalize_playback_started(None, Some("response-1"), Some(450)),
            None
        );
    }

    #[tokio::test]
    async fn mock_backends_and_database_are_ready() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.asr_backend = "mock".to_owned();
        settings.agent_backend = "mock".to_owned();
        settings.tts_backend = "mock".to_owned();
        let context = ContextStore::open_test().await.unwrap();

        let report = check_readiness(&settings, &context).await;

        assert!(report.ok);
        assert!(report.dependencies.values().all(|dependency| dependency.ok));
    }

    #[tokio::test]
    async fn unreachable_agent_makes_readiness_fail_but_liveness_stays_ok() {
        let (_directory, mut state, _token, _conversation, _foreign) = test_state().await;
        Arc::make_mut(&mut state.settings).agent_backend = "openai".to_owned();
        Arc::make_mut(&mut state.settings).agent_readiness_url =
            "http://127.0.0.1:1/v1/models".to_owned();

        let live = app(state.clone())
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let ready = app(state)
            .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(live.status(), StatusCode::OK);
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json_body(ready).await["dependencies"]["agent"]["error"],
            "unreachable"
        );
    }

    #[test]
    fn realtime_protocol_accepts_v4_and_v5_during_rollout() {
        assert_eq!(validate_protocol_version(Some(4)), Ok(4));
        assert_eq!(validate_protocol_version(Some(5)), Ok(5));
        assert_eq!(validate_protocol_version(None), Err(()));
        assert_eq!(validate_protocol_version(Some(3)), Err(()));
        assert_eq!(validate_protocol_version(Some(6)), Err(()));
    }

    #[test]
    fn v4_mode_change_rejection_preserves_request_correlation() {
        let event: ClientEvent = serde_json::from_value(json!({
            "type": "session.mode.set",
            "response_id": "mode-request-4",
            "mode": "video"
        }))
        .unwrap();

        let error = unsupported_mode_change_event(4, &event).unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "unsupported_protocol");
        assert_eq!(error["response_id"], "mode-request-4");
        assert_eq!(error["mode"], "video");
        assert!(unsupported_mode_change_event(5, &event).is_none());

        let recoverable = validate_mode_set(4, &event).unwrap_err();
        assert_eq!(recoverable["code"], "unsupported_protocol");
        assert_eq!(recoverable["response_id"], "mode-request-4");
    }

    #[test]
    fn invalid_v5_mode_is_a_correlated_recoverable_error() {
        let event: ClientEvent = serde_json::from_value(json!({
            "type": "session.mode.set",
            "response_id": "mode-request-invalid",
            "mode": "continuous_video"
        }))
        .unwrap();

        let error = validate_mode_set(5, &event).unwrap_err();
        assert_eq!(error["code"], "invalid_mode");
        assert_eq!(error["response_id"], "mode-request-invalid");
        assert_eq!(error["mode"], "continuous_video");
    }

    #[test]
    fn switching_video_to_audio_releases_pending_turn_without_frames() {
        let pending = PendingTurn {
            response_id: "response-1".to_owned(),
            transcript: "继续回答".to_owned(),
        };
        let decision = plan_mode_change(SessionMode::Video, SessionMode::Audio, Some(&pending));
        assert_eq!(decision, ModeChangePlan::ReleasePendingAudioTurn);
    }

    #[test]
    fn switching_audio_to_video_only_changes_future_turns() {
        assert_eq!(
            plan_mode_change(SessionMode::Audio, SessionMode::Video, None),
            ModeChangePlan::ChangeOnly,
        );
        assert_eq!(
            plan_mode_change(SessionMode::Audio, SessionMode::Audio, None),
            ModeChangePlan::Unchanged,
        );
    }

    #[test]
    fn in_flight_turns_never_gain_a_camera_requirement_after_mode_change() {
        let old_audio_turn =
            mode_snapshot_after_change(SessionMode::Audio, SessionMode::Audio, SessionMode::Video);
        let old_video_turn =
            mode_snapshot_after_change(SessionMode::Video, SessionMode::Video, SessionMode::Audio);

        assert_eq!(old_audio_turn, SessionMode::Audio);
        assert_eq!(old_video_turn, SessionMode::Audio);
        assert!(!turn_needs_frame(old_audio_turn));
        assert!(!turn_needs_frame(old_video_turn));
        assert!(turn_needs_frame(SessionMode::Video));
        assert!(requested_frame_event(old_audio_turn, "response-old").is_none());
        assert_eq!(
            requested_frame_event(SessionMode::Video, "response-new").unwrap(),
            json!({
                "type": "input.frame.requested",
                "response_id": "response-new"
            })
        );
    }

    fn mode_set_event(mode: &str, response_id: &str) -> ClientEvent {
        serde_json::from_value(json!({
            "type": "session.mode.set",
            "response_id": response_id,
            "mode": mode
        }))
        .unwrap()
    }

    #[test]
    fn duplicate_mode_event_acks_without_changing_or_closing_session() {
        let mut mode = SessionMode::Audio;
        let mut pending_turn = None;
        let mut frames = VecDeque::new();
        let effects = apply_mode_set_transaction(
            5,
            &mode_set_event("audio", "duplicate"),
            &mut mode,
            None,
            None,
            &mut pending_turn,
            &mut frames,
        );

        assert_eq!(mode, SessionMode::Audio);
        assert!(matches!(
            effects[0],
            ModeChangeEffect::PreserveActiveResponse
        ));
        assert!(matches!(
            &effects[1],
            ModeChangeEffect::Send(event)
                if event["type"] == "session.mode.changed" && event["mode"] == "audio"
        ));
    }

    #[test]
    fn recoverable_mode_errors_allow_the_next_valid_event() {
        let mut mode = SessionMode::Audio;
        let mut pending_turn = None;
        let mut frames = VecDeque::new();

        let v4 = apply_mode_set_transaction(
            4,
            &mode_set_event("video", "v4-request"),
            &mut mode,
            None,
            None,
            &mut pending_turn,
            &mut frames,
        );
        assert!(matches!(
            &v4[1],
            ModeChangeEffect::Send(event)
                if event["code"] == "unsupported_protocol"
                    && event["response_id"] == "v4-request"
        ));
        assert_eq!(mode, SessionMode::Audio);

        let invalid = apply_mode_set_transaction(
            5,
            &mode_set_event("continuous_video", "invalid-request"),
            &mut mode,
            None,
            None,
            &mut pending_turn,
            &mut frames,
        );
        assert!(matches!(
            &invalid[1],
            ModeChangeEffect::Send(event)
                if event["code"] == "invalid_mode"
                    && event["response_id"] == "invalid-request"
        ));

        let valid = apply_mode_set_transaction(
            5,
            &mode_set_event("video", "valid-request"),
            &mut mode,
            None,
            None,
            &mut pending_turn,
            &mut frames,
        );
        assert_eq!(mode, SessionMode::Video);
        assert!(matches!(
            &valid[1],
            ModeChangeEffect::Send(event)
                if event["type"] == "session.mode.changed" && event["mode"] == "video"
        ));
    }

    #[test]
    fn pending_video_release_is_effected_without_frames_before_ack() {
        let mut mode = SessionMode::Video;
        let mut pending_turn = Some(PendingTurn {
            response_id: "response-pending".to_owned(),
            transcript: "继续回答".to_owned(),
        });
        let mut frames = VecDeque::from([VideoFrame {
            bytes: vec![1, 2, 3],
            mime_type: "image/jpeg".to_owned(),
            captured_at_ms: None,
            received_at_ms: 0,
        }]);

        let effects = apply_mode_set_transaction(
            5,
            &mode_set_event("audio", "close-camera"),
            &mut mode,
            None,
            None,
            &mut pending_turn,
            &mut frames,
        );

        assert!(matches!(
            effects[0],
            ModeChangeEffect::PreserveActiveResponse
        ));
        assert!(matches!(
            &effects[1],
            ModeChangeEffect::SpawnAudioTurn { response_id, frames, .. }
                if response_id == "response-pending" && frames.is_empty()
        ));
        assert!(matches!(
            &effects[2],
            ModeChangeEffect::Send(event)
                if event["type"] == "session.mode.changed" && event["mode"] == "audio"
        ));
        assert!(pending_turn.is_none());
        assert!(frames.is_empty());
    }

    #[test]
    fn audio_to_video_keeps_old_turn_audio_and_requests_only_for_next_turn() {
        let mut mode = SessionMode::Audio;
        let mut old_gate_mode = SessionMode::Audio;
        let mut pending_turn = None;
        let mut frames = VecDeque::new();
        let _ = apply_mode_set_transaction(
            5,
            &mode_set_event("video", "open-camera"),
            &mut mode,
            None,
            Some(&mut old_gate_mode),
            &mut pending_turn,
            &mut frames,
        );

        assert!(matches!(
            plan_accepted_turn(old_gate_mode, "response-old", "旧回合"),
            AcceptedTurnAction::SpawnAudioTurn { .. }
        ));
        assert!(matches!(
            plan_accepted_turn(mode, "response-new", "新回合"),
            AcceptedTurnAction::RequestFrame { event, .. }
                if event["type"] == "input.frame.requested"
                    && event["response_id"] == "response-new"
        ));
    }

    #[test]
    fn pending_frame_requires_exact_response_id() {
        assert_eq!(
            correlate_pending_frame("response-current", Some("response-current")),
            FrameCorrelation::Matched("response-current".to_owned())
        );
        assert_eq!(
            correlate_pending_frame("response-current", None),
            FrameCorrelation::Stale
        );
        assert_eq!(
            correlate_pending_frame("response-current", Some("response-stale")),
            FrameCorrelation::Stale
        );
    }

    async fn test_state() -> (TempDir, AppState, String, String, String) {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = Settings::from_env().unwrap();
        settings.data_dir = directory.path().join("data");
        settings.skills_dir = directory.path().join("skills");
        settings.agent_backend = "mock".to_owned();
        settings.asr_backend = "mock".to_owned();
        settings.tts_backend = "mock".to_owned();
        tokio::fs::create_dir_all(&settings.skills_dir)
            .await
            .unwrap();
        let settings = Arc::new(settings);
        let context = ContextStore::open_test().await.unwrap();
        context
            .seed_invitation_codes(&["route-one".to_owned(), "route-two".to_owned()], 1, 24)
            .await
            .unwrap();
        let (user, token) = context
            .register_user("route@example.com", "password-route", "route-one", 24)
            .await
            .unwrap();
        let (other, _) = context
            .register_user("other-route@example.com", "password-route", "route-two", 24)
            .await
            .unwrap();
        let conversation = context.create_conversation(&user.id).await.unwrap();
        context
            .add_turn(&conversation, "user", "蓝色转接头放在哪里", None)
            .await
            .unwrap();
        let foreign = context.create_conversation(&other.id).await.unwrap();
        let memories = MemoryService::new(context.clone(), settings.data_dir.join("assets"))
            .await
            .unwrap();
        let orchestrator = AgentOrchestrator::new(
            Arc::clone(&settings),
            context.clone(),
            ModelAdapters::new((*settings).clone()).unwrap(),
            memories.clone(),
        )
        .unwrap();
        (
            directory,
            AppState {
                settings,
                context,
                memories,
                orchestrator,
            },
            token,
            conversation,
            foreign,
        )
    }

    fn authenticated_request(method: &str, uri: &str, token: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn avatar_route_persists_and_serves_the_authenticated_users_image() {
        let (_directory, state, token, _, _) = test_state().await;
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 8)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();

        let uploaded = app(state.clone())
            .oneshot(
                Request::put("/v1/auth/me/avatar")
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header(axum::http::header::CONTENT_TYPE, "image/jpeg")
                    .body(Body::from(jpeg.into_inner()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::OK);
        let avatar_url = json_body(uploaded).await["user"]["avatar_url"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(avatar_url.starts_with("/v1/assets/"));

        let me = app(state.clone())
            .oneshot(authenticated_request("GET", "/v1/auth/me", &token, ""))
            .await
            .unwrap();
        assert_eq!(json_body(me).await["user"]["avatar_url"], avatar_url);

        let image = app(state.clone())
            .oneshot(authenticated_request("GET", &avatar_url, &token, ""))
            .await
            .unwrap();
        assert_eq!(image.status(), StatusCode::OK);
        assert_eq!(
            image.headers()[axum::http::header::CONTENT_TYPE],
            "image/jpeg"
        );

        let cleared = app(state)
            .oneshot(authenticated_request(
                "DELETE",
                "/v1/auth/me/avatar",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(cleared.status(), StatusCode::OK);
        assert!(json_body(cleared).await["user"]["avatar_url"].is_null());
    }

    #[tokio::test]
    async fn exact_conversation_route_authenticates_and_does_not_leak() {
        let (_directory, state, token, conversation, foreign) = test_state().await;

        let unauthorized = app(state.clone())
            .oneshot(
                Request::get(format!("/v1/conversations/{conversation}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let listed = app(state.clone())
            .oneshot(authenticated_request(
                "GET",
                "/v1/conversations?scope=all",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = json_body(listed).await["data"][0].clone();

        let exact = app(state.clone())
            .oneshot(authenticated_request(
                "GET",
                &format!("/v1/conversations/{conversation}"),
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(exact.status(), StatusCode::OK);
        assert_eq!(json_body(exact).await["data"], listed);

        for inaccessible in [foreign, "missing-conversation".to_owned()] {
            let response = app(state.clone())
                .oneshot(authenticated_request(
                    "GET",
                    &format!("/v1/conversations/{inaccessible}"),
                    &token,
                    "",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(json_body(response).await["error"]["message"], "对话不存在");
        }

        let malformed = app(state)
            .oneshot(authenticated_request(
                "GET",
                "/v1/conversations/%FF",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert!(malformed.status().is_client_error());
        let malformed_body = to_bytes(malformed.into_body(), usize::MAX).await.unwrap();
        assert!(!String::from_utf8_lossy(&malformed_body).contains(&conversation));
    }

    #[tokio::test]
    async fn conversation_routes_auth_filter_search_and_mutate_atomically() {
        let (_directory, state, token, conversation, foreign) = test_state().await;

        let unauthorized = app(state.clone())
            .oneshot(
                Request::patch(format!("/v1/conversations/{conversation}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"is_pinned":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let pinned = app(state.clone())
            .oneshot(authenticated_request(
                "PATCH",
                &format!("/v1/conversations/{conversation}"),
                &token,
                r#"{"is_pinned":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(pinned.status(), StatusCode::OK);
        assert_eq!(json_body(pinned).await["data"]["is_pinned"], true);

        let foreign_patch = app(state.clone())
            .oneshot(authenticated_request(
                "PATCH",
                &format!("/v1/conversations/{foreign}"),
                &token,
                r#"{"archived":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(foreign_patch.status(), StatusCode::NOT_FOUND);

        let mixed = app(state.clone())
            .oneshot(authenticated_request(
                "POST",
                "/v1/conversations/batch",
                &token,
                &serde_json::json!({
                    "ids": [conversation, foreign],
                    "action": "archive"
                })
                .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(mixed.status(), StatusCode::NOT_FOUND);

        let active = app(state.clone())
            .oneshot(authenticated_request(
                "GET",
                "/v1/conversations?scope=active&query=%E8%BD%AC%E6%8E%A5%E5%A4%B4",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(active.status(), StatusCode::OK);
        assert_eq!(json_body(active).await["data"].as_array().unwrap().len(), 1);

        let empty = app(state.clone())
            .oneshot(authenticated_request(
                "POST",
                "/v1/conversations/batch",
                &token,
                r#"{"ids":[],"action":"delete"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

        let archived = app(state.clone())
            .oneshot(authenticated_request(
                "PATCH",
                &format!("/v1/conversations/{conversation}"),
                &token,
                r#"{"archived":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(archived.status(), StatusCode::OK);
        let archived_list = app(state.clone())
            .oneshot(authenticated_request(
                "GET",
                "/v1/conversations?scope=archived",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(
            json_body(archived_list).await["data"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let deleted = app(state)
            .oneshot(authenticated_request(
                "DELETE",
                &format!("/v1/conversations/{conversation}"),
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn profile_routes_persist_and_require_authentication() {
        let (_directory, state, token, _, _) = test_state().await;

        let unauthorized = app(state.clone())
            .oneshot(Request::get("/v1/profile").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let empty = app(state.clone())
            .oneshot(authenticated_request("GET", "/v1/profile", &token, ""))
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::OK);
        assert_eq!(json_body(empty).await["data"]["preferred_name"], "");

        let updated = app(state.clone())
            .oneshot(authenticated_request(
                "PUT",
                "/v1/profile",
                &token,
                r#"{
                    "ai_identity":"你是温柔直接的长期伙伴",
                    "user_identity":"我是独立开发者",
                    "preferred_name":"Lake",
                    "basic_memory":"回答先给结论"
                }"#,
            ))
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);
        let updated = json_body(updated).await;
        assert_eq!(updated["data"]["preferred_name"], "Lake");
        assert!(updated["data"]["updated_at"].is_number());

        let loaded = app(state)
            .oneshot(authenticated_request("GET", "/v1/profile", &token, ""))
            .await
            .unwrap();
        assert_eq!(loaded.status(), StatusCode::OK);
        assert_eq!(json_body(loaded).await["data"], updated["data"]);
    }

    #[tokio::test]
    async fn conversation_message_route_preserves_attachments_and_filters_actions() {
        let (_directory, state, token, conversation, _) = test_state().await;
        let user = state.context.authenticate(&token).await.unwrap().unwrap();
        let turn = state
            .context
            .add_turn(&conversation, "user", "记住插头并提醒我", None)
            .await
            .unwrap();
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 3)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let memory = state
            .memories
            .create(CreateMemoryRequest {
                user_id: user.id.clone(),
                conversation_id: conversation.clone(),
                source_turn_id: turn,
                response_id: "route-action-memory-response".to_owned(),
                tool_call_id: "route-action-memory-call".to_owned(),
                user_note: "蓝色插头在抽屉".to_owned(),
                visual_summary: "抽屉里的蓝色插头".to_owned(),
                frames: vec![VideoFrame {
                    bytes: jpeg.into_inner(),
                    mime_type: "image/jpeg".to_owned(),
                    captured_at_ms: Some(1_700_000_000_000),
                    received_at_ms: 1_700_000_000_100,
                }],
            })
            .await
            .unwrap();
        let cover = memory.memory.cover.clone().unwrap();
        state
            .memories
            .attach_to_turn(turn, std::slice::from_ref(&cover))
            .await
            .unwrap();
        let todo = state
            .memories
            .create_todo(MemoryCreateTodoRequest {
                user_id: user.id.clone(),
                conversation_id: conversation.clone(),
                source_turn_id: turn,
                response_id: "route-action-todo-response".to_owned(),
                tool_call_id: "route-action-todo-call".to_owned(),
                title: "周一带充电器".to_owned(),
                visual_summary: String::new(),
                due_at: Some(1_900_000_000.0),
                frames: vec![],
            })
            .await
            .unwrap();

        state
            .context
            .seed_invitation_codes(&["route-action-foreign".to_owned()], 1, 24)
            .await
            .unwrap();
        let (foreign_user, _) = state
            .context
            .register_user(
                "route-action-foreign@example.com",
                "password-route-foreign",
                "route-action-foreign",
                24,
            )
            .await
            .unwrap();
        state
            .memories
            .create(CreateMemoryRequest {
                user_id: foreign_user.id,
                conversation_id: conversation.clone(),
                source_turn_id: turn,
                response_id: "route-action-foreign-response".to_owned(),
                tool_call_id: "route-action-foreign-call".to_owned(),
                user_note: "不应泄漏".to_owned(),
                visual_summary: String::new(),
                frames: vec![],
            })
            .await
            .expect_err("foreign user must not attach memory to another user's conversation");

        let response = app(state)
            .oneshot(authenticated_request(
                "GET",
                &format!("/v1/conversations/{conversation}/messages"),
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let message = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["id"] == turn)
            .unwrap();
        assert_eq!(message["actions"].as_array().unwrap().len(), 2);
        assert_eq!(message["actions"][0]["kind"], "memory");
        assert_eq!(message["actions"][0]["target_id"], memory.memory.id);
        assert_eq!(message["actions"][1]["kind"], "todo");
        assert_eq!(message["actions"][1]["target_id"], todo.id);
        assert_eq!(message["actions"][1]["due_at"], 1_900_000_000.0);
        assert!(!message["actions"].to_string().contains("不应泄漏"));
        assert_eq!(message["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(message["attachments"][0]["id"], cover.id);
        assert_eq!(message["attachments"][0]["kind"], "image");
        assert_eq!(
            message["attachments"][0]["content_url"],
            format!("/v1/assets/{}/content", cover.id)
        );
    }

    #[tokio::test]
    async fn memory_routes_patch_filter_batch_and_delete() {
        let (_directory, state, token, conversation, _) = test_state().await;
        let turn = state
            .context
            .add_turn(&conversation, "user", "记住电源线", None)
            .await
            .unwrap();
        let memory = state
            .memories
            .create(CreateMemoryRequest {
                user_id: state
                    .context
                    .authenticate(&token)
                    .await
                    .unwrap()
                    .unwrap()
                    .id,
                conversation_id: conversation,
                source_turn_id: turn,
                response_id: "route-memory-response".to_owned(),
                tool_call_id: "route-memory-call".to_owned(),
                user_note: "白色电源线".to_owned(),
                visual_summary: "电源线放在桌面".to_owned(),
                frames: vec![],
            })
            .await
            .unwrap()
            .memory;

        let patched = app(state.clone())
            .oneshot(authenticated_request(
                "PATCH",
                &format!("/v1/memories/{}", memory.id),
                &token,
                r#"{"user_note":"白色 USB 电源线","is_pinned":true,"archived":true}"#,
            ))
            .await
            .unwrap();
        assert_eq!(patched.status(), StatusCode::OK);
        let patched = json_body(patched).await;
        assert_eq!(patched["data"]["user_note"], "白色 USB 电源线");
        assert_eq!(patched["data"]["is_pinned"], false);
        assert!(patched["data"]["archived_at"].is_number());

        let archived = app(state.clone())
            .oneshot(authenticated_request(
                "GET",
                "/v1/memories?scope=archived&query=USB",
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(
            json_body(archived).await["data"].as_array().unwrap().len(),
            1
        );

        let empty = app(state.clone())
            .oneshot(authenticated_request(
                "POST",
                "/v1/memories/batch",
                &token,
                r#"{"ids":[],"action":"archive"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

        let deleted = app(state)
            .oneshot(authenticated_request(
                "DELETE",
                &format!("/v1/memories/{}", memory.id),
                &token,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }
}

fn append_audio(
    output: &mut Vec<f32>,
    encoded: Option<&str>,
    retain_full_turn: bool,
    settings: &Settings,
) -> anyhow::Result<()> {
    let payload = STANDARD.decode(encoded.ok_or_else(|| anyhow::anyhow!("音频缺少 audio"))?)?;
    let samples = decode_le_f32(&payload)?;
    if retain_full_turn {
        let max_samples = settings.max_audio_seconds * settings.sample_rate_in as usize;
        if output.len() + samples.len() > max_samples {
            anyhow::bail!("单轮音频超过最大时长");
        }
        output.extend(samples);
    } else {
        output.extend(samples);
        keep_pre_roll(output, settings.sample_rate_in as usize / 2);
    }
    Ok(())
}

fn keep_pre_roll(audio: &mut Vec<f32>, limit: usize) {
    if audio.len() > limit {
        audio.drain(..audio.len() - limit);
    }
}

fn failed_response_event(response_id: &str, error: &anyhow::Error) -> Value {
    let chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    let (code, message) = if chain
        .iter()
        .any(|item| item.contains("AGENT_REQUEST_REJECTED"))
    {
        ("agent_request_rejected", "Agent 请求格式不兼容")
    } else if chain.iter().any(|item| item.contains("ASR_FAILED")) {
        ("asr_failed", "语音识别暂时不可用")
    } else if chain
        .iter()
        .any(|item| item.contains("AGENT_EMPTY_RESPONSE"))
    {
        ("agent_empty_response", "模型暂未生成回复，请重试")
    } else if chain.iter().any(|item| item.contains("AGENT_FAILED")) {
        ("agent_unavailable", "Agent 服务暂时不可用")
    } else if chain.iter().any(|item| item.contains("TTS_FAILED")) {
        ("tts_failed", "语音合成暂时不可用")
    } else {
        ("internal_error", "本次处理失败，请重试")
    };
    json!({
        "type": "response.failed",
        "response_id": response_id,
        "code": code,
        "message": message
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_turn(
    orchestrator: AgentOrchestrator,
    context: ContextStore,
    user_id: String,
    session_id: String,
    sender: mpsc::Sender<Value>,
    audio: Vec<f32>,
    frames: Vec<VideoFrame>,
    transcript: Option<String>,
    response_id: String,
) -> ActiveResponse {
    let task_response_id = response_id.clone();
    let handle = tokio::spawn(async move {
        if let Err(error) = orchestrator
            .run_turn(
                &user_id,
                &session_id,
                &sender,
                audio,
                frames,
                transcript,
                &task_response_id,
            )
            .await
        {
            if let Some(rejection) = error
                .chain()
                .map(ToString::to_string)
                .find(|item| item.contains("AGENT_REQUEST_REJECTED"))
            {
                error!(%session_id, upstream_rejection = %rejection, "turn failed");
            } else {
                error!(%session_id, %error, "turn failed");
            }
            record_event_best_effort(
                &context,
                &session_id,
                "server.turn.failed",
                &json!({
                    "response_id": task_response_id,
                    "error": error.to_string()
                }),
            )
            .await;
            let _ = send_event(&sender, failed_response_event(&task_response_id, &error)).await;
        }
    });
    ActiveResponse {
        id: response_id,
        handle,
    }
}

async fn cancel_response(
    active: &mut Option<ActiveResponse>,
    sender: &mpsc::Sender<Value>,
    context: &ContextStore,
    session_id: &str,
    reason: &str,
) {
    if let Some(response) = active.take()
        && !response.handle.is_finished()
    {
        response.handle.abort();
        info!(%session_id, response_id = %response.id, %reason, "response cancelled");
        record_event_best_effort(
            context,
            session_id,
            "server.response.cancelled",
            &json!({"response_id": response.id, "reason": reason}),
        )
        .await;
        let _ = send_event(
            sender,
            json!({
                "type": "response.cancelled",
                "response_id": response.id
            }),
        )
        .await;
    }
}

async fn record_event_best_effort(
    context: &ContextStore,
    session_id: &str,
    kind: &str,
    payload: &Value,
) {
    if let Err(error) = context.record_event(session_id, kind, payload).await {
        warn!(%session_id, %kind, %error, "failed to record server event");
    }
}

async fn send_event(sender: &mpsc::Sender<Value>, event: Value) -> anyhow::Result<()> {
    sender
        .send(event)
        .await
        .map_err(|_| anyhow::anyhow!("client disconnected"))
}
