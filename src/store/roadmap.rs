//! Epic progress rollup: for each `epic` in a project, aggregate its full
//! descendant subtree (walked recursively via `parent`) into counts by state
//! and by category, plus a done-count and completion percent. Read-only; drives
//! `GET /v1/projects/{project}/roadmap` and `takomo roadmap`.

use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

/// Aggregate over an epic's descendant subtree.
struct Rollup {
    total: i64,
    done: i64,
    by_state: Map<String, Value>,
    by_category: Map<String, Value>,
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
    let rows = stmt
        .query_map(params![epic_id], |r| {
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

impl Store {
    /// Roadmap rollup for every epic in `project`. Returns a 404 for an unknown
    /// project. Each epic carries its own metadata plus a subtree rollup.
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
                let percent = if r.total > 0 {
                    ((r.done as f64 / r.total as f64) * 100.0).round() as i64
                } else {
                    0
                };
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
                }));
            }

            Ok(json!({
                "project": project,
                "generated_at": iso(now),
                "epics": out,
            }))
        })
    }
}
