//! /v1/projects and per-project workflow endpoints.

use super::{blocking_read, body_object, first, query_pairs, reject_unknown, require_str, ApiJson};
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;
    reject_unknown(
        obj,
        &[
            "id",
            "name",
            "workflow",
            "question_language",
            "style_guide",
            "answer_link_ttl_seconds",
            "claim_ttl_seconds",
            "max_claim_ttl_seconds",
        ],
    )?;
    let id = require_str(obj, "id")?;
    let name = require_str(obj, "name")?;
    ctx.require_project(&id)?;
    let workflow = match obj.get("workflow") {
        None | Some(Value::Null) => None,
        Some(raw) => Some(parse_workflow(raw)?),
    };
    // Validated before the insert so an oversized guide is a clean 422 rather
    // than a created-but-unconfigured project.
    let style =
        crate::store::normalize_style_guide(super::get_str(obj, "style_guide")?.as_deref())?;
    // Same reason: an out-of-range answer-link default is a 422 before the row
    // exists, not a project created with the setting silently dropped.
    let link_ttl =
        crate::store::normalize_answer_link_ttl(super::get_i64(obj, "answer_link_ttl_seconds")?)?;
    // Same reason again, and as a pair: a lease default above its ceiling is a
    // 422 before the row exists, not a project whose every claim is clamped.
    let (claim_ttl, max_claim_ttl) = crate::store::normalize_claim_ttls(
        super::get_i64(obj, "claim_ttl_seconds")?,
        super::get_i64(obj, "max_claim_ttl_seconds")?,
    )?;
    let mut project = state
        .store
        .create_project(&id, &name, workflow, &ctx.actor)?;
    // Optional per-project human-facing question language, set at creation.
    if let Some(lang) = super::get_str(obj, "question_language")? {
        project = state
            .store
            .set_question_language(&id, Some(&lang), &ctx.actor)?;
    }
    // Optional per-project style guide for agent-written text, set at creation.
    if let Some(style) = style {
        project = state.store.set_style_guide(&id, Some(&style), &ctx.actor)?;
    }
    // Optional per-project answer-link default lifetime, set at creation.
    if let Some(ttl) = link_ttl {
        project = state
            .store
            .set_answer_link_ttl(&id, Some(ttl), &ctx.actor)?;
    }
    // Optional per-project lease policy, set at creation.
    if claim_ttl.is_some() || max_claim_ttl.is_some() {
        project = state
            .store
            .set_claim_ttls(&id, claim_ttl, max_claim_ttl, &ctx.actor)?;
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["language"])?;
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

/// PUT /v1/projects/{project}/style (admin) — set the project's style guide:
/// the house style agents should write ticket text and human-facing questions
/// in. Body: `{"style_guide": "Keep it short…"}`, or `{"style_guide": null}` to
/// clear it. Advisory, like the language setting — nothing is enforced.
pub async fn put_style(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["style_guide"])?;
    // `style_guide` present-and-null clears it; a string sets it; absent is an
    // error, so a typo'd field name can never silently clear the guide.
    let style = match obj.get("style_guide") {
        None => return Err(ApiError::bad_request(
            "validation.field_required",
            "Field 'style_guide' is required (a string of writing conventions, or null to clear).",
        )),
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(ApiError::bad_request(
                "validation.field_type",
                "Field 'style_guide' must be a string or null.",
            ))
        }
    };
    let project = state
        .store
        .set_style_guide(&project, style.as_deref(), &ctx.actor)?;
    state.wake();
    Ok(Json(project.to_json()))
}

/// PUT /v1/projects/{project}/answer-link-ttl (admin) — set how long an answer
/// link minted for one of this project's questions stays valid. Body:
/// `{"ttl_seconds": 604800}`, or `{"ttl_seconds": null}` to clear the default
/// and fall back to the built-in 7 days.
///
/// Admin-only, like the other two project settings and for a sharper reason: an
/// answer link is a bearer credential handed to someone outside the org, so how
/// long it lives is not a preference a worker token gets to change.
pub async fn put_answer_link_ttl(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["ttl_seconds"])?;
    // Present-and-null clears it; an integer sets it; absent is an error, so a
    // typo'd field name can never silently reset the project to the default.
    let ttl = match obj.get("ttl_seconds") {
        None => {
            return Err(ApiError::bad_request(
                "validation.field_required",
                "Field 'ttl_seconds' is required (a positive number of seconds up to 2592000, or null to clear the project default and fall back to 7 days).",
            ))
        }
        Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| {
            ApiError::bad_request(
                "validation.field_type",
                "Field 'ttl_seconds' must be an integer number of seconds or null.",
            )
        })?),
    };
    let project = state.store.set_answer_link_ttl(&project, ttl, &ctx.actor)?;
    state.wake();
    Ok(Json(project.to_json()))
}

/// PUT /v1/projects/{project}/claim-ttl (admin) — set this project's lease
/// policy. Body: `{"ttl_seconds": 1800, "max_ttl_seconds": 7200}`, either one
/// null to clear it and fall back to the built-in (900 / 3600).
///
/// One endpoint for both because they are validated as a pair: a default above
/// the ceiling would be silently clamped on every claim. Two endpoints would
/// force an ordering (raise the cap first, or the default is refused) and let a
/// half-applied change through when the second call failed.
///
/// Not to be confused with `/answer-link-ttl`: that bounds a credential handed to
/// someone outside the org, this bounds how long a worker may hold a ticket.
pub async fn put_claim_ttl(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["ttl_seconds", "max_ttl_seconds"])?;
    // At least one field must be present, so a typo'd name cannot silently reset
    // the whole policy to the built-in. Each present-and-null clears that half.
    if !obj.contains_key("ttl_seconds") && !obj.contains_key("max_ttl_seconds") {
        return Err(ApiError::bad_request(
            "validation.field_required",
            "Send 'ttl_seconds' (the default lease a claim gets) and/or 'max_ttl_seconds' (the \
             ceiling an explicit ttl_seconds is checked against) — a positive number of seconds, \
             or null to clear that one and fall back to the built-in (900 default, 3600 max). \
             Both are omitted here, and this endpoint replaces both, so it would be a no-op.",
        ));
    }
    // Absent means "leave as it is", which is only knowable by reading the
    // current row: this endpoint writes both columns in one UPDATE, so an absent
    // field must be re-sent as its stored value rather than as NULL.
    let current = state
        .store
        .get_project(&project)?
        .ok_or_else(|| ApiError::not_found("project", &project))?;
    let field = |name: &str, stored: Option<i64>| -> ApiResult<Option<i64>> {
        match obj.get(name) {
            None => Ok(stored),
            Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(v.as_i64().ok_or_else(|| {
                ApiError::bad_request(
                    "validation.field_type",
                    format!("Field '{name}' must be an integer number of seconds or null."),
                )
            })?)),
        }
    };
    let ttl = field("ttl_seconds", current.claim_ttl_seconds)?;
    let max_ttl = field("max_ttl_seconds", current.max_claim_ttl_seconds)?;
    let project = state
        .store
        .set_claim_ttls(&project, ttl, max_ttl, &ctx.actor)?;
    state.wake();
    Ok(Json(project.to_json()))
}

/// POST /v1/projects/{project}/archive (admin) — freeze the project.
///
/// Every write under it is then refused with a 409 `project.archived` — tickets,
/// claims, transitions, comments, questions, tags, schedules, checklist, and the
/// project's own settings — and its tickets leave the ready queue. Reads are
/// untouched, so the board, the history and the export all still answer.
///
/// `?force=true` archives even while a worker holds a lease, releasing those
/// leases (see [`Store::set_project_archived`]). Without it, a live claim is a
/// 409 `project.active_claims`, because archiving would otherwise freeze that
/// worker mid-lease with no call left that it could make. Same spelling as
/// `DELETE /v1/projects/{project}?force=true`, which overrides the same guard.
///
/// A POST rather than a `PUT .../archived` flag: this is an act with
/// consequences for everyone working the project, not a field being set, and the
/// undo is a different act with a different name. It takes no body at all — a
/// caller with nothing to say should not have to send `{}`.
///
/// [`Store::set_project_archived`]: crate::store::Store::set_project_archived
pub async fn archive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let force = matches!(first(&pairs, "force"), Some("true" | "1"));
    let project = state
        .store
        .set_project_archived(&project, true, force, &ctx.actor)?;
    // Wakes the long-pollers so a worker parked on /v1/ready re-runs the query
    // and stops seeing this project's tickets, instead of waiting out its poll
    // against a queue that has already changed.
    state.wake();
    Ok(Json(project.to_json()))
}

/// POST /v1/projects/{project}/unarchive (admin) — put the project back to work.
///
/// The undo, and the reason archiving is safe to reach for: nothing was moved or
/// deleted, so this restores the project exactly as it stood. Idempotent on a
/// live project.
pub async fn unarchive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let project = state
        .store
        .set_project_archived(&project, false, false, &ctx.actor)?;
    state.wake();
    Ok(Json(project.to_json()))
}

/// DELETE /v1/projects/{project} (admin) — cascade-delete the project and every
/// ticket, comment, dep, event, question (with its follow-up thread and answer
/// grants), promotion, and tag registry entry under it, in one transaction.
/// Refuses with 409 when a ticket holds an active claim unless `?force=true` is
/// passed; 404 for an unknown project. Tokens scoped to the project are left as-is (they
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
    // A rollup per epic over its whole descendant subtree: a scan, off the
    // runtime (see `blocking_read`).
    let state = state.clone();
    let out = blocking_read(move || state.store.roadmap(&project)).await?;
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Workflow>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let wf = parse_workflow(&body)?;
    let stored = state.store.put_workflow(&project, wf, &ctx.actor)?;
    state.wake();
    Ok(Json(stored))
}

/// POST /v1/projects/{project}/workflow/validate (admin) — would this document
/// be accepted, without writing it?
///
/// The editor needs to tell someone their draft is wrong WHILE they edit it, and
/// the rules that decide are subtle: reverse-BFS terminal reachability, the
/// claimable-with-done-category trap, and — the one no client could compute —
/// whether any ticket currently sits in a state the draft drops. Reimplementing
/// that in the browser would be a second copy of the rules that drifts, so the
/// editor asks the server the same question `put_workflow` answers, minus the
/// write.
///
/// A separate route rather than `?dry_run=1` on the PUT: a query parameter that
/// silently turns a write into a read is the kind of thing a proxy strips, a log
/// hides, and a caller forgets — and getting it wrong writes the database.
pub async fn validate_workflow_dry_run(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let wf = parse_workflow(&body)?;
    // Same call `put_workflow` makes, so "valid here" and "accepted there" can
    // never disagree — including the stranded-ticket rule, which needs the
    // project's live states.
    let problems = state.store.workflow_problems(&project, &wf)?;
    Ok(Json(serde_json::json!({
        "valid": problems.is_empty(),
        "problems": problems,
    })))
}

/// GET /v1/projects/{project}/workflow-layout — where the editor's nodes sit.
pub async fn get_workflow_layout(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let layout = state.store.get_workflow_layout(&project)?;
    Ok(Json(serde_json::json!({ "layout": layout })))
}

/// PUT /v1/projects/{project}/workflow-layout (admin) — store node positions.
///
/// Deliberately NOT part of `put_workflow`. Moving a node changes nothing about
/// how the project behaves, so it emits no `workflow_changed` event and does not
/// wake long-pollers: a board that refetched every time someone dragged a box
/// would be reacting to nothing. It also cannot live INSIDE the document —
/// `Workflow` is `deny_unknown_fields`, so a `positions` key would 422 on the
/// way back in.
pub async fn put_workflow_layout(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["layout"])?;
    let layout = obj.get("layout").ok_or_else(|| {
        ApiError::validation(
            "layout.missing",
            "Field 'layout' is required: an object of state id -> {x, y}.",
        )
    })?;
    state.store.put_workflow_layout(&project, layout)?;
    Ok(Json(serde_json::json!({ "layout": layout })))
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
