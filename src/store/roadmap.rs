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

/// Aggregate over a set of tickets (an epic's descendant subtree, or the
/// unparented bucket).
struct Rollup {
    total: i64,
    done: i64,
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

/// Fold `(state, category, count)` rows — the shape every rollup query below
/// returns — into a `Rollup`.
fn collect_rollup(stmt: &mut Statement, args: &[&dyn rusqlite::ToSql]) -> ApiResult<Rollup> {
    let rows = stmt
        .query_map(args, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut total = 0i64;
    let mut done = 0i64;
    let mut by_state: Map<String, Value> = Map::new();
    let mut by_category: Map<String, Value> = Map::new();
    for (st, cat, n) in rows {
        total += n;
        by_state.insert(st, json!(n));
        if cat == "done" {
            done += n;
        }
        if !cat.is_empty() {
            let prev = by_category.get(&cat).and_then(Value::as_i64).unwrap_or(0);
            by_category.insert(cat, json!(prev + n));
        }
    }
    Ok(Rollup {
        total,
        done,
        by_state,
        by_category,
    })
}

/// One epic plus the rollup over its descendants. `total`/`done`/`percent`
/// count the whole subtree beneath the epic (the epic itself is the container,
/// not counted). `done` is the number of descendants whose state category is
/// `done`; `percent` is `done/total` rounded to a whole percent (0 when empty).
fn rollup_for_epic(conn: &Connection, epic_id: &str) -> ApiResult<Rollup> {
    let mut stmt = conn.prepare(
        r#"
        WITH RECURSIVE sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?1
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
        SELECT t.state,
               COALESCE((SELECT ws.category FROM workflow_states ws
                         WHERE ws.project = t.project AND ws.state = t.state), '') AS category,
               COUNT(*) AS n
        FROM sub JOIN tickets t ON t.id = sub.id
        GROUP BY t.state
        "#,
    )?;
    collect_rollup(&mut stmt, params![epic_id])
}

/// Rollup over the project's non-epic tickets that no epic owns: the recursive
/// term grows the set of tickets reachable *downward* from any epic, and the
/// outer select keeps everything else. A ticket is excluded exactly when its
/// `parent` chain reaches an epic, so a NULL parent, an all-non-epic ancestor
/// chain, and a dangling parent id all land in the bucket.
fn rollup_unparented(conn: &Connection, project: &str) -> ApiResult<Rollup> {
    let mut stmt = conn.prepare(
        r#"
        WITH RECURSIVE owned(id) AS (
            SELECT t.id FROM tickets t
              JOIN tickets p ON t.parent = p.id
             WHERE t.project = ?1 AND p.type = 'epic'
            UNION
            SELECT t.id FROM tickets t JOIN owned ON t.parent = owned.id
        )
        SELECT t.state,
               COALESCE((SELECT ws.category FROM workflow_states ws
                         WHERE ws.project = t.project AND ws.state = t.state), '') AS category,
               COUNT(*) AS n
        FROM tickets t
        WHERE t.project = ?1
          AND t.type <> 'epic'
          AND t.id NOT IN (SELECT id FROM owned)
        GROUP BY t.state
        "#,
    )?;
    collect_rollup(&mut stmt, params![project])
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
    pub fn roadmap(&self, project: &str) -> ApiResult<Value> {
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

            let mut stmt = conn.prepare(
                r#"
                SELECT t.id, t.title, t.state, t.priority,
                       COALESCE((SELECT ws.category FROM workflow_states ws
                                 WHERE ws.project = t.project AND ws.state = t.state), '') AS category
                FROM tickets t
                WHERE t.project = ?1 AND t.type = 'epic'
                ORDER BY t.created_at ASC, t.rowid ASC
                "#,
            )?;
            let epics = stmt
                .query_map(params![project], |r| {
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
                let r = rollup_for_epic(conn, &id)?;
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
                    "by_state": Value::Object(r.by_state),
                    "by_category": Value::Object(r.by_category),
                    "flags": flags,
                }));
            }

            let u = rollup_unparented(conn, project)?;

            Ok(json!({
                "project": project,
                "generated_at": iso(now),
                "epics": out,
                "unparented": {
                    "total": u.total,
                    "done": u.done,
                    "percent": u.percent(),
                    "by_state": Value::Object(u.by_state),
                    "by_category": Value::Object(u.by_category),
                },
            }))
        })
    }
}
