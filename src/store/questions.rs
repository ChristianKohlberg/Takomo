//! The "ask a human" board.
//!
//! A question is an agent's request for a human decision, tied to a ticket the
//! agent parks in a blocked state before ending its run (the block-and-resume
//! model). A human with the `human` scope answers; answering records the reply
//! and performs the workflow's human-gated transition that resumes the ticket,
//! so a fresh worker can re-claim it. The whole exchange is mirrored onto the
//! append-only event log (question_asked / question_answered / ...); this table
//! is the queryable read-model the inbox view and the expiry sweep run against.
//!
//! Routing is by expertise tag (e.g. `domain:billing`). Tags are advisory: any
//! `human`-scoped token may answer (a question is never stranded because its
//! expert is away), while the inbox and notifications route by tag. A human who
//! also holds the matching free-form `expert:<tag>` scope sees the question in
//! their "my expertise" filter.

use super::helpers::{
    check_fence_for_write, clear_expired_claim, emit_event, get_ticket_required, get_workflow,
    touch_ticket,
};
use super::model::{Question, Ticket, MAX_BODY, MAX_TITLE};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, question_id};
use crate::workflow::{Requirement, Workflow};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::collections::HashSet;

/// The question kinds. Each renders a distinct answer control on the board and
/// carries a distinct answer shape (see `validate_answer`).
pub const QUESTION_KINDS: [&str; 4] = ["confirm", "choose", "clarify", "approve"];

/// Question modes. `blocking` parks the ticket and resumes it on answer (the
/// default). `advisory` routes + records a decision without touching ticket
/// state — for epic-level or strategic questions that shouldn't freeze work.
pub const QUESTION_MODES: [&str; 2] = ["blocking", "advisory"];

const URGENCIES: [&str; 4] = ["critical", "high", "normal", "low"];
/// Minimum window for `on_timeout=recommended`. That timeout auto-applies the
/// agent's recommendation *through the ticket's human-gated resume edge* (as
/// `system`), so a too-short deadline would let a `write`-only agent satisfy a
/// `scope:human` gate with no human. The floor forces a real response window;
/// it is an audited, opt-in SLA fallback, never an instant self-approval.
const MIN_RECOMMENDED_TIMEOUT_SECS: i64 = 300;
const MAX_OPTIONS: usize = 20;
const MAX_OPTION_LEN: usize = 200;
const MAX_EXPERTISE: usize = 10;
const MAX_EXPERTISE_LEN: usize = 100;

const QUESTION_COLS: &str = "id, project, ticket, asked_by, mode, kind, title, body, options, \
    recommended, expertise, urgency, status, answer, answered_by, answered_at, resolved_to, \
    expires_at, on_timeout, created_at, updated_at, version";

/// What the expiry sweep does when a question's `expires_at` passes unanswered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutAction {
    /// Auto-answer with the agent's `recommended` value and resume the ticket.
    Recommended,
    /// Re-route to the open pool: clear the expertise tags and keep it open.
    Escalate,
    /// Cancel the ticket (best-effort) and close the question.
    Cancel,
}

impl TimeoutAction {
    pub fn parse(raw: &str) -> ApiResult<TimeoutAction> {
        match raw {
            "recommended" => Ok(TimeoutAction::Recommended),
            "escalate" => Ok(TimeoutAction::Escalate),
            "cancel" => Ok(TimeoutAction::Cancel),
            other => Err(ApiError::validation(
                "validation.on_timeout",
                format!(
                    "Unknown on_timeout '{other}'. Use one of: recommended, escalate, cancel (or omit to just flag it expired)."
                ),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TimeoutAction::Recommended => "recommended",
            TimeoutAction::Escalate => "escalate",
            TimeoutAction::Cancel => "cancel",
        }
    }
}

/// The parameters for raising a question. Built by the REST/MCP layer.
#[derive(Debug, Clone, Default)]
pub struct AskRequest {
    pub ticket: String,
    /// "blocking" (default) or "advisory".
    pub mode: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub options: Vec<String>,
    pub recommended: Value,
    pub expertise: Vec<String>,
    pub urgency: Option<String>,
    /// Milliseconds since epoch when the question times out, or None.
    pub expires_at: Option<i64>,
    pub on_timeout: Option<TimeoutAction>,
    /// The asking agent's fencing token (echoed when it holds the ticket's lease).
    pub fence: Option<i64>,
}

/// Filter for listing questions (the inbox read-model).
#[derive(Debug, Clone, Default)]
pub struct QuestionFilter {
    pub project: Option<String>,
    pub ticket: Option<String>,
    /// Statuses to include; empty = open only.
    pub statuses: Vec<String>,
    /// Match questions carrying ANY of these expertise tags; empty = no filter.
    pub expertise: Vec<String>,
    /// Token project scoping. None = unrestricted.
    pub allowed_projects: Option<Vec<String>>,
}

fn row_to_question(r: &Row) -> rusqlite::Result<Question> {
    let options_raw: String = r.get("options")?;
    let expertise_raw: String = r.get("expertise")?;
    let recommended_raw: Option<String> = r.get("recommended")?;
    let answer_raw: Option<String> = r.get("answer")?;
    Ok(Question {
        id: r.get("id")?,
        project: r.get("project")?,
        ticket: r.get("ticket")?,
        asked_by: r.get("asked_by")?,
        mode: r.get("mode")?,
        kind: r.get("kind")?,
        title: r.get("title")?,
        body: r.get("body")?,
        options: serde_json::from_str(&options_raw).unwrap_or_default(),
        recommended: recommended_raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
        expertise: serde_json::from_str(&expertise_raw).unwrap_or_default(),
        urgency: r.get("urgency")?,
        status: r.get("status")?,
        answer: answer_raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
        answered_by: r.get("answered_by")?,
        answered_at: r.get("answered_at")?,
        resolved_to: r.get("resolved_to")?,
        expires_at: r.get("expires_at")?,
        on_timeout: r.get("on_timeout")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        version: r.get("version")?,
    })
}

fn get_question_row(conn: &Connection, id: &str) -> ApiResult<Question> {
    let sql = format!("SELECT {QUESTION_COLS} FROM questions WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_question)
        .optional()?
        .ok_or_else(|| ApiError::not_found("question", id))
}

/// Validate + normalize a proposed answer against the question kind. Returns the
/// canonical JSON stored as the answer.
fn validate_answer(kind: &str, options: &[String], answer: &Value) -> ApiResult<Value> {
    // Accept either a bare scalar or a {"value": ...} wrapper for ergonomics.
    let value = match answer {
        Value::Object(m) if m.contains_key("value") => m.get("value").cloned().unwrap(),
        other => other.clone(),
    };
    let note = answer.as_object().and_then(|m| m.get("note")).cloned();

    let normalized = match kind {
        "confirm" | "approve" => {
            let b = coerce_bool(&value).ok_or_else(|| {
                ApiError::validation(
                    "validation.answer",
                    format!(
                        "A '{kind}' question needs a yes/no answer. Send answer: true or false (or \"yes\"/\"no\")."
                    ),
                )
            })?;
            json!(b)
        }
        "choose" => {
            let s = value.as_str().ok_or_else(|| {
                ApiError::validation(
                    "validation.answer",
                    "A 'choose' question needs the selected option as a string.",
                )
            })?;
            if !options.iter().any(|o| o == s) {
                return Err(ApiError::validation(
                    "validation.answer",
                    format!(
                        "'{s}' is not one of the offered options: {}. Answer with one of them exactly.",
                        options.join(", ")
                    ),
                ));
            }
            json!(s)
        }
        "clarify" => {
            let s = value.as_str().ok_or_else(|| {
                ApiError::validation(
                    "validation.answer",
                    "A 'clarify' question needs a free-text answer as a string.",
                )
            })?;
            if s.trim().is_empty() {
                return Err(ApiError::validation(
                    "validation.answer",
                    "The clarification answer is empty; provide the explanation the agent asked for.",
                ));
            }
            json!(s)
        }
        other => {
            return Err(ApiError::internal(format!(
                "question '{other}' has an unknown kind stored"
            )))
        }
    };

    match note {
        Some(n) if !n.is_null() => Ok(json!({ "value": normalized, "note": n })),
        _ => Ok(json!({ "value": normalized })),
    }
}

fn coerce_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "yes" | "y" | "true" | "approve" | "approved" | "ok" => Some(true),
            "no" | "n" | "false" | "reject" | "rejected" | "deny" | "denied" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A short human-legible rendering of an answer, for the resume comment.
fn answer_summary(kind: &str, answer: &Value) -> String {
    let value = answer.get("value").unwrap_or(&Value::Null);
    let note = answer.get("note").and_then(|n| n.as_str());
    let head = match (kind, value) {
        ("confirm", Value::Bool(b)) | ("approve", Value::Bool(b)) => {
            if *b {
                "yes / approved".to_string()
            } else {
                "no / rejected".to_string()
            }
        }
        (_, Value::String(s)) => s.clone(),
        (_, other) => other.to_string(),
    };
    match note {
        Some(n) => format!("{head} — {n}"),
        None => head,
    }
}

/// First transition target from `from` whose category is `category` and whose
/// requirements are satisfiable without a scope the caller lacks (guards and
/// claim aside). Used to find a blocked state to park into.
fn park_target(wf: &Workflow, from: &str) -> Option<String> {
    wf.transitions_from(from).into_iter().find_map(|t| {
        if wf.state(&t.to).map(|s| s.category.as_str()) != Some("blocked") {
            return None;
        }
        // Only self-service park edges: no scope/guard gate (a claim the agent
        // already holds is fine).
        let only_claim = t
            .requires
            .iter()
            .all(|r| matches!(Requirement::parse(r), Ok(Requirement::Claim)));
        if only_claim {
            Some(t.to.clone())
        } else {
            None
        }
    })
}

/// Choose the state to resume a parked ticket into once a human answers.
/// Preference: a human-gated, claimable `todo` state (the ready queue re-entry),
/// then any human-gated claimable state, then any human-gated target. Returns
/// the chosen state and whether the caller's scopes satisfy that edge.
fn resume_target(
    wf: &Workflow,
    from: &str,
    requested: Option<&str>,
    scopes: &HashSet<String>,
) -> ApiResult<String> {
    let edges = wf.transitions_from(from);
    let human_gated: Vec<&crate::workflow::WorkflowTransition> = edges
        .iter()
        .copied()
        .filter(|t| {
            t.requires
                .iter()
                .any(|r| matches!(Requirement::parse(r), Ok(Requirement::Scope(s)) if s == "human"))
        })
        .collect();

    if let Some(req) = requested {
        // An explicit target must be a legal edge whose scope requirements the
        // caller meets, and must carry no guard (guards belong on the normal
        // transition endpoint).
        let edge = edges.iter().find(|t| t.to == req).ok_or_else(|| {
            ApiError::validation(
                "validation.resume_to",
                format!(
                    "'{req}' is not a legal next state from '{from}'. Legal targets: {}.",
                    edges
                        .iter()
                        .map(|t| t.to.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })?;
        ensure_edge_answerable(edge, scopes)?;
        return Ok(req.to_string());
    }

    // Only consider edges the caller can actually take (scope satisfied, no
    // claim/guard requirement) — so a guarded edge never shadows a clean one.
    let answerable: Vec<&crate::workflow::WorkflowTransition> = human_gated
        .iter()
        .copied()
        .filter(|t| ensure_edge_answerable(t, scopes).is_ok())
        .collect();
    let pick = answerable
        .iter()
        .find(|t| {
            wf.state(&t.to)
                .map(|s| s.claimable && s.category == "todo")
                .unwrap_or(false)
        })
        .or_else(|| {
            answerable
                .iter()
                .find(|t| wf.state(&t.to).map(|s| s.claimable).unwrap_or(false))
        })
        .or_else(|| answerable.first())
        .ok_or_else(|| {
            ApiError::conflict(
                "question.no_resume",
                format!(
                    "Ticket state '{from}' has no human-gated transition this token can take to resume the ticket. Resume it manually via POST /v1/tickets/{{id}}/transition, or pass an explicit resume_to."
                ),
            )
        })?;
    Ok(pick.to.clone())
}

/// A resume edge must have all its scope requirements met by the caller and no
/// guard requirement (guards route through the normal transition endpoint).
fn ensure_edge_answerable(
    edge: &crate::workflow::WorkflowTransition,
    scopes: &HashSet<String>,
) -> ApiResult<()> {
    for raw in &edge.requires {
        match Requirement::parse(raw) {
            Ok(Requirement::Scope(s)) if !scopes.contains(&s) => {
                return Err(ApiError::new(
                    axum::http::StatusCode::FORBIDDEN,
                    "question.answer_scope",
                    format!(
                        "Resuming into '{}' needs the '{s}' scope, which your token lacks. Have an operator with that scope answer, or pass resume_to a state your token can reach.",
                        edge.to
                    ),
                ));
            }
            // No lease is held while answering, so a claim-gated resume edge is
            // not takeable here — refuse rather than move the ticket unclaimed
            // (which the real transition() path would never allow).
            Ok(Requirement::Claim) => {
                return Err(ApiError::conflict(
                    "question.resume_claim",
                    format!(
                        "Resuming into '{}' requires an active claim, which no one holds while a question is open. Pick a resume_to that is not claim-gated, or resume the ticket by claiming it and transitioning normally.",
                        edge.to
                    ),
                ));
            }
            Ok(Requirement::Guard(g)) => {
                return Err(ApiError::conflict(
                    "question.resume_guard",
                    format!(
                        "Resuming into '{}' is guarded by '{g}'. Satisfy the guard, then transition the ticket explicitly (POST /v1/tickets/{{id}}/transition); the recorded answer stays on the ticket.",
                        edge.to
                    ),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Move a parked ticket to `to`, clearing any claim (auto-release) and emitting
/// the transition + release events. Must run in a write tx; `to` is assumed
/// already validated as a legal, answerable edge from the ticket's state.
fn apply_resume(
    conn: &Connection,
    t: &Ticket,
    to: &str,
    actor: &str,
    reason: &str,
    now: i64,
) -> ApiResult<()> {
    let had_claim = t.active_claim(now).is_some();
    if had_claim {
        conn.execute(
            "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL WHERE id = ?1",
            params![t.id],
        )?;
    }
    let from = t.state.clone();
    conn.execute(
        "UPDATE tickets SET state = ?2 WHERE id = ?1",
        params![t.id, to],
    )?;
    touch_ticket(conn, &t.id, now)?;
    emit_event(
        conn,
        Some(&t.id),
        Some(&t.project),
        actor,
        "transitioned",
        json!({ "from": from, "to": to, "reason": reason, "auto_released": had_claim }),
        now,
    )?;
    if had_claim {
        emit_event(
            conn,
            Some(&t.id),
            Some(&t.project),
            actor,
            "released",
            json!({ "fence": t.fence_seq, "reason": reason }),
            now,
        )?;
    }
    Ok(())
}

impl Store {
    /// Raise a question: validate, park the ticket in a blocked state, release
    /// the asking agent's lease (block-and-resume), and record the question.
    /// All in one transaction. Returns (question, fresh ticket).
    pub fn ask_question(&self, req: &AskRequest, actor: &str) -> ApiResult<(Question, Ticket)> {
        if !QUESTION_KINDS.contains(&req.kind.as_str()) {
            return Err(ApiError::validation(
                "validation.kind",
                format!(
                    "Unknown question kind '{}'. Use one of: {}.",
                    req.kind,
                    QUESTION_KINDS.join(", ")
                ),
            ));
        }
        if req.title.trim().is_empty() || req.title.len() > MAX_TITLE {
            return Err(ApiError::validation(
                "validation.title",
                format!("question title must be 1-{MAX_TITLE} characters."),
            ));
        }
        if req.body.len() > MAX_BODY {
            return Err(ApiError::validation(
                "validation.body",
                format!("question body must be at most {MAX_BODY} bytes."),
            ));
        }
        if req.kind == "choose" && req.options.len() < 2 {
            return Err(ApiError::validation(
                "validation.options",
                "A 'choose' question needs at least 2 options.",
            ));
        }
        if req.options.len() > MAX_OPTIONS {
            return Err(ApiError::validation(
                "validation.options",
                format!("at most {MAX_OPTIONS} options are allowed."),
            ));
        }
        for o in &req.options {
            if o.is_empty() || o.len() > MAX_OPTION_LEN {
                return Err(ApiError::validation(
                    "validation.options",
                    format!("each option must be 1-{MAX_OPTION_LEN} characters."),
                ));
            }
        }
        if req.expertise.len() > MAX_EXPERTISE {
            return Err(ApiError::validation(
                "validation.expertise",
                format!("at most {MAX_EXPERTISE} expertise tags are allowed."),
            ));
        }
        for tag in &req.expertise {
            if tag.is_empty() || tag.len() > MAX_EXPERTISE_LEN {
                return Err(ApiError::validation(
                    "validation.expertise",
                    format!("each expertise tag must be 1-{MAX_EXPERTISE_LEN} characters."),
                ));
            }
        }
        let urgency = req.urgency.clone().unwrap_or_else(|| "normal".to_string());
        if !URGENCIES.contains(&urgency.as_str()) {
            return Err(ApiError::validation(
                "validation.urgency",
                format!(
                    "Unknown urgency '{urgency}'. Use one of: {}.",
                    URGENCIES.join(", ")
                ),
            ));
        }
        let mode = req.mode.clone().unwrap_or_else(|| "blocking".to_string());
        if !QUESTION_MODES.contains(&mode.as_str()) {
            return Err(ApiError::validation(
                "validation.mode",
                format!(
                    "Unknown mode '{mode}'. Use 'blocking' (parks + resumes the ticket) or 'advisory' (routed + recorded, no state change — e.g. an epic-level decision)."
                ),
            ));
        }
        // Advisory questions never touch ticket state, so a cancel-on-timeout is
        // nonsensical (there's nothing to cancel on their behalf).
        if mode == "advisory" && req.on_timeout == Some(TimeoutAction::Cancel) {
            return Err(ApiError::validation(
                "validation.on_timeout",
                "on_timeout=cancel is only for blocking questions; an advisory question doesn't gate the ticket. Use escalate/recommended, or omit it.",
            ));
        }
        // `approve` is the strong gate (vs `confirm`, which any human may answer):
        // it must name the domain(s) that must sign off, and it can never be
        // auto-resolved on timeout — an approval requires a real expert.
        if req.kind == "approve" {
            if req.expertise.is_empty() {
                return Err(ApiError::validation(
                    "validation.expertise",
                    "An 'approve' question must name at least one expertise tag: approval is gated to a domain expert holding the matching expert:<tag> scope. Use 'confirm' for a yes/no any human can answer.",
                ));
            }
            if req.on_timeout == Some(TimeoutAction::Recommended) {
                return Err(ApiError::validation(
                    "validation.on_timeout",
                    "on_timeout=recommended cannot auto-resolve an 'approve' question — approvals require a domain expert, not a timeout. Use escalate or cancel, or omit on_timeout.",
                ));
            }
        }
        // `on_timeout=recommended` on a BLOCKING question traverses the
        // human-gated resume edge as the system on expiry; require a real
        // response window so it can never be a near-instant self-approval by the
        // asking (non-human) agent. (Advisory questions change no state, so
        // there is no gate to protect and no minimum.)
        if mode == "blocking" && req.on_timeout == Some(TimeoutAction::Recommended) {
            let window_ok = req
                .expires_at
                .map(|exp| exp - now_ms() >= MIN_RECOMMENDED_TIMEOUT_SECS * 1000)
                .unwrap_or(false);
            if !window_ok {
                return Err(ApiError::validation(
                    "validation.on_timeout",
                    format!(
                        "on_timeout=recommended auto-resolves the human decision on expiry, so it requires expires_in_seconds of at least {MIN_RECOMMENDED_TIMEOUT_SECS} (a real window for a human to respond). Use a longer deadline, or on_timeout=escalate/cancel for shorter ones."
                    ),
                ));
            }
        }

        let now = now_ms();
        self.with_tx(|tx| {
            let mut t = get_ticket_required(tx, &req.ticket)?;
            let wf = get_workflow(tx, &t.project)?;
            if clear_expired_claim(tx, &t, now)? {
                t.claim_holder = None;
                t.claim_expires_at = None;
            }
            // A blocking asker must be able to write to the ticket (hold the
            // lease and echo the fence, or find it unclaimed). Advisory questions
            // change no ticket state — like a comment — so they don't fence-check
            // and may be posted on a ticket someone else has claimed.
            if mode == "blocking" {
                check_fence_for_write(&t, actor, req.fence, now, "ask a question")?;
            }

            // A ticket may carry several open questions at once (e.g. two
            // decisions for two different domain experts). It resumes only when
            // all of its BLOCKING questions are answered — the barrier is
            // enforced in `answer_question`. Retries must not pile up
            // duplicates, so an identical open question (same asker + mode + kind
            // + title) is treated as an idempotent replay and returned as-is.
            let dup: Option<String> = tx
                .query_row(
                    "SELECT id FROM questions WHERE ticket = ?1 AND status = 'open' AND asked_by = ?2 AND mode = ?3 AND kind = ?4 AND title = ?5 LIMIT 1",
                    params![t.id, actor, mode, req.kind, req.title],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(qid) = dup {
                let question = get_question_row(tx, &qid)?;
                let fresh = get_ticket_required(tx, &t.id)?;
                return Ok((question, fresh));
            }

            // Only a blocking question parks the ticket / releases the lease.
            if mode == "blocking" {
                let already_blocked =
                    wf.state(&t.state).map(|s| s.category.as_str()) == Some("blocked");
                if !already_blocked {
                    let target = park_target(&wf, &t.state).ok_or_else(|| {
                        ApiError::conflict(
                            "question.no_park",
                            format!(
                                "Ticket state '{}' has no self-service transition into a blocked state, so it cannot be parked for a blocking question. Move it to a blocked state first, ask from an in-progress state, or use mode=advisory (which doesn't park the ticket).",
                                t.state
                            ),
                        )
                        .current_state(t.state.clone())
                    })?;
                    apply_resume(tx, &t, &target, actor, "parked to ask a human", now)?;
                } else if t.active_claim(now).is_some() {
                    // Already blocked but still leased: release so it can be re-picked.
                    tx.execute(
                        "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                        params![t.id, now],
                    )?;
                    emit_event(
                        tx,
                        Some(&t.id),
                        Some(&t.project),
                        actor,
                        "released",
                        json!({ "fence": t.fence_seq, "reason": "released to ask a human" }),
                        now,
                    )?;
                }
            }

            let id = question_id();
            tx.execute(
                "INSERT INTO questions (id, project, ticket, asked_by, mode, kind, title, body, options, recommended, expertise, urgency, status, expires_at, on_timeout, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'open', ?13, ?14, ?15, ?15)",
                params![
                    id,
                    t.project,
                    t.id,
                    actor,
                    mode,
                    req.kind,
                    req.title,
                    req.body,
                    serde_json::to_string(&req.options).unwrap(),
                    if req.recommended.is_null() { None } else { Some(req.recommended.to_string()) },
                    serde_json::to_string(&req.expertise).unwrap(),
                    urgency,
                    req.expires_at,
                    req.on_timeout.map(|a| a.as_str()),
                    now,
                ],
            )?;
            emit_event(
                tx,
                Some(&t.id),
                Some(&t.project),
                actor,
                "question_asked",
                json!({
                    "question": id,
                    "mode": mode,
                    "kind": req.kind,
                    "title": req.title,
                    "expertise": req.expertise,
                    "urgency": urgency,
                }),
                now,
            )?;
            let question = get_question_row(tx, &id)?;
            let fresh = get_ticket_required(tx, &t.id)?;
            Ok((question, fresh))
        })
    }

    /// Record a human's answer and resume the parked ticket via its human-gated
    /// transition. `resume_to` overrides the default resume state. One tx.
    pub fn answer_question(
        &self,
        id: &str,
        actor: &str,
        scopes: &HashSet<String>,
        answer: &Value,
        resume_to: Option<&str>,
    ) -> ApiResult<(Question, Ticket)> {
        let now = now_ms();
        self.with_tx(|tx| {
            let q = get_question_row(tx, id)?;
            if q.status != "open" {
                return Err(ApiError::conflict(
                    "question.not_open",
                    format!(
                        "Question '{id}' is '{}', not open, so it cannot be answered again.",
                        q.status
                    ),
                ));
            }
            // `approve` has teeth: only a matching domain expert may answer it
            // (the `human` scope alone is enough for every other kind).
            if q.kind == "approve" {
                let has_expert = q
                    .expertise
                    .iter()
                    .any(|t| scopes.contains(&format!("expert:{t}")));
                if !has_expert {
                    return Err(ApiError::new(
                        axum::http::StatusCode::FORBIDDEN,
                        "question.approve_expertise",
                        format!(
                            "Approving this needs a domain expert: your token must hold one of {} (an expert:<tag> scope). A general human answer is not sufficient for an 'approve' question — an operator can mint a token with the scope, or answer from one that has it.",
                            q.expertise
                                .iter()
                                .map(|t| format!("expert:{t}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
            let normalized = validate_answer(&q.kind, &q.options, answer)?;

            let mut t = get_ticket_required(tx, &q.ticket)?;
            let wf = get_workflow(tx, &t.project)?;
            if clear_expired_claim(tx, &t, now)? {
                t.claim_holder = None;
                t.claim_expires_at = None;
            }

            // Barrier: a ticket resumes only when every open BLOCKING question on
            // it is answered. Advisory questions never gate — answering one only
            // records the decision. So resume iff this is a blocking question,
            // the ticket is still parked, and no other blocking question remains.
            let others_blocking_open: i64 = tx.query_row(
                "SELECT COUNT(*) FROM questions WHERE ticket = ?1 AND status = 'open' AND mode = 'blocking' AND id != ?2",
                params![q.ticket, id],
                |r| r.get(0),
            )?;
            let is_blocked = wf.state(&t.state).map(|s| s.category.as_str()) == Some("blocked");
            let should_resume = q.mode == "blocking" && is_blocked && others_blocking_open == 0;

            // Resume rules (only on the barrier-clearing answer):
            //   - explicit resume_to: strict — a bad target aborts before
            //     anything is recorded, so the caller can retry with a valid one.
            //   - auto (no resume_to): best-effort — if there is no clean
            //     human-gated edge (e.g. an unusual workflow), still RECORD the
            //     answer and leave the ticket parked, rather than discarding it.
            let resolved_to = if !should_resume {
                None
            } else if resume_to.is_some() {
                let target = resume_target(&wf, &t.state, resume_to, scopes)?;
                apply_resume(tx, &t, &target, actor, &format!("resolved by human ({id})"), now)?;
                Some(target)
            } else {
                match resume_target(&wf, &t.state, None, scopes) {
                    Ok(target) => {
                        apply_resume(tx, &t, &target, actor, &format!("resolved by human ({id})"), now)?;
                        Some(target)
                    }
                    Err(_) => None,
                }
            };

            // Record the answer on the question.
            tx.execute(
                "UPDATE questions SET status = 'answered', answer = ?2, answered_by = ?3, answered_at = ?4, resolved_to = ?5, version = version + 1, updated_at = ?4 WHERE id = ?1",
                params![id, normalized.to_string(), actor, now, resolved_to],
            )?;

            // Mirror the answer into the ticket thread so the resuming agent
            // sees it on `takomo_show`, and emit the question_answered event.
            let summary = answer_summary(&q.kind, &normalized);
            let comment = crate::ids::comment_id();
            let comment_body = format!("Human answered \"{}\": {summary}", q.title);
            tx.execute(
                "INSERT INTO comments (id, ticket, author, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![comment, t.id, actor, comment_body, now],
            )?;
            emit_event(
                tx,
                Some(&t.id),
                Some(&t.project),
                actor,
                "question_answered",
                json!({
                    "question": id,
                    "answer": normalized,
                    "resolved_to": resolved_to,
                    "comment": comment,
                }),
                now,
            )?;

            let question = get_question_row(tx, id)?;
            let fresh = get_ticket_required(tx, &t.id)?;
            Ok((question, fresh))
        })
    }

    /// Withdraw an open question (the agent no longer needs the answer). The
    /// ticket stays parked; the agent resumes it via a normal transition.
    pub fn withdraw_question(
        &self,
        id: &str,
        actor: &str,
        reason: Option<&str>,
    ) -> ApiResult<Question> {
        let now = now_ms();
        self.with_tx(|tx| {
            let q = get_question_row(tx, id)?;
            if q.status != "open" {
                return Err(ApiError::conflict(
                    "question.not_open",
                    format!("Question '{id}' is '{}', not open; nothing to withdraw.", q.status),
                ));
            }
            tx.execute(
                "UPDATE questions SET status = 'withdrawn', version = version + 1, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            emit_event(
                tx,
                Some(&q.ticket),
                Some(&q.project),
                actor,
                "question_withdrawn",
                json!({ "question": id, "reason": reason }),
                now,
            )?;
            get_question_row(tx, id)
        })
    }

    pub fn get_question(&self, id: &str) -> ApiResult<Option<Question>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {QUESTION_COLS} FROM questions WHERE id = ?1");
            conn.query_row(&sql, params![id], row_to_question)
                .optional()
                .map_err(ApiError::from)
        })
    }

    /// All open questions on a ticket, for enriching ticket detail. A ticket can
    /// carry several (the barrier: it resumes only when all are answered), so a
    /// resuming agent needs to see every one. Ordered like the inbox — urgency,
    /// then oldest — so the REST and Node `takomo_show` surfaces agree.
    pub fn open_questions_for_ticket(&self, ticket: &str) -> ApiResult<Vec<Question>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {QUESTION_COLS} FROM questions WHERE ticket = ?1 AND status = 'open' \
                 ORDER BY CASE urgency WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END, created_at ASC, id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![ticket], row_to_question)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// List questions (the inbox). Ordered by urgency then age (oldest first).
    pub fn list_questions(&self, filter: &QuestionFilter) -> ApiResult<Vec<Question>> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {QUESTION_COLS} FROM questions WHERE 1=1");
            let mut p: Vec<SqlValue> = Vec::new();
            let statuses = if filter.statuses.is_empty() {
                vec!["open".to_string()]
            } else {
                filter.statuses.clone()
            };
            sql.push_str(" AND status IN (");
            for (i, s) in statuses.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                sql.push('?');
                p.push(SqlValue::Text(s.clone()));
            }
            sql.push(')');
            if let Some(pr) = &filter.project {
                sql.push_str(" AND project = ?");
                p.push(SqlValue::Text(pr.clone()));
            }
            if let Some(t) = &filter.ticket {
                sql.push_str(" AND ticket = ?");
                p.push(SqlValue::Text(t.clone()));
            }
            if let Some(allowed) = &filter.allowed_projects {
                sql.push_str(" AND project IN (");
                for (i, pr) in allowed.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    p.push(SqlValue::Text(pr.clone()));
                }
                sql.push(')');
            }
            // Expertise: match any tag via a JSON-array overlap check.
            for tag in &filter.expertise {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM json_each(questions.expertise) WHERE json_each.value = ?)",
                );
                p.push(SqlValue::Text(tag.clone()));
            }
            sql.push_str(
                " ORDER BY CASE urgency WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END, created_at ASC, id ASC",
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(p), row_to_question)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Sweep open questions whose deadline has passed, applying `on_timeout`.
    /// Returns how many were acted on. Mirrors the lease sweeper.
    ///
    /// Each due question is handled in its OWN transaction: one poison question
    /// (e.g. a ticket whose project has a corrupt stored workflow) is logged and
    /// skipped, never aborting the batch or wedging every other expiry.
    pub fn sweep_expired_questions(&self) -> ApiResult<usize> {
        let now = now_ms();
        let due: Vec<String> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM questions WHERE status = 'open' AND expires_at IS NOT NULL AND expires_at <= ?1 ORDER BY id",
            )?;
            let rows = stmt
                .query_map(params![now], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;

        let mut acted = 0;
        for id in &due {
            match self.expire_one_question(id, now) {
                Ok(true) => acted += 1,
                Ok(false) => {} // no longer due (answered/withdrawn between passes)
                Err(e) => eprintln!("question expiry skipped '{id}': {}", e.body.message),
            }
        }
        Ok(acted)
    }

    /// Apply the `on_timeout` action to one question in its own transaction.
    /// Re-checks the question is still open + still due (it may have been
    /// answered between the read and this tx). Returns whether it acted.
    fn expire_one_question(&self, id: &str, now: i64) -> ApiResult<bool> {
        self.with_tx(|tx| {
            let q = get_question_row(tx, id)?;
            let still_due = q.status == "open" && q.expires_at.map(|e| e <= now).unwrap_or(false);
            if !still_due {
                return Ok(false);
            }
            let action = q
                .on_timeout
                .as_deref()
                .and_then(|a| TimeoutAction::parse(a).ok());
            match action {
                Some(TimeoutAction::Escalate) => {
                    // Widen to the open pool: clear expertise + deadline, keep open.
                    tx.execute(
                        "UPDATE questions SET expertise = '[]', expires_at = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                        params![q.id, now],
                    )?;
                    emit_event(
                        tx,
                        Some(&q.ticket),
                        Some(&q.project),
                        "system",
                        "question_escalated",
                        json!({ "question": q.id, "reason": "timeout" }),
                        now,
                    )?;
                }
                Some(TimeoutAction::Recommended) if !q.recommended.is_null() => {
                    expire_with_recommendation(tx, &q, now)?;
                }
                Some(TimeoutAction::Cancel) => {
                    expire_and_cancel(tx, &q, now)?;
                }
                _ => {
                    // Default / recommended-without-recommendation: just flag it.
                    tx.execute(
                        "UPDATE questions SET status = 'expired', version = version + 1, updated_at = ?2 WHERE id = ?1",
                        params![q.id, now],
                    )?;
                    emit_event(
                        tx,
                        Some(&q.ticket),
                        Some(&q.project),
                        "system",
                        "question_expired",
                        json!({ "question": q.id }),
                        now,
                    )?;
                }
            }
            Ok(true)
        })
    }
}

/// Timeout: apply the agent's recommendation as the answer and resume the ticket
/// (as actor `system`). Best-effort resume: if the ticket is no longer parked or
/// has no clean human-gated resume edge, the answer is still recorded.
fn expire_with_recommendation(conn: &Connection, q: &Question, now: i64) -> ApiResult<()> {
    let normalized = match validate_answer(&q.kind, &q.options, &q.recommended) {
        Ok(v) => v,
        Err(_) => {
            // Recommendation is not a valid answer; fall back to flagging.
            conn.execute(
                "UPDATE questions SET status = 'expired', version = version + 1, updated_at = ?2 WHERE id = ?1",
                params![q.id, now],
            )?;
            return Ok(());
        }
    };
    let mut resolved_to: Option<String> = None;
    if let Ok(mut t) = get_ticket_required(conn, &q.ticket) {
        clear_expired_claim(conn, &t, now)?;
        if t.active_claim(now).is_none() {
            t.claim_holder = None;
        }
        let wf = get_workflow(conn, &t.project)?;
        if wf.state(&t.state).map(|s| s.category.as_str()) == Some("blocked") {
            // System applies the recommendation; "human" scope is implied.
            let sys_scopes: HashSet<String> = ["human".to_string()].into_iter().collect();
            if let Ok(target) = resume_target(&wf, &t.state, None, &sys_scopes) {
                apply_resume(
                    conn,
                    &t,
                    &target,
                    "system",
                    &format!("timeout: applied recommendation ({})", q.id),
                    now,
                )?;
                resolved_to = Some(target);
            }
        }
    }
    conn.execute(
        "UPDATE questions SET status = 'answered', answer = ?2, answered_by = 'system', answered_at = ?3, resolved_to = ?4, version = version + 1, updated_at = ?3 WHERE id = ?1",
        params![q.id, normalized.to_string(), now, resolved_to],
    )?;
    emit_event(
        conn,
        Some(&q.ticket),
        Some(&q.project),
        "system",
        "question_answered",
        json!({ "question": q.id, "answer": normalized, "resolved_to": resolved_to, "reason": "timeout-recommended" }),
        now,
    )?;
    Ok(())
}

/// Timeout: close the question expired and best-effort cancel the ticket via a
/// no-scope transition to a cancelled-category state.
fn expire_and_cancel(conn: &Connection, q: &Question, now: i64) -> ApiResult<()> {
    conn.execute(
        "UPDATE questions SET status = 'expired', version = version + 1, updated_at = ?2 WHERE id = ?1",
        params![q.id, now],
    )?;
    emit_event(
        conn,
        Some(&q.ticket),
        Some(&q.project),
        "system",
        "question_expired",
        json!({ "question": q.id, "on_timeout": "cancel" }),
        now,
    )?;
    if let Ok(t) = get_ticket_required(conn, &q.ticket) {
        let wf = get_workflow(conn, &t.project)?;
        let cancel_edge = wf.transitions_from(&t.state).into_iter().find(|e| {
            wf.state(&e.to).map(|s| s.category.as_str()) == Some("cancelled")
                && e.requires
                    .iter()
                    .all(|r| matches!(Requirement::parse(r), Ok(Requirement::Claim)))
        });
        if let Some(edge) = cancel_edge {
            let to = edge.to.clone();
            apply_resume(
                conn,
                &t,
                &to,
                "system",
                &format!("timeout: cancelled ({})", q.id),
                now,
            )?;
        }
    }
    Ok(())
}
