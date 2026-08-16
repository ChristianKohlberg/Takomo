//! Blocker impact: which open tickets are holding the most work, ranked.
//! Read-only; drives `GET /v1/projects/{project}/impact` and `takomo impact`.
//!
//! The question this answers is "if I close exactly one thing, what buys the
//! most?". That is a COUNTERFACTUAL, not a reachability count, and the
//! difference matters: a ticket with two independent blockers is released by
//! neither of them alone, so counting everything reachable from a blocker would
//! promise work that would still not be claimable. So for each blocker `B` the
//! count is
//!
//! ```text
//!   |blocked(now)|  -  |blocked(with B treated as terminal)|
//! ```
//!
//! restricted to the project, which is exactly the set of tickets that leave
//! the blocked set when `B` closes — no more.
//!
//! **`direct` vs `downstream`.** `blocked` propagates two ways: a `blocked_by`
//! edge, and inheritance by descendants (a child of a blocked ticket is
//! blocked). `direct` is the released tickets that carry an edge to `B`
//! themselves; `downstream` is the rest — released because an ancestor of
//! theirs was. Splitting them shows the shape of the hold: `4 direct, 0
//! downstream` is a flat fan-out, `2 direct, 11 downstream` means `B` is
//! pinning two subtrees.
//!
//! **What a chain does NOT do.** If `B` blocks `X` and `X` blocks `Y`, closing
//! `B` releases `X` but not `Y` — `X` is now claimable but still not terminal,
//! so it goes on blocking `Y`. `Y` therefore does not count towards `B`. This
//! is deliberate and is why the numbers are smaller than a naive graph walk;
//! they are what a reader can act on.
//!
//! Candidates are the non-terminal tickets that appear in `deps.blocked_by` at
//! all. A blocker may live in another project (deps cross projects) while the
//! work it releases is in this one — it is still reported, because the point is
//! to name the thing worth closing.

use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashSet;

/// The blocked set for `project`, optionally pretending one ticket is already
/// terminal. `exclude` is the counterfactual knob: passing `Some(b)` drops
/// every edge whose blocker is `b`, which is what "if b closed" means to the
/// ready queue.
///
/// Mirrors the ready queue's own CTE (`store::claims`) including the `UNION`
/// that makes a `parent` cycle terminate.
fn blocked_set(
    conn: &Connection,
    project: &str,
    exclude: Option<&str>,
) -> ApiResult<HashSet<String>> {
    let sql = r#"
        WITH RECURSIVE blocked(id) AS (
            SELECT DISTINCT d.ticket
            FROM deps d
            JOIN tickets b ON b.id = d.blocked_by
            JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
            WHERE bs.terminal = 0
              AND (?2 IS NULL OR d.blocked_by <> ?2)
            UNION
            SELECT c.id FROM tickets c JOIN blocked ON c.parent = blocked.id
        )
        SELECT b.id
        FROM blocked b
        JOIN tickets t ON t.id = b.id
        WHERE t.project = ?1 AND t.archived_at IS NULL
    "#;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![project, exclude], |r| r.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

impl Store {
    /// Rank the open blockers holding back work in `project`, most impactful
    /// first. Returns a 404 for an unknown project, for the same reason
    /// `roadmap` does: an empty list would hide a typo.
    ///
    /// Ties break on id so the order is stable between calls — a ranking that
    /// reshuffled on every poll would be unreadable in a dashboard.
    pub fn impact(&self, project: &str) -> ApiResult<Value> {
        let now = now_ms();
        self.with_conn(|conn| {
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

            let base = blocked_set(conn, project, None)?;

            // Candidate blockers: non-terminal tickets that actually block
            // something in this project. Restricting to those that block
            // in-project work keeps a shared blocker from listing here with a
            // count of 0 just because it blocks elsewhere.
            let mut stmt = conn.prepare(
                r#"
                SELECT DISTINCT b.id, b.project, b.title, b.state, b.priority, b.type
                FROM deps d
                JOIN tickets b ON b.id = d.blocked_by
                JOIN tickets t ON t.id = d.ticket
                JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
                WHERE bs.terminal = 0
                  AND t.project = ?1
                  AND b.archived_at IS NULL
                ORDER BY b.id ASC
                "#,
            )?;
            let candidates = stmt
                .query_map(params![project], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // Tickets holding a direct edge to a given blocker — used to split
            // the released set into direct and downstream.
            let mut direct_stmt = conn.prepare("SELECT ticket FROM deps WHERE blocked_by = ?1")?;

            let mut out: Vec<(i64, String, Value)> = Vec::with_capacity(candidates.len());
            for (id, proj, title, state, priority, ty) in candidates {
                let without = blocked_set(conn, project, Some(&id))?;
                // Everything that leaves the blocked set when `id` closes.
                let released: Vec<&String> = base.difference(&without).collect();
                if released.is_empty() {
                    continue;
                }
                let edges = direct_stmt
                    .query_map(params![id], |r| r.get::<_, String>(0))?
                    .collect::<Result<HashSet<_>, _>>()?;
                let direct = released.iter().filter(|t| edges.contains(**t)).count() as i64;
                let unblocks = released.len() as i64;
                let mut ids: Vec<String> = released.into_iter().cloned().collect();
                ids.sort();

                out.push((
                    unblocks,
                    id.clone(),
                    json!({
                        "id": id,
                        "project": proj,
                        "title": title,
                        "state": state,
                        "priority": priority,
                        "type": ty,
                        "unblocks": unblocks,
                        "direct": direct,
                        "downstream": unblocks - direct,
                        "unblocks_ids": ids,
                    }),
                ));
            }

            // Most impactful first; ties by id so the order never reshuffles.
            out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let blockers: Vec<Value> = out.into_iter().map(|(_, _, v)| v).collect();

            Ok(json!({
                "project": project,
                "generated_at": iso(now),
                // How many tickets in this project are blocked right now, so a
                // reader can see what share the top blocker accounts for.
                "blocked_total": base.len() as i64,
                "blockers": blockers,
            }))
        })
    }
}
