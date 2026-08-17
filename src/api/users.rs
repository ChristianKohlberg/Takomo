//! /v1/users — the people directory: who work can be addressed to.
//!
//! Reads are open to any valid token, because a name has to be renderable by
//! whoever is looking at a question; writes need `admin`, because membership is
//! what decides who may be handed work and a named assignee may answer an
//! `approve`. See `src/store/users.rs` for the doctrine and docs/users.md for the
//! one place it bends.

use super::{
    body_object, first, get_str, paged, parse_i64_param, query_pairs, reject_unknown, require_str,
    ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{UserCreate, UserListFilter, UserPatch, MAX_USERS_PAGE};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 5] = ["handle", "name", "email", "meta", "projects"];
const PATCH_FIELDS: [&str; 3] = ["name", "email", "meta_merge"];
const MEMBER_FIELDS: [&str; 1] = ["project"];

const DEFAULT_USERS_LIMIT: i64 = 100;

/// GET /v1/users?q=&project=&include_disabled=&limit=&offset= (read) — the
/// directory, ordered by handle.
///
/// A bounded envelope like every other list here: `items` + `total` + the `limit`
/// applied, and a `note` when the page left someone out. Ordering is by handle and
/// therefore stable, so the continuation is an `offset` (the `cases` shape) rather
/// than a cursor.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    if let Some(project) = first(&pairs, "project") {
        ctx.require_project(project)?;
    }
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(DEFAULT_USERS_LIMIT)
        .clamp(1, MAX_USERS_PAGE);
    let offset = parse_i64_param(&pairs, "offset")?.unwrap_or(0).max(0);
    let filter = UserListFilter {
        q: first(&pairs, "q").map(str::to_string),
        project: first(&pairs, "project").map(str::to_string),
        include_disabled: matches!(first(&pairs, "include_disabled"), Some("true" | "1")),
        limit,
        offset,
    };
    let (users, total) = state.store.list_users(&filter)?;
    let items: Vec<Value> = users.iter().map(|u| u.to_json()).collect();
    Ok(Json(paged(
        items,
        total,
        limit,
        "Raise 'limit' (max 200) or page with 'offset'; ordering is by handle, so offsets are stable.",
    )))
}

/// POST /v1/users (admin) — add a person. Body:
/// `{"handle":"ada","name":"Ada Lovelace","email":"ada@…","projects":["tp"]}`.
///
/// Admin rather than `write` because this is the directory assignment draws from,
/// and assignment is the one route by which a name confers authority.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CREATE_FIELDS)?;
    let projects = super::get_string_array(obj, "projects")?.unwrap_or_default();
    for project in &projects {
        ctx.require_project(project)?;
    }
    let req = UserCreate {
        handle: require_str(obj, "handle")?,
        name: get_str(obj, "name")?,
        email: get_str(obj, "email")?,
        meta: obj.get("meta").filter(|v| !v.is_null()).cloned(),
        projects,
    };
    let user = state.store.create_user(&req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(user.to_json())))
}

/// GET /v1/users/{handle} (read) — one person, with their memberships. Accepts a
/// handle or a `usr-…` id, because both are in circulation: a person types the
/// handle, a stored reference is the id.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(handle): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let user = state
        .store
        .get_user(&handle)?
        .ok_or_else(|| ApiError::not_found("user", &handle))?;
    Ok(Json(user.to_json()))
}

/// PATCH /v1/users/{handle} (admin) — change the display name, email, or merge
/// into `meta` (RFC 7386: a null value deletes a key).
///
/// The handle is deliberately not patchable. It is the identity every
/// `person:<handle>` reference and every stored assignment resolves through, so
/// renaming it would silently orphan them; the display name is what changes when a
/// person's name does.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(handle): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;
    // `email: null` clears the address; an absent key leaves it alone. The two
    // mean different things on the wire, so they are distinguished here rather
    // than collapsed by `get_str`.
    let email = match obj.get("email") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => {
            return Err(ApiError::validation(
                "validation.user_email",
                "Field 'email' must be a string, or null to clear it.",
            ))
        }
    };
    let patch = UserPatch {
        name: get_str(obj, "name")?,
        email,
        meta_merge: obj.get("meta_merge").filter(|v| !v.is_null()).cloned(),
    };
    if patch.name.is_none() && patch.email.is_none() && patch.meta_merge.is_none() {
        return Err(ApiError::bad_request(
            "validation.no_changes",
            "The patch contains no changes. Provide 'name', 'email' and/or 'meta_merge'.",
        ));
    }
    let user = state.store.patch_user(&handle, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(user.to_json()))
}

/// POST /v1/users/{handle}/disable (admin) — stop new work being addressed to
/// this person, keeping every record that names them readable.
///
/// Not a delete and not a revocation: their tokens keep working, because what a
/// credential may do is its scopes. What ends is being assignable, and with it the
/// assignee route to approving. Revoke the token to end access.
pub async fn disable(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(handle): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    let user = state.store.set_user_disabled(&handle, true, &ctx.actor)?;
    state.wake();
    Ok(Json(user.to_json()))
}

/// POST /v1/users/{handle}/enable (admin) — the reverse, unchanged by the pause.
pub async fn enable(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(handle): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    let user = state.store.set_user_disabled(&handle, false, &ctx.actor)?;
    state.wake();
    Ok(Json(user.to_json()))
}

/// POST /v1/users/{handle}/projects (admin) — make this person a member, so work
/// in that project can be addressed to them. Body: `{"project":"tp"}`.
/// Idempotent: re-adding a member is the state the caller asked for, not a 409.
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(handle): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &MEMBER_FIELDS)?;
    let project = require_str(obj, "project")?;
    ctx.require_project(&project)?;
    let membership = state.store.add_member(&handle, &project, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(membership.to_json())))
}

/// DELETE /v1/users/{handle}/projects/{project} (admin) — end the membership.
///
/// Questions already waiting on this person stay waiting on them: retracting an
/// open decision silently would leave it with nobody looking at it. They just
/// cannot be handed anything new here.
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((handle, project)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let removed = state.store.remove_member(&handle, &project, &ctx.actor)?;
    if !removed {
        return Err(ApiError::not_found(
            "membership",
            &format!("{handle} in {project}"),
        ));
    }
    state.wake();
    Ok(Json(json!({ "ok": true })))
}
