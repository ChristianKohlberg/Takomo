//! /v1/projects/{project}/tags — the project tag registry (person, component,
//! team, … entities that tickets reference by `kind:handle`). Reference
//! metadata only: creating, editing, or deleting a tag never touches ticket
//! state, claims, or question routing.

use super::{body_object, first, get_str, query_pairs, reject_unknown, require_str, ApiJson};
use crate::auth::AuthCtx;
use crate::error::ApiError;
use crate::error::ApiResult;
use crate::server::AppState;
use crate::store::{TagCreate, TagListFilter, TagPatch};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 4] = ["kind", "handle", "label", "meta"];
const PATCH_FIELDS: [&str; 2] = ["label", "meta_merge"];

/// GET /v1/projects/{project}/tags?kind=&q= (read) — list the registry, ordered
/// by (kind, handle). `kind` narrows to one kind; `q` is a case-insensitive
/// substring match on handle or label.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let filter = TagListFilter {
        project: project.clone(),
        kind: first(&pairs, "kind").map(str::to_string),
        q: first(&pairs, "q").map(str::to_string),
    };
    let tags = state.store.list_tags(&filter)?;
    let items: Vec<Value> = tags.iter().map(|t| t.to_json()).collect();
    Ok(Json(json!({ "items": items })))
}

/// POST /v1/projects/{project}/tags (write) — register a tag. Body:
/// `{"kind":"person","handle":"ada","label":"Ada Lovelace","meta":{...}}`.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CREATE_FIELDS)?;
    let req = TagCreate {
        kind: require_str(obj, "kind")?,
        handle: require_str(obj, "handle")?,
        label: get_str(obj, "label")?,
        meta: obj.get("meta").filter(|v| !v.is_null()).cloned(),
    };
    let tag = state.store.create_tag(&project, &req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(tag.to_json())))
}

/// GET /v1/projects/{project}/tags/{kind}/{handle} (read).
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((project, kind, handle)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let tag = state
        .store
        .get_tag(&project, &kind, &handle)?
        .ok_or_else(|| ApiError::not_found("tag", &format!("{kind}:{handle}")))?;
    Ok(Json(tag.to_json()))
}

/// PATCH /v1/projects/{project}/tags/{kind}/{handle} (write) — update the
/// display label and/or merge into `meta` (RFC 7386: a null value deletes a key).
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((project, kind, handle)): Path<(String, String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;
    let patch = TagPatch {
        label: get_str(obj, "label")?,
        meta_merge: obj.get("meta_merge").filter(|v| !v.is_null()).cloned(),
    };
    if patch.label.is_none() && patch.meta_merge.is_none() {
        return Err(ApiError::bad_request(
            "validation.no_changes",
            "The patch contains no changes. Provide 'label' and/or 'meta_merge'.",
        ));
    }
    let tag = state
        .store
        .patch_tag(&project, &kind, &handle, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(tag.to_json()))
}

/// DELETE /v1/projects/{project}/tags/{kind}/{handle} (write) — remove the
/// registry entry. Ticket references are intentionally left intact (the
/// reference is the source of truth); the response reports how many tickets
/// still point at the deleted tag.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((project, kind, handle)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let still_referenced = state
        .store
        .delete_tag(&project, &kind, &handle, &ctx.actor)?;
    state.wake();
    Ok(Json(
        json!({ "ok": true, "still_referenced": still_referenced }),
    ))
}
