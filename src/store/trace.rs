//! The plan's history: what happened to a section, who did it, and when.
//!
//! A project has one plan, which the map and the document render two ways. This
//! is the record of what people did to it.
//!
//! **It is not the CRDT update log.** That is the mechanism that rebuilds the
//! text: written on every flush, merged, and rewritten wholesale by compaction.
//! Asking it "who changed 2.1, and when did anybody last agree with it?" is
//! asking a storage format a question about people. This table answers that one.
//!
//! Two rules keep it worth reading:
//!
//! - **Sparse.** An act somebody would name — written, renamed, moved,
//!   reviewed, accepted — never a keystroke. The event log already follows this
//!   rule for the same reason: one entry per keystroke-batch buries every entry
//!   worth having.
//! - **A person, not a capability.** Every entry records `users.id` where the
//!   credential was bound to one, because a scope is not an identity and an
//!   actor string does not survive somebody leaving.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};

use super::Store;

/// The most entries one read returns.
pub const MAX_TRACE_PAGE: i64 = 500;

/// The most of a section's prose one entry keeps.
pub const MAX_TRACE_TEXT: usize = 8_000;

/// What can happen to a section.
///
/// A closed set, and deliberately small: a vocabulary anybody can hold in their
/// head is what makes a history readable rather than a log.
pub const TRACE_KINDS: [&str; 9] = [
    "authored", "renamed", "edited", "moved", "pruned", "reviewed", "proposed", "accepted",
    "rejected",
];

/// The kinds a client may write directly.
///
/// The line is what the server can OBSERVE. It performs a move, a rename, a
/// prune and a proposal, so it records those and a caller cannot claim to have
/// done them. It cannot observe the other four: prose is edited over the sync
/// socket and never reaches it as a request, a review is somebody saying they
/// agree, and accepting or rejecting a proposal is the browser applying ops to
/// the replica. Those four are reported, and reporting is all anybody could do.
pub const CLIENT_TRACE_KINDS: [&str; 4] = ["edited", "reviewed", "accepted", "rejected"];

#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub id: String,
    pub node: Option<String>,
    pub kind: String,
    pub actor: String,
    pub user: Option<String>,
    pub note: Option<String>,
    /// What the section said at that moment. `None` when the act did not change
    /// the prose.
    pub text: Option<String>,
    pub at: i64,
}

impl TraceEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "node": self.node,
            "kind": self.kind,
            "actor": self.actor,
            "user": self.user,
            "note": self.note,
            "text": self.text,
            "at": iso(self.at),
        })
    }
}

/// One thing that happened, on its way in.
pub struct Record<'a> {
    pub project: &'a str,
    pub mindmap: &'a str,
    pub node: Option<&'a str>,
    pub kind: &'a str,
    pub actor: &'a str,
    pub user: Option<&'a str>,
    pub note: Option<&'a str>,
    /// The section's prose as it stands, for the acts that changed it.
    pub text: Option<&'a str>,
}

/// Write one entry on a transaction the caller already holds.
///
/// Taking the transaction rather than opening one is what lets a trace entry
/// land in the SAME commit as the thing it describes — a history that can
/// disagree with the state it records is worse than none.
pub(crate) fn record(tx: &Connection, entry: &Record) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO plan_trace (id, project, mindmap, node, kind, actor, \"user\", note, text, at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            crate::ids::trace_id(),
            entry.project,
            entry.mindmap,
            entry.node,
            entry.kind,
            entry.actor,
            entry.user,
            entry.note,
            // Bounded: a diff is worth keeping, an unbounded copy of every
            // revision is not.
            entry.text.map(|t| {
                t.chars().take(MAX_TRACE_TEXT).collect::<String>()
            }),
            now_ms(),
        ],
    )?;
    Ok(())
}

impl Store {
    /// Record one act against the plan.
    pub fn record_trace(&self, entry: &Record) -> ApiResult<()> {
        if !TRACE_KINDS.contains(&entry.kind) {
            return Err(ApiError::validation(
                "validation.trace_kind",
                format!(
                    "Unknown kind '{}'. Use one of: {}.",
                    entry.kind,
                    TRACE_KINDS.join(", ")
                ),
            ));
        }
        self.with_tx(|tx| record(tx, entry))
    }

    /// A plan's history, newest first, optionally for one section.
    pub fn plan_trace(
        &self,
        mindmap: &str,
        node: Option<&str>,
        limit: i64,
    ) -> ApiResult<(Vec<TraceEntry>, i64)> {
        self.with_conn(|conn| {
            let node_owned = node.map(str::to_string);
            let (where_sql, args): (&str, Vec<&dyn rusqlite::ToSql>) = match &node_owned {
                Some(node) => ("mindmap = ?1 AND node = ?2", vec![&mindmap, node]),
                None => ("mindmap = ?1", vec![&mindmap]),
            };

            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM plan_trace WHERE {where_sql}"),
                args.as_slice(),
                |r| r.get(0),
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT id, node, kind, actor, \"user\", note, text, at FROM plan_trace \
                 WHERE {where_sql} ORDER BY at DESC, id DESC LIMIT {limit}"
            ))?;
            let rows = stmt.query_map(args.as_slice(), |r| {
                Ok(TraceEntry {
                    id: r.get(0)?,
                    node: r.get(1)?,
                    kind: r.get(2)?,
                    actor: r.get(3)?,
                    user: r.get(4)?,
                    note: r.get(5)?,
                    text: r.get(6)?,
                    at: r.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok((out, total))
        })
    }

    /// When each section was last edited and last confirmed.
    ///
    /// This is what the trust reading is made of, and why it is a reading rather
    /// than a stored flag: a section confirmed BEFORE its last edit is not
    /// confirmed any more, and no boolean can express that. One grouped query
    /// rather than one per node, because a map is drawn all at once.
    pub fn plan_standing(&self, mindmap: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT node, \
                    MAX(CASE WHEN kind IN ('authored','renamed','edited','accepted') THEN at END), \
                    MAX(CASE WHEN kind = 'reviewed' THEN at END) \
                 FROM plan_trace WHERE mindmap = ?1 AND node IS NOT NULL GROUP BY node",
            )?;
            let rows = stmt.query_map(params![mindmap], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?;
            let mut out = serde_json::Map::new();
            for row in rows {
                let (node, changed, reviewed) = row?;
                out.insert(
                    node,
                    json!({
                        "changed_at": changed.map(iso),
                        "reviewed_at": reviewed.map(iso),
                        // The whole point: confirmed only if somebody agreed
                        // AFTER the last time it changed.
                        "confirmed": match (changed, reviewed) {
                            (Some(changed), Some(reviewed)) => reviewed >= changed,
                            (None, Some(_)) => true,
                            _ => false,
                        },
                    }),
                );
            }
            Ok(Value::Object(out))
        })
    }
}
