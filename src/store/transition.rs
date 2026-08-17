//! State transitions — the only way a ticket's state changes. Every rejection
//! is a teaching error: stable code, LLM-legible message, exact remedy, and
//! the full list of allowed transitions from the current state.
//!
//! "The only way" is close to literal: every module reaches ticket state
//! through `apply_transition` — including ask-a-human, which parks a ticket on a
//! blocking question and resumes it on the answer (`store::questions`). What
//! differs between those callers is captured in [`MoveKind`], not in a second
//! code path, so no caller can quietly skip a workflow's approval gate.
//!
//! There is exactly one other `UPDATE tickets SET state` in the codebase —
//! `store::questions::override_state`, the administrative reversal behind
//! reopening an answered question, for which the workflow has no edge. Its doc
//! comment enumerates every check it therefore skips and what gates it instead.
//! `grep -rn "SET state" src/` should return those two and nothing else; a third
//! is a bug.

use super::claims::claim_ticket_tx;
use super::helpers::{
    clear_expired_claim, emit_event, get_ticket_required, get_workflow, stale_fence_error,
    touch_ticket,
};
use super::model::{Ticket, MAX_COMMENT};
use super::tickets::insert_comment_tx;
use super::Store;
use crate::error::{AllowedTransition, ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::workflow::{Requirement, Workflow, WorkflowTransition, GUARD_HAS_LINK};
use axum::http::StatusCode;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashSet;

/// What kind of state move this is — the **only** axis on which a
/// store-internal move differs from a caller's own transition. Every variant
/// runs the same legality, scope and guard checks against the same workflow;
/// that identity is the point of routing every state change through here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MoveKind {
    /// A caller-initiated transition (`POST /v1/tickets/{id}/transition`) —
    /// and also the resume of a parked ticket when a human answers its last
    /// blocking question, which needs no exemption at all: the answerer is a
    /// real caller with a real token, the fence is only ever demanded of the
    /// lease holder, and a `scope:human` resume edge is authoritative over
    /// someone else's lease by the ordinary rule in step (3) below.
    Normal,
    /// The park half of block-and-resume (`Store::ask_question`). Two narrow,
    /// deliberate differences from `Normal`, and nothing else:
    ///
    /// 1. The lease is always released on entry — parking hands the ticket back
    ///    while a human decides, and the asking agent ends its run. `Normal`
    ///    would keep it, since a blocked-category state is not terminal.
    /// 2. A `claim` requirement on the park edge counts as met. The asker's
    ///    right to write was already established by `check_fence_for_write`
    ///    (holder plus a matching fence, or an unclaimed ticket), and an agent
    ///    whose lease expired mid-task must still be able to hand its decision
    ///    to a human instead of being told to walk back through the ready queue
    ///    (takomo-jb5i). The holder lock, the fence echo, and every `scope:`
    ///    and `guard:` requirement still apply unchanged.
    ParkForQuestion,
}

/// Why a single requirement failed on a candidate edge.
#[derive(Debug, Clone)]
enum ReqFailure {
    NeedsClaim,
    NeedsScope(String),
    GuardFailed {
        guard: String,
        offenders: Vec<String>,
    },
}

fn allowed_from(wf: &Workflow, state: &str) -> Vec<AllowedTransition> {
    wf.transitions_from(state)
        .into_iter()
        .map(|t| AllowedTransition {
            to: t.to.clone(),
            requires: t.requires.clone(),
        })
        .collect()
}

fn eval_guard(conn: &Connection, guard: &str, ticket: &Ticket) -> ApiResult<Option<ReqFailure>> {
    match guard {
        "no_open_children" => {
            let mut stmt = conn.prepare(
                r#"
                SELECT c.id FROM tickets c
                JOIN workflow_states ws ON ws.project = c.project AND ws.state = c.state
                WHERE c.parent = ?1 AND ws.terminal = 0
                ORDER BY c.id
                "#,
            )?;
            let open: Vec<String> = stmt
                .query_map(params![ticket.id], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if open.is_empty() {
                Ok(None)
            } else {
                Ok(Some(ReqFailure::GuardFailed {
                    guard: guard.to_string(),
                    offenders: open,
                }))
            }
        }
        "no_open_blockers" => {
            let mut stmt = conn.prepare(
                r#"
                SELECT d.blocked_by FROM deps d
                JOIN tickets b ON b.id = d.blocked_by
                JOIN workflow_states ws ON ws.project = b.project AND ws.state = b.state
                WHERE d.ticket = ?1 AND ws.terminal = 0
                ORDER BY d.blocked_by
                "#,
            )?;
            let open: Vec<String> = stmt
                .query_map(params![ticket.id], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if open.is_empty() {
                Ok(None)
            } else {
                Ok(Some(ReqFailure::GuardFailed {
                    guard: guard.to_string(),
                    offenders: open,
                }))
            }
        }
        // `has_link:<key>` — the ticket must carry a non-empty links.<key>.
        // `offenders` stays empty on purpose: the subject is this ticket, not a
        // related one, and requirement_error has a dedicated arm that explains
        // which key is missing instead of naming ticket ids.
        g if g.starts_with(GUARD_HAS_LINK) => {
            let key = &g[GUARD_HAS_LINK.len()..];
            let present = ticket
                .links
                .get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty());
            if present {
                Ok(None)
            } else {
                Ok(Some(ReqFailure::GuardFailed {
                    guard: g.to_string(),
                    offenders: Vec::new(),
                }))
            }
        }
        other => Err(ApiError::internal(format!(
            "unknown guard '{other}' in stored workflow"
        ))),
    }
}

impl Store {
    pub fn transition(
        &self,
        id: &str,
        to: &str,
        reason: Option<&str>,
        fence: Option<i64>,
        actor: &str,
        scopes: &HashSet<String>,
    ) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            apply_transition(
                tx,
                id,
                to,
                reason,
                fence,
                actor,
                scopes,
                now,
                MoveKind::Normal,
            )
        })
    }

    /// Claim if the caller does not already hold the lease, then transition —
    /// one transaction so a guard or scope failure cannot strand a claim.
    #[allow(clippy::too_many_arguments)]
    pub fn start_ticket(
        &self,
        id: &str,
        to: &str,
        reason: Option<&str>,
        fence_override: Option<i64>,
        actor: &str,
        scopes: &HashSet<String>,
        ttl_seconds: Option<i64>,
        try_claim: bool,
    ) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            let mut fence = fence_override.or_else(|| {
                t.active_claim(now)
                    .and_then(|(holder, _)| (holder == actor).then_some(t.fence_seq))
            });
            if fence.is_none() && try_claim {
                let (_, lease) = claim_ticket_tx(tx, id, actor, ttl_seconds, now)?;
                fence = Some(lease.fence);
            }
            apply_transition(
                tx,
                id,
                to,
                reason,
                fence,
                actor,
                scopes,
                now,
                MoveKind::Normal,
            )
        })
    }

    /// Optionally record a blocker comment, then transition to a blocked state
    /// — one transaction so a failed advance cannot leave an orphan comment.
    pub fn block_ticket(
        &self,
        id: &str,
        to: &str,
        comment: Option<&str>,
        fence_override: Option<i64>,
        actor: &str,
        scopes: &HashSet<String>,
    ) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            if let Some(body) = comment {
                if body.is_empty() || body.len() > MAX_COMMENT {
                    return Err(ApiError::validation(
                        "validation.comment",
                        format!("comment body must be 1-{MAX_COMMENT} bytes."),
                    ));
                }
                insert_comment_tx(tx, id, actor, body, now)?;
            }
            let t = get_ticket_required(tx, id)?;
            let fence = fence_override.or_else(|| {
                t.active_claim(now)
                    .and_then(|(holder, _)| (holder == actor).then_some(t.fence_seq))
            });
            apply_transition(
                tx,
                id,
                to,
                None,
                fence,
                actor,
                scopes,
                now,
                MoveKind::Normal,
            )
        })
    }
}

/// Move a ticket's state — **the single writer of `tickets.state`**, and the
/// reason every rejection above is a teaching error.
///
/// Runs inside the caller's write transaction so a state change and the
/// `transitioned` event it emits cannot drift apart, and so a caller that has
/// more to record atomically with the move (parking a ticket to ask a human,
/// resuming it when the answer lands) shares one transaction with it instead of
/// hand-rolling the `UPDATE`. `Store::transition` is the thin `with_tx` wrapper
/// for the HTTP route.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_transition(
    tx: &Connection,
    id: &str,
    to: &str,
    reason: Option<&str>,
    fence: Option<i64>,
    actor: &str,
    scopes: &HashSet<String>,
    now: i64,
    kind: MoveKind,
) -> ApiResult<Ticket> {
    let mut t = get_ticket_required(tx, id)?;
    // The chokepoint for the archive gate on ticket state: this function is the
    // single writer of `tickets.state`, so every way a ticket could move — a
    // plain transition, a question parking or resuming it, a bulk move — is
    // refused here while the project is archived, with no per-caller check to
    // forget. The sweepers, which transition on nobody's behalf, filter archived
    // projects out of their queries instead of arriving here and erroring.
    super::helpers::ensure_ticket_writable(tx, &t)?;
    let wf = get_workflow(tx, &t.project)?;
    if clear_expired_claim(tx, &t, now)? {
        // Mirror exactly what the clear wrote, marker included: the holder moved
        // to `lapsed_claim_holder`, and that is what tells the claim_required
        // remedy below whether this caller may resume the lease in place rather
        // than walk the ticket back through the ready queue.
        t.lapsed_claim_holder = t.claim_holder.take();
        t.claim_expires_at = None;
        t.claim_since = None;
    }
    let allowed = allowed_from(&wf, &t.state);

    // Validation is ordered legality -> scope -> claim/fence so the
    // headline error names the FIRST real blocker (pilot finding B):
    // an illegal target or a missing authorization scope must never be
    // masked by a fencing complaint.

    // (1a) Legality — the target must be a real state in this workflow.
    if wf.state(to).is_none() {
        return Err(ApiError::conflict(
            "transition.unknown_state",
            format!(
                "State '{to}' does not exist in project '{}''s workflow '{}'. See allowed_transitions for the legal moves from '{}'.",
                t.project, wf.name, t.state
            ),
        )
        .current_state(t.state.clone())
        .allowed_transitions(allowed));
    }

    // (1b) Legality — a defined (from, to) edge must exist. Multiple
    // edges with different `requires` may exist (e.g. a human gate plus
    // an autoland gate); the transition succeeds if any one edge's
    // requirements all hold.
    let candidates: Vec<&WorkflowTransition> = wf
        .transitions_from(&t.state)
        .into_iter()
        .filter(|e| e.to == to)
        .collect();

    if candidates.is_empty() {
        let remedy = if allowed.is_empty() {
            format!(
                "'{}' is a terminal state; no transitions leave it.",
                t.state
            )
        } else {
            format!(
                "Legal next states from '{}': {}. Pick one of those with POST /v1/tickets/{id}/transition.",
                t.state,
                allowed
                    .iter()
                    .map(|a| a.to.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(ApiError::conflict(
            "transition.illegal",
            format!(
                "Transition '{}' -> '{to}' is not defined in workflow '{}' for project '{}'. State changes only happen along defined transitions.",
                t.state, wf.name, t.project
            ),
        )
        .remedy(remedy)
        .current_state(t.state.clone())
        .allowed_transitions(allowed));
    }

    // Parse every candidate edge's requirements once.
    let parsed: Vec<Vec<Requirement>> = candidates
        .iter()
        .map(|edge| {
            edge.requires
                .iter()
                .map(|raw| {
                    Requirement::parse(raw)
                        .map_err(|e| ApiError::internal(format!("stored workflow corrupt: {e}")))
                })
                .collect::<ApiResult<Vec<_>>>()
        })
        .collect::<ApiResult<Vec<_>>>()?;

    // (2) Scope — the caller must satisfy the scope requirements of at
    // least one candidate edge. A missing scope (e.g. human approval)
    // is an authorization gate, not a fencing mistake, so it is decided
    // before the claim/fence checks below.
    let missing_scopes_per_edge: Vec<Vec<String>> = parsed
        .iter()
        .map(|reqs| {
            reqs.iter()
                .filter_map(|r| match r {
                    Requirement::Scope(s) if !scopes.contains(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        })
        .collect();
    if !missing_scopes_per_edge.iter().any(|m| m.is_empty()) {
        // Best edge: the one demanding the fewest missing scopes.
        let missing = missing_scopes_per_edge
            .into_iter()
            .min_by_key(|m| m.len())
            .unwrap_or_default();
        return Err(scope_error(&t, missing, allowed));
    }

    // Finding A: a human-required transition the caller is authorized
    // for is authoritative over a claim held by another actor — it is
    // allowed despite the holder lock and auto-releases the claim as a
    // side effect. Scoped to `scope:human` edges only; ordinary
    // `claim`-required transitions keep the holder lock unchanged.
    let human_authoritative = parsed
        .iter()
        .zip(&missing_scopes_per_edge)
        .any(|(reqs, missing)| {
            missing.is_empty()
                && reqs
                    .iter()
                    .any(|r| matches!(r, Requirement::Scope(s) if s == "human"))
        });

    // (3) Claim / fence — amended by finding A's human override.
    let active_claim: Option<(String, Option<i64>)> =
        t.active_claim(now).map(|(h, e)| (h.to_string(), e));
    let has_active_claim = active_claim.is_some();
    let caller_holds_claim = match &active_claim {
        Some((holder, expires)) => {
            if human_authoritative {
                // Authoritative human transition: bypass the holder lock
                // and fence echo; the held claim is auto-released below.
                holder == actor
            } else if holder != actor {
                return Err(ApiError::conflict(
                    "claim.held",
                    format!(
                        "Ticket '{id}' is claimed by '{holder}' {}. Only the lease holder may transition a claimed ticket. Ask the holder to release it (POST /v1/tickets/{id}/release), wait for the claim to end, or work something else via POST /v1/ready/claim.",
                        super::helpers::held_phrase(*expires)
                    ),
                )
                .details(json!({ "holder": holder, "expires_at": expires.map(iso) }))
                .current_state(t.state.clone())
                .allowed_transitions(allowed));
            } else {
                match fence {
                    None => {
                        return Err(ApiError::conflict(
                            "fence.required",
                            format!(
                                "Ticket '{id}' is claimed by you; transitions must echo the lease's fencing token. Include \"fence\": {} in the request body.",
                                t.fence_seq
                            ),
                        )
                        .current_state(t.state.clone())
                        .allowed_transitions(allowed));
                    }
                    Some(f) if f != t.fence_seq => {
                        return Err(ApiError::conflict(
                            "fence.stale",
                            format!(
                                "Fencing token {f} is stale (current fence is {}). Your lease was lost; the ticket may have been reclaimed. Stop writing and re-claim via POST /v1/tickets/{id}/claim if appropriate.",
                                t.fence_seq
                            ),
                        )
                        .current_state(t.state.clone())
                        .allowed_transitions(allowed));
                    }
                    Some(_) => {}
                }
                true
            }
        }
        None => {
            // Unclaimed — but an echoed fence must still be current: a
            // zombie writer bounces even after release/expiry cleared
            // the claim it once held. A human override does not echo a
            // fence, so this check is skipped for it.
            if !human_authoritative {
                if let Some(f) = fence {
                    if f != t.fence_seq {
                        return Err(stale_fence_error(id, f, t.fence_seq)
                            .current_state(t.state.clone())
                            .allowed_transitions(allowed));
                    }
                }
            }
            false
        }
    };

    // A park for a blocking question satisfies `claim` by other means —
    // see MoveKind::ParkForQuestion for exactly which, and why.
    let claim_requirement_met = caller_holds_claim || kind == MoveKind::ParkForQuestion;

    // (4) Remaining requirements (claim + guard). Scope was decided
    // above; an edge succeeds when all of its requirements hold.
    let mut edge_failures: Vec<Vec<ReqFailure>> = Vec::new();
    let mut passed = false;
    for reqs in &parsed {
        let mut failures = Vec::new();
        for req in reqs {
            match req {
                Requirement::Claim => {
                    if !claim_requirement_met {
                        failures.push(ReqFailure::NeedsClaim);
                    }
                }
                Requirement::Scope(scope) => {
                    if !scopes.contains(scope) {
                        failures.push(ReqFailure::NeedsScope(scope.clone()));
                    }
                }
                Requirement::Guard(guard) => {
                    if let Some(f) = eval_guard(tx, guard, &t)? {
                        failures.push(f);
                    }
                }
            }
        }
        if failures.is_empty() {
            passed = true;
            break;
        }
        edge_failures.push(failures);
    }

    if !passed {
        return Err(requirement_error(
            id,
            &t,
            to,
            &wf,
            actor,
            now,
            edge_failures,
            allowed,
        ));
    }

    // Apply. A held claim is auto-released when the ticket is parked to
    // ask a human (block-and-resume hands it back), when a human
    // transition supersedes it (finding A), or when entering a
    // done/cancelled-category state; leaving a claimable state
    // otherwise keeps the lease.
    let target = wf.state(to).expect("validated above");
    let (do_release, release_reason) = if has_active_claim {
        if kind == MoveKind::ParkForQuestion {
            (true, "released to ask a human")
        } else if human_authoritative {
            (true, "superseded by human transition")
        } else if matches!(target.category.as_str(), "done" | "cancelled") {
            (true, "auto-release on terminal-category entry")
        } else {
            (false, "")
        }
    } else {
        (false, "")
    };
    if do_release {
        tx.execute(
            "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, claim_since = NULL WHERE id = ?1",
            params![id],
        )?;
    }
    let from = t.state.clone();
    tx.execute(
        "UPDATE tickets SET state = ?2 WHERE id = ?1",
        params![id, to],
    )?;
    touch_ticket(tx, id, now)?;
    emit_event(
        tx,
        Some(id),
        Some(&t.project),
        actor,
        "transitioned",
        json!({
            "from": from,
            "to": to,
            "reason": reason,
            "auto_released": do_release,
        }),
        now,
    )?;
    if do_release {
        emit_event(
            tx,
            Some(id),
            Some(&t.project),
            actor,
            "released",
            json!({ "fence": t.fence_seq, "reason": release_reason }),
            now,
        )?;
    }
    get_ticket_required(tx, id)
}

/// The 403 for a transition whose scope requirements the caller cannot meet.
/// Shared by the up-front scope gate and the fallback requirement resolver so
/// the wording stays identical.
fn scope_error(
    t: &Ticket,
    missing_scopes: Vec<String>,
    allowed: Vec<AllowedTransition>,
) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "transition.scope",
        format!(
            "This transition requires scope(s) your token lacks: {}. This is an authorization gate (for example, human approval), not a workflow mistake. Ask an operator holding that scope to perform the transition, or have such a token minted (takomo token create).",
            missing_scopes.join(", ")
        ),
    )
    .details(json!({ "missing_scopes": missing_scopes }))
    .current_state(t.state.clone())
    .allowed_transitions(allowed)
}

/// The 409 for a claim-gated transition the caller holds no lease for.
///
/// The remedy has to name a call that actually works *from the current state*.
/// Claiming only works in a state the workflow marks `claimable`, and the state
/// a worker is stuck in when its lease expires mid-task is precisely one that is
/// not (`in_progress`, `implementing`): a flat "POST /claim" remedy used to be
/// answered by `claim.state` — "state X is not claimable" — and the two errors
/// pointed at each other with no way out (takomo-jb5i).
///
/// Three cases now, and each names a call the store will honour:
///
/// 1. The state is claimable — the plain claim, then retry echoing the new fence.
/// 2. The state is not, but the caller is the holder whose own lease lapsed here
///    and nobody has claimed since — the claim works anyway, as a resume in place
///    ([`super::claims::may_resume_lapsed_lease`]). This is the common case behind
///    the ticket: work that outlived its lease, no competing worker, and no reason
///    to send it back through the ready queue.
/// 3. Neither — the re-entry route: the edges out of here that land somewhere a
///    lease can be taken, plus the warning that they pass through the ready queue
///    where another worker may take the ticket.
fn claim_required_error(
    id: &str,
    t: &Ticket,
    to: &str,
    wf: &Workflow,
    actor: &str,
    now: i64,
    allowed: Vec<AllowedTransition>,
) -> ApiError {
    let claimable_states: Vec<&str> = wf
        .states
        .iter()
        .filter(|s| s.claimable)
        .map(|s| s.id.as_str())
        .collect();

    if wf.state(&t.state).is_some_and(|s| s.claimable) {
        return ApiError::conflict(
            "transition.claim_required",
            format!(
                "This transition requires an active claim on '{id}', and you do not hold one. State '{}' is claimable, so take the lease first and retry echoing its fence.",
                t.state
            ),
        )
        .remedy(format!(
            "POST /v1/tickets/{id}/claim, then POST /v1/tickets/{id}/transition with {{\"to\":\"{to}\",\"fence\":<the fence from the claim response>}}."
        ))
        .details(json!({ "claimable_states": claimable_states, "resume_in_place": false }))
        .current_state(t.state.clone())
        .allowed_transitions(allowed);
    }

    // Case 2: the caller's own lease lapsed right here and nothing has taken the
    // ticket since, so claiming works despite the state — as a resume in place,
    // with no trip through the ready queue for anyone else to intercept.
    let resumable = wf.state(&t.state).is_some_and(|s| {
        super::claims::may_resume_lapsed_lease(t, actor, now, s.claimable, s.terminal, &s.category)
    });
    if resumable {
        return ApiError::conflict(
            "transition.claim_required",
            format!(
                "This transition requires an active claim on '{id}', and you do not hold one — your lease expired while the work ran (nothing heartbeats it for you). State '{}' is not claimable, but the lapsed lease was yours and nobody has claimed the ticket since, so you can take it back where it stands: claim it again and retry.",
                t.state
            ),
        )
        .remedy(format!(
            "POST /v1/tickets/{id}/claim — it resumes your lapsed lease in place (the response carries a fresh `fence` and `\"resumed\": true`) — then POST /v1/tickets/{id}/transition with {{\"to\":\"{to}\",\"fence\":<the new fence>}}. The ticket does NOT go back to the ready queue, so nothing else can take it from under you. Heartbeat it (POST /v1/tickets/{id}/heartbeat) if the next stretch of work is long."
        ))
        .details(json!({
            "claimable_states": claimable_states,
            "resume_in_place": true,
            "lapsed_holder": actor,
        }))
        .current_state(t.state.clone())
        .allowed_transitions(allowed);
    }

    // Re-entry routes: edges out of here into a claimable state that do not
    // themselves demand a claim — one that did would deadlock the same way.
    let reentry: Vec<&AllowedTransition> = allowed
        .iter()
        .filter(|a| wf.state(&a.to).is_some_and(|s| s.claimable))
        .filter(|a| !a.requires.iter().any(|r| r == "claim"))
        .collect();
    let reentry_states: Vec<&str> = reentry.iter().map(|a| a.to.as_str()).collect();

    // Case 3. Whose lapsed lease (if any) sits here decides how much of this is
    // the caller's business: "someone else's, and they may still resume it" is a
    // different situation from "nobody holds anything".
    let lapsed = t.lapsed_holder(now).filter(|h| *h != actor);
    let whose = match lapsed {
        Some(h) => format!(
            " The lease that lapsed in this state was '{h}''s, and only '{h}' can resume it in place."
        ),
        None => String::new(),
    };
    let stuck = format!(
        "This transition requires an active claim on '{id}', and you do not hold one. State '{}' is not claimable in project '{}', so POST /v1/tickets/{id}/claim would be refused with 'claim.state': a lease can only be taken in {}.{whose}",
        t.state,
        t.project,
        if claimable_states.is_empty() {
            "no state of this workflow".to_string()
        } else {
            claimable_states.join(", ")
        }
    );

    let (message, remedy) = match reentry.first() {
        Some(route) => {
            let gate = if route.requires.is_empty() {
                String::new()
            } else {
                format!(" (that edge requires {})", route.requires.join(", "))
            };
            (
                format!("{stuck} Re-enter a claimable state first, then claim, then walk forward again."),
                format!(
                    "POST /v1/tickets/{id}/transition {{\"to\":\"{0}\"}}{gate}, then POST /v1/tickets/{id}/claim for a fresh lease, then transition forward to '{to}' again echoing the new fence on every claim-gated move. Note that '{0}' puts the ticket back in the ready queue, where another worker may pick it up before you re-claim it.",
                    route.to
                ),
            )
        }
        None => (
            format!("{stuck} No transition out of '{}' leads to a claimable state either, so as this workflow stands no one can take a lease on this ticket.", t.state),
            format!(
                "Ask an operator to move the ticket along a scope-gated edge into a claimable state, or to adjust the workflow so one is reachable (PUT /v1/projects/{}/workflow, admin scope).",
                t.project
            ),
        ),
    };

    ApiError::conflict("transition.claim_required", message)
        .remedy(remedy)
        .details(json!({
            "claimable_states": claimable_states,
            "reentry_states": reentry_states,
            "resume_in_place": false,
            "lapsed_holder": lapsed,
        }))
        .current_state(t.state.clone())
        .allowed_transitions(allowed)
}

/// Turn per-edge requirement failures into the single most actionable error.
/// Preference order: an edge failing only on claim/guard (the caller is
/// authorized, just not set up) beats scope-failing edges; scope-only failures
/// become a 403.
#[allow(clippy::too_many_arguments)]
fn requirement_error(
    id: &str,
    t: &Ticket,
    to: &str,
    wf: &Workflow,
    actor: &str,
    now: i64,
    edge_failures: Vec<Vec<ReqFailure>>,
    allowed: Vec<AllowedTransition>,
) -> ApiError {
    // Best edge: fewest scope failures, then fewest failures overall.
    let best = edge_failures
        .into_iter()
        .min_by_key(|fs| {
            let scope_fails = fs
                .iter()
                .filter(|f| matches!(f, ReqFailure::NeedsScope(_)))
                .count();
            (scope_fails, fs.len())
        })
        .unwrap_or_default();

    // Claim first (most common agent mistake), then scope, then guard.
    if best.iter().any(|f| matches!(f, ReqFailure::NeedsClaim)) {
        return claim_required_error(id, t, to, wf, actor, now, allowed);
    }

    let missing_scopes: Vec<String> = best
        .iter()
        .filter_map(|f| match f {
            ReqFailure::NeedsScope(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    if !missing_scopes.is_empty() {
        return scope_error(t, missing_scopes, allowed);
    }

    if let Some(ReqFailure::GuardFailed { guard, offenders }) = best
        .iter()
        .find(|f| matches!(f, ReqFailure::GuardFailed { .. }))
    {
        let (explain, remedy) = match guard.as_str() {
            "no_open_children" => (
                format!(
                    "guard 'no_open_children' failed: child ticket(s) {} are not in a terminal state",
                    offenders.join(", ")
                ),
                format!(
                    "Finish or cancel the open children ({}) first, then retry this transition.",
                    offenders.join(", ")
                ),
            ),
            "no_open_blockers" => (
                format!(
                    "guard 'no_open_blockers' failed: blocking ticket(s) {} are not in a terminal state",
                    offenders.join(", ")
                ),
                format!(
                    "Finish or cancel the blockers ({}) — or remove the dependency edges with DELETE /v1/tickets/{id}/deps?blocked_by=<id> — then retry.",
                    offenders.join(", ")
                ),
            ),
            g if g.starts_with(GUARD_HAS_LINK) => {
                let key = &g[GUARD_HAS_LINK.len()..];
                let proof = if key == "commit" {
                    " Use the full commit SHA (or its commit URL) of the work that closes this ticket — a short SHA is ambiguous."
                } else {
                    ""
                };
                (
                    format!(
                        "guard '{g}' failed: this transition must prove itself with a '{key}' link on the ticket, and none is set"
                    ),
                    format!(
                        "Attach it first: PATCH /v1/tickets/{id} with {{\"links\":{{\"{key}\":\"<value>\"}}}} (MCP: takomo_link, CLI: takomo link {id} --{key} <value>), then retry.{proof}"
                    ),
                )
            }
            other => (
                format!(
                    "guard '{other}' failed for ticket(s) {}",
                    offenders.join(", ")
                ),
                "Resolve the named tickets, then retry this transition.".to_string(),
            ),
        };
        let mut details = json!({ "guard": guard });
        if !offenders.is_empty() {
            details["offending_tickets"] = json!(offenders);
        }
        return ApiError::conflict(
            "transition.guard",
            format!("Transition blocked on '{id}': {explain}."),
        )
        .remedy(remedy)
        .details(details)
        .current_state(t.state.clone())
        .allowed_transitions(allowed);
    }

    ApiError::conflict(
        "transition.requirements",
        format!("Transition requirements not met for '{id}'."),
    )
    .current_state(t.state.clone())
    .allowed_transitions(allowed)
}
