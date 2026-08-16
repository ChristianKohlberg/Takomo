//! Claim/lease endpoints and the ready queue.

use super::tickets::load_visible;
use super::{
    attach_conventions, body_object, clamp_wait, first, get_i64, get_str, get_string_array,
    long_poll, parse_i64_param, query_pairs, reject_unknown, ApiJson,
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
        Some(Json(v)) => {
            let obj = body_object(v)?;
            reject_unknown(obj, &["ttl_seconds"])?;
            get_i64(obj, "ttl_seconds")?
        }
    };
    let (ticket, lease) = state.store.claim_ticket(&id, &ctx.actor, ttl)?;
    state.wake();
    // The response *is* the lease, so the hints ride as siblings of its fields
    // rather than in an envelope — `/v1` is additive only, and a caller reading
    // `fence` never sees the difference.
    let mut out = lease.to_json();
    attach_conventions(&state, &mut out, &ticket.project);
    Ok(Json(out))
}

/// GET the claim on a ticket: who holds it, since when, whether it expires —
/// and, while held, what has moved in its subtree since the grant. The read
/// that makes an indefinite epic claim judgeable: with no TTL to wait out, the
/// caller decides "mid-flight or abandoned" from the movement, and an admin
/// force-releases the abandoned ones.
pub async fn claim_status(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    load_visible(&state, &ctx, &id)?;
    let status = state.store.claim_status(&id)?;
    Ok(Json(status.to_json()))
}

pub async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["fence", "ttl_seconds"])?;
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<StatusCode> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["fence", "reason"])?;
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

/// Admin force-release — a route of its own rather than a `force` flag on
/// `/release`, so an ordinary worker's typo can never become a force: reaching
/// it at all takes the `admin` scope AND the distinct path.
pub async fn force_release(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    load_visible(&state, &ctx, &id)?;
    // No `fence` is accepted, let alone required — the caller is displacing a
    // holder whose fence it has no way to know.
    let reason = match &body {
        None => None,
        Some(Json(v)) => {
            let obj = body_object(v)?;
            reject_unknown(obj, &["reason"])?;
            get_str(obj, "reason")?
        }
    };
    let forced = state
        .store
        .force_release(&id, &ctx.actor, reason.as_deref())?;
    state.wake();
    Ok(Json(forced.to_json()))
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
    let (tickets, total) = state.store.ready_peek(&filter, limit)?;
    let now = now_ms();
    let items: Vec<Value> = tickets.iter().map(|t| t.to_json(now)).collect();
    // An envelope rather than the bare array this used to return, because a bare
    // array has nowhere to say how much it left out: a caller that asked for 20
    // and received 20 could not distinguish a queue of 20 from the head of a
    // queue of 137, and silently reading a fraction of the work as if it were all
    // of it is the failure this shape exists to prevent.
    Ok(Json(super::paged(
        items,
        total,
        limit,
        "Raise the page size with ?limit=N (max 200), or narrow with ?project=/?type=/?label=. \
         The queue is live — other workers claim from it as you read, so treat this as a \
         snapshot rather than a stable page.",
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
            reject_unknown(
                obj,
                &["project", "type", "labels", "wait_seconds", "ttl_seconds"],
            )?;
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
            attach_conventions(&state, &mut out, &ticket.project);
            Ok(Json(out).into_response())
        }
    }
}
