use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use ripple_agent_gateway::{
    adapters::ModelAdapters,
    audio::decode_le_f32,
    config::Settings,
    context::ContextStore,
    orchestrator::AgentOrchestrator,
    protocol::{ClientEvent, VideoFrame},
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

#[derive(Clone)]
struct AppState {
    settings: Arc<Settings>,
    context: ContextStore,
    orchestrator: AgentOrchestrator,
}

struct ActiveResponse {
    id: String,
    handle: JoinHandle<()>,
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
    let context = ContextStore::open(&settings.database_path()).await?;
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
    let orchestrator = AgentOrchestrator::new(Arc::clone(&settings), context.clone(), adapters)?;
    let state = AppState {
        settings: Arc::clone(&settings),
        context,
        orchestrator,
    };
    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static("*"))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(tower_http::cors::Any);
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/me", get(me))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/conversations", get(list_conversations))
        .route(
            "/v1/conversations/{conversation_id}/messages",
            get(conversation_messages),
        )
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
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(settings.address).await?;
    info!(address = %settings.address, "Ripple Rust Agent Gateway started");
    axum::serve(listener, app).await?;
    Ok(())
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

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return api_error(StatusCode::UNAUTHORIZED, "需要登录");
    };
    if let Err(error) = state.context.revoke_token(token).await {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state
        .context
        .list_conversations(&user.id, query.limit.unwrap_or(50))
        .await
    {
        Ok(conversations) => Json(json!({"data": conversations})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
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
    match state
        .orchestrator
        .run_text_response(&conversation_id, &input)
        .await
    {
        Ok(output) => {
            let response_id = format!("resp_{}", Uuid::new_v4().simple());
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

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn realtime(
    websocket: WebSocketUpgrade,
    Query(query): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Response {
    let Some(token) = query.get("access_token") else {
        return api_error(StatusCode::UNAUTHORIZED, "需要登录");
    };
    let user = match state.context.authenticate(token).await {
        Ok(Some(user)) => user,
        Ok(None) => return api_error(StatusCode::UNAUTHORIZED, "登录已失效，请重新登录"),
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
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
        .on_upgrade(move |socket| handle_socket(socket, state, conversation_id))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: AppState, session_id: String) {
    if let Err(error) = state.context.touch_session(&session_id).await {
        error!(%session_id, %error, "failed to initialize session");
        return;
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
        return;
    }

    let mut audio = Vec::<f32>::new();
    let mut frames = VecDeque::<VideoFrame>::with_capacity(state.settings.max_frames);
    let mut speech_active = false;
    let mut active_response: Option<ActiveResponse> = None;

    while let Some(message) = ws_receiver.next().await {
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

        let result = match event.kind.as_str() {
            "session.start" => {
                let result = send_event(
                    &event_sender,
                    json!({"type": "session.ready", "session_id": session_id}),
                )
                .await;
                if result.is_ok() {
                    info!(%session_id, "session ready");
                    record_event_best_effort(
                        &state.context,
                        &session_id,
                        "server.session.ready",
                        &json!({"mode": event.extra.get("mode")}),
                    )
                    .await;
                }
                result
            }
            "input.audio.append" => {
                let result = append_audio(
                    &mut audio,
                    event.audio.as_deref(),
                    speech_active,
                    &state.settings,
                );
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
                match event
                    .image
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("视频帧缺少 image"))
                    .and_then(|encoded| STANDARD.decode(encoded).map_err(Into::into))
                {
                    Ok(bytes) => {
                        if frames.len() == state.settings.max_frames {
                            frames.pop_front();
                        }
                        frames.push_back(VideoFrame {
                            bytes,
                            mime_type: event.mime_type.unwrap_or_else(|| "image/jpeg".to_owned()),
                        });
                        Ok(())
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
                speech_active = true;
                keep_pre_roll(&mut audio, state.settings.sample_rate_in as usize / 2);
                info!(%session_id, pre_roll_samples = audio.len(), "speech started");
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.speech_started",
                    &json!({"pre_roll_samples": audio.len()}),
                )
                .await;
                send_event(&event_sender, json!({"type": "input.speech_started"})).await
            }
            "input.commit" => {
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "new_audio_commit",
                )
                .await;
                speech_active = false;
                let captured_audio = std::mem::take(&mut audio);
                let captured_frames: Vec<_> = frames.drain(..).collect();
                let response_id = Uuid::new_v4().to_string();
                info!(
                    %session_id,
                    %response_id,
                    audio_samples = captured_audio.len(),
                    frames = captured_frames.len(),
                    "audio input committed"
                );
                record_event_best_effort(
                    &state.context,
                    &session_id,
                    "server.input.committed",
                    &json!({
                        "response_id": response_id,
                        "input": "audio",
                        "audio_samples": captured_audio.len(),
                        "audio_ms": captured_audio.len() * 1_000 / state.settings.sample_rate_in as usize,
                        "frames": captured_frames.len()
                    }),
                )
                .await;
                active_response = Some(spawn_turn(
                    state.orchestrator.clone(),
                    state.context.clone(),
                    session_id.clone(),
                    event_sender.clone(),
                    captured_audio,
                    captured_frames,
                    None,
                    response_id,
                ));
                Ok(())
            }
            "input.text.commit" => {
                cancel_response(
                    &mut active_response,
                    &event_sender,
                    &state.context,
                    &session_id,
                    "new_text_commit",
                )
                .await;
                speech_active = false;
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
                    session_id.clone(),
                    event_sender.clone(),
                    Vec::new(),
                    captured_frames,
                    Some(text),
                    response_id,
                ));
                Ok(())
            }
            "response.cancel" => {
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
            "session.close" => break,
            _ => {
                warn!(kind = %event.kind, extra = ?event.extra, "unknown client event");
                Ok(())
            }
        };
        if result.is_err() {
            break;
        }
    }

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
}

fn append_audio(
    output: &mut Vec<f32>,
    encoded: Option<&str>,
    speech_active: bool,
    settings: &Settings,
) -> anyhow::Result<()> {
    let payload = STANDARD.decode(encoded.ok_or_else(|| anyhow::anyhow!("音频缺少 audio"))?)?;
    let samples = decode_le_f32(&payload)?;
    if speech_active {
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

fn spawn_turn(
    orchestrator: AgentOrchestrator,
    context: ContextStore,
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
                &session_id,
                &sender,
                audio,
                frames,
                transcript,
                &task_response_id,
            )
            .await
        {
            error!(%session_id, %error, "turn failed");
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
            let _ = send_event(
                &sender,
                json!({
                    "type": "error",
                    "response_id": task_response_id,
                    "message": error.to_string()
                }),
            )
            .await;
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
