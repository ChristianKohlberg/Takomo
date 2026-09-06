//! Read-only agent conversations anchored to specification sections.
use super::{long_poll, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::{
        agent_chat::{Claim, Heartbeat, ResultInput, SendMessage},
        mindmapdoc,
    },
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

fn decode<T: DeserializeOwned>(value: Value) -> ApiResult<T> {
    serde_json::from_value(value)
        .map_err(|e| ApiError::validation("validation.agent_chat", e.to_string()))
}

/// The snapshot is read from the shared replica, never trusted from the client.
async fn section(
    state: &Arc<AppState>,
    ctx: &AuthCtx,
    map: &str,
    node: &str,
) -> ApiResult<(String, String)> {
    let row = state
        .store
        .get_mindmap(map)?
        .ok_or_else(|| ApiError::not_found("mindmap", map))?;
    ctx.require_project(&row.project)?;
    let room = crate::api::docsync::open_room(state, map).await?;
    let snapshot = room.read(|doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, map);
        nodes
            .into_iter()
            .find(|n| n.id == node)
            .map(|n| format!("# {}\n\n{}", n.title, n.notes))
            .ok_or_else(|| ApiError::not_found("section", node))
    })?;
    Ok((row.project, snapshot))
}
pub async fn conversation(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((map, node)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    section(&state, &ctx, &map, &node).await?;
    Ok(Json(state.store.agent_conversation(&map, &node)?))
}
pub async fn send(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((map, node)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_scope("human")?;
    let req: SendMessage = decode(body)?;
    let (project, snapshot) = section(&state, &ctx, &map, &node).await?;
    let result = state
        .store
        .send_agent_message(&ctx, &map, &node, &project, &snapshot, &req)?;
    state.wake();
    Ok(Json(result))
}
pub async fn claim(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    let req: Claim = decode(body)?;
    if req.wait_seconds > 25 {
        return Err(ApiError::validation(
            "validation.agent_chat",
            "wait_seconds must be between 0 and 25.",
        ));
    }
    let job = long_poll(&state, Duration::from_secs(req.wait_seconds), || {
        state.store.claim_agent_job(&ctx, &req.service_id)
    })
    .await?;
    if job.is_some() {
        state.wake();
    }
    Ok(Json(json!({"job":job})))
}
pub async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    let result = state
        .store
        .heartbeat_agent_job(&ctx, &id, &decode::<Heartbeat>(body)?)?;
    Ok(Json(result))
}
pub async fn result(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    let result = state
        .store
        .finish_agent_job(&ctx, &id, &decode::<ResultInput>(body)?)?;
    state.wake();
    Ok(Json(result))
}
