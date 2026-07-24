//! /v1/questions: the "ask a human" board API.
//!
//! `POST /questions` (write) raises a question and parks the ticket;
//! `POST /questions/{id}/answer` (human) records the reply and resumes the
//! ticket; `GET /questions` is the inbox read-model. See `store/questions.rs`.

use super::{
    all, body_object, first, get_i64, get_str, get_string_array, query_pairs, require_str,
};
use crate::auth::{AnswerCtx, AuthCtx};
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::server::AppState;
use crate::store::{
    AskRequest, QuestionFilter, TimeoutAction, DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

const ASK_FIELDS: [&str; 11] = [
    "ticket",
    "mode",
    "kind",
    "title",
    "body",
    "options",
    "recommended",
    "expertise",
    "urgency",
    "expires_in_seconds",
    "on_timeout",
];

/// Expertise tags a token covers, derived from its free-form `expert:<tag>`
/// scopes. `expert:domain:billing` -> `domain:billing`.
fn my_expertise(ctx: &AuthCtx) -> Vec<String> {
    let mut tags: Vec<String> = ctx
        .scopes
        .iter()
        .filter_map(|s| s.strip_prefix("expert:").map(str::to_string))
        .collect();
    tags.sort();
    tags
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    if let Some(p) = first(&pairs, "project") {
        ctx.require_project(p)?;
    }

    // `mine=true` scopes the inbox to the caller's own expertise tags.
    let expertise = if matches!(first(&pairs, "mine"), Some("true" | "1")) {
        let tags = my_expertise(&ctx);
        if tags.is_empty() {
            // No expert scopes: nothing is "mine".
            return Ok(Json(
                json!({ "items": [], "note": "Your token carries no expert:<tag> scopes, so no questions are routed to you. Drop ?mine=true to see the whole queue." }),
            ));
        }
        tags
    } else {
        all(&pairs, "expertise")
    };

    let filter = QuestionFilter {
        project: first(&pairs, "project").map(str::to_string),
        ticket: first(&pairs, "ticket").map(str::to_string),
        statuses: first(&pairs, "status")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        expertise,
        allowed_projects: ctx.allowed_projects_vec(),
    };
    let questions = state.store.list_questions(&filter)?;
    Ok(Json(json!({
        "items": questions.iter().map(|q| q.to_json()).collect::<Vec<_>>(),
    })))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    let unknown: Vec<&String> = obj
        .keys()
        .filter(|k| !ASK_FIELDS.contains(&k.as_str()) && k.as_str() != "fence")
        .collect();
    if !unknown.is_empty() {
        return Err(ApiError::bad_request(
            "validation.unknown_field",
            format!(
                "Unknown field(s): {}. Accepted: {}, fence.",
                unknown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                ASK_FIELDS.join(", ")
            ),
        ));
    }

    let ticket = require_str(obj, "ticket")?;
    // Enforce project scoping via the ticket's project.
    let t = state
        .store
        .get_ticket(&ticket)?
        .ok_or_else(|| ApiError::not_found("ticket", &ticket))?;
    ctx.require_project(&t.project)?;

    let expires_at = match get_i64(obj, "expires_in_seconds")? {
        Some(secs) if secs > 0 => Some(now_ms() + secs * 1000),
        _ => None,
    };
    let on_timeout = match get_str(obj, "on_timeout")? {
        Some(raw) => Some(TimeoutAction::parse(&raw)?),
        None => None,
    };

    let req = AskRequest {
        ticket,
        mode: get_str(obj, "mode")?,
        kind: require_str(obj, "kind")?,
        title: require_str(obj, "title")?,
        body: get_str(obj, "body")?.unwrap_or_default(),
        options: get_string_array(obj, "options")?.unwrap_or_default(),
        recommended: obj
            .get("recommended")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null),
        expertise: get_string_array(obj, "expertise")?.unwrap_or_default(),
        urgency: get_str(obj, "urgency")?,
        expires_at,
        on_timeout,
        fence: get_i64(obj, "fence")?,
    };

    let (question, ticket) = state.store.ask_question(&req, &ctx.actor)?;
    state.wake();
    crate::notify::question_asked(&state, &question);
    let lang_note = match state.store.get_project(&question.project)? {
        Some(p) => p
            .question_language
            .filter(|l| !l.trim().is_empty())
            .map(|l| format!(" This project expects the question (and any options) written in {l} — re-ask in {l} if this one wasn't.")),
        None => None,
    }
    .unwrap_or_default();
    let note = if question.mode == "advisory" {
        format!(
            "Advisory question recorded on '{}' — no state change, no lease effect. A human answers via the board or POST /v1/questions/{}/answer; the decision is recorded (the ticket is not resumed).",
            ticket.id, question.id
        )
    } else {
        format!(
            "Ticket parked in '{}' and your lease released. A human answers via the board or POST /v1/questions/{}/answer; the ticket resumes once every open question on it is answered. Re-check with GET /v1/tickets/{} later.",
            ticket.state, question.id, ticket.id
        )
    };
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "question": question.to_json(),
            "ticket": ticket.to_json(now_ms()),
            "note": note + &lang_note,
        })),
    ))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    Ok(Json(q.to_json()))
}

pub async fn answer(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    // Answering is the human authorization gate — it performs the ticket's
    // human-gated resume transition.
    ctx.require_scope("human")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;

    let obj = body_object(&body)?;
    reject_unknown(obj, &["answer", "resume_to"])?;
    let answer = obj
        .get("answer")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(
                "validation.answer_required",
                "Field 'answer' is required. For confirm/approve send true/false; for choose the option string; for clarify the explanation text. A note goes inside: {\"answer\": {\"value\": ..., \"note\": \"...\"}}.",
            )
        })?;
    let resume_to = get_str(obj, "resume_to")?;

    let (question, ticket) =
        state
            .store
            .answer_question(&id, &ctx.actor, &ctx.scopes, &answer, resume_to.as_deref())?;
    state.wake();
    Ok(Json(json!({
        "question": question.to_json(),
        "ticket": ticket.to_json(now_ms()),
    })))
}

pub async fn withdraw(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    // Body is optional; when present, only `reason` is accepted.
    if let Value::Object(obj) = &body {
        reject_unknown(obj, &["reason"])?;
    }
    let reason = body
        .as_object()
        .and_then(|o| o.get("reason"))
        .and_then(|v| v.as_str());
    let question = state.store.withdraw_question(&id, &ctx.actor, reason)?;
    state.wake();
    Ok(Json(question.to_json()))
}

// ---------------------------------------------------------------------------
// Answer links: a per-question, expiring, write-once grant (see auth::AnswerCtx
// and store/answer_grants.rs). Minting/revoking run on the normal token path;
// the `/v1/answer/self*` endpoints run on the distinct answer-grant auth path.

const ANSWER_LINK_FIELDS: [&str; 2] = ["ttl_seconds", "actor"];

/// POST /v1/questions/{id}/answer-link (human scope) — mint a scoped, expiring,
/// single-use link an outside expert can use to answer this one question. You
/// can only delegate authority you hold: minting for an `approve` question
/// requires the matching `expert:<tag>` scope. Returns `{token, path, ...}` —
/// `token` (a `tka_...`) is shown ONCE.
pub async fn create_link(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("human")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    if q.status != "open" {
        return Err(ApiError::conflict(
            "question.not_open",
            format!(
                "Question '{id}' is '{}', not open — there is nothing to answer.",
                q.status
            ),
        ));
    }
    // Can't delegate an approval you couldn't give yourself.
    if q.kind == "approve" {
        let has_expert = q
            .expertise
            .iter()
            .any(|t| ctx.scopes.contains(&format!("expert:{t}")));
        if !has_expert {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "question.approve_expertise",
                format!(
                    "Minting an answer link for this 'approve' question needs a matching domain expert scope ({}). You can only delegate authority you hold.",
                    q.expertise.iter().map(|t| format!("expert:{t}")).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }

    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let obj = body_object(&body)?;
    reject_unknown(obj, &ANSWER_LINK_FIELDS)?;
    let ttl = match get_i64(obj, "ttl_seconds")? {
        None => DEFAULT_ANSWER_TTL_SECONDS,
        Some(s) if s <= 0 => {
            return Err(ApiError::validation(
                "answer_link.ttl",
                "Field 'ttl_seconds' must be a positive integer number of seconds.",
            ))
        }
        Some(s) if s > MAX_ANSWER_TTL_SECONDS => {
            return Err(ApiError::validation(
                "answer_link.ttl",
                format!("Field 'ttl_seconds' exceeds the maximum of {MAX_ANSWER_TTL_SECONDS} seconds (30 days)."),
            ))
        }
        Some(s) => s,
    };
    // Who the answer is attributed to; defaults to a link-scoped actor.
    let actor = get_str(obj, "actor")?.unwrap_or_else(|| format!("human:link:{id}"));

    let expires_at = now_ms() + ttl * 1000;
    let (row, plaintext) = state
        .store
        .create_answer_grant(&id, &q.project, &actor, expires_at, &ctx.actor)?;

    let mut out = row.to_json();
    if let Value::Object(map) = &mut out {
        map.insert("token".to_string(), Value::String(plaintext.clone()));
        map.insert(
            "path".to_string(),
            Value::String(format!("/board#a={plaintext}")),
        );
        if let Ok(base) = std::env::var("TAKOMO_PUBLIC_URL") {
            if !base.trim().is_empty() {
                map.insert(
                    "url".to_string(),
                    Value::String(format!(
                        "{}/board#a={plaintext}",
                        base.trim_end_matches('/')
                    )),
                );
            }
        }
        map.insert(
            "warning".to_string(),
            Value::String(
                "This answer-link token is shown ONCE. Anyone with the link can answer this one question until it expires or is used (single-use). Share it only with the intended person."
                    .to_string(),
            ),
        );
    }
    Ok((StatusCode::CREATED, Json(out)))
}

/// DELETE /v1/answer-links/{id} (human scope) — revoke an answer link. Allowed
/// for its creator or an admin. Immediate: the token then returns 410.
pub async fn revoke_link(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("human")?;
    let grant = state
        .store
        .get_answer_grant(&id)?
        .ok_or_else(|| ApiError::not_found("answer-link", &id))?;
    if grant.created_by != ctx.actor && !ctx.scopes.contains("admin") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "answer_link.not_owner",
            "Only the link's creator or an admin can revoke it.",
        ));
    }
    state.store.revoke_answer_grant(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/answer/self (answer-grant auth) — the one question this link can
/// answer, plus minimal ticket context, so the board can render it.
pub async fn self_get(
    State(state): State<Arc<AppState>>,
    Extension(grant): Extension<AnswerCtx>,
) -> ApiResult<Json<Value>> {
    let q = state
        .store
        .get_question(&grant.question)?
        .ok_or_else(|| ApiError::not_found("question", &grant.question))?;
    let ticket = state.store.get_ticket(&q.ticket)?;
    Ok(Json(json!({
        "question": q.to_json(),
        "ticket": ticket.map(|t| json!({ "id": t.id, "title": t.title, "state": t.state })),
        "expires_at": iso(grant.expires_at),
    })))
}

/// POST /v1/answer/self (answer-grant auth) — answer the one question. The grant
/// IS the human authorization: it answers as its recorded actor with a
/// synthesized scope set (`human` plus the question's expertise), so it can
/// satisfy the human gate and an `approve`'s expert requirement for THIS
/// question only. Marks the grant used (single-use) on success.
pub async fn self_answer(
    State(state): State<Arc<AppState>>,
    Extension(grant): Extension<AnswerCtx>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let q = state
        .store
        .get_question(&grant.question)?
        .ok_or_else(|| ApiError::not_found("question", &grant.question))?;

    let obj = body_object(&body)?;
    reject_unknown(obj, &["answer", "resume_to"])?;
    let answer = obj
        .get("answer")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(
                "validation.answer_required",
                "Field 'answer' is required (a value, or { value, note }).",
            )
        })?;
    let resume_to = get_str(obj, "resume_to")?;

    // The grant delegates exactly the authority needed for this question.
    let mut scopes: HashSet<String> = HashSet::from(["human".to_string()]);
    for tag in &q.expertise {
        scopes.insert(format!("expert:{tag}"));
    }

    let (question, ticket) = state.store.answer_question(
        &grant.question,
        &grant.actor,
        &scopes,
        &answer,
        resume_to.as_deref(),
    )?;
    state.store.mark_answer_grant_used(&grant.grant_id)?;
    state.wake();
    Ok(Json(json!({
        "question": question.to_json(),
        "ticket": ticket.to_json(now_ms()),
    })))
}

/// Reject unknown top-level fields with the codebase's standard teaching error.
fn reject_unknown(obj: &serde_json::Map<String, Value>, known: &[&str]) -> ApiResult<()> {
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !known.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "validation.unknown_field",
        format!(
            "Unknown field(s): {}. Accepted: {}.",
            unknown.join(", "),
            known.join(", ")
        ),
    ))
}
