use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

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
        Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "会议记录暂时不可用"),
    }
}

pub(super) async fn list_meetings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match authenticated_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.meetings.list(&user.id).await {
        Ok(meetings) => Json(json!({"data": meetings})).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
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
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
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
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => api_error(StatusCode::NOT_FOUND, "会议记录不存在"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
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
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}
