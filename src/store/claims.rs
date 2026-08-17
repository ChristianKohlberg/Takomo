//! Claims, leases, fencing, and the ready queue.
//!
//! Correctness model: every claim is a single IMMEDIATE transaction behind the
//! store mutex — SQLite's single-writer serialization *is* the exactly-one-
//! claimant guarantee. Fencing tokens are a per-ticket monotonic counter
//! (`fence_seq`) bumped on every new claim; a zombie writer holding an old
//! fence is rejected with a teaching 409.

use super::helpers::{
    clear_expired_claim, emit_event, ensure_ticket_writable, fence_mismatch_error,
    get_ticket_required, held_phrase, lease_expired_error, load_blocked_by, open_blockers,
    row_to_ticket, TICKET_COLS,
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

/// What an admin force-release displaced, so the response and the audit event
/// can both name it. `previous_fence` is what the ousted holder is still
/// carrying; `fence` is the bumped value that makes its next write bounce.
#[derive(Debug, Clone)]
pub struct ForcedRelease {
    pub ticket: String,
    pub project: String,
    pub previous_holder: String,
    pub previous_fence: i64,
    pub fence: i64,
    /// True when the lease had already lapsed by the time the force landed —
    /// the claim row was still there, so the force still fenced the holder off.
    pub lease_expired: bool,
    pub reason: Option<String>,
}

impl ForcedRelease {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "ticket": self.ticket,
            "project": self.project,
            "previous_holder": self.previous_holder,
            "previous_fence": self.previous_fence,
            "fence": self.fence,
            "lease_expired": self.lease_expired,
            "reason": self.reason,
        })
    }
}

/// What has moved inside a ticket's subtree since its claim was granted — the
/// liveness signal an indefinite epic claim is judged by, since it has no
/// expiry to read. `created`/`closed` are since-the-claim counts; `in_progress`
/// and `blocked` are current snapshots, because that is how the question is
/// asked ("2 in progress right now", not "2 entered in_progress since").
#[derive(Debug, Clone)]
pub struct ClaimMovement {
    pub since: i64,
    /// Subtree tickets created at or after the claim.
    pub created: i64,
    /// Distinct subtree tickets that entered a terminal state since the claim
    /// (event-based, so a later reopen does not un-count the close).
    pub closed: i64,
    /// Currently in an `in_progress`-category state (archived excluded).
    pub in_progress: i64,
    /// Currently blocked: a `blocked`-category state, or an open dependency
    /// (directly or inherited from an ancestor inside the subtree).
    pub blocked: i64,
    /// Currently non-terminal and not archived — the work still open.
    pub open: i64,
    /// The newest subtree event at or after the claim, if any.
    pub last_activity_at: Option<i64>,
    /// Seconds since the last movement, anchored at the claim itself when
    /// nothing has moved — "idle for as long as it has been held".
    pub idle_seconds: i64,
}

/// The answer to `GET /v1/tickets/{id}/claim`: who holds the ticket, for how
/// long, and — when the claim is active and its grant time is known — what has
/// moved beneath it. Exists for any ticket, but earns its keep on an epic whose
/// indefinite claim has no expiry to judge it by.
#[derive(Debug, Clone)]
pub struct ClaimStatus {
    pub ticket: String,
    pub ty: String,
    pub holder: Option<String>,
    pub held_since: Option<i64>,
    pub held_for_seconds: Option<i64>,
    pub expires_at: Option<i64>,
    /// True for an active claim with no expiry (an epic claim without a TTL).
    pub indefinite: bool,
    /// None when unclaimed, or when the claim predates the `claim_since`
    /// column and there is no honest anchor to count from.
    pub movement: Option<ClaimMovement>,
}

impl ClaimStatus {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "ticket": self.ticket,
            "type": self.ty,
            "holder": self.holder,
            "held_since": self.held_since.map(iso),
            "held_for_seconds": self.held_for_seconds,
            "expires_at": self.expires_at.map(iso),
            "indefinite": self.indefinite,
            "movement": self.movement.as_ref().map(|m| json!({
                "since": iso(m.since),
                "created": m.created,
                "closed": m.closed,
                "in_progress": m.in_progress,
                "blocked": m.blocked,
                "open": m.open,
                "last_activity_at": m.last_activity_at.map(iso),
                "idle_seconds": m.idle_seconds,
            })),
        })
    }
}

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

/// The ready-queue scope: everything up to and including the `WHERE`, with the
/// projection left to the caller. Ready = claimable state, unclaimed (or lease
/// expired), and unblocked — where blocked propagates from ancestors: a ticket
/// is blocked if it, or any ancestor, has a blocked_by edge to a non-terminal
/// ticket, or sits under an epic with an active claim. A claimed epic reserves
/// its whole subtree for the holder, so the shared queue must stop offering
/// those tickets — the holder claims them by id, which the epic gate permits.
///
/// Shared by [`ready_query`] and [`ready_total`] so the count and the page it
/// annotates cannot answer different questions. Duplicating this recursive CTE
/// to count it would be the obvious way to drift: a filter added to one copy and
/// not the other reports "20 of 137" where 137 counts tickets the page would
/// never have offered.
///
/// The CTEs and conditions themselves live in [`super::ready_sql`], because
/// `store::roadmap` counts the same predicate to report what the queue would
/// offer, and a second copy there had already drifted once.
fn ready_scope(projection: &str, filter: &ReadyFilter, now: i64) -> (String, Vec<SqlValue>) {
    let mut sql = format!(
        r#"
        WITH RECURSIVE {ctes}
        SELECT {projection} FROM tickets t
        JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        JOIN projects p ON p.id = t.project
        WHERE {conditions}
        "#,
        ctes = super::ready_sql::ready_ctes("?"),
        conditions = super::ready_sql::ready_conditions("?"),
    );
    // Four `now` binds, in the order `ready_sql`'s positional form emits them:
    // the claimed-epic check inside the blocked CTE, the active-claim check
    // inside anc_of_claimed, then the claim-expiry and occurrence-expiry checks.
    let mut params_vec: Vec<SqlValue> = vec![
        SqlValue::Integer(now),
        SqlValue::Integer(now),
        SqlValue::Integer(now),
        SqlValue::Integer(now),
    ];
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
    (sql, params_vec)
}

/// One page of the ready queue, ordered by priority then age.
fn ready_query(
    conn: &Connection,
    filter: &ReadyFilter,
    now: i64,
    limit: i64,
) -> ApiResult<Vec<Ticket>> {
    let (mut sql, mut params_vec) = ready_scope(TICKET_COLS, filter, now);
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

/// How many tickets the ready queue holds for this filter, ignoring any page
/// size — the number a caller needs to know whether the page it just read is
/// the whole queue or the first 20 of 137.
fn ready_total(conn: &Connection, filter: &ReadyFilter, now: i64) -> ApiResult<i64> {
    let (sql, params_vec) = ready_scope("COUNT(*)", filter, now);
    let mut stmt = conn.prepare(&sql)?;
    let total = stmt.query_row(rusqlite::params_from_iter(params_vec), |r| {
        r.get::<_, i64>(0)
    })?;
    Ok(total)
}

/// Grant a lease inside a write tx: bump fence, set holder + expiry, emit.
///
/// `ttl_seconds: None` grants a claim with **no expiry** — an epic claim held
/// until released. Only the epic path passes it; every ordinary claim arrives
/// with a clamped TTL.
///
/// `resumed` says this grant is a lapsed holder taking its own lease back in a
/// non-claimable state (see [`Store::claim_ticket`]); it changes nothing about how
/// the lease is written — same fence bump, same expiry, same holder lock — only
/// what the event and the response say happened.
fn grant_claim(
    conn: &Connection,
    ticket: &Ticket,
    actor: &str,
    ttl_seconds: Option<i64>,
    now: i64,
    resumed: bool,
) -> ApiResult<Lease> {
    // If an expired claim is still recorded, clear it first (emits lease_expired).
    clear_expired_claim(conn, ticket, now)?;
    let expires = ttl_seconds.map(|ttl| now + ttl * 1000);
    // `lapsed_claim_holder = NULL`: a lease exists again, so there is no lapsed
    // one to resume. Whoever wins this claim is the only actor the ticket answers
    // to, which is what makes the marker's absence mean "nothing to resume"
    // rather than "we forgot".
    conn.execute(
        "UPDATE tickets SET fence_seq = fence_seq + 1, claim_holder = ?2, claim_expires_at = ?3, claim_since = ?4, lapsed_claim_holder = NULL, version = version + 1, updated_at = ?5 WHERE id = ?1",
        params![ticket.id, actor, expires, now, now],
    )?;
    let fence: i64 = conn.query_row(
        "SELECT fence_seq FROM tickets WHERE id = ?1",
        params![ticket.id],
        |r| r.get(0),
    )?;
    // Still a `claimed` event, not a kind of its own: this *is* a claim, and every
    // consumer that tracks who holds what has to see it. The payload flag is what
    // tells a supervisor counting lapses apart from fresh grants. `ttl_seconds:
    // null` + `indefinite: true` = an epic claim with no expiry.
    let mut payload = json!({ "fence": fence, "ttl_seconds": ttl_seconds });
    if ttl_seconds.is_none() {
        payload["indefinite"] = json!(true);
    }
    if resumed {
        payload["resumed_after_expiry"] = json!(true);
        payload["state"] = json!(ticket.state);
    }
    emit_event(
        conn,
        Some(&ticket.id),
        Some(&ticket.project),
        actor,
        "claimed",
        payload,
        now,
    )?;
    Ok(Lease {
        ticket: ticket.id.clone(),
        holder: actor.to_string(),
        fence,
        expires_at: expires,
        resumed,
    })
}

/// What the project workflow says about the state a ticket sits in, as far as
/// claiming is concerned. Read from the denormalized `workflow_states` table
/// rather than the stored workflow document for the reason
/// [`project_claim_ttls`] gives: this is the claim path, and parsing the whole
/// state machine to learn three flags would put that cost on every claim.
///
/// A state the table does not know is treated as not claimable and not
/// resumable — the fail-closed direction.
struct StateFacts {
    claimable: bool,
    terminal: bool,
    category: String,
}

fn state_facts(tx: &Connection, project: &str, state: &str) -> ApiResult<StateFacts> {
    let row = tx
        .query_row(
            "SELECT claimable, terminal, category FROM workflow_states WHERE project = ?1 AND state = ?2",
            params![project, state],
            |r| {
                Ok(StateFacts {
                    claimable: r.get::<_, i64>(0)? != 0,
                    terminal: r.get::<_, i64>(1)? != 0,
                    category: r.get::<_, String>(2)?,
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or(StateFacts {
        claimable: false,
        terminal: false,
        category: String::new(),
    }))
}

/// **The** rule for resuming a lapsed lease in place, in one place, because both
/// the claim path and the `transition.claim_required` remedy have to agree about
/// it — an error that offers a call the store then refuses is the defect
/// takomo-jb5i is about.
///
/// True when `actor` may take a lease on a ticket whose state the workflow does
/// **not** mark claimable. Every clause is load-bearing:
///
/// - `!claimable` — where the state *is* claimable the ordinary rules already
///   work, and they let anyone claim. This path exists only for the states where
///   the ready queue cannot help.
/// - `lapsed_holder(now) == Some(actor)` — the whole safety argument. The lease
///   here ended by expiry, it was *this* actor's, and nothing has claimed,
///   released or revoked one since (a new claim clears the marker and bumps the
///   fence). So there is no second worker to diverge from: this is the same actor
///   picking its own work back up, not a zombie racing a successor. Any other
///   actor falls through to the ordinary `claim.state` refusal.
/// - `!terminal` — a finished ticket has nothing to resume, and a lease on it
///   would show up on the board as work in flight.
/// - `category != "blocked"` — a parked ticket belongs to the ask-a-human
///   machinery until the answer lands; taking a lease there would collide with the
///   resume transition rather than unblock anything.
pub(super) fn may_resume_lapsed_lease(
    ticket: &Ticket,
    actor: &str,
    now: i64,
    claimable: bool,
    terminal: bool,
    category: &str,
) -> bool {
    !claimable && !terminal && category != "blocked" && ticket.lapsed_holder(now) == Some(actor)
}

/// A claimed epic somewhere above a ticket: who holds it and since when.
/// What the epic gate reports when it refuses a claim.
struct EpicHold {
    epic: String,
    holder: String,
    since: Option<i64>,
    expires_at: Option<i64>,
}

/// The nearest reason `actor` may not claim under this ticket: an ancestor
/// (any distance up) of type `epic` carrying an active claim held by someone
/// else. The holder's own claims never match — the whole point of the hold is
/// that the subtree is reserved *for* the holder.
fn foreign_epic_hold_above(
    tx: &Connection,
    id: &str,
    actor: &str,
    now: i64,
) -> ApiResult<Option<EpicHold>> {
    let row = tx
        .query_row(
            r#"
            WITH RECURSIVE anc(id) AS (
                SELECT parent FROM tickets WHERE id = ?1 AND parent IS NOT NULL
                UNION
                SELECT t.parent FROM tickets t JOIN anc ON t.id = anc.id
                WHERE t.parent IS NOT NULL
            )
            SELECT t.id, t.claim_holder, t.claim_since, t.claim_expires_at
            FROM tickets t JOIN anc ON t.id = anc.id
            WHERE t.type = 'epic' AND t.claim_holder IS NOT NULL
              AND (t.claim_expires_at IS NULL OR t.claim_expires_at > ?2)
              AND t.claim_holder != ?3
            LIMIT 1
            "#,
            params![id, now, actor],
            |r| {
                Ok(EpicHold {
                    epic: r.get(0)?,
                    holder: r.get(1)?,
                    since: r.get(2)?,
                    expires_at: r.get(3)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Active claims by anyone other than `actor` on descendants of `id`, as
/// `(ticket, holder)` pairs. What stops an epic claim from being taken over
/// live work: the lock never fights a lease already granted.
fn foreign_claims_below(
    tx: &Connection,
    id: &str,
    actor: &str,
    now: i64,
) -> ApiResult<Vec<(String, String)>> {
    let mut stmt = tx.prepare(
        r#"
        WITH RECURSIVE sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?1
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
        SELECT t.id, t.claim_holder FROM tickets t JOIN sub ON t.id = sub.id
        WHERE t.claim_holder IS NOT NULL
          AND (t.claim_expires_at IS NULL OR t.claim_expires_at > ?2)
          AND t.claim_holder != ?3
        ORDER BY t.id
        "#,
    )?;
    let rows = stmt
        .query_map(params![id, now, actor], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count what moved in `id`'s subtree since `since`. Read-only; every count is
/// taken in one connection so the numbers describe one instant.
fn movement_since(conn: &Connection, id: &str, since: i64, now: i64) -> ApiResult<ClaimMovement> {
    // The shared subtree scope (descendants only — the claimed ticket itself is
    // the lock, not the work being measured).
    const SUB: &str = r#"
        WITH RECURSIVE sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?1
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
    "#;
    let created: i64 = conn.query_row(
        &format!("{SUB} SELECT COUNT(*) FROM tickets t JOIN sub s ON t.id = s.id WHERE t.created_at >= ?2"),
        params![id, since],
        |r| r.get(0),
    )?;
    // A close is a `transitioned` event whose target state is terminal in that
    // ticket's project. Read from the event log rather than current rows so a
    // ticket closed and later reopened still counts as "closed since the
    // claim" — the movement happened, whatever came after.
    let closed: i64 = conn.query_row(
        &format!(
            "{SUB}
            SELECT COUNT(DISTINCT e.ticket) FROM events e
            JOIN sub s ON e.ticket = s.id
            JOIN tickets t ON t.id = e.ticket
            JOIN workflow_states ws ON ws.project = t.project
                AND ws.state = json_extract(e.payload, '$.to')
            WHERE e.kind = 'transitioned' AND e.at >= ?2 AND ws.terminal = 1"
        ),
        params![id, since],
        |r| r.get(0),
    )?;
    let (in_progress, open): (i64, i64) = conn.query_row(
        &format!(
            "{SUB}
            SELECT
                COALESCE(SUM(ws.category = 'in_progress'), 0),
                COUNT(*)
            FROM tickets t JOIN sub s ON t.id = s.id
            JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
            WHERE ws.terminal = 0 AND t.archived_at IS NULL"
        ),
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Blocked now: parked in a blocked-category state, or carrying an open
    // dependency (directly or inherited through a parent) — the same
    // propagation the ready queue uses, so the two never disagree about what
    // "blocked" means.
    let blocked: i64 = conn.query_row(
        &format!(
            "{SUB},
            dep_blocked(id) AS (
                SELECT DISTINCT d.ticket
                FROM deps d
                JOIN tickets b ON b.id = d.blocked_by
                JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
                WHERE bs.terminal = 0
                UNION
                SELECT c.id FROM tickets c JOIN dep_blocked db ON c.parent = db.id
            )
            SELECT COUNT(*) FROM tickets t JOIN sub s ON t.id = s.id
            JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
            WHERE ws.terminal = 0 AND t.archived_at IS NULL
              AND (ws.category = 'blocked' OR t.id IN (SELECT id FROM dep_blocked))"
        ),
        params![id],
        |r| r.get(0),
    )?;
    let last_activity_at: Option<i64> = conn.query_row(
        &format!(
            "{SUB} SELECT MAX(e.at) FROM events e JOIN sub s ON e.ticket = s.id WHERE e.at >= ?2"
        ),
        params![id, since],
        |r| r.get(0),
    )?;
    Ok(ClaimMovement {
        since,
        created,
        closed,
        in_progress,
        blocked,
        open,
        last_activity_at,
        idle_seconds: (now - last_activity_at.unwrap_or(since)).max(0) / 1000,
    })
}

impl Store {
    /// Claim a specific ticket. Idempotent renewal when the caller already
    /// holds it.
    ///
    /// Two ways to succeed. The ordinary one: the state is claimable, the ticket
    /// is unclaimed and unblocked. The narrow one, [`may_resume_lapsed_lease`]:
    /// the caller is the holder whose own lease expired here, in a state the
    /// workflow does not mark claimable, with nobody having claimed since — the
    /// way out of the deadlock where `done` demands a claim and `claim` refuses
    /// the state that `done` is being called from (takomo-jb5i). Fencing is
    /// untouched by it: the fence still bumps, so the resumed lease supersedes
    /// every echo of the old one, and an actor that is *not* the lapsed holder is
    /// refused exactly as before.
    pub fn claim_ticket(
        &self,
        id: &str,
        actor: &str,
        ttl_seconds: Option<i64>,
    ) -> ApiResult<(Ticket, Lease)> {
        let now = now_ms();
        self.with_tx(|tx| claim_ticket_tx(tx, id, actor, ttl_seconds, now))
    }
}

/// Claim (or idempotently renew) inside an existing write transaction. Shared
/// with [`Store::start_ticket`] so a claim that succeeds but a transition that
/// follows cannot leave a stranded lease.
pub(super) fn claim_ticket_tx(
    tx: &Connection,
    id: &str,
    actor: &str,
    ttl_seconds: Option<i64>,
    now: i64,
) -> ApiResult<(Ticket, Lease)> {
    let t = get_ticket_required(tx, id)?;
    ensure_ticket_writable(tx, &t)?;
    // Resolved inside the transaction, because the bounds are the
    // *project's* now and only the ticket names its project. An epic is
    // the one shape with no default TTL: claimed without one, it is
    // held until released (None all the way to the row); an explicit
    // TTL is clamped exactly like any other claim's.
    let ttl: Option<i64> = if t.ty == "epic" {
        ttl_seconds
            .map(|s| clamp_ttl_for(tx, &t.project, Some(s)))
            .transpose()?
    } else {
        Some(clamp_ttl_for(tx, &t.project, ttl_seconds)?)
    };

    if let Some((holder, expires)) = t.active_claim(now) {
        if holder == actor {
            // Idempotent renewal: keep the fence, recompute the expiry
            // the way a fresh claim would — which for an epic claimed
            // without a TTL means staying (or becoming) indefinite.
            let new_expires = ttl.map(|s| now + s * 1000);
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
                resumed: false,
            };
            let fresh = get_ticket_required(tx, id)?;
            return Ok((fresh, lease));
        }
        return Err(ApiError::conflict(
                    "claim.held",
                    format!(
                        "Ticket '{id}' is already claimed by '{holder}' {}. Pick different work (POST /v1/ready/claim), or read the claim's age and what has moved under it with GET /v1/tickets/{id}/claim.",
                        held_phrase(expires)
                    ),
                )
                .details(json!({ "holder": holder, "expires_at": expires.map(iso) })));
    }

    // The epic gate: an epic with an active claim reserves its whole
    // subtree for the holder, so anyone else claiming beneath it is
    // refused — that is what the epic claim *is*. The holder passes
    // through and works children with ordinary leased claims.
    if let Some(hold) = foreign_epic_hold_above(tx, id, actor, now)? {
        let EpicHold {
            epic,
            holder,
            since,
            expires_at,
        } = hold;
        return Err(ApiError::conflict(
                    "claim.epic_held",
                    format!(
                        "Ticket '{id}' sits under epic '{epic}', which '{holder}' has claimed {} — the whole subtree is reserved for that holder until the epic claim ends. Read the claim's age and what has moved under it with GET /v1/tickets/{epic}/claim; if it looks abandoned, ask an admin to POST /v1/tickets/{epic}/force-release. Otherwise pick different work via POST /v1/ready/claim (this subtree is not offered there).",
                        held_phrase(expires_at)
                    ),
                )
                .details(json!({
                    "epic": epic,
                    "holder": holder,
                    "held_since": since.map(iso),
                    "expires_at": expires_at.map(iso),
                })));
    }

    // State must be claimable per the project workflow — or the caller
    // must be the holder whose lease lapsed right here.
    let facts = state_facts(tx, &t.project, &t.state)?;
    let resumed = may_resume_lapsed_lease(
        &t,
        actor,
        now,
        facts.claimable,
        facts.terminal,
        &facts.category,
    );
    if !facts.claimable && !resumed {
        let claimable_states: Vec<String> = {
            let mut stmt = tx.prepare(
                        "SELECT state FROM workflow_states WHERE project = ?1 AND claimable = 1 ORDER BY state",
                    )?;
            let states = stmt
                .query_map(params![t.project], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            states
        };
        // Naming the lapsed holder is the difference between "not
        // claimable" and "not claimable *by you*": an agent that has lost
        // a race to a successor needs to know it lost, not retry.
        let whose = match t.lapsed_holder(now) {
                    Some(h) if h != actor => format!(
                        " The lease that lapsed in this state belonged to '{h}', so only '{h}' can resume it in place; you cannot."
                    ),
                    _ => String::new(),
                };
        return Err(ApiError::conflict(
                    "claim.state",
                    format!(
                        "Ticket '{id}' is in state '{}', which is not claimable. Claimable states in project '{}': {}. Move it with POST /v1/tickets/{id}/transition first, or pick ready work via POST /v1/ready/claim.{whose}",
                        t.state,
                        t.project,
                        claimable_states.join(", ")
                    ),
                )
                .details(json!({
                    "claimable_states": claimable_states,
                    "lapsed_holder": t.lapsed_holder(now),
                }))
                .current_state(t.state.clone()));
    }

    // Must be unblocked (directly or via ancestors). This applies to a
    // resume too: a dependency that opened while the lease was lapsing is
    // a real answer, and every guard on the transition the caller wants to
    // make would stop it a moment later anyway.
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

    // A fresh epic claim must not be taken over live work: a lease
    // already granted to someone else inside the subtree wins, and the
    // would-be locker waits or coordinates instead of fighting it.
    // The caller's own claims below don't count — the subtree is being
    // reserved for exactly that actor.
    if t.ty == "epic" {
        let held = foreign_claims_below(tx, id, actor, now)?;
        if !held.is_empty() {
            let listed: Vec<String> = held
                .iter()
                .map(|(tid, holder)| format!("'{tid}' (claimed by '{holder}')"))
                .collect();
            let holders: Vec<serde_json::Value> = held
                .iter()
                .map(|(tid, holder)| json!({ "ticket": tid, "holder": holder }))
                .collect();
            return Err(ApiError::conflict(
                        "claim.children_held",
                        format!(
                            "Epic '{id}' cannot be claimed while other workers hold live claims inside it: {}. Claiming an epic reserves its whole subtree, and the reservation never displaces a lease already granted. Wait for those claims to end, coordinate with the holders, or (admin) force-release them first.",
                            listed.join(", ")
                        ),
                    )
                    .details(json!({ "held": holders })));
        }
    }

    let lease = grant_claim(tx, &t, actor, ttl, now, resumed)?;
    let fresh = get_ticket_required(tx, id)?;
    Ok((fresh, lease))
}

impl Store {
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
            ensure_ticket_writable(tx, &t)?;
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
                Some((_, None)) => {
                    // An epic claim with no expiry has nothing to renew. Answer
                    // the beat with the lease as it stands rather than a 409, so
                    // a harness that heartbeats on a schedule keeps working —
                    // and never write the TTL, or a routine beat would quietly
                    // convert "held until released" into "expires in 15
                    // minutes".
                    Ok(Lease {
                        ticket: id.to_string(),
                        holder: actor.to_string(),
                        fence,
                        expires_at: None,
                        resumed: false,
                    })
                }
                Some((_, Some(_))) => {
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
                        expires_at: Some(expires),
                        resumed: false,
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
                    // `lapsed_claim_holder = NULL` for completeness rather than
                    // repair: the marker cannot be set while a lease is active.
                    // Letting go on purpose is not a lapse, so nothing here may be
                    // resumed afterwards.
                    tx.execute(
                        "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, claim_since = NULL, lapsed_claim_holder = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
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

    /// Admin force-release: drop whatever claim the ticket carries without
    /// asking who holds it or what fence they have. The recovery path for a
    /// worker that is gone, since [`Store::release`] answers only to the holder
    /// and the sweeper frees a lease exactly when it expires and not a moment
    /// sooner — and `max_claim_ttl_seconds` is a per-project setting with no
    /// ceiling, so "when it expires" can be arbitrarily far away (takomo-cjel).
    ///
    /// **Bumps `fence_seq`.** That is the whole point: the displaced worker,
    /// which may still be alive and mid-write, now carries a stale fence and its
    /// next mutating call gets a teaching 409 instead of winning. A force that
    /// left the fence alone would hand the ticket to a new claimant while the
    /// old one kept writing to it.
    ///
    /// An already-lapsed lease is force-released too, rather than refused: the
    /// holder is still recorded on the row and — because natural expiry does
    /// *not* bump the fence — a zombie echoing that fence would still be
    /// accepted. Refusing here would make the outcome depend on whether the TTL
    /// happened to elapse a moment before the admin's call.
    ///
    /// That holds after the sweep has cleared the claim row too: the lapsed holder
    /// is then recorded as `lapsed_claim_holder`, and that is not nothing — it is
    /// permission to resume the lease in place (takomo-jb5i). So a force lands on
    /// it as well, and the operator's recovery path stays total: for every state in
    /// which a worker could still take this ticket back, there is a call that stops
    /// it. Only a ticket with neither an active claim nor a lapsed one is a
    /// `claim.none`.
    pub fn force_release(
        &self,
        id: &str,
        actor: &str,
        reason: Option<&str>,
    ) -> ApiResult<ForcedRelease> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            let Some(holder) = t
                .claim_holder
                .clone()
                .or_else(|| t.lapsed_claim_holder.clone())
            else {
                return Err(ApiError::conflict(
                    "claim.none",
                    format!(
                        "Ticket '{id}' holds no claim and no lapsed one, so there is nothing to force-release. It is already free for the next worker (if its state is claimable) and no earlier holder can resume it. A force-release is not idempotent bookkeeping — it reports what it displaced — so a second call on the same ticket lands here."
                    ),
                )
                .remedy(format!(
                    "Nothing to do. Confirm with GET /v1/tickets/{id} (a `claim` of null means unclaimed) and check GET /v1/events?ticket={id} for the released / lease_expired / lease_revoked event that already freed it."
                ))
                .current_state(t.state.clone()));
            };
            // True for both lapsed shapes: a claim row whose TTL has passed, and a
            // claim already swept away leaving only `lapsed_claim_holder` — that
            // one is by definition an expiry, which is the only thing that sets it.
            let lease_expired =
                t.claim_holder.is_none() || t.claim_expires_at.is_some_and(|exp| exp <= now);
            // `lapsed_claim_holder = NULL` on purpose, and it matters: a force is
            // an operator taking the ticket away, so the displaced holder must not
            // be able to resume the lease in place afterwards (takomo-jb5i). The
            // fence bump alone would not stop that — the resume path does not echo
            // a fence — so the marker has to go too.
            tx.execute(
                "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, claim_since = NULL, lapsed_claim_holder = NULL, fence_seq = fence_seq + 1, version = version + 1, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            let fence: i64 = tx.query_row(
                "SELECT fence_seq FROM tickets WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            // Its own kind, not `released` with a flag: `kind` is the only
            // server-side event filter (GET /v1/events?kind=), so an audit
            // consumer asking "what has an admin forcibly taken?" needs one —
            // and a `released` event whose actor is not the holder would read to
            // every existing consumer as the holder letting go voluntarily.
            emit_event(
                tx,
                Some(id),
                Some(&t.project),
                actor,
                "lease_revoked",
                json!({
                    "holder": holder,
                    "fence": t.fence_seq,
                    "new_fence": fence,
                    "lease_expired": lease_expired,
                    "expires_at": t.claim_expires_at.map(iso),
                    "reason": reason,
                }),
                now,
            )?;
            Ok(ForcedRelease {
                ticket: id.to_string(),
                project: t.project.clone(),
                previous_holder: holder,
                previous_fence: t.fence_seq,
                fence,
                lease_expired,
                reason: reason.map(str::to_string),
            })
        })
    }

    /// Peek the ready queue (no side effects): one page, plus how many tickets
    /// the queue holds in total.
    ///
    /// The total is what makes a short page readable. Without it a caller that
    /// asked for 20 and got 20 cannot tell a queue of exactly 20 from a queue of
    /// 137, and an agent draining work has no way to know it is looking at a
    /// fraction. Both come from one `with_conn`, so they are consistent with each
    /// other even though the queue mutates constantly around them.
    ///
    /// Deliberately a total and not a cursor: the ready queue is a *live*
    /// priority queue that other workers are claiming from as you read it, so a
    /// positional cursor would promise a stable sequence that does not exist.
    /// "20 of 137" is true when it is read; "page 2" would not be.
    pub fn ready_peek(&self, filter: &ReadyFilter, limit: i64) -> ApiResult<(Vec<Ticket>, i64)> {
        self.with_conn(|conn| {
            let now = now_ms();
            let tickets = ready_query(conn, filter, now, limit)?;
            let total = ready_total(conn, filter, now)?;
            Ok((tickets, total))
        })
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
            // Never a resume: the ready queue only ever hands out tickets in
            // claimable states, which is the one case the resume path excludes.
            // Always leased, epics included: an agent that asked for "next
            // work" gets a TTL-bounded claim, never an indefinite hold it did
            // not know it was taking — the no-expiry shape is only ever an
            // explicit claim-by-id on the epic.
            let lease = grant_claim(tx, &t, actor, Some(ttl), now, false)?;
            let fresh = get_ticket_required(tx, &t.id)?;
            Ok(Some((fresh, lease)))
        })
    }

    /// Who holds `id`, for how long, and what has moved beneath it — the read
    /// that makes an indefinite epic claim judgeable. A lock with no expiry is
    /// only safe to offer because this exists: "held five hours, ten tickets
    /// created, five closed, movement two minutes ago" is an agent mid-flight;
    /// the same five hours with nothing moved is a lock to force-release.
    pub fn claim_status(&self, id: &str) -> ApiResult<ClaimStatus> {
        let now = now_ms();
        self.with_conn(|conn| {
            let t = get_ticket_required(conn, id)?;
            let (holder, expires) = match t.active_claim(now) {
                Some((h, e)) => (Some(h.to_string()), e),
                None => (None, None),
            };
            let held_since = holder.as_ref().and(t.claim_since);
            // Movement needs an anchor: an active claim whose grant time is
            // known. A claim from before the `claim_since` column reports no
            // movement rather than counting from an invented instant.
            let movement = match held_since {
                Some(since) => Some(movement_since(conn, id, since, now)?),
                None => None,
            };
            Ok(ClaimStatus {
                ticket: t.id,
                ty: t.ty,
                indefinite: holder.is_some() && expires.is_none(),
                holder,
                held_since,
                held_for_seconds: held_since.map(|s| (now - s).max(0) / 1000),
                expires_at: expires,
                movement,
            })
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
