//! Epic progress rollup: for each `epic` in a project, aggregate its full
//! descendant subtree (walked recursively via `parent`) into counts by state
//! and by category, plus a done-count and completion percent. Read-only; drives
//! `GET /v1/projects/{project}/roadmap` and `takomo roadmap`.
//!
//! Two things sit alongside the per-epic rollups so the response accounts for
//! all the work, not just the work someone remembered to file under an epic:
//!
//! - `unparented`: a rollup with the same shape as an epic's (`total`, `done`,
//!   `percent`, `by_state`, `by_category`, and no ticket identity) over every
//!   non-epic ticket in the project whose `parent` chain never reaches an
//!   `epic`. That covers a NULL parent, a chain of non-epic ancestors, and a
//!   dangling `parent` pointing at a row that no longer exists. Without it the
//!   percentages read as complete while real work is invisible.
//! - `flags` on each epic: short codes for an epic whose own state contradicts
//!   its children — `done_with_open_children`, `open_with_all_children_done`,
//!   `empty_epic`. Empty when the epic is consistent. `empty_epic` is a flag
//!   and not an error: an epic filed ahead of its work is legitimate, and the
//!   flag lets a client render it differently from a 0%-complete epic that
//!   does have children.
//!
//! Both recursive walks use `WITH RECURSIVE ... UNION` (not `UNION ALL`), which
//! stops at an already-visited id — a malformed `parent` cycle terminates
//! rather than hanging the endpoint.

use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use rusqlite::{params, Connection, Statement};
use serde_json::{json, Map, Value};

/// The blocked-set CTE, verbatim from the ready queue (`store::claims`):
/// a ticket is blocked if it, or any ancestor, has a `blocked_by` edge to a
/// non-terminal ticket. Shared as a string constant rather than re-typed per
/// query so `ready` here can never drift from the queue that actually hands
/// work out — a rollup that disagreed with `takomo ready` would be worse than
/// no rollup at all.
///
/// `UNION` (not `UNION ALL`) again: it stops at an already-visited id, so a
/// malformed `parent` cycle terminates instead of hanging.
const BLOCKED_CTE: &str = r#"
        blocked(id) AS (
            SELECT DISTINCT d.ticket
            FROM deps d
            JOIN tickets b ON b.id = d.blocked_by
            JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
            WHERE bs.terminal = 0
            UNION
            SELECT c.id FROM tickets c JOIN blocked ON c.parent = blocked.id
        )"#;

/// The two per-ticket predicates every rollup query selects alongside its
/// counts. `?{n}` is the `now` millis bound for the lease check.
///
/// `ready` mirrors the ready queue exactly: claimable state, not archived,
/// unclaimed or lease-expired, and not in the blocked set.
///
/// `awaiting` counts a ticket carrying at least one OPEN question, in ANY mode.
/// Advisory questions are included deliberately: the number answers "is a
/// decision outstanding here", which is what makes a queue entry misleading to
/// pick up, and a `blocking` question has already moved its ticket out of a
/// claimable state anyway — so restricting to blocking would report ~0 on the
/// very tickets this count exists to surface.
fn rollup_selects(now_param: usize) -> String {
    format!(
        // Column ORDER is the contract with `collect_rollup`, which reads by
        // index: 3 = ready, 4 = awaiting, 5 = claimable_state.
        r#",
               SUM(CASE WHEN ws.claimable = 1
                         AND t.archived_at IS NULL
                         AND (t.claim_holder IS NULL OR t.claim_expires_at <= ?{now_param})
                         AND t.id NOT IN (SELECT id FROM blocked)
                        THEN 1 ELSE 0 END) AS ready,
               SUM(CASE WHEN EXISTS (SELECT 1 FROM questions q
                                      WHERE q.ticket = t.id AND q.status = 'open')
                        THEN 1 ELSE 0 END) AS awaiting,
               COALESCE(MAX(ws.claimable), 0) AS claimable_state"#
    )
}

/// Aggregate over a set of tickets (an epic's descendant subtree, or the
/// unparented bucket).
struct Rollup {
    total: i64,
    done: i64,
    /// Claimable, unclaimed and unblocked — what `takomo ready` would hand out.
    ready: i64,
    /// In a claimable state but NOT ready: blocked by a dep, or already claimed.
    /// `ready + backlog` is the whole claimable category, so the two split it.
    backlog: i64,
    /// Carrying at least one open question. An OVERLAY, not a partition — such a
    /// ticket is also counted in its own state, and may or may not be `ready`.
    awaiting_answer: i64,
    by_state: Map<String, Value>,
    by_category: Map<String, Value>,
}

impl Rollup {
    /// `done/total` rounded to a whole percent (0 when the set is empty).
    fn percent(&self) -> i64 {
        if self.total > 0 {
            ((self.done as f64 / self.total as f64) * 100.0).round() as i64
        } else {
            0
        }
    }
}

/// Fold `(state, category, count, ready, awaiting, claimable)` rows — the shape
/// every rollup query below returns — into a `Rollup`.
fn collect_rollup(stmt: &mut Statement, args: &[&dyn rusqlite::ToSql]) -> ApiResult<Rollup> {
    let rows = stmt
        .query_map(args, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut total = 0i64;
    let mut done = 0i64;
    let mut ready = 0i64;
    let mut claimable = 0i64;
    let mut awaiting_answer = 0i64;
    let mut by_state: Map<String, Value> = Map::new();
    let mut by_category: Map<String, Value> = Map::new();
    for (st, cat, n, rdy, awaiting, is_claimable) in rows {
        total += n;
        by_state.insert(st, json!(n));
        if cat == "done" {
            done += n;
        }
        // The CLAIMABLE flag, not the `todo` CATEGORY: `draft` is category
        // `todo` and deliberately not claimable, so counting the category would
        // put an un-pickable draft in the backlog and make ready+backlog
        // disagree with what the queue can actually hand out.
        if is_claimable == 1 {
            claimable += n;
        }
        ready += rdy;
        awaiting_answer += awaiting;
        if !cat.is_empty() {
            let prev = by_category.get(&cat).and_then(Value::as_i64).unwrap_or(0);
            by_category.insert(cat, json!(prev + n));
        }
    }
    Ok(Rollup {
        total,
        done,
        ready,
        // Never negative: `ready` is a subset of the claimable rows by
        // construction (its CASE requires `ws.claimable = 1`).
        backlog: (claimable - ready).max(0),
        awaiting_answer,
        by_state,
        by_category,
    })
}

/// One epic plus the rollup over its descendants. `total`/`done`/`percent`
/// count the whole subtree beneath the epic (the epic itself is the container,
/// not counted). `done` is the number of descendants whose state category is
/// `done`; `percent` is `done/total` rounded to a whole percent (0 when empty).
fn rollup_for_epic(conn: &Connection, epic_id: &str, now: i64) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE{BLOCKED_CTE},
        sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?1
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM sub
        JOIN tickets t ON t.id = sub.id
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        GROUP BY t.state
        "#,
        selects = rollup_selects(2)
    );
    let mut stmt = conn.prepare(&sql)?;
    collect_rollup(&mut stmt, params![epic_id, now])
}

/// Rollup over the project's non-epic tickets that no epic owns: the recursive
/// term grows the set of tickets reachable *downward* from any epic, and the
/// outer select keeps everything else. A ticket is excluded exactly when its
/// `parent` chain reaches an epic, so a NULL parent, an all-non-epic ancestor
/// chain, and a dangling parent id all land in the bucket.
fn rollup_unparented(conn: &Connection, project: &str, now: i64) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE{BLOCKED_CTE},
        owned(id) AS (
            SELECT t.id FROM tickets t
              JOIN tickets p ON t.parent = p.id
             WHERE t.project = ?1 AND p.type = 'epic'
            UNION
            SELECT t.id FROM tickets t JOIN owned ON t.parent = owned.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM tickets t
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        WHERE t.project = ?1
          AND t.type <> 'epic'
          AND t.id NOT IN (SELECT id FROM owned)
        GROUP BY t.state
        "#,
        selects = rollup_selects(2)
    );
    let mut stmt = conn.prepare(&sql)?;
    collect_rollup(&mut stmt, params![project, now])
}

/// Contradiction codes for an epic whose own state disagrees with its subtree.
/// Pure derivation over the epic's `state_category` and its rollup counts — no
/// extra query per epic.
fn epic_flags(state_category: &str, r: &Rollup) -> Vec<&'static str> {
    let mut flags = Vec::new();
    if state_category == "done" && r.done < r.total {
        flags.push("done_with_open_children");
    }
    if state_category != "done" && r.total > 0 && r.done == r.total {
        flags.push("open_with_all_children_done");
    }
    if r.total == 0 {
        flags.push("empty_epic");
    }
    flags
}

impl Store {
    /// Roadmap rollup for every epic in `project`, plus the `unparented` bucket
    /// for work no epic owns. Returns a 404 for an unknown project. Each epic
    /// carries its own metadata, a subtree rollup, and contradiction `flags`.
    /// `epic` narrows the report to ONE epic. The `unparented` bucket is then
    /// omitted rather than returned empty: it is a project-wide statement about
    /// work no epic owns, and repeating it under a single-epic view would read
    /// as "this epic has no unparented work", which is not a thing an epic can
    /// have. An unknown epic 404s instead of returning an empty list, for the
    /// same reason an unknown project does.
    pub fn roadmap(&self, project: &str, epic: Option<&str>) -> ApiResult<Value> {
        let now = now_ms();
        self.with_conn(|conn| {
            // 404 for an unknown project, so a scoped caller gets a clean error
            // rather than an empty list that hides a typo.
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .ok();
            if exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }

            // A filter naming a ticket that is not an epic OF THIS PROJECT is a
            // 404, not an empty report — the three ways to get it wrong (typo,
            // wrong project, naming a task) are indistinguishable in an empty
            // list and all of them are worth telling the caller about.
            if let Some(e) = epic {
                let ok: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM tickets WHERE id = ?1 AND project = ?2 AND type = 'epic'",
                        params![e, project],
                        |r| r.get(0),
                    )
                    .ok();
                if ok.is_none() {
                    return Err(ApiError::not_found("epic", e));
                }
            }

            let mut stmt = conn.prepare(
                r#"
                SELECT t.id, t.title, t.state, t.priority,
                       COALESCE((SELECT ws.category FROM workflow_states ws
                                 WHERE ws.project = t.project AND ws.state = t.state), '') AS category
                FROM tickets t
                WHERE t.project = ?1 AND t.type = 'epic'
                  AND (?2 IS NULL OR t.id = ?2)
                ORDER BY t.created_at ASC, t.rowid ASC
                "#,
            )?;
            let epics = stmt
                .query_map(params![project, epic], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut out = Vec::with_capacity(epics.len());
            for (id, title, st, priority, category) in epics {
                let r = rollup_for_epic(conn, &id, now)?;
                let flags = epic_flags(&category, &r);
                let percent = r.percent();
                out.push(json!({
                    "id": id,
                    "title": title,
                    "state": st,
                    "state_category": category,
                    "priority": priority,
                    "total": r.total,
                    "done": r.done,
                    "percent": percent,
                    "ready": r.ready,
                    "backlog": r.backlog,
                    "awaiting_answer": r.awaiting_answer,
                    "by_state": Value::Object(r.by_state),
                    "by_category": Value::Object(r.by_category),
                    "flags": flags,
                }));
            }

            let mut body = json!({
                "project": project,
                "generated_at": iso(now),
                "epics": out,
            });
            if let Some(e) = epic {
                // Echo the filter so a stored response says what it is a view of.
                body["epic"] = json!(e);
            } else {
                let u = rollup_unparented(conn, project, now)?;
                body["unparented"] = json!({
                    "total": u.total,
                    "done": u.done,
                    "percent": u.percent(),
                    "ready": u.ready,
                    "backlog": u.backlog,
                    "awaiting_answer": u.awaiting_answer,
                    "by_state": Value::Object(u.by_state),
                    "by_category": Value::Object(u.by_category),
                });
            }
            Ok(body)
        })
    }
}
