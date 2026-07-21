//! Shared SQL helpers used by multiple store submodules.

use super::model::Ticket;
use crate::error::{ApiError, ApiResult};
use crate::ids::iso;
use crate::workflow::Workflow;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

/// Column list every ticket SELECT uses, with `t` as the tickets alias.
pub const TICKET_COLS: &str = "t.id, t.project, t.type, t.parent, t.title, t.body, t.state, \
    COALESCE((SELECT ws.category FROM workflow_states ws WHERE ws.project = t.project AND ws.state = t.state), '') AS state_category, \
    t.priority, t.labels, t.metadata, t.links, t.claim_holder, t.claim_expires_at, \
    t.fence_seq, t.version, t.created_by, t.created_at, t.updated_at, t.archived_at";

pub fn row_to_ticket(row: &Row) -> rusqlite::Result<Ticket> {
    let labels_raw: String = row.get("labels")?;
    let metadata_raw: String = row.get("metadata")?;
    let links_raw: String = row.get("links")?;
    Ok(Ticket {
        id: row.get("id")?,
        project: row.get("project")?,
        ty: row.get("type")?,
        parent: row.get("parent")?,
        title: row.get("title")?,
        body: row.get("body")?,
        state: row.get("state")?,
        state_category: row.get("state_category")?,
        priority: row.get("priority")?,
        labels: serde_json::from_str(&labels_raw).unwrap_or_default(),
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        links: serde_json::from_str(&links_raw).unwrap_or(Value::Null),
        blocked_by: Vec::new(), // filled by load_blocked_by
        claim_holder: row.get("claim_holder")?,
        claim_expires_at: row.get("claim_expires_at")?,
        fence_seq: row.get("fence_seq")?,
        version: row.get("version")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
    })
}

pub fn load_blocked_by(conn: &Connection, ticket: &mut Ticket) -> ApiResult<()> {
    let mut stmt =
        conn.prepare("SELECT blocked_by FROM deps WHERE ticket = ?1 ORDER BY blocked_by")?;
    let ids = stmt
        .query_map(params![ticket.id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ticket.blocked_by = ids;
    Ok(())
}

/// Load one ticket (with blocked_by) or None.
pub fn get_ticket_opt(conn: &Connection, id: &str) -> ApiResult<Option<Ticket>> {
    let sql = format!("SELECT {TICKET_COLS} FROM tickets t WHERE t.id = ?1");
    let ticket = conn
        .query_row(&sql, params![id], row_to_ticket)
        .optional()?;
    match ticket {
        Some(mut t) => {
            load_blocked_by(conn, &mut t)?;
            Ok(Some(t))
        }
        None => Ok(None),
    }
}

/// Load one ticket or a teaching 404.
pub fn get_ticket_required(conn: &Connection, id: &str) -> ApiResult<Ticket> {
    get_ticket_opt(conn, id)?.ok_or_else(|| ApiError::not_found("ticket", id))
}

/// Load a project's workflow or a teaching 404.
pub fn get_workflow(conn: &Connection, project: &str) -> ApiResult<Workflow> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT workflow_json FROM projects WHERE id = ?1",
            params![project],
            |r| r.get(0),
        )
        .optional()?;
    let raw = raw.ok_or_else(|| ApiError::not_found("project", project))?;
    serde_json::from_str(&raw)
        .map_err(|e| ApiError::internal(format!("stored workflow for '{project}' is corrupt: {e}")))
}

/// Append an event inside the caller's transaction. Returns the new seq.
pub fn emit_event(
    conn: &Connection,
    ticket: Option<&str>,
    project: Option<&str>,
    actor: &str,
    kind: &str,
    payload: Value,
    now: i64,
) -> ApiResult<i64> {
    conn.execute(
        "INSERT INTO events (ticket, project, actor, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![ticket, project, actor, kind, payload.to_string(), now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Lazily expire a stale claim on `ticket`: clear it and emit `lease_expired`.
/// Returns true if an expired claim was cleared. Must run inside a write tx.
///
/// Note: when the surrounding operation ultimately fails, the transaction —
/// including this clear and its event — rolls back; the periodic sweep then
/// owns the expiry. Readers never see the stale claim as active either way
/// because `active_claim(now)` and the ready query treat it as unclaimed.
pub fn clear_expired_claim(conn: &Connection, ticket: &Ticket, now: i64) -> ApiResult<bool> {
    if let (Some(holder), Some(exp)) = (&ticket.claim_holder, ticket.claim_expires_at) {
        if exp <= now {
            conn.execute(
                "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                params![ticket.id, now],
            )?;
            emit_event(
                conn,
                Some(&ticket.id),
                Some(&ticket.project),
                "system",
                "lease_expired",
                serde_json::json!({
                    "holder": holder,
                    "fence": ticket.fence_seq,
                    "expired_at": iso(exp),
                }),
                now,
            )?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Ids of tickets that block `id` (directly or via an ancestor) and are not
/// terminal. Empty = unblocked.
pub fn open_blockers(conn: &Connection, id: &str) -> ApiResult<Vec<String>> {
    let mut stmt = conn.prepare(
        r#"
        WITH RECURSIVE lineage(node) AS (
            SELECT ?1
            UNION
            SELECT t.parent FROM tickets t JOIN lineage l ON t.id = l.node
            WHERE t.parent IS NOT NULL
        )
        SELECT DISTINCT d.blocked_by
        FROM lineage l
        JOIN deps d ON d.ticket = l.node
        JOIN tickets b ON b.id = d.blocked_by
        JOIN workflow_states ws ON ws.project = b.project AND ws.state = b.state
        WHERE ws.terminal = 0
        ORDER BY d.blocked_by
        "#,
    )?;
    let ids = stmt
        .query_map(params![id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// Bump version + updated_at (call inside a write tx after field changes).
pub fn touch_ticket(conn: &Connection, id: &str, now: i64) -> ApiResult<i64> {
    conn.execute(
        "UPDATE tickets SET version = version + 1, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    let v: i64 = conn.query_row(
        "SELECT version FROM tickets WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(v)
}

/// Teaching 409 for a fencing token that does not match the current fence.
///
/// Two distinct failure modes are separated so the caller can tell a lost lease
/// from a client bug:
/// - presented < current -> `fence.stale`: a fence the store really issued, but
///   an intervening (re)claim has since superseded your lease.
/// - presented > current -> `fence.invalid`: a fence the store never issued
///   (the counter is monotonic, so nothing above the current value can exist).
///   This is a client bug — a fabricated or corrupted fence — not a lost lease.
pub fn fence_mismatch_error(id: &str, presented: i64, current: i64) -> ApiError {
    if presented > current {
        return ApiError::conflict(
            "fence.invalid",
            format!(
                "Fencing token {presented} for ticket '{id}' was never issued: the highest fence this ticket has ever reached is {current}, and fences only ever increase. This is a client bug — you are echoing a fence the store did not give you (fabricated, incremented by hand, or from a different ticket). Do NOT retry; re-read the lease's fence from your claim response (POST /v1/tickets/{id}/claim) and echo that exact value."
            ),
        )
        .details(serde_json::json!({ "presented_fence": presented, "current_fence": current }));
    }
    stale_fence_error(id, presented, current)
}

/// Teaching 409 for a stale (superseded) fencing token: presented < current.
pub fn stale_fence_error(id: &str, presented: i64, current: i64) -> ApiError {
    ApiError::conflict(
        "fence.stale",
        format!(
            "Fencing token {presented} for ticket '{id}' is stale (current fence is {current}, so yours was superseded by a later claim). Your lease expired and the ticket may have been reclaimed by another worker. STOP writing to this ticket immediately; any work in flight may duplicate someone else's. Re-claim with POST /v1/tickets/{id}/claim only if the ticket is ready again."
        ),
    )
    .details(serde_json::json!({ "presented_fence": presented, "current_fence": current }))
}

/// Teaching 409 for a lease that has already expired (fence unchanged).
pub fn lease_expired_error(id: &str) -> ApiError {
    ApiError::conflict(
        "fence.stale",
        format!(
            "Your lease on ticket '{id}' has expired; the ticket returned to the ready queue and may be reclaimed by another worker. STOP writing. Re-claim with POST /v1/tickets/{id}/claim if the work is still yours, and have your harness heartbeat before the TTL next time."
        ),
    )
}

/// Enforce the fencing rules shared by every mutating call:
/// - actively claimed by someone else -> 409 claim.held
/// - actively claimed by the caller  -> fence required and must match
/// - unclaimed but a fence was sent  -> must match the last issued fence
///   (a zombie echoing an old fence bounces even after release/expiry)
pub fn check_fence_for_write(
    ticket: &Ticket,
    actor: &str,
    fence: Option<i64>,
    now: i64,
    what: &str,
) -> ApiResult<()> {
    match ticket.active_claim(now) {
        Some((holder, expires)) if holder != actor => Err(ApiError::conflict(
            "claim.held",
            format!(
                "Ticket '{}' is claimed by '{holder}' until {}. Only the lease holder may {what} while the ticket is claimed. Add a comment instead (POST /v1/tickets/{}/comments), or wait for the lease to expire.",
                ticket.id,
                iso(expires),
                ticket.id
            ),
        )
        .details(serde_json::json!({ "holder": holder, "expires_at": iso(expires) }))),
        Some(_) => match fence {
            None => Err(ApiError::conflict(
                "fence.required",
                format!(
                    "Ticket '{}' is claimed by you; mutating calls must echo the lease's fencing token. Include \"fence\": {} in the request.",
                    ticket.id, ticket.fence_seq
                ),
            )),
            Some(f) if f != ticket.fence_seq => {
                Err(fence_mismatch_error(&ticket.id, f, ticket.fence_seq))
            }
            Some(_) => Ok(()),
        },
        None => match fence {
            Some(f) if f != ticket.fence_seq => {
                Err(fence_mismatch_error(&ticket.id, f, ticket.fence_seq))
            }
            _ => Ok(()),
        },
    }
}

/// Replace the workflow_states denormalization for a project.
pub fn sync_workflow_states(conn: &Connection, project: &str, wf: &Workflow) -> ApiResult<()> {
    conn.execute(
        "DELETE FROM workflow_states WHERE project = ?1",
        params![project],
    )?;
    for s in &wf.states {
        conn.execute(
            "INSERT INTO workflow_states (project, state, category, claimable, terminal) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project, s.id, s.category, s.claimable as i64, s.terminal as i64],
        )?;
    }
    Ok(())
}
