//! Verification — `/v1/projects/{project}/behaviors`, `/v1/behaviors/{id}`,
//! `/v1/projects/{project}/runs` and `/v1/projects/{project}/verification`.
//!
//! Takomo stores; the reporter computes. CI or an agent runs the tests and
//! reports pass/fail per test key; this layer validates shapes and scope, and
//! the store derives each behavior's status from what was reported. See
//! `docs/verification.md`.

use super::{
    body_object, first, get_str, get_string_array, parse_i64_param, query_pairs, reject_unknown,
    require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{
    BehaviorCreate, BehaviorFilter, BehaviorPatch, ResultInput, RunReport, MAX_BEHAVIORS_PAGE,
    MAX_RUNS_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{Map, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 4] = ["title", "statement", "section", "tests"];
const PATCH_FIELDS: [&str; 4] = ["title", "statement", "section", "tests"];
const RUN_FIELDS: [&str; 3] = ["commit", "note", "results"];
const RESULT_FIELDS: [&str; 3] = ["test", "outcome", "detail"];
const MAX_IDEMPOTENCY_HEADER: usize = 128;

/// The section must be a node of this project's plan.
///
/// Checked here rather than in the store because sections live in the plan's
/// CRDT document, not in a table, and only this layer can open it. A project
/// holds at most one plan, which is what makes a bare node id resolvable.
pub async fn validate_section(
    state: &Arc<AppState>,
    project: &str,
    section: &str,
) -> ApiResult<()> {
    let index = crate::api::mindmaps::project_node_index(state, project).await?;
    if index.iter().any(|(_, n)| n.id == section) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.behavior_section",
        format!(
            "No section '{section}' in this project's plan. A behavior names the part of the \
             specification it comes from, so the id has to be one of its sections."
        ),
    )
    .remedy(
        "Read the plan with takomo_plan_read or GET /v1/mindmaps/{id} — every section carries \
         its node id — or leave `section` out for a behavior the specification does not state yet."
            .to_string(),
    ))
}

/// A field that may be absent (leave alone), null (clear) or a string (set).
fn nullable_str(obj: &Map<String, Value>, key: &str) -> ApiResult<Option<Option<String>>> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.trim().to_string()))),
        Some(_) => Err(ApiError::validation(
            "validation.behavior_section",
            format!("'{key}' must be a plan node id string or null."),
        )
        .remedy("Send a section id, or null to unlink it.".to_string())),
    }
}

fn idempotency_key(headers: &HeaderMap) -> ApiResult<Option<String>> {
    let key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string);
    if key
        .as_ref()
        .is_some_and(|k| k.len() > MAX_IDEMPOTENCY_HEADER)
    {
        return Err(ApiError::bad_request(
            "validation.idempotency_key",
            "Idempotency-Key must be at most 128 characters.",
        ));
    }
    Ok(key)
}

/// POST /v1/projects/{project}/behaviors (write).
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
    let section = nullable_str(obj, "section")?.flatten();
    if let Some(s) = &section {
        validate_section(&state, &project, s).await?;
    }
    let req = BehaviorCreate {
        project,
        title: require_str(obj, "title")?,
        statement: get_str(obj, "statement")?.unwrap_or_default(),
        section,
        tests: get_string_array(obj, "tests")?.unwrap_or_default(),
    };
    let behavior = state.store.create_behavior(&req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(behavior.to_json())))
}

/// GET /v1/projects/{project}/behaviors?section=&status=&q=&limit=&offset= (read).
///
/// `section=none` narrows to behaviors tied to no section — the ones the
/// specification does not state yet.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let filter = BehaviorFilter {
        project,
        section: first(&pairs, "section").map(|s| {
            if s == "none" {
                String::new()
            } else {
                s.to_string()
            }
        }),
        status: first(&pairs, "status").map(str::to_string),
        q: first(&pairs, "q").map(str::to_string),
        limit: parse_i64_param(&pairs, "limit")?,
        offset: parse_i64_param(&pairs, "offset")?,
    };
    let limit = filter
        .limit
        .unwrap_or(MAX_BEHAVIORS_PAGE)
        .clamp(1, MAX_BEHAVIORS_PAGE);
    let store = state.clone();
    let (items, total) = super::blocking_read(move || store.store.list_behaviors(&filter)).await?;
    Ok(Json(super::paged(
        items.iter().map(|b| b.to_json()).collect(),
        total,
        limit,
        "Page with ?offset=N, or narrow with ?section=, ?status= or ?q=.",
    )))
}

/// GET /v1/behaviors/{id} (read) — the behavior, each linked test's latest
/// result, and recent history.
pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let existing = state.store.get_behavior(&id)?;
    ctx.require_project(&existing.project)?;
    Ok(Json(state.store.behavior_detail(&id)?))
}

/// PATCH /v1/behaviors/{id} (write). `section: null` unlinks it; `tests`
/// replaces the whole list.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_behavior(&id)?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;
    let section = nullable_str(obj, "section")?;
    if let Some(Some(s)) = &section {
        validate_section(&state, &existing.project, s).await?;
    }
    let patch = BehaviorPatch {
        title: get_str(obj, "title")?,
        statement: get_str(obj, "statement")?,
        section,
        tests: get_string_array(obj, "tests")?,
    };
    let behavior = state.store.patch_behavior(&id, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(behavior.to_json()))
}

/// DELETE /v1/behaviors/{id} (write). Reported results stay.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ctx.require_scope("write")?;
    let existing = state.store.get_behavior(&id)?;
    ctx.require_project(&existing.project)?;
    state.store.delete_behavior(&id, &ctx.actor)?;
    state.wake();
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/projects/{project}/runs (write) — report results. An optional
/// `Idempotency-Key` header makes a retried report record once.
pub async fn report(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &RUN_FIELDS)?;
    let raw = match obj.get("results") {
        Some(Value::Array(items)) => items,
        _ => {
            return Err(ApiError::validation(
                "validation.run_results",
                "'results' must be an array of {test, outcome, detail?} objects.",
            )
            .remedy(
                "Send {\"results\": [{\"test\": \"<key>\", \"outcome\": \"pass\"}]}.".to_string(),
            ))
        }
    };
    let mut results = Vec::with_capacity(raw.len());
    for item in raw {
        let r = body_object(item)?;
        reject_unknown(r, &RESULT_FIELDS)?;
        results.push(ResultInput {
            test: require_str(r, "test")?,
            outcome: require_str(r, "outcome")?,
            detail: get_str(r, "detail")?,
        });
    }
    let req = RunReport {
        project,
        commit: get_str(obj, "commit")?,
        note: get_str(obj, "note")?,
        results,
        idempotency_key: idempotency_key(&headers)?,
        user: ctx.user.clone(),
    };
    let (mut out, replayed) = state.store.report_run(&req, &ctx.actor)?;
    state.wake();
    out["replayed"] = serde_json::json!(replayed);
    let status = if replayed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(out)))
}

/// GET /v1/projects/{project}/runs?limit=&offset= (read), newest first.
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let limit = parse_i64_param(&pairs, "limit")?;
    let offset = parse_i64_param(&pairs, "offset")?;
    let (items, total) = state.store.list_runs(&project, limit, offset)?;
    Ok(Json(super::paged(
        items,
        total,
        limit.unwrap_or(50).clamp(1, MAX_RUNS_PAGE),
        "Page with ?offset=N or raise ?limit= (max 200).",
    )))
}

/// GET /v1/projects/{project}/verification (read) — status counts overall and
/// per section, and reported tests no behavior links.
pub async fn summary(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let store = state.clone();
    let out = super::blocking_read(move || store.store.verification_summary(&project)).await?;
    Ok(Json(out))
}
