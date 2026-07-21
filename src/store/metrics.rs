//! Store-level metrics: ticket counts by state/category per project, open
//! claims, and the total event count. Read-only observability over the same
//! SQLite the API serves. Honors token project scoping so a scoped caller only
//! sees the projects it may read.

use super::Store;
use crate::error::ApiResult;
use crate::ids::{iso, now_ms};
use rusqlite::types::Value as SqlValue;
use serde_json::{json, Map, Value};

impl Store {
    /// Aggregate counts for `GET /v1/metrics`. `allowed_projects` (None =
    /// unrestricted) scopes every count to the caller's readable projects.
    pub fn metrics(&self, allowed_projects: Option<&[String]>) -> ApiResult<Value> {
        let now = now_ms();
        self.with_conn(|conn| {
            // project filter clause reused across queries.
            let (proj_clause, proj_params): (String, Vec<SqlValue>) = match allowed_projects {
                None => (String::new(), Vec::new()),
                Some(list) => {
                    let mut clause = String::from(" AND t.project IN (");
                    let mut ps = Vec::new();
                    for (i, p) in list.iter().enumerate() {
                        if i > 0 {
                            clause.push(',');
                        }
                        clause.push('?');
                        ps.push(SqlValue::Text(p.clone()));
                    }
                    clause.push(')');
                    (clause, ps)
                }
            };

            // Per project + state (+ category) ticket counts.
            let sql = format!(
                "SELECT t.project, t.state, \
                 COALESCE((SELECT ws.category FROM workflow_states ws WHERE ws.project = t.project AND ws.state = t.state), '') AS category, \
                 COUNT(*) AS n \
                 FROM tickets t WHERE t.archived_at IS NULL{proj_clause} GROUP BY t.project, t.state"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(proj_params.clone()), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut projects: Map<String, Value> = Map::new();
            let mut total_tickets = 0i64;
            for (project, state, category, n) in rows {
                total_tickets += n;
                let entry = projects.entry(project).or_insert_with(|| {
                    json!({
                        "total": 0,
                        "open_claims": 0,
                        "by_state": {},
                        "by_category": {},
                    })
                });
                let obj = entry.as_object_mut().expect("project metrics object");
                let total = obj["total"].as_i64().unwrap_or(0) + n;
                obj["total"] = json!(total);
                obj["by_state"]
                    .as_object_mut()
                    .unwrap()
                    .insert(state, json!(n));
                if !category.is_empty() {
                    let by_cat = obj["by_category"].as_object_mut().unwrap();
                    let prev = by_cat.get(&category).and_then(Value::as_i64).unwrap_or(0);
                    by_cat.insert(category, json!(prev + n));
                }
            }

            // Open claims per project.
            let sql = format!(
                "SELECT t.project, COUNT(*) FROM tickets t \
                 WHERE t.claim_holder IS NOT NULL AND t.claim_expires_at > ?1 AND t.archived_at IS NULL{proj_clause} \
                 GROUP BY t.project"
            );
            let mut claim_params: Vec<SqlValue> = vec![SqlValue::Integer(now)];
            claim_params.extend(proj_params.clone());
            let mut stmt = conn.prepare(&sql)?;
            let claim_rows = stmt
                .query_map(rusqlite::params_from_iter(claim_params), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut total_open_claims = 0i64;
            for (project, n) in claim_rows {
                total_open_claims += n;
                let entry = projects.entry(project).or_insert_with(|| {
                    json!({
                        "total": 0,
                        "open_claims": 0,
                        "by_state": {},
                        "by_category": {},
                    })
                });
                entry.as_object_mut().unwrap()["open_claims"] = json!(n);
            }

            // Event total (scoped to readable projects when the token is scoped;
            // events always carry a project so the IN-filter is exact).
            let events: i64 = match allowed_projects {
                None => conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?,
                Some([]) => 0,
                Some(list) => {
                    let mut sql = String::from("SELECT COUNT(*) FROM events WHERE project IN (");
                    let mut ps: Vec<SqlValue> = Vec::new();
                    for (i, p) in list.iter().enumerate() {
                        if i > 0 {
                            sql.push(',');
                        }
                        sql.push('?');
                        ps.push(SqlValue::Text(p.clone()));
                    }
                    sql.push(')');
                    conn.query_row(&sql, rusqlite::params_from_iter(ps), |r| r.get(0))?
                }
            };

            Ok(json!({
                "generated_at": iso(now),
                "totals": {
                    "projects": projects.len(),
                    "tickets": total_tickets,
                    "open_claims": total_open_claims,
                    "events": events,
                },
                "projects": Value::Object(projects),
            }))
        })
    }
}
