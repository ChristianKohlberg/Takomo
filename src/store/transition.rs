//! State transitions — the only way a ticket's state changes. Every rejection
//! is a teaching error: stable code, LLM-legible message, exact remedy, and
//! the full list of allowed transitions from the current state.

use super::helpers::{
    clear_expired_claim, emit_event, get_ticket_required, get_workflow, stale_fence_error,
    touch_ticket,
};
use super::model::Ticket;
use super::Store;
use crate::error::{AllowedTransition, ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::workflow::{Requirement, Workflow, WorkflowTransition};
use axum::http::StatusCode;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashSet;

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
        other => Err(ApiError::internal(format!(
            "unknown guard '{other}' in stored workflow"
        ))),
    }
}

impl Store {
    #[allow(clippy::too_many_arguments)]
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
            let mut t = get_ticket_required(tx, id)?;
            let wf = get_workflow(tx, &t.project)?;
            if clear_expired_claim(tx, &t, now)? {
                t.claim_holder = None;
                t.claim_expires_at = None;
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
                    format!("'{}' is a terminal state; no transitions leave it.", t.state)
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
                            Requirement::parse(raw).map_err(|e| {
                                ApiError::internal(format!("stored workflow corrupt: {e}"))
                            })
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
            let active_claim: Option<(String, i64)> =
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
                                "Ticket '{id}' is claimed by '{holder}' until {}. Only the lease holder may transition a claimed ticket. Ask the holder to release it (POST /v1/tickets/{id}/release), wait for the lease to expire, or work something else via POST /v1/ready/claim.",
                                iso(*expires)
                            ),
                        )
                        .details(json!({ "holder": holder, "expires_at": iso(*expires) }))
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

            // (4) Remaining requirements (claim + guard). Scope was decided
            // above; an edge succeeds when all of its requirements hold.
            let mut edge_failures: Vec<Vec<ReqFailure>> = Vec::new();
            let mut passed = false;
            for reqs in &parsed {
                let mut failures = Vec::new();
                for req in reqs {
                    match req {
                        Requirement::Claim => {
                            if !caller_holds_claim {
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
                return Err(requirement_error(id, &t, edge_failures, allowed));
            }

            // Apply. A held claim is auto-released when a human transition
            // supersedes it (finding A) or when entering a done/cancelled-
            // category state; leaving a claimable state otherwise keeps the
            // lease.
            let target = wf.state(to).expect("validated above");
            let (do_release, release_reason) = if has_active_claim {
                if human_authoritative {
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
                    "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL WHERE id = ?1",
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
        })
    }
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

/// Turn per-edge requirement failures into the single most actionable error.
/// Preference order: an edge failing only on claim/guard (the caller is
/// authorized, just not set up) beats scope-failing edges; scope-only failures
/// become a 403.
fn requirement_error(
    id: &str,
    t: &Ticket,
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
    if let Some(ReqFailure::NeedsClaim) = best.iter().find(|f| matches!(f, ReqFailure::NeedsClaim))
    {
        return ApiError::conflict(
            "transition.claim_required",
            format!(
                "This transition requires an active claim on '{id}', and you do not hold one. Claim the ticket first, then retry the transition echoing the lease's fence."
            ),
        )
        .remedy(format!("POST /v1/tickets/{id}/claim"))
        .current_state(t.state.clone())
        .allowed_transitions(allowed);
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
            other => (
                format!(
                    "guard '{other}' failed for ticket(s) {}",
                    offenders.join(", ")
                ),
                "Resolve the named tickets, then retry this transition.".to_string(),
            ),
        };
        return ApiError::conflict(
            "transition.guard",
            format!("Transition blocked on '{id}': {explain}."),
        )
        .remedy(remedy)
        .details(json!({ "guard": guard, "offending_tickets": offenders }))
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
