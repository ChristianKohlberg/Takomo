//! Environments — `/v1/projects/{project}/environments` and `/v1/environments/{id}`.
//!
//! The registry of places a check can be run: a URL, how to bring the thing up,
//! what is in it, and whether writing to it is safe. Takomo runs none of it; see
//! `store/environments.rs` for why that is the whole design.
//!
//! Writes take `write`, not `human`. An agent that just leased an ephemeral
//! instance should be able to register it — that is the case this exists to
//! serve — and gating it on a person would push exactly that back out of band.

use super::{
    body_object, first, get_bool, get_str, parse_i64_param, query_pairs, reject_unknown,
    require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{EnvironmentCreate, EnvironmentFilter, EnvironmentPatch, MAX_ENVIRONMENTS_PAGE};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

const ENV_CREATE_FIELDS: [&str; 11] = [
    "slug",
    "name",
    "kind",
    "base_url",
    "bring_up",
    "teardown",
    "data_state",
    "writable",
    "credentials_hint",
    "notes",
    "metadata",
];

// No `slug`: it is the handle checks and tool calls carry, so renaming one would
// silently break every reference. A new name is a new environment.
const ENV_PATCH_FIELDS: [&str; 10] = [
    "name",
    "kind",
    "base_url",
    "bring_up",
    "teardown",
    "data_state",
    "writable",
    "credentials_hint",
    "notes",
    "metadata_merge",
];

/// Read a field that is present-but-null distinctly from absent, so "clear this
/// again" stays expressible once a value has been set.
fn override_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> ApiResult<Option<Option<String>>> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a string or null."),
        )),
    }
}

/// POST /v1/projects/{project}/environments (write) — register an environment.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &ENV_CREATE_FIELDS)?;
    let req = EnvironmentCreate {
        project: project.clone(),
        slug: require_str(obj, "slug")?,
        name: get_str(obj, "name")?,
        kind: get_str(obj, "kind")?,
        base_url: get_str(obj, "base_url")?,
        bring_up: get_str(obj, "bring_up")?,
        teardown: get_str(obj, "teardown")?,
        data_state: get_str(obj, "data_state")?,
        writable: get_bool(obj, "writable")?,
        credentials_hint: get_str(obj, "credentials_hint")?,
        notes: get_str(obj, "notes")?,
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
    };
    let env = state.store.create_environment(&req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(env.to_json())))
}

/// GET /v1/projects/{project}/environments?kind=&archived=include&limit= (read).
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let filter = EnvironmentFilter {
        project: project.clone(),
        kind: first(&pairs, "kind").map(str::to_string),
        include_archived: first(&pairs, "archived") == Some("include"),
        limit: parse_i64_param(&pairs, "limit")?,
    };
    let limit = filter
        .limit
        .unwrap_or(MAX_ENVIRONMENTS_PAGE)
        .clamp(1, MAX_ENVIRONMENTS_PAGE);
    let (envs, total) = state.store.list_environments(&filter)?;
    Ok(Json(super::paged(
        envs.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
        total,
        limit,
        "Raise the page size with ?limit=N (max 200), or narrow with ?kind=.",
    )))
}

/// GET /v1/environments/{id} (read).
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let env = state.store.get_environment(&id)?;
    ctx.require_project(&env.project)?;
    Ok(Json(env.to_json()))
}

/// PATCH /v1/environments/{id} (write). `base_url` and `credentials_hint` accept
/// an explicit null to clear them; `slug` is not patchable.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_environment(&id)?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &ENV_PATCH_FIELDS)?;
    let req = EnvironmentPatch {
        name: get_str(obj, "name")?,
        kind: get_str(obj, "kind")?,
        base_url: override_str(obj, "base_url")?,
        bring_up: get_str(obj, "bring_up")?,
        teardown: get_str(obj, "teardown")?,
        data_state: get_str(obj, "data_state")?,
        writable: get_bool(obj, "writable")?,
        credentials_hint: override_str(obj, "credentials_hint")?,
        notes: get_str(obj, "notes")?,
        metadata_merge: obj.get("metadata_merge").cloned(),
    };
    let env = state.store.patch_environment(&id, &req, &ctx.actor)?;
    state.wake();
    Ok(Json(env.to_json()))
}

/// DELETE /v1/environments/{id} (write) — archive it.
///
/// Archive rather than delete: a decommissioned box is still the evidence behind
/// every verdict ever taken there, and deleting it would orphan that history.
pub async fn archive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_environment(&id)?;
    ctx.require_project(&existing.project)?;
    let env = state.store.archive_environment(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(env.to_json()))
}

/// POST /v1/environments/{id}/unarchive (write) — bring it back.
pub async fn unarchive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_environment(&id)?;
    ctx.require_project(&existing.project)?;
    let env = state.store.unarchive_environment(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(env.to_json()))
}
