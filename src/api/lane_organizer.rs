//! Human-triggered organization of pending work; accepting never dispatches work.
use super::{body_object, reject_unknown, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::agent_chat::SendMessage,
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde_json::Value;
use std::sync::Arc;
pub async fn get(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(s.store.lane_organizer_view(&ctx, &project)?))
}
pub async fn send(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let req: SendMessage = serde_json::from_value(body)
        .map_err(|e| ApiError::validation("validation.lane_organizer", e.to_string()))?;
    let result = s.store.lane_organizer_send(&ctx, &project, &req)?;
    s.wake();
    Ok(Json(result))
}
pub async fn accept(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((project, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    reject_unknown(body_object(&body)?, &[])?;
    let result = s.store.lane_organizer_accept(&ctx, &project, &id)?;
    s.wake();
    Ok(Json(result))
}
