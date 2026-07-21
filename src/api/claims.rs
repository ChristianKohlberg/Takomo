//! Claim/lease endpoints and the ready queue.

use super::tickets::load_visible;
use super::{
    body_object, clamp_wait, first, get_i64, get_str, get_string_array, long_poll, parse_i64_param,
    query_pairs,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use crate::store::ReadyFilter;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

pub async fn claim(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let ttl = match &body {
        None => None,
        Some(Json(v)) => get_i64(body_object(v)?, "ttl_seconds")?,
    };
    let (_ticket, lease) = state.store.claim_ticket(&id, &ctx.actor, ttl)?;
    state.wake();
    Ok(Json(lease.to_json()))
}

pub async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    let fence = get_i64(obj, "fence")?.ok_or_else(|| {
        ApiError::bad_request(
            "validation.field_required",
            "Field 'fence' is required: echo the fencing token from your lease.",
        )
    })?;
    let ttl = get_i64(obj, "ttl_seconds")?;
    let lease = state.store.heartbeat(&id, fence, &ctx.actor, ttl)?;
    state.wake();
    Ok(Json(lease.to_json()))
}

pub async fn release(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<StatusCode> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    let fence = get_i64(obj, "fence")?.ok_or_else(|| {
        ApiError::bad_request(
            "validation.field_required",
            "Field 'fence' is required: echo the fencing token from your lease.",
        )
    })?;
    let reason = get_str(obj, "reason")?;
    state
        .store
        .release(&id, fence, &ctx.actor, reason.as_deref())?;
    state.wake();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn ready_peek(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    if let Some(p) = first(&pairs, "project") {
        ctx.require_project(p)?;
    }
    let filter = ReadyFilter {
        project: first(&pairs, "project").map(str::to_string),
        ty: first(&pairs, "type").map(str::to_string),
        labels: first(&pairs, "label")
            .map(str::to_string)
            .into_iter()
            .collect(),
        allowed_projects: ctx.allowed_projects_vec(),
    };
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(20)
        .clamp(1, 200);
    let tickets = state.store.ready_peek(&filter, limit)?;
    let now = now_ms();
    Ok(Json(Value::Array(
        tickets.iter().map(|t| t.to_json(now)).collect(),
    )))
}

/// Atomic pop with optional long-poll: the worker primitive.
pub async fn ready_claim(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    body: Option<Json<Value>>,
) -> ApiResult<Response> {
    ctx.require_scope("write")?;
    let (filter, wait, ttl) = match &body {
        None => (
            ReadyFilter {
                allowed_projects: ctx.allowed_projects_vec(),
                ..Default::default()
            },
            clamp_wait(None),
            None,
        ),
        Some(Json(v)) => {
            let obj = body_object(v)?;
            let project = get_str(obj, "project")?;
            if let Some(p) = &project {
                ctx.require_project(p)?;
            }
            (
                ReadyFilter {
                    project,
                    ty: get_str(obj, "type")?,
                    labels: get_string_array(obj, "labels")?.unwrap_or_default(),
                    allowed_projects: ctx.allowed_projects_vec(),
                },
                clamp_wait(get_i64(obj, "wait_seconds")?),
                get_i64(obj, "ttl_seconds")?,
            )
        }
    };

    let actor = ctx.actor.clone();
    let claimed = long_poll(&state, wait, || {
        state.store.ready_claim(&filter, &actor, ttl)
    })
    .await?;

    match claimed {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some((ticket, lease)) => {
            state.wake();
            let mut out = ticket.to_json(now_ms());
            out["lease"] = lease.to_json();
            Ok(Json(out).into_response())
        }
    }
}
