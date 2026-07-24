//! /v1/projects and per-project workflow endpoints.

use super::{body_object, first, query_pairs, require_str};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::workflow::Workflow;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let projects = state.store.list_projects()?;
    let out: Vec<Value> = projects
        .iter()
        .filter(|p| ctx.can_project(&p.id))
        .map(|p| p.to_json())
        .collect();
    Ok(Json(Value::Array(out)))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    let id = require_str(obj, "id")?;
    let name = require_str(obj, "name")?;
    ctx.require_project(&id)?;
    let workflow = match obj.get("workflow") {
        None | Some(Value::Null) => None,
        Some(raw) => Some(parse_workflow(raw)?),
    };
    let mut project = state
        .store
        .create_project(&id, &name, workflow, &ctx.actor)?;
    // Optional per-project human-facing question language, set at creation.
    if let Some(lang) = super::get_str(obj, "question_language")? {
        project = state
            .store
            .set_question_language(&id, Some(&lang), &ctx.actor)?;
    }
    state.wake();
    Ok((StatusCode::CREATED, Json(project.to_json())))
}

/// PUT /v1/projects/{project}/language (admin) — set the human-facing language
/// agents should phrase ask-a-human questions in for this project. Body:
/// `{"language": "German"}`, or `{"language": null}` to clear it.
pub async fn put_language(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    // `language` present-and-null clears it; a string sets it; absent is an error.
    let language = match obj.get("language") {
        None => {
            return Err(ApiError::bad_request(
                "validation.field_required",
                "Field 'language' is required (a string like \"German\", or null to clear).",
            ))
        }
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(ApiError::bad_request(
                "validation.field_type",
                "Field 'language' must be a string or null.",
            ))
        }
    };
    let project = state
        .store
        .set_question_language(&project, language.as_deref(), &ctx.actor)?;
    state.wake();
    Ok(Json(project.to_json()))
}

/// DELETE /v1/projects/{project} (admin) — cascade-delete the project and every
/// ticket, comment, dep, and event under it, in one transaction. Refuses with
/// 409 when a ticket holds an active claim unless `?force=true` is passed; 404
/// for an unknown project. Tokens scoped to the project are left as-is (they
/// simply stop resolving once the project is gone). Returns 204 on success.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let force = matches!(first(&pairs, "force"), Some("true" | "1"));
    state.store.delete_project(&project, force, &ctx.actor)?;
    state.wake();
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/projects/{project}/roadmap (read scope) — epic progress rollup. For
/// each epic in the project, returns the epic plus a rollup over its full
/// descendant subtree: counts by state and category, total, done-count,
/// completion percent, and `flags` for an epic whose own state contradicts its
/// children. Alongside `epics`, `unparented` rolls up the non-epic tickets no
/// epic owns, so the response accounts for all of the project's work.
pub async fn roadmap(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let out = state.store.roadmap(&project)?;
    Ok(Json(out))
}

pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Workflow>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let p = state
        .store
        .get_project(&project)?
        .ok_or_else(|| ApiError::not_found("project", &project))?;
    Ok(Json(p.workflow))
}

pub async fn put_workflow(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Workflow>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let wf = parse_workflow(&body)?;
    let stored = state.store.put_workflow(&project, wf, &ctx.actor)?;
    state.wake();
    Ok(Json(stored))
}

fn parse_workflow(raw: &Value) -> ApiResult<Workflow> {
    serde_json::from_value(raw.clone()).map_err(|e| {
        ApiError::validation(
            "workflow.parse",
            format!(
                "The workflow document does not match the expected shape ({e}). Required: name, initial, states (id+category each), transitions (from+to each). See workflow-format.md."
            ),
        )
    })
}
