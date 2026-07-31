//! /v1/schedules — recurrence rules that materialize ordinary tickets.
//!
//! The scope split here is the security decision in the feature, so it is worth
//! stating plainly. **Creating** a schedule needs only `write`, because an agent
//! noticing "this keeps coming back every week" and proposing a cadence is a
//! genuinely good behaviour we want. What that agent creates is inert: it lands
//! `pending` with `next_slot = NULL`, and the sweep's partial index cannot see it.
//!
//! **Activating** one needs `human`. That is what keeps the escalation shut: a
//! schedule outlives the token that made it, so a `write` credential able to
//! start one could write tickets long after it was revoked. A `human` caller's
//! own schedule is born active — asking someone to approve their own proposal is
//! theatre when they already hold the authority the flag protects.
//!
//! The per-project `schedule_approval` flag is what an operator turns off to let
//! a fleet schedule its own recurring work. Default on.

use super::{
    body_object, first, get_str, parse_i64_param, query_pairs, reject_unknown, require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::schedule::Cadence;
use crate::server::AppState;
use crate::store::{
    ScheduleCreate, ScheduleListFilter, SchedulePatch, ScheduleTemplate, MAX_SCHEDULES_PAGE,
    SCHEDULE_STATUSES,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 7] = [
    "project",
    "name",
    "cadence",
    "template",
    "starts_at",
    "ends_at",
    "rationale",
];
const PATCH_FIELDS: [&str; 4] = ["name", "cadence", "template", "ends_at"];

/// How many upcoming slots a create/get response previews.
///
/// Three, because the point is for a caller — or a human about to approve a
/// proposal — to see *what they just bought* without having to run the cadence in
/// their head. One is not a pattern; ten is a wall.
const PREVIEW_SLOTS: usize = 3;

/// Turn a cadence-parse failure into the house error.
///
/// The message already carries the teaching (including the cron translation), so
/// this only attaches the code and the status.
fn cadence_error(msg: String) -> ApiError {
    ApiError::validation("validation.schedule.cadence", msg)
        .remedy("See spec/schedule-format.md for the cadence grammar.")
}

fn template_error(msg: String) -> ApiError {
    ApiError::validation("validation.schedule.template", msg)
        .remedy("See spec/schedule-format.md for the template fields.")
}

fn parse_cadence(obj: &serde_json::Map<String, Value>) -> ApiResult<Cadence> {
    let raw = obj.get("cadence").ok_or_else(|| {
        cadence_error(
            "cadence is required. It is an object: every (day|week|month), interval (optional), \
             on (week only), day (month only), at (HH:MM), tz (optional IANA name, default UTC)."
                .to_string(),
        )
    })?;
    Cadence::parse(raw).map_err(cadence_error)
}

fn parse_template(obj: &serde_json::Map<String, Value>) -> ApiResult<ScheduleTemplate> {
    let raw = obj.get("template").ok_or_else(|| {
        template_error(
            "template is required — it is the ticket each occurrence creates. At minimum \
             {\"title\":\"…\"}."
                .to_string(),
        )
    })?;
    ScheduleTemplate::parse(raw).map_err(template_error)
}

/// Parse an optional RFC 3339 instant field into unix ms.
fn parse_instant(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<i64>> {
    // Dispatch on the JSON type rather than trying one accessor then the other:
    // `get_i64` *errors* on a string instead of declining it, so an
    // integer-first version would reject every RFC 3339 timestamp before it ever
    // reached the string branch.
    //
    // Integers are accepted at all because the store speaks unix ms, and a caller
    // echoing back a value it read from `/v1/events` should not have to convert
    // it first.
    let bad = |got: &str| {
        ApiError::validation(
            "validation.schedule.instant",
            format!(
                "'{key}' must be an RFC 3339 timestamp (e.g. \"2026-08-03T09:00:00Z\") or unix milliseconds, got {got}."
            ),
        )
    };
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| bad("a fractional number")),
        Some(Value::String(raw)) => chrono::DateTime::parse_from_rfc3339(raw)
            .map(|d| Some(d.timestamp_millis()))
            .map_err(|e| bad(&format!("'{raw}' ({e})"))),
        Some(other) => Err(bad(&format!("{other}"))),
    }
}

/// POST /v1/schedules (write) — create a schedule.
///
/// Lands `pending` when the project requires approval and the caller is not a
/// human; `active` otherwise. The response says which, in words an agent will
/// act on rather than leaving it to infer from a status string.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CREATE_FIELDS)?;
    let project = require_str(obj, "project")?;
    ctx.require_project(&project)?;

    let req = ScheduleCreate {
        project: project.clone(),
        name: require_str(obj, "name")?,
        cadence: parse_cadence(obj)?,
        template: parse_template(obj)?,
        starts_at: parse_instant(obj, "starts_at")?,
        ends_at: parse_instant(obj, "ends_at")?,
        rationale: get_str(obj, "rationale")?,
    };

    // A human's own schedule is born active whatever the flag says; the flag
    // exists to gate what an agent proposes.
    let is_human = ctx.scopes.contains("human");
    let needs_approval = !is_human && state.store.schedule_approval_required(&project)?;

    let sched = state
        .store
        .create_schedule(&req, &ctx.actor, needs_approval)?;
    state.wake();
    let mut out = sched.to_json(&state.store.upcoming_slots(&sched, PREVIEW_SLOTS));
    if sched.status == "pending" {
        out["message"] = json!(
            "Recorded, but NOT active: this project requires a human to activate a schedule an \
             agent proposed, so nothing will fire yet. Do not wait on it — finish your ticket. A \
             human activates it with POST /v1/schedules/{id}/activate, and `upcoming` shows the \
             slots it would then use."
        );
    }
    Ok((StatusCode::CREATED, Json(out)))
}

/// GET /v1/schedules (read) — list, newest first within status groups.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(q): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(q.as_deref());
    if let Some(p) = first(&pairs, "project") {
        ctx.require_project(p)?;
    }
    let status = first(&pairs, "status").map(|s| s.to_string());
    if let Some(s) = &status {
        if !SCHEDULE_STATUSES.contains(&s.as_str()) {
            return Err(ApiError::validation(
                "validation.schedule.status",
                format!(
                    "status must be one of {}, got '{s}'.",
                    SCHEDULE_STATUSES.join(", ")
                ),
            ));
        }
    }
    let filter = ScheduleListFilter {
        project: first(&pairs, "project").map(|s| s.to_string()),
        status,
        allowed_projects: ctx.allowed_projects_vec(),
    };
    let limit = parse_i64_param(&pairs, "limit")?.unwrap_or(MAX_SCHEDULES_PAGE);
    let rows = state.store.list_schedules(&filter, limit)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|s| s.to_json(&state.store.upcoming_slots(s, 1)))
        .collect();
    Ok(Json(json!({ "schedules": items })))
}

/// GET /v1/schedules/{id} (read) — one schedule, its next slots, and its
/// occurrence history with each outcome derived.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("read")?;
    let sched = state.store.get_schedule(&id)?;
    ctx.require_project(&sched.project)?;
    let pairs = query_pairs(q.as_deref());
    let history = parse_i64_param(&pairs, "occurrences")?.unwrap_or(8);
    let occurrences = state.store.schedule_occurrences(&id, history)?;
    let mut out = sched.to_json(&state.store.upcoming_slots(&sched, PREVIEW_SLOTS));
    out["occurrences"] = json!(occurrences.iter().map(|o| o.to_json()).collect::<Vec<_>>());
    if sched.cadence.is_none() {
        // A row whose cadence no longer parses is corrupt, not a schedule with
        // default behaviour. Say so rather than rendering a plausible blank.
        out["cadence_error"] = json!(format!(
            "the stored cadence does not parse: {}",
            sched.cadence_raw
        ));
    }
    let etag = format!("\"{}\"", sched.version);
    Ok(([("ETag", etag)], Json(out)))
}

/// GET /v1/schedules/{id}/occurrences (read) — the history on its own, for a
/// longer window than the detail view previews.
pub async fn occurrences(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let sched = state.store.get_schedule(&id)?;
    ctx.require_project(&sched.project)?;
    let pairs = query_pairs(q.as_deref());
    let limit = parse_i64_param(&pairs, "limit")?.unwrap_or(40);
    let rows = state.store.schedule_occurrences(&id, limit)?;
    Ok(Json(json!({
        "schedule": id,
        "occurrences": rows.iter().map(|o| o.to_json()).collect::<Vec<_>>(),
    })))
}

/// PATCH /v1/schedules/{id} (human) — edit the rule behind an If-Match.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("human")?;
    let existing = state.store.get_schedule(&id)?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;

    let patch = SchedulePatch {
        name: get_str(obj, "name")?,
        cadence: match obj.get("cadence") {
            Some(v) if !v.is_null() => Some(Cadence::parse(v).map_err(cadence_error)?),
            _ => None,
        },
        template: match obj.get("template") {
            Some(v) if !v.is_null() => Some(ScheduleTemplate::parse(v).map_err(template_error)?),
            _ => None,
        },
        // Present-and-null clears the window; absent leaves it alone.
        ends_at: match obj.get("ends_at") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(_) => Some(parse_instant(obj, "ends_at")?),
        },
    };
    let if_match = parse_if_match(&headers)?;
    let sched = state
        .store
        .patch_schedule(&id, &patch, if_match, &ctx.actor)?;
    state.wake();
    let etag = format!("\"{}\"", sched.version);
    Ok((
        [("ETag", etag)],
        Json(sched.to_json(&state.store.upcoming_slots(&sched, PREVIEW_SLOTS))),
    ))
}

/// DELETE /v1/schedules/{id} (human) — remove the rule. Its tickets stay.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let sched = state.store.get_schedule(&id)?;
    ctx.require_project(&sched.project)?;
    state.store.delete_schedule(&id, &ctx.actor)?;
    Ok(Json(json!({
        "deleted": id,
        "note": "The tickets this schedule created were kept — they are real work with real \
                 history, and their `schedule` field remains as the record of where they came from.",
    })))
}

/// The four status moves share one handler: they differ only in the target.
async fn move_status(
    state: Arc<AppState>,
    ctx: AuthCtx,
    id: String,
    to: &str,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let existing = state.store.get_schedule(&id)?;
    ctx.require_project(&existing.project)?;
    let sched = state.store.set_schedule_status(&id, to, &ctx.actor)?;
    state.wake();
    Ok(Json(sched.to_json(
        &state.store.upcoming_slots(&sched, PREVIEW_SLOTS),
    )))
}

/// POST /v1/schedules/{id}/activate (human) — the human half of a proposal.
pub async fn activate(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    move_status(state, ctx, id, "active").await
}

/// POST /v1/schedules/{id}/reject (human) — turn a proposal down. Terminal.
pub async fn reject(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    move_status(state, ctx, id, "rejected").await
}

/// POST /v1/schedules/{id}/pause (human) — stop firing, keep the history.
pub async fn pause(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    move_status(state, ctx, id, "paused").await
}

/// POST /v1/schedules/{id}/resume (human) — resume from the *next* slot. The
/// pause is never backfilled.
pub async fn resume(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    move_status(state, ctx, id, "active").await
}

/// POST /v1/schedules/{id}/run (human) — fire now, off-cycle.
///
/// The cadence is not shifted: `next_slot` still advances to the next real slot,
/// so a manual run is an extra occurrence rather than a moved one.
pub async fn run_now(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let existing = state.store.get_schedule(&id)?;
    ctx.require_project(&existing.project)?;
    let ticket = state.store.run_schedule_now(&id, &ctx.actor)?;
    state.wake();
    match ticket {
        Some(t) => Ok(Json(
            json!({ "schedule": id, "ticket": t, "created": true }),
        )),
        // Not an error: the slot was already materialized, which is the
        // exactly-once guarantee doing its job.
        None => Ok(Json(json!({
            "schedule": id,
            "ticket": Value::Null,
            "created": false,
            "note": "That occurrence already exists — one ticket per slot is enforced by a unique \
                     index, so nothing was created. GET /v1/schedules/{id} shows the history.",
        }))),
    }
}

/// PUT /v1/projects/{project}/schedule-approval (admin) — require, or stop
/// requiring, a human to activate an agent-proposed schedule.
pub async fn put_approval(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["required"])?;
    let required = obj
        .get("required")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            ApiError::validation(
                "validation.schedule.approval",
                "'required' must be a boolean. true = a schedule an agent proposes lands pending \
                 until a human activates it; false = it goes live immediately.",
            )
        })?;
    let now = state
        .store
        .set_schedule_approval(&project, required, &ctx.actor)?;
    Ok(Json(
        json!({ "project": project, "schedule_approval": now }),
    ))
}

fn parse_if_match(headers: &HeaderMap) -> ApiResult<Option<i64>> {
    let Some(raw) = headers.get("If-Match").and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let trimmed = raw.trim().trim_matches('"');
    trimmed.parse::<i64>().map(Some).map_err(|_| {
        ApiError::validation(
            "validation.if_match",
            format!(
                "If-Match must be the schedule version as returned in the ETag header, e.g. If-Match: \"3\" (got '{raw}')."
            ),
        )
    })
}
