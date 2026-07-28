//! Claims, leases, fencing, and the ready queue.
//!
//! Correctness model: every claim is a single IMMEDIATE transaction behind the
//! store mutex — SQLite's single-writer serialization *is* the exactly-one-
//! claimant guarantee. Fencing tokens are a per-ticket monotonic counter
//! (`fence_seq`) bumped on every new claim; a zombie writer holding an old
//! fence is rejected with a teaching 409.

use super::helpers::{
    clear_expired_claim, emit_event, fence_mismatch_error, get_ticket_required,
    lease_expired_error, load_blocked_by, open_blockers, row_to_ticket, TICKET_COLS,
};
use super::model::{Lease, Ticket};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

pub const DEFAULT_TTL_SECONDS: i64 = 900;
pub const MAX_TTL_SECONDS: i64 = 3600;

#[derive(Debug, Clone, Default)]
pub struct ReadyFilter {
    pub project: Option<String>,
    pub ty: Option<String>,
    /// AND semantics.
    pub labels: Vec<String>,
    /// Token project scoping. None = unrestricted.
    pub allowed_projects: Option<Vec<String>>,
}

/// Read a project's lease policy inside an open transaction: `(default, max)`,
/// each falling back to the built-in when the project sets nothing.
///
/// A direct two-column read rather than `get_project`, for the same reason
/// `Store::answer_link_ttl` avoids it — this runs on the claim path, and
/// deserializing the whole stored workflow document to learn two integers would
/// put that cost on every claim and every heartbeat. A missing row falls back to
/// the built-ins: the caller has already resolved and authorized the ticket, so
/// absence here can only mean the project was deleted underneath us.
fn project_claim_ttls(tx: &Connection, project: &str) -> ApiResult<(i64, i64)> {
    let row = tx
        .query_row(
            "SELECT claim_ttl_seconds, max_claim_ttl_seconds FROM projects WHERE id = ?1",
            params![project],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    let (d, m) = row.unwrap_or((None, None));
    Ok((
        d.unwrap_or(DEFAULT_TTL_SECONDS),
        m.unwrap_or(MAX_TTL_SECONDS),
    ))
}

/// Resolve the lease length for a claim or heartbeat on a ticket in `project`:
/// an explicit `ttl_seconds` if given, else the project's default, bounded by the
/// project's maximum. Both bounds are per-project settings (takomo-2ztv) that fall
/// back to [`DEFAULT_TTL_SECONDS`] / [`MAX_TTL_SECONDS`].
///
/// Over the maximum is a 422, not a silent clamp — a caller that asked for four
/// hours and received one would otherwise heartbeat on the wrong schedule and
/// lose the lease it thought it had.
pub fn clamp_ttl_for(tx: &Connection, project: &str, ttl_seconds: Option<i64>) -> ApiResult<i64> {
    let (default_ttl, max_ttl) = project_claim_ttls(tx, project)?;
    let ttl = ttl_seconds.unwrap_or(default_ttl);
    if !(1..=max_ttl).contains(&ttl) {
        return Err(ApiError::validation(
            "validation.ttl",
            format!(
                "ttl_seconds must be between 1 and {max_ttl} (this project's default is \
                 {default_ttl}). The bounds are per-project settings — an admin can change them \
                 with PUT /v1/projects/{project}/claim-ttl."
            ),
        )
        .details(json!({
            "ttl_seconds": ttl,
            "project": project,
            "default_seconds": default_ttl,
            "max_seconds": max_ttl,
        })));
    }
    Ok(ttl)
}

/// The ready-queue SELECT. Ready = claimable state, unclaimed (or lease
/// expired), and unblocked — where blocked propagates from ancestors: a ticket
/// is blocked if it, or any ancestor, has a blocked_by edge to a non-terminal
/// ticket. Ordered by priority then age.
fn ready_query(
    conn: &Connection,
    filter: &ReadyFilter,
    now: i64,
    limit: i64,
) -> ApiResult<Vec<Ticket>> {
    let mut sql = format!(
        r#"
        WITH RECURSIVE blocked(id) AS (
            SELECT DISTINCT d.ticket
            FROM deps d
            JOIN tickets b ON b.id = d.blocked_by
            JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
            WHERE bs.terminal = 0
            UNION
            SELECT c.id FROM tickets c JOIN blocked ON c.parent = blocked.id
        )
        SELECT {TICKET_COLS} FROM tickets t
        JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        WHERE ws.claimable = 1
          AND t.archived_at IS NULL
          AND (t.claim_holder IS NULL OR t.claim_expires_at <= ?)
          AND t.id NOT IN (SELECT id FROM blocked)
        "#
    );
    let mut params_vec: Vec<SqlValue> = vec![SqlValue::Integer(now)];
    if let Some(p) = &filter.project {
        sql.push_str(" AND t.project = ?");
        params_vec.push(SqlValue::Text(p.clone()));
    }
    if let Some(allowed) = &filter.allowed_projects {
        sql.push_str(" AND t.project IN (");
        for (i, p) in allowed.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
            params_vec.push(SqlValue::Text(p.clone()));
        }
        sql.push(')');
    }
    if let Some(ty) = &filter.ty {
        sql.push_str(" AND t.type = ?");
        params_vec.push(SqlValue::Text(ty.clone()));
    }
    for label in &filter.labels {
        sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = ?)");
        params_vec.push(SqlValue::Text(label.clone()));
    }
    sql.push_str(
        " ORDER BY CASE t.priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END, t.created_at ASC, t.rowid ASC LIMIT ?",
    );
    params_vec.push(SqlValue::Integer(limit));

    let mut stmt = conn.prepare(&sql)?;
    let mut tickets = stmt
        .query_map(rusqlite::params_from_iter(params_vec), row_to_ticket)?
        .collect::<Result<Vec<_>, _>>()?;
    for t in &mut tickets {
        load_blocked_by(conn, t)?;
    }
    Ok(tickets)
}

/// Grant a lease inside a write tx: bump fence, set holder + expiry, emit.
fn grant_claim(
    conn: &Connection,
    ticket: &Ticket,
    actor: &str,
    ttl_seconds: i64,
    now: i64,
) -> ApiResult<Lease> {
    // If an expired claim is still recorded, clear it first (emits lease_expired).
    clear_expired_claim(conn, ticket, now)?;
    let expires = now + ttl_seconds * 1000;
    conn.execute(
        "UPDATE tickets SET fence_seq = fence_seq + 1, claim_holder = ?2, claim_expires_at = ?3, version = version + 1, updated_at = ?4 WHERE id = ?1",
        params![ticket.id, actor, expires, now],
    )?;
    let fence: i64 = conn.query_row(
        "SELECT fence_seq FROM tickets WHERE id = ?1",
        params![ticket.id],
        |r| r.get(0),
    )?;
    emit_event(
        conn,
        Some(&ticket.id),
        Some(&ticket.project),
        actor,
        "claimed",
        json!({ "fence": fence, "ttl_seconds": ttl_seconds }),
        now,
    )?;
    Ok(Lease {
        ticket: ticket.id.clone(),
        holder: actor.to_string(),
        fence,
        expires_at: expires,
    })
}

impl Store {
    /// Claim a specific ticket. Idempotent renewal when the caller already
    /// holds it.
    pub fn claim_ticket(
        &self,
        id: &str,
        actor: &str,
        ttl_seconds: Option<i64>,
    ) -> ApiResult<(Ticket, Lease)> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            // Resolved inside the transaction, because the bounds are the
            // *project's* now and only the ticket names its project.
            let ttl = clamp_ttl_for(tx, &t.project, ttl_seconds)?;

            if let Some((holder, expires)) = t.active_claim(now) {
                if holder == actor {
                    // Idempotent renewal: keep the fence, extend the lease.
                    let new_expires = now + ttl * 1000;
                    tx.execute(
                        "UPDATE tickets SET claim_expires_at = ?2 WHERE id = ?1",
                        params![id, new_expires],
                    )?;
                    // Lease renewal is silent bookkeeping: emitting a heartbeat
                    // event per renewal floods the append-only log at fleet
                    // scale (ts-8zks). claimed/released/lease_expired still tell
                    // a supervisor everything it needs about lease ownership.
                    let lease = Lease {
                        ticket: id.to_string(),
                        holder: actor.to_string(),
                        fence: t.fence_seq,
                        expires_at: new_expires,
                    };
                    let fresh = get_ticket_required(tx, id)?;
                    return Ok((fresh, lease));
                }
                return Err(ApiError::conflict(
                    "claim.held",
                    format!(
                        "Ticket '{id}' is already claimed by '{holder}' until {}. Pick different work (POST /v1/ready/claim) or retry after the lease expires.",
                        iso(expires)
                    ),
                )
                .details(json!({ "holder": holder, "expires_at": iso(expires) })));
            }

            // State must be claimable per the project workflow.
            let claimable: bool = tx
                .query_row(
                    "SELECT claimable FROM workflow_states WHERE project = ?1 AND state = ?2",
                    params![t.project, t.state],
                    |r| r.get::<_, i64>(0).map(|v| v != 0),
                )
                .unwrap_or(false);
            if !claimable {
                let claimable_states: Vec<String> = {
                    let mut stmt = tx.prepare(
                        "SELECT state FROM workflow_states WHERE project = ?1 AND claimable = 1 ORDER BY state",
                    )?;
                    let states = stmt
                        .query_map(params![t.project], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    states
                };
                return Err(ApiError::conflict(
                    "claim.state",
                    format!(
                        "Ticket '{id}' is in state '{}', which is not claimable. Claimable states in project '{}': {}. Move it with POST /v1/tickets/{id}/transition first, or pick ready work via POST /v1/ready/claim.",
                        t.state,
                        t.project,
                        claimable_states.join(", ")
                    ),
                )
                .current_state(t.state.clone()));
            }

            // Must be unblocked (directly or via ancestors).
            let blockers = open_blockers(tx, id)?;
            if !blockers.is_empty() {
                return Err(ApiError::conflict(
                    "claim.blocked",
                    format!(
                        "Ticket '{id}' is blocked by open ticket(s): {}. Finish or cancel the blockers first; blocked tickets never enter the ready queue.",
                        blockers.join(", ")
                    ),
                )
                .details(json!({ "open_blockers": blockers })));
            }

            let lease = grant_claim(tx, &t, actor, ttl, now)?;
            let fresh = get_ticket_required(tx, id)?;
            Ok((fresh, lease))
        })
    }

    /// Renew a lease. The fence must match the active claim.
    pub fn heartbeat(
        &self,
        id: &str,
        fence: i64,
        actor: &str,
        ttl_seconds: Option<i64>,
    ) -> ApiResult<Lease> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            let ttl = clamp_ttl_for(tx, &t.project, ttl_seconds)?;
            // An expired lease cannot be heartbeated back to life.
            if clear_expired_claim(tx, &t, now)? {
                return Err(lease_expired_error(id));
            }
            match t.active_claim(now) {
                None => Err(ApiError::conflict(
                    "fence.stale",
                    format!(
                        "Ticket '{id}' has no active lease; yours expired or was released. Stop writing. Re-claim with POST /v1/tickets/{id}/claim if the work is still yours."
                    ),
                )),
                Some((holder, _)) if holder != actor || fence != t.fence_seq => {
                    Err(fence_mismatch_error(id, fence, t.fence_seq))
                }
                Some((_, _)) => {
                    let expires = now + ttl * 1000;
                    tx.execute(
                        "UPDATE tickets SET claim_expires_at = ?2 WHERE id = ?1",
                        params![id, expires],
                    )?;
                    // Heartbeats renew the lease silently — no event per beat
                    // (ts-8zks). Lease lifecycle stays observable via
                    // claimed/released/lease_expired.
                    Ok(Lease {
                        ticket: id.to_string(),
                        holder: actor.to_string(),
                        fence,
                        expires_at: expires,
                    })
                }
            }
        })
    }

    /// Voluntary release. The fence must match the active claim.
    pub fn release(
        &self,
        id: &str,
        fence: i64,
        actor: &str,
        reason: Option<&str>,
    ) -> ApiResult<()> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            if clear_expired_claim(tx, &t, now)? {
                return Err(lease_expired_error(id));
            }
            match t.active_claim(now) {
                None => Err(ApiError::conflict(
                    "claim.none",
                    format!("Ticket '{id}' is not claimed; nothing to release."),
                )),
                Some((holder, _)) if holder != actor || fence != t.fence_seq => {
                    Err(fence_mismatch_error(id, fence, t.fence_seq))
                }
                Some((_, _)) => {
                    tx.execute(
                        "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                        params![id, now],
                    )?;
                    emit_event(
                        tx,
                        Some(id),
                        Some(&t.project),
                        actor,
                        "released",
                        json!({ "fence": fence, "reason": reason }),
                        now,
                    )?;
                    Ok(())
                }
            }
        })
    }

    /// Peek the ready queue (no side effects).
    pub fn ready_peek(&self, filter: &ReadyFilter, limit: i64) -> ApiResult<Vec<Ticket>> {
        self.with_conn(|conn| ready_query(conn, filter, now_ms(), limit))
    }

    /// Atomically pop-and-lease the next ready ticket. None = nothing ready.
    pub fn ready_claim(
        &self,
        filter: &ReadyFilter,
        actor: &str,
        ttl_seconds: Option<i64>,
    ) -> ApiResult<Option<(Ticket, Lease)>> {
        let now = now_ms();
        self.with_tx(|tx| {
            let candidates = ready_query(tx, filter, now, 1)?;
            let Some(t) = candidates.into_iter().next() else {
                return Ok(None);
            };
            // After the pick, not before: the queue can span projects when the
            // filter names none, so the lease bounds are only knowable once we
            // know which ticket we got.
            let ttl = clamp_ttl_for(tx, &t.project, ttl_seconds)?;
            let lease = grant_claim(tx, &t, actor, ttl, now)?;
            let fresh = get_ticket_required(tx, &t.id)?;
            Ok(Some((fresh, lease)))
        })
    }

    /// Clear all expired leases (periodic sweep). Returns how many were
    /// cleared; each emits a `lease_expired` event.
    pub fn sweep_expired(&self) -> ApiResult<usize> {
        let now = now_ms();
        self.with_tx(|tx| {
            let sql = format!(
                "SELECT {TICKET_COLS} FROM tickets t WHERE t.claim_holder IS NOT NULL AND t.claim_expires_at <= ?1"
            );
            let expired = {
                let mut stmt = tx.prepare(&sql)?;
                let rows = stmt
                    .query_map(params![now], row_to_ticket)?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            };
            let mut cleared = 0;
            for t in &expired {
                if clear_expired_claim(tx, t, now)? {
                    cleared += 1;
                }
            }
            Ok(cleared)
        })
    }
}
