//! Workflow transitions, including atomic claim/start and comment/block operations.

use super::tickets::load_visible;
use super::{body_object, get_i64, get_str, reject_unknown, require_str, ApiJson};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

pub async fn transition(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["to", "reason", "fence"])?;
    let to = require_str(obj, "to")?;
    let reason = get_str(obj, "reason")?;
    let fence = get_i64(obj, "fence")?;

    let ticket =
        state
            .store
            .transition(&id, &to, reason.as_deref(), fence, &ctx.actor, &ctx.scopes)?;
    state.wake();
    Ok(Json(ticket.to_json(now_ms())))
}

/// Claim if needed and transition in one transaction. A rejected transition
/// rolls back the claim and its events, just like the hosted MCP start tool.
pub async fn start(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let ticket = load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["to", "fence", "ttl_seconds"])?;
    let to = require_str(obj, "to")?;
    let fence = get_i64(obj, "fence")?;
    let ttl = get_i64(obj, "ttl_seconds")?;
    let project = state
        .store
        .get_project(&ticket.project)?
        .ok_or_else(|| ApiError::not_found("project", &ticket.project))?;
    let try_claim = project
        .workflow
        .state(&ticket.state)
        .is_some_and(|s| s.claimable);
    let updated = state.store.start_ticket(
        &id,
        &to,
        None,
        fence,
        &ctx.actor,
        &ctx.scopes,
        ttl,
        try_claim,
    )?;
    state.wake();
    let now = now_ms();
    let mut out = updated.to_json(now);
    // Only the holder receives the fence. Return it from the same transaction's
    // snapshot so a stdio client can heartbeat without taking a second claim.
    if updated
        .active_claim(now)
        .is_some_and(|(holder, _)| holder == ctx.actor)
    {
        let mut lease = out["claim"].clone();
        lease["fence"] = serde_json::json!(updated.fence_seq);
        out["lease"] = lease;
    }
    Ok(Json(out))
}

/// Record a blocker note and transition together. Neither survives a refusal.
pub async fn block(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["to", "fence", "comment"])?;
    let to = require_str(obj, "to")?;
    let fence = get_i64(obj, "fence")?;
    let comment = get_str(obj, "comment")?;
    let updated =
        state
            .store
            .block_ticket(&id, &to, comment.as_deref(), fence, &ctx.actor, &ctx.scopes)?;
    state.wake();
    Ok(Json(updated.to_json(now_ms())))
}
