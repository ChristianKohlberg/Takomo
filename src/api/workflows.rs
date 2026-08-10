//! The workflow library: `GET/POST /v1/workflows`, `GET/PATCH/DELETE
//! /v1/workflows/{id}`.
//!
//! A workflow has always been a column on `projects`, so two projects wanting
//! the same lifecycle each carried their own copy and improving one improved
//! neither the other nor the next project created. This is the shared shelf.
//!
//! It stores documents; it never applies one. Applying stays
//! `PUT /v1/projects/{p}/workflow`, which is the single place the
//! never-strand-a-ticket check runs. A library that could write a project's
//! workflow directly would be a second door into the one operation here that can
//! break a live project.

use super::{body_object, reject_unknown, require_str, ApiJson};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::workflow::Workflow;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

/// Parse and validate a document destined for the library.
///
/// Validated against an EMPTY in-use list: a library entry is not applied to
/// anything, so it cannot strand a ticket yet. The stranding check runs where it
/// belongs — when the document is applied to a project that has tickets.
fn parse_and_validate(raw: &Value) -> ApiResult<Workflow> {
    let wf: Workflow = serde_json::from_value(raw.clone()).map_err(|e| {
        ApiError::validation(
            "workflow.parse",
            format!(
                "The workflow document does not match the expected shape ({e}). Required: name, initial, states (id+category each), transitions (from+to each). See workflow-format.md."
            ),
        )
    })?;
    let problems = wf.validate(&[]);
    if !problems.is_empty() {
        return Err(ApiError::validation(
            "workflow.invalid",
            format!(
                "The workflow definition is invalid: {}. Fix the definition and retry; see workflow-format.md for the format.",
                problems.join("; ")
            ),
        )
        .details(serde_json::json!({ "problems": problems })));
    }
    Ok(wf)
}

/// GET /v1/workflows — every named workflow, built-ins first. Read scope.
///
/// Read, not admin: a non-admin can already see the workflow of any project it
/// can read, and choosing which one to ask an admin for is not privileged.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let rows = state.store.list_workflow_entries()?;
    Ok(Json(Value::Array(
        rows.iter().map(|e| e.to_json()).collect(),
    )))
}

/// GET /v1/workflows/{id}
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let entry = state
        .store
        .get_workflow_entry(&id)?
        .ok_or_else(|| ApiError::not_found("workflow", &id))?;
    Ok(Json(entry.to_json()))
}

/// POST /v1/workflows (admin) — save a named workflow.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["name", "description", "workflow", "layout"])?;

    let name = require_str(obj, "name")?;
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ApiError::validation(
            "workflow.name",
            "Field 'name' must be 1-100 characters.",
        ));
    }
    let raw = obj.get("workflow").ok_or_else(|| {
        ApiError::validation(
            "workflow.missing",
            "Field 'workflow' is required: the state machine document to save. See workflow-format.md.",
        )
    })?;
    let wf = parse_and_validate(raw)?;
    let description = obj.get("description").and_then(|v| v.as_str());
    let layout = obj.get("layout");

    let entry = state
        .store
        .create_workflow_entry(&name, description, &wf, layout, &ctx.actor)?;
    Ok((StatusCode::CREATED, Json(entry.to_json())))
}

/// PATCH /v1/workflows/{id} (admin) — rename, re-describe, or replace the
/// document or the layout. Built-ins refuse.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["name", "description", "workflow", "layout"])?;

    let name = obj.get("name").and_then(|v| v.as_str());
    // `description: null` CLEARS it; an absent key leaves it alone. The nested
    // Option is what distinguishes those two, and collapsing them would make
    // clearing a description impossible.
    let description = obj.get("description").map(|v| v.as_str());
    let wf = obj.get("workflow").map(parse_and_validate).transpose()?;
    let layout = obj
        .get("layout")
        .map(|v| if v.is_null() { None } else { Some(v) });

    let entry = state
        .store
        .patch_workflow_entry(&id, name, description, wf.as_ref(), layout)?;
    Ok(Json(entry.to_json()))
}

/// DELETE /v1/workflows/{id} (admin). Built-ins refuse.
///
/// Deleting does not touch a project already using the document: applying copies
/// it onto the project, so the library is never load-bearing at runtime.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    if !state.store.delete_workflow_entry(&id)? {
        return Err(ApiError::not_found("workflow", &id));
    }
    Ok(StatusCode::NO_CONTENT)
}
