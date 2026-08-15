//! Event log reads. Writes happen via helpers::emit_event inside the same
//! transaction as each mutation.

use super::model::Event;
use super::sql::Value as SqlValue;
use super::Store;
use crate::error::ApiResult;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub project: Option<String>,
    pub ticket: Option<String>,
    /// Comma-split list of kinds; empty = all.
    pub kinds: Vec<String>,
    /// Restrict to these projects (token scoping). None = unrestricted.
    pub allowed_projects: Option<Vec<String>>,
}

impl Store {
    /// Events with seq > since, oldest first. Returns (events, cursor) where
    /// cursor is the last returned seq (or `since` when empty).
    pub fn events_since(
        &self,
        since: i64,
        filter: &EventFilter,
        limit: i64,
    ) -> ApiResult<(Vec<Event>, i64)> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT seq, ticket, project, actor, kind, payload, at FROM events WHERE seq > ?",
            );
            let mut params: Vec<SqlValue> = vec![SqlValue::Integer(since)];
            if let Some(p) = &filter.project {
                sql.push_str(" AND project = ?");
                params.push(SqlValue::Text(p.clone()));
            }
            if let Some(t) = &filter.ticket {
                sql.push_str(" AND ticket = ?");
                params.push(SqlValue::Text(t.clone()));
            }
            if !filter.kinds.is_empty() {
                sql.push_str(" AND kind IN (");
                for (i, k) in filter.kinds.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    params.push(SqlValue::Text(k.clone()));
                }
                sql.push(')');
            }
            if let Some(allowed) = &filter.allowed_projects {
                // An EMPTY allowlist means "no projects at all". SQLite accepts
                // `IN ()` and reads it as always-false; Postgres rejects it as a
                // syntax error, so a token minted with an empty project list
                // (reachable via `takomo token create --projects ""`, which the CLI
                // does not guard the way the REST path does) turned every list route
                // into a 500. `FALSE` is the same answer, spelled portably.
                if allowed.is_empty() {
                    sql.push_str(" AND FALSE");
                } else {
                    sql.push_str(" AND project IN (");
                    for (i, p) in allowed.iter().enumerate() {
                        if i > 0 {
                            sql.push(',');
                        }
                        sql.push('?');
                        params.push(SqlValue::Text(p.clone()));
                    }
                    sql.push(')');
                }
            }
            sql.push_str(" ORDER BY seq ASC LIMIT ?");
            params.push(SqlValue::Integer(limit));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(super::sql::params_from_iter(params), |r| {
                let payload_raw: String = r.get(5)?;
                Ok(Event {
                    seq: r.get(0)?,
                    ticket: r.get(1)?,
                    project: r.get(2)?,
                    actor: r.get(3)?,
                    kind: r.get(4)?,
                    payload: serde_json::from_str(&payload_raw).unwrap_or(Value::Null),
                    at: r.get(6)?,
                })
            })?;
            let events = rows.collect::<Result<Vec<_>, _>>()?;
            let cursor = events.last().map(|e| e.seq).unwrap_or(since);
            Ok((events, cursor))
        })
    }

    /// Recent events for one ticket (for GET /tickets/{id}?include=events).
    pub fn events_for_ticket(&self, ticket: &str, limit: i64) -> ApiResult<Vec<Event>> {
        let (events, _) = self.events_since(
            0,
            &EventFilter {
                ticket: Some(ticket.to_string()),
                ..Default::default()
            },
            limit,
        )?;
        Ok(events)
    }
}
