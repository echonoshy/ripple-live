use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, State, rejection::JsonRejection},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use ripple_agent_gateway::meeting::{
    storage::{PutChunkOutcome, StorageError},
    types::{
        ChunkWrite, FinalAudioMetadata, FinalizeOutcome, MAX_MEETING_CHUNK_SEQUENCE, MeetingState,
    },
};
use serde::Deserialize;
use serde_json::json;
use tracing::error;

use crate::{AppState, api_error, authenticated_user};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateMeetingRequest {
    idempotency_key: String,
    started_at: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeetingTodoPatch {
    completed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinalizeMeetingRequest {
    last_sequence: i64,
    ended_at: f64,
}

pub(super) async fn create_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<CreateMeetingRequest>, JsonRejection>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &error.body_text()),
    };
    if request.idempotency_key.trim().is_empty()
        || !request.started_at.is_finite()
        || request.started_at < 0.0
    {
        return api_error(StatusCode::BAD_REQUEST, "会议创建参数无效");
    }
    match state
        .meetings
        .create(&user.id, &request.idempotency_key, request.started_at)
        .await
    {
        Ok((meeting, created)) => (
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({"data": meeting})),
        )
            .into_response(),
        Err(error) => meeting_internal_error(error),
    }
}

pub(super) async fn list_meetings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.meetings.list(&user.id).await {
        Ok(meetings) => Json(json!({"data": meetings})).into_response(),
        Err(error) => meeting_internal_error(error),
    }
}

pub(super) async fn upload_meeting_chunk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((meeting_id, sequence)): Path<(String, i64)>,
    body: Body,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.meetings.get_owned(&user.id, &meeting_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Err(error) => return meeting_internal_error(error),
    }
    if !(0..=MAX_MEETING_CHUNK_SEQUENCE).contains(&sequence)
        || headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("audio/mp4")
    {
        return api_error(StatusCode::BAD_REQUEST, "音频分片参数无效");
    }
    let checksum = match required_header(&headers, "X-Chunk-SHA256") {
        Some(value) => value,
        None => return api_error(StatusCode::BAD_REQUEST, "音频分片参数无效"),
    };
    let Some(start_ms) = parsed_header::<i64>(&headers, "X-Start-Ms") else {
        return api_error(StatusCode::BAD_REQUEST, "音频分片参数无效");
    };
    let Some(end_ms) = parsed_header::<i64>(&headers, "X-End-Ms") else {
        return api_error(StatusCode::BAD_REQUEST, "音频分片参数无效");
    };
    if start_ms < 0 || end_ms <= start_ms {
        return api_error(StatusCode::BAD_REQUEST, "音频分片参数无效");
    }

    let stored = match state
        .meeting_storage
        .put_chunk(&meeting_id, sequence, &checksum, body.into_data_stream())
        .await
    {
        Ok(stored) => stored,
        Err(error) => return meeting_storage_error(error),
    };
    let size_bytes = match i64::try_from(stored.size_bytes) {
        Ok(size_bytes) => size_bytes,
        Err(_) => {
            if stored.outcome == PutChunkOutcome::Inserted {
                let _ = state
                    .meeting_storage
                    .remove_chunk(&stored.relative_path)
                    .await;
            }
            return api_error(StatusCode::PAYLOAD_TOO_LARGE, "音频分片过大");
        }
    };
    let write = state
        .meetings
        .record_verified_chunk(
            &meeting_id,
            sequence,
            start_ms,
            end_ms,
            &stored.checksum,
            size_bytes,
            &stored.relative_path,
        )
        .await;
    let write = match write {
        Ok(write) => write,
        Err(error) => {
            if stored.outcome == PutChunkOutcome::Inserted {
                let _ = state
                    .meeting_storage
                    .remove_chunk(&stored.relative_path)
                    .await;
            }
            return meeting_internal_error(error);
        }
    };
    match write {
        ChunkWrite::Conflict => {
            if stored.outcome == PutChunkOutcome::Inserted {
                let _ = state
                    .meeting_storage
                    .remove_chunk(&stored.relative_path)
                    .await;
            }
            api_error(StatusCode::CONFLICT, "音频分片元数据冲突")
        }
        ChunkWrite::Inserted | ChunkWrite::Existing => {
            if write == ChunkWrite::Inserted {
                let _ = state.meeting_processor.spawn_transcribe_chunk(
                    meeting_id,
                    sequence,
                    start_ms,
                    end_ms,
                    stored.relative_path.clone(),
                );
            }
            (
                if write == ChunkWrite::Inserted {
                    StatusCode::CREATED
                } else {
                    StatusCode::OK
                },
                Json(json!({
                    "data": {
                        "sequence": sequence,
                        "size_bytes": size_bytes,
                        "checksum": stored.checksum,
                        "existing": write == ChunkWrite::Existing
                    }
                })),
            )
                .into_response()
        }
    }
}

pub(super) async fn finalize_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(meeting_id): Path<String>,
    request: Result<Json<FinalizeMeetingRequest>, JsonRejection>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let meeting = match state.meetings.get_owned(&user.id, &meeting_id).await {
        Ok(Some(meeting)) => meeting,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Err(error) => return meeting_internal_error(error),
    };
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &error.body_text()),
    };
    if !(0..=MAX_MEETING_CHUNK_SEQUENCE).contains(&request.last_sequence)
        || !request.ended_at.is_finite()
        || request.ended_at < meeting.started_at
    {
        return api_error(StatusCode::BAD_REQUEST, "会议结束参数无效");
    }
    let legacy_audio = match state
        .meetings
        .claim_finalization(
            &user.id,
            &meeting_id,
            request.last_sequence,
            request.ended_at,
        )
        .await
    {
        Ok(FinalizeOutcome::Pending) => None,
        Ok(FinalizeOutcome::LegacyVerificationRequired(audio)) => Some(audio),
        Ok(FinalizeOutcome::Finalized(meeting_state)) => {
            if meeting_state == MeetingState::Processing {
                let audio = match state
                    .meetings
                    .owned_final_audio(&user.id, &meeting_id)
                    .await
                {
                    Ok(Some(audio)) => audio,
                    Ok(None) => {
                        return meeting_internal_error(anyhow::anyhow!(
                            "processing meeting has no final audio"
                        ));
                    }
                    Err(error) => return meeting_internal_error(error),
                };
                state
                    .meeting_processor
                    .spawn_finalize_transcript(meeting_id.clone(), audio.relative_path);
            }
            return finalized_response(&meeting_id, meeting_state);
        }
        Ok(FinalizeOutcome::Conflict) => {
            return api_error(StatusCode::CONFLICT, "会议结束边界冲突");
        }
        Ok(FinalizeOutcome::NotFound) => {
            return api_error(StatusCode::NOT_FOUND, "会议记录不存在");
        }
        Err(error) => return meeting_internal_error(error),
    };
    let missing = match state
        .meetings
        .missing_verified_sequences(&meeting_id, request.last_sequence)
        .await
    {
        Ok(missing) => missing,
        Err(error) => return meeting_internal_error(error),
    };
    if !missing.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"missing_sequences": missing})),
        )
            .into_response();
    }
    let chunks = match state
        .meetings
        .verified_chunks(&meeting_id, request.last_sequence)
        .await
    {
        Ok(chunks) => chunks,
        Err(error) => return meeting_internal_error(error),
    };
    if chunks.len() != request.last_sequence as usize + 1
        || chunks
            .iter()
            .enumerate()
            .any(|(sequence, chunk)| chunk.sequence != sequence as i64)
    {
        return api_error(StatusCode::CONFLICT, "音频分片不完整");
    }
    let relative_paths = chunks
        .iter()
        .map(|chunk| chunk.relative_path.clone())
        .collect::<Vec<_>>();
    if let Some(expected) = legacy_audio {
        let final_audio_path = expected.relative_path.clone();
        let proof = match state
            .meeting_storage
            .verify_legacy_finalization(
                &meeting_id,
                request.last_sequence,
                &relative_paths,
                &expected,
            )
            .await
        {
            Ok(Some(proof)) => proof,
            Ok(None) => return api_error(StatusCode::CONFLICT, "会议结束边界冲突"),
            Err(error) => return meeting_storage_error(error),
        };
        return match state
            .meetings
            .recover_legacy_finalization(&user.id, proof)
            .await
        {
            Ok(FinalizeOutcome::Finalized(meeting_state)) => {
                state
                    .meeting_processor
                    .spawn_finalize_transcript(meeting_id.clone(), final_audio_path);
                finalized_response(&meeting_id, meeting_state)
            }
            Ok(FinalizeOutcome::Conflict) => api_error(StatusCode::CONFLICT, "会议结束边界冲突"),
            Ok(FinalizeOutcome::NotFound) => api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
            Ok(FinalizeOutcome::Pending) => meeting_internal_error(anyhow::anyhow!(
                "legacy meeting finalization remained pending"
            )),
            Ok(FinalizeOutcome::LegacyVerificationRequired(_)) => meeting_internal_error(
                anyhow::anyhow!("legacy meeting finalization still requires verification"),
            ),
            Err(error) => meeting_internal_error(error),
        };
    }
    let audio = match state
        .meeting_storage
        .assemble_final_audio(&meeting_id, &relative_paths)
        .await
    {
        Ok(audio) => audio,
        Err(error) => return meeting_storage_error(error),
    };
    let size_bytes = match i64::try_from(audio.size_bytes) {
        Ok(size_bytes) => size_bytes,
        Err(_) => return meeting_internal_error(anyhow::anyhow!("final audio is too large")),
    };
    let metadata = FinalAudioMetadata {
        relative_path: audio.relative_path,
        size_bytes,
        checksum: audio.checksum,
    };
    match state
        .meetings
        .complete_owned_finalization(&user.id, &meeting_id, request.last_sequence, &metadata)
        .await
    {
        Ok(FinalizeOutcome::Finalized(meeting_state)) => {
            state
                .meeting_processor
                .spawn_finalize_transcript(meeting_id.clone(), metadata.relative_path.clone());
            finalized_response(&meeting_id, meeting_state)
        }
        Ok(FinalizeOutcome::Conflict) => api_error(StatusCode::CONFLICT, "会议结束边界冲突"),
        Ok(FinalizeOutcome::NotFound) => api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Ok(FinalizeOutcome::Pending) => {
            meeting_internal_error(anyhow::anyhow!("meeting finalization remained pending"))
        }
        Ok(FinalizeOutcome::LegacyVerificationRequired(_)) => meeting_internal_error(
            anyhow::anyhow!("meeting finalization unexpectedly requires legacy verification"),
        ),
        Err(error) => meeting_internal_error(error),
    }
}

fn finalized_response(meeting_id: &str, state: MeetingState) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(json!({"data": {"id": meeting_id, "state": state}})),
    )
        .into_response()
}

pub(super) async fn get_meeting_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(meeting_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let audio = match state
        .meetings
        .owned_final_audio(&user.id, &meeting_id)
        .await
    {
        Ok(Some(audio)) => audio,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "会议音频不存在"),
        Err(error) => return meeting_internal_error(error),
    };
    let file = match state.meeting_storage.open_audio(&audio.relative_path).await {
        Ok(file) => file,
        Err(error) => return meeting_storage_error(error),
    };
    let stream = async_stream::stream! {
        let mut file = file;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            match tokio::io::AsyncReadExt::read(&mut file, &mut buffer).await {
                Ok(0) => break,
                Ok(count) => yield Ok::<Bytes, std::io::Error>(
                    Bytes::copy_from_slice(&buffer[..count]),
                ),
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(error);
                    break;
                }
            }
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("audio/mp4"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("inline; filename=\"recording.m4a\""),
    );
    if let Ok(value) = HeaderValue::from_str(&audio.size_bytes.to_string()) {
        headers.insert(CONTENT_LENGTH, value);
    }
    response
}

pub(super) async fn get_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(meeting_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.meetings.get_owned(&user.id, &meeting_id).await {
        Ok(Some(meeting)) => Json(json!({"data": meeting})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Err(error) => meeting_internal_error(error),
    }
}

pub(super) async fn delete_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(meeting_id): Path<String>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.meetings.delete_owned(&user.id, &meeting_id).await {
        Ok(true) => {
            if let Err(error) = state.meeting_storage.delete_meeting(&meeting_id).await {
                error!(error = %error, "deleted meeting left storage files for later cleanup");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Err(error) => meeting_internal_error(error),
    }
}

pub(super) async fn update_meeting_todo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((meeting_id, todo_id)): Path<(String, String)>,
    request: Result<Json<MeetingTodoPatch>, JsonRejection>,
) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &error.body_text()),
    };
    match state
        .meetings
        .update_todo_completed(&user.id, &meeting_id, &todo_id, request.completed)
        .await
    {
        Ok(Some(todo)) => Json(json!({"data": todo})).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "会议待办不存在"),
        Err(error) => meeting_internal_error(error),
    }
}

fn meeting_internal_error(error: anyhow::Error) -> Response {
    error!(error = %error, "meeting persistence request failed");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "会议记录暂时不可用")
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn parsed_header<T>(headers: &HeaderMap, name: &'static str) -> Option<T>
where
    T: std::str::FromStr,
{
    required_header(headers, name)?.parse().ok()
}

fn meeting_storage_error(error: StorageError) -> Response {
    match error {
        StorageError::UnsafePath | StorageError::InvalidChecksum => {
            api_error(StatusCode::BAD_REQUEST, "音频分片参数无效")
        }
        StorageError::TooLarge => api_error(StatusCode::PAYLOAD_TOO_LARGE, "音频分片过大"),
        StorageError::ChecksumMismatch => api_error(StatusCode::BAD_REQUEST, "音频分片校验失败"),
        StorageError::Conflict => api_error(StatusCode::CONFLICT, "音频分片已存在且内容不同"),
        StorageError::Io(_) | StorageError::Stream(_) | StorageError::Ffmpeg(_) => {
            error!(error = %error, "meeting audio storage request failed");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "会议音频暂时不可用")
        }
    }
}
