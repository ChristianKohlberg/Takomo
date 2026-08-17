//! /v1/questions: the "ask a human" board API.
//!
//! `POST /questions` (write) raises a question and parks the ticket;
//! `POST /questions/{id}/answer` (human) records the reply and resumes the
//! ticket; `GET /questions` is the inbox read-model. See `store/questions.rs`.

use super::{
    all, body_object, first, get_bool, get_i64, get_str, get_string_array, parse_i64_param,
    query_pairs, require_str, ApiJson,
};
use crate::auth::{AnswerCtx, AuthCtx};
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::server::AppState;
use crate::store::{
    Answerer, AskRequest, Question, QuestionFilter, Ticket, TimeoutAction,
    DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

const ASK_FIELDS: [&str; 18] = [
    "ticket",
    "mode",
    "kind",
    "title",
    "body",
    "options",
    "option_notes",
    "multi",
    "recommended",
    "recommended_multi",
    "recommended_note",
    "confidence",
    "summary",
    "expertise",
    "assignee",
    "urgency",
    "expires_in_seconds",
    "on_timeout",
];

/// Parse `options` accepting either plain strings or `{value, desc}` objects,
/// returning the value list and a parallel description list. A separate
/// top-level `option_notes` array (if present) overrides/fills the descriptions.
fn parse_options(obj: &serde_json::Map<String, Value>) -> ApiResult<(Vec<String>, Vec<String>)> {
    let raw = match obj.get("options") {
        None | Some(Value::Null) => return Ok((Vec::new(), Vec::new())),
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(ApiError::validation(
                "validation.options",
                "Field 'options' must be an array of strings or of {value, desc} objects.",
            ))
        }
    };
    let mut values = Vec::with_capacity(raw.len());
    let mut notes = Vec::with_capacity(raw.len());
    let mut any_note = false;
    for item in raw {
        match item {
            Value::String(s) => {
                values.push(s.clone());
                notes.push(String::new());
            }
            Value::Object(m) => {
                let v = m
                    .get("value")
                    .or_else(|| m.get("label"))
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        ApiError::validation(
                            "validation.options",
                            "each option object needs a string 'value' (the choice text).",
                        )
                    })?;
                values.push(v.to_string());
                let d = m
                    .get("desc")
                    .or_else(|| m.get("description"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if !d.is_empty() {
                    any_note = true;
                }
                notes.push(d.to_string());
            }
            _ => {
                return Err(ApiError::validation(
                    "validation.options",
                    "each option must be a string or a {value, desc} object.",
                ))
            }
        }
    }
    // An explicit parallel option_notes array overrides object descs.
    if let Some(list) = get_string_array(obj, "option_notes")? {
        return Ok((values, list));
    }
    Ok((values, if any_note { notes } else { Vec::new() }))
}

// ---------------------------------------------------------------------------
// Ask: the parts of raising a question that are policy rather than parsing, so
// REST (`POST /v1/questions`) and MCP (`takomo_ask`) cannot answer the same
// question differently. Field mapping into `AskRequest` deliberately stays at
// each call site: the struct has no `Default`, so a new field breaks both
// surfaces at compile time — the drift that needs a human is the *prose*.

/// Which surface an ask arrived on. It selects only the one sentence that
/// legitimately differs — REST names the HTTP endpoint a human will use, MCP
/// tells the agent what to do with its run — and both wordings live here, side by
/// side, so an edit to one is made where the other is visible.
pub(crate) enum AskSurface {
    Rest,
    Mcp,
}

/// The `note` on an ask response: what just happened to the ticket, plus the
/// project's writing conventions echoed back.
///
/// The nudge is the reason this is one function. It is agent-facing instruction
/// about the project's house style, and it was written out twice — so the two
/// surfaces could, and did, tell agents different things about the same
/// convention. Asking is also the last moment the text is still fixable (re-ask),
/// unlike a question already sitting in someone's inbox, which is why it rides on
/// the response at all rather than only on the work-loop hints
/// ([`crate::api::attach_conventions`]).
pub(crate) fn ask_note(
    state: &AppState,
    question: &Question,
    ticket: &Ticket,
    surface: AskSurface,
) -> String {
    let mut note = match (question.mode.as_str(), surface) {
        ("advisory", AskSurface::Rest) => format!(
            "Advisory question recorded on '{}' — no state change, no lease effect. A human answers via the board or POST /v1/questions/{}/answer; the decision is recorded (the ticket is not resumed).",
            ticket.id, question.id
        ),
        ("advisory", AskSurface::Mcp) => format!(
            "Advisory question recorded on '{}' — it does not change the ticket's state or your lease. It's routed to the inbox for a human; keep working or end your run as you see fit.",
            ticket.id
        ),
        (_, AskSurface::Rest) => format!(
            "Ticket parked in '{}' and your lease released. A human answers via the board or POST /v1/questions/{}/answer; the ticket resumes once every open question on it is answered. Re-check with GET /v1/tickets/{} later.",
            ticket.state, question.id, ticket.id
        ),
        (_, AskSurface::Mcp) => format!(
            "Parked '{}' in '{}' and released your lease. End your run; resume once every open question on it is answered (the answers land on this ticket).",
            ticket.id, ticket.state
        ),
    };
    // A hint is advisory: a project that vanished under us must not turn a
    // successful ask into an error.
    if let Ok(Some(p)) = state.store.get_project(&question.project) {
        if let Some(lang) = p.question_language.filter(|l| !l.trim().is_empty()) {
            note.push_str(&format!(
                " This project expects the question (and any options) written in {lang} — re-ask in {lang} if this one wasn't."
            ));
        }
        if let Some(style) = p.style_guide.filter(|s| !s.trim().is_empty()) {
            note.push_str(&format!(
                " This project's style guide for what you write: {style}"
            ));
        }
    }
    note
}

/// Absolute expiry for an ask's `expires_in_seconds`, or `None` when it names no
/// deadline. A non-positive window means "no deadline", not "already expired" —
/// both surfaces have always agreed on that, and now they agree by construction.
pub(crate) fn ask_expires_at(expires_in_seconds: Option<i64>) -> Option<i64> {
    match expires_in_seconds {
        Some(secs) if secs > 0 => Some(now_ms() + secs * 1000),
        _ => None,
    }
}

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

    // `mine=true` is "waiting on me", which this board now means in two senses:
    // what is addressed to me by name, and what my expertise covers. They are
    // ORed, because a person owes an answer on either — requiring both would
    // silently hide a question aimed straight at them.
    let mine = matches!(first(&pairs, "mine"), Some("true" | "1"));
    let mut assignee: Vec<String> = Vec::new();
    let expertise = if mine {
        let tags = my_expertise(&ctx);
        if let Some(user) = &ctx.user {
            assignee.push(user.clone());
        }
        if tags.is_empty() && assignee.is_empty() {
            // Neither route: nothing can be "mine".
            return Ok(Json(
                json!({ "items": [], "note": "Your token carries no expert:<tag> scopes and is not bound to a person, so nothing is routed to you. Drop ?mine=true to see the whole queue, or have an operator bind this token to a user (takomo token create --user <handle>)." }),
            ));
        }
        tags
    } else {
        // An explicit `?assignee=` reads one person's queue — including someone
        // else's, which is the point of a shared board. `none` is the triage read:
        // what has nobody's name on it.
        if let Some(raw) = first(&pairs, "assignee") {
            if raw != "none" {
                let person = state
                    .store
                    .get_user(raw)?
                    .ok_or_else(|| ApiError::not_found("user", raw))?;
                assignee.push(person.id);
            }
        }
        all(&pairs, "expertise")
    };
    let unassigned = !mine && matches!(first(&pairs, "assignee"), Some("none"));

    // Bound the page: `limit` clamped to the server cap, `cursor` = row offset.
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(crate::store::MAX_QUESTIONS_PAGE)
        .clamp(1, crate::store::MAX_QUESTIONS_PAGE);
    let offset = parse_i64_param(&pairs, "cursor")?.unwrap_or(0).max(0);

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
        assignee,
        unassigned,
        assignee_or_expertise: mine,
        allowed_projects: ctx.allowed_projects_vec(),
        limit: Some(limit),
        offset: Some(offset),
    };
    let (questions, total) = state.store.list_questions(&filter)?;
    // With a real `total` the cursor no longer has to be inferred from a full
    // page. The old heuristic — "len == limit, so there is probably more" —
    // handed back a cursor to an empty page whenever the queue happened to be an
    // exact multiple of the page size.
    let seen = offset + questions.len() as i64;
    let next_cursor = (seen < total).then_some(offset + limit);
    let mut out = super::paged(
        questions.iter().map(|q| q.to_json()).collect::<Vec<_>>(),
        total,
        limit,
        &format!(
            "Read the next page with ?cursor={}&limit={limit} (max page {}), and repeat while \
             next_cursor is set.",
            offset + limit,
            crate::store::MAX_QUESTIONS_PAGE
        ),
    );
    out["next_cursor"] = json!(next_cursor);
    Ok(Json(out))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
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

    let expires_at = ask_expires_at(get_i64(obj, "expires_in_seconds")?);
    let on_timeout = match get_str(obj, "on_timeout")? {
        Some(raw) => Some(TimeoutAction::parse(&raw)?),
        None => None,
    };

    let (options, option_notes) = parse_options(obj)?;
    let req = AskRequest {
        ticket,
        mode: get_str(obj, "mode")?,
        kind: require_str(obj, "kind")?,
        title: require_str(obj, "title")?,
        body: get_str(obj, "body")?.unwrap_or_default(),
        options,
        option_notes,
        multi: get_bool(obj, "multi")?.unwrap_or(false),
        recommended_multi: get_string_array(obj, "recommended_multi")?.unwrap_or_default(),
        recommended: obj
            .get("recommended")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null),
        recommended_note: get_str(obj, "recommended_note")?,
        confidence: get_i64(obj, "confidence")?,
        summary: get_str(obj, "summary")?,
        expertise: get_string_array(obj, "expertise")?.unwrap_or_default(),
        assignee: get_str(obj, "assignee")?,
        urgency: get_str(obj, "urgency")?,
        expires_at,
        on_timeout,
        fence: get_i64(obj, "fence")?,
    };

    let (question, ticket) = state.store.ask_question(&req, &ctx.actor)?;
    state.wake();
    crate::notify::question_asked(&state, &question);
    let note = ask_note(&state, &question, &ticket, AskSurface::Rest);
    let hints = crate::store::question_quality_hints(&question);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "question": question.to_json(),
            "ticket": ticket.to_json(now_ms()),
            "note": note,
            "hints": hints,
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
    // Question detail always carries its follow-up thread (usually empty).
    let thread = state.store.question_thread(&id)?;
    let mut out = q.to_json();
    if let Value::Object(map) = &mut out {
        map.insert(
            "thread".to_string(),
            Value::Array(thread.iter().map(|m| m.to_json()).collect()),
        );
    }
    Ok(Json(out))
}

pub async fn answer(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let obj = body_object(&body)?;
    reject_unknown(obj, &["answer", "resume_to", "on_behalf_of"])?;
    // Two ways in, and the body picks which. Absent `on_behalf_of` is the
    // original path: answering IS the human authorization gate, because it
    // performs the ticket's human-gated resume transition. Present
    // `on_behalf_of` is a relay (takomo-22xj) — recording a decision a human
    // already made elsewhere — which needs its own scope and names the decider.
    //
    // Deliberately not interchangeable: a `human` token answers as itself and
    // may not claim to be relaying, so `answered_by` is never a guess. The
    // refusals that make relaying safe (no relaying your own question, no
    // relaying an `approve`) live in the store, not here.
    let on_behalf_of = get_str(obj, "on_behalf_of")?;
    match &on_behalf_of {
        None => ctx.require_scope("human")?,
        Some(_) => {
            // Redundancy before scope, deliberately: a caller that already holds
            // `human` should be told to answer as itself, not sent to fetch a
            // scope it does not need. The most actionable message wins.
            if ctx.scopes.contains("human") {
                return Err(ApiError::bad_request(
                    "answer.relay_redundant",
                    "This token holds the 'human' scope, so it should answer as itself rather than relay: drop 'on_behalf_of'. Relaying exists for a caller that cannot answer — recording a decision someone else made — and a human relaying would make 'answered_by' a claim instead of a fact.",
                ));
            }
            ctx.require_scope("answer:relay")?;
        }
    }
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;

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

    let outcome = match &on_behalf_of {
        // Who the server can vouch this caller is. For an `approve` addressed to a
        // person, being them is the second of the two authorities that can answer
        // it; for every other kind the person changes nothing.
        None => state.store.answer_question(
            &id,
            Answerer::new(&ctx.actor, &ctx.scopes, ctx.user.as_deref()),
            &answer,
            resume_to.as_deref(),
        )?,
        Some(decider) => {
            let decider = decider.trim();
            if decider.is_empty() {
                return Err(ApiError::validation(
                    "answer.relay_actor",
                    "'on_behalf_of' must name the human who made this decision — it is what 'answered_by' records and what a later reader will hold someone to. An empty value would file the decision under nobody.",
                ));
            }
            state.store.answer_question_relayed(
                &id,
                &ctx.actor,
                // The named human is the answerer of record; the store drops the
                // relaying credential's own person, because a relayed name is a
                // claim rather than proof.
                Answerer::new(decider, &ctx.scopes, ctx.user.as_deref()),
                &answer,
                resume_to.as_deref(),
            )?
        }
    };
    state.wake();
    Ok(Json(json!({
        "question": outcome.question.to_json(),
        "ticket": outcome.ticket.to_json(now_ms()),
        "resume": outcome.resume_json(),
    })))
}

/// POST /v1/questions/{id}/reopen (human scope) — take back an answered question
/// (a conditional undo beyond the 30s client window). Refused with a teaching
/// 409 if the ticket already relies on the answer (claimed / moved on / archived).
pub async fn reopen(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    let (question, ticket) = state.store.reopen_question(
        &id,
        Answerer::new(&ctx.actor, &ctx.scopes, ctx.user.as_deref()),
    )?;
    state.wake();
    Ok(Json(json!({
        "question": question.to_json(),
        "ticket": ticket.to_json(now_ms()),
    })))
}

/// POST /v1/questions/{id}/assign (human scope) — address an open question to a
/// person, or return it to the open pool with `{"assignee": null}`.
///
/// Separate from asking because the agent that raised the question usually cannot
/// know who should decide it. Assignment is routing: any `human` token may still
/// answer the ordinary kinds, so nothing is stranded when the assignee is away. The
/// one thing it does confer is the second route to answering an `approve`.
pub async fn assign(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["assignee"])?;
    // `assignee: null` unassigns; a missing key is a caller who forgot which
    // operation they wanted, and guessing either way would be a surprise.
    let assignee = match obj.get("assignee") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::String(_)) | None => {
            return Err(ApiError::validation(
                "validation.assignee",
                "Field 'assignee' is required: a user handle (or usr- id) to address this question to, or null to return it to the open pool.",
            ))
        }
        Some(_) => {
            return Err(ApiError::validation(
                "validation.assignee",
                "Field 'assignee' must be a string (a user handle) or null.",
            ))
        }
    };
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    let question = state
        .store
        .assign_question(&id, assignee.as_deref(), &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "question": question.to_json() })))
}

pub async fn withdraw(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
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

/// POST /v1/questions/{id}/followup (human scope) — bounce the question back to
/// the asking agent for more research before deciding. Records a message on the
/// thread and flips it to await the agent; the question stays open and a
/// blocking ticket stays parked.
pub async fn followup(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["message"])?;
    let message = require_str(obj, "message")?;
    let question = state.store.request_followup(&id, &ctx.actor, &message)?;
    state.wake();
    Ok(Json(question.to_json()))
}

/// POST /v1/questions/{id}/reply (write scope) — the asking agent replies to a
/// follow-up with the context the human asked for. Flips the thread back to
/// await the human so the inbox shows it is ready to answer again.
pub async fn reply(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["message"])?;
    let message = require_str(obj, "message")?;
    let question = state.store.reply_followup(&id, &ctx.actor, &message)?;
    state.wake();
    Ok(Json(question.to_json()))
}

const REVISE_FIELDS: [&str; 6] = [
    "options",
    "option_notes",
    "recommended",
    "recommended_multi",
    "recommended_note",
    "reason",
];

/// POST /v1/questions/{id}/options (write scope) — revise a still-open `choose`
/// question's options.
///
/// Exists so an agent that learns something while answering a follow-up can fix
/// the choices instead of withdrawing the question and losing the whole thread.
/// `recommended`, `recommended_multi` and `recommended_note` are only touched
/// when present in the body (send `recommended: null` to clear it); a
/// recommendation left pointing at a removed option is a 422, not a silent drop.
pub async fn revise_options(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let q = state
        .store
        .get_question(&id)?
        .ok_or_else(|| ApiError::not_found("question", &id))?;
    ctx.require_project(&q.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &REVISE_FIELDS)?;
    if !obj.contains_key("options") {
        return Err(ApiError::validation(
            "validation.options",
            "Field 'options' is required — send the full revised set (this replaces the options, it does not merge into them).",
        ));
    }
    let (options, option_notes) = parse_options(obj)?;
    let req = crate::store::ReviseOptionsRequest {
        options,
        option_notes,
        recommended: obj.get("recommended").cloned(),
        recommended_multi: match obj.get("recommended_multi") {
            Some(_) => Some(get_string_array(obj, "recommended_multi")?.unwrap_or_default()),
            None => None,
        },
        recommended_note: match obj.get("recommended_note") {
            Some(Value::Null) => Some(None),
            Some(v) => Some(Some(
                v.as_str()
                    .ok_or_else(|| {
                        ApiError::validation(
                            "validation.recommended_note",
                            "recommended_note must be a string, or null to clear it.",
                        )
                    })?
                    .to_string(),
            )),
            None => None,
        },
        reason: obj
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    let question = state.store.revise_question_options(&id, &ctx.actor, &req)?;
    state.wake();
    Ok(Json(question.to_json()))
}

// ---------------------------------------------------------------------------
// Answer links: a per-question, expiring, write-once grant (see auth::AnswerCtx
// and store/answer_grants.rs). Minting/revoking run on the normal token path;
// the `/v1/answer/self*` endpoints run on the distinct answer-grant auth path.

const ANSWER_LINK_FIELDS: [&str; 2] = ["ttl_seconds", "actor"];

/// Shown once, on every surface that mints a link — the same convention the other
/// two credential mints follow (`api/tokens.rs`, `api/shares.rs`).
const ANSWER_LINK_WARNING: &str = "This answer-link token is shown ONCE. Anyone with the link can answer this one question until it expires or is used (single-use). Share it only with the intended person.";

/// Mint a per-question answer link — the whole of it, for every surface.
///
/// An answer link is a **bearer credential handed to someone outside the org**,
/// so what it is allowed to do must not depend on which transport the caller
/// happened to use. That is why the authority checks, the lifetime rules and the
/// response body all live here and nowhere else; REST
/// (`POST /v1/questions/{id}/answer-link`) and MCP (`takomo_answer_link`) do
/// nothing but shape the envelope around what this returns.
///
/// - `human` scope, and the question's project must be in the token's allowlist.
/// - The question must still be `open` — a spent or withdrawn one has nothing to
///   answer.
/// - Minting for an `approve` question requires the matching `expert:<tag>`
///   scope: you can only delegate authority you hold.
/// - Lifetime precedence, most specific first: an explicit `ttl_seconds` on this
///   call, else the project's `answer_link_ttl_seconds`, else
///   [`DEFAULT_ANSWER_TTL_SECONDS`]. The explicit value wins outright — someone
///   who has looked at this one question and chosen a window knows more than the
///   project setting does — and is bounded by [`MAX_ANSWER_TTL_SECONDS`]. The
///   stored project value was bounded by the same rule when it was written
///   (`normalize_answer_link_ttl`), so it needs no re-check here.
///
/// Returns the grant row plus the one-time `token`, the `#a=` board `path`, an
/// absolute `url` when `TAKOMO_PUBLIC_URL` is set, the `ttl_seconds` actually
/// applied and its `ttl_source`, and the [`ANSWER_LINK_WARNING`].
pub(crate) fn mint_answer_link(
    state: &AppState,
    ctx: &AuthCtx,
    question_id: &str,
    ttl_seconds: Option<i64>,
    actor: Option<String>,
) -> ApiResult<Value> {
    ctx.require_scope("human")?;
    let q = state
        .store
        .get_question(question_id)?
        .ok_or_else(|| ApiError::not_found("question", question_id))?;
    ctx.require_project(&q.project)?;
    if q.status != "open" {
        return Err(ApiError::conflict(
            "question.not_open",
            format!(
                "Question '{question_id}' is '{}', not open — there is nothing to answer.",
                q.status
            ),
        ));
    }
    // Can't delegate an approval you couldn't give yourself. Two ways to hold the
    // authority the link would carry, matching the two that can answer:
    //
    // 1. A matching `expert:<tag>` scope — the original rule.
    // 2. Being the assignee, minting your OWN link (to answer from a phone, say).
    //
    // Deliberately NOT allowed: any other `human` token minting a link *for* the
    // assignee — that is the escalation this feature could have shipped, since
    // whoever holds the link string can spend it, so a colleague could mint
    // "Ada's" link and approve as Ada. `admin` is not a route either, because in
    // this codebase admin has never stood in for domain expertise; the admin act
    // that reaches a person with no credential is binding them one
    // (`takomo token create --user <handle>`), which is explicit and auditable
    // where a silently-mintable link would not be.
    if q.kind == "approve" {
        let has_expert = q
            .expertise
            .iter()
            .any(|t| ctx.scopes.contains(&format!("expert:{t}")));
        let is_assignee = match (&q.assignee, &ctx.user) {
            (Some(assignee), Some(caller)) => assignee == caller,
            _ => false,
        };
        if !has_expert && !is_assignee {
            let scopes = q
                .expertise
                .iter()
                .map(|t| format!("expert:{t}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut message = String::from(
                "Minting an answer link for this 'approve' question needs the authority the link would carry — you can only delegate authority you hold. ",
            );
            if q.expertise.is_empty() {
                message.push_str(
                    "It is addressed to a specific person, so only they can mint it, from the credential bound to them.",
                );
            } else {
                message.push_str(&format!(
                    "It needs a matching domain expert scope ({scopes}), or the credential of the person it is addressed to."
                ));
            }
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "question.approve_expertise",
                message,
            ));
        }
    }

    let (ttl, ttl_source) = match ttl_seconds {
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
        Some(s) => (s, "explicit"),
        None => match state.store.answer_link_ttl(&q.project)? {
            Some(s) => (s, "project"),
            None => (DEFAULT_ANSWER_TTL_SECONDS, "default"),
        },
    };
    // The identity this link carries. A question addressed to a person mints a
    // link *for that person*, so their name — not a link-scoped placeholder — is
    // what `answered_by` records, and so the grant can satisfy an approval
    // addressed to them. An unassigned question mints an anonymous link exactly as
    // before, which is still the outside-expert case.
    //
    // Only the assignee, never a caller-supplied name: the authority check above
    // has already established who may mint this, and reading a person off the
    // request would let it name someone else.
    let link_user = q.assignee.clone();
    let assignee_handle = q
        .assignee_person
        .as_ref()
        .map(|person| person.handle.clone());

    // Who the answer is attributed to; defaults to the named person, else a
    // link-scoped actor.
    let actor = actor
        .or(assignee_handle)
        .unwrap_or_else(|| format!("human:link:{question_id}"));

    let expires_at = now_ms() + ttl * 1000;
    let (row, plaintext) = state.store.create_answer_grant(
        question_id,
        &q.project,
        &actor,
        expires_at,
        &ctx.actor,
        link_user.as_deref(),
    )?;

    let mut out = row.to_json();
    if let Value::Object(map) = &mut out {
        map.insert("token".to_string(), Value::String(plaintext.clone()));
        map.insert(
            "path".to_string(),
            Value::String(format!("/board#a={plaintext}")),
        );
        // The lifetime actually applied, and where it came from ("explicit" |
        // "project" | "default"). `expires_at` alone cannot tell an operator
        // whether a link is short because they asked for that or because the
        // project says so — which is the only way to notice a project default
        // nobody meant to set.
        map.insert("ttl_seconds".to_string(), Value::from(ttl));
        map.insert(
            "ttl_source".to_string(),
            Value::String(ttl_source.to_string()),
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
            Value::String(ANSWER_LINK_WARNING.to_string()),
        );
    }
    Ok(out)
}

/// POST /v1/questions/{id}/answer-link (human scope) — mint a scoped, expiring,
/// single-use link an outside expert can use to answer this one question. All the
/// policy is in [`mint_answer_link`]; this only rejects unknown fields and wraps
/// the result in a `201`.
pub async fn create_link(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<impl IntoResponse> {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let obj = body_object(&body)?;
    reject_unknown(obj, &ANSWER_LINK_FIELDS)?;
    let out = mint_answer_link(
        &state,
        &ctx,
        &id,
        get_i64(obj, "ttl_seconds")?,
        get_str(obj, "actor")?,
    )?;
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
    ensure_grant_question_project(&grant, &q)?;
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
/// question only. The grant is spent inside the same transaction that records
/// the answer, so exactly one of N concurrent attempts on one link can win; the
/// losers get a 410 `answer_link.spent` and change nothing.
pub async fn self_answer(
    State(state): State<Arc<AppState>>,
    Extension(grant): Extension<AnswerCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let q = state
        .store
        .get_question(&grant.question)?
        .ok_or_else(|| ApiError::not_found("question", &grant.question))?;
    ensure_grant_question_project(&grant, &q)?;

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

    // ONE transaction spends the link and records the answer, so single-use is
    // the transaction itself rather than a follow-up write that a concurrent
    // holder could slip past (or a crash could lose).
    // A link minted for a named person carries that person's identity, which is how
    // it satisfies an approval addressed to them — an approval with no expertise tag
    // has no scope to synthesize, so identity is the only proof there is. The mint
    // path is what guarantees only they could have created it.
    let outcome = state.store.answer_question_via_grant(
        &grant.question,
        Answerer::new(&grant.actor, &scopes, grant.user.as_deref()),
        &answer,
        resume_to.as_deref(),
        &grant.grant_id,
    )?;
    state.wake();
    Ok(Json(json!({
        "question": outcome.question.to_json(),
        "ticket": outcome.ticket.to_json(now_ms()),
        "resume": outcome.resume_json(),
    })))
}

/// Defence in depth on the outside-the-org path: a grant's stored project must
/// match the question it references. Minting keeps them aligned today, but this
/// is the one handler that does not go through `require_project`.
fn ensure_grant_question_project(grant: &AnswerCtx, question: &Question) -> ApiResult<()> {
    if grant.project == question.project {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        "answer.project",
        format!(
            "This answer link is scoped to project '{}' but the question belongs to '{}'. The link cannot be used.",
            grant.project, question.project
        ),
    ))
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
