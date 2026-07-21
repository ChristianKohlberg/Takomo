//! Shareable read-only web links. A share mints a bearer token (hashed at rest,
//! plaintext shown once — exactly like a normal token) that grants a scoped,
//! read-only, auto-expiring view of the board. Two kinds:
//!
//! - `project`  — every ticket in the referenced project.
//! - `subtree`  — the referenced root ticket plus its FULL recursive descendant
//!   subtree, walked via `parent` with the same recursive-CTE the roadmap uses.
//!
//! A share token is validated on a distinct auth path (see `auth::share_auth`)
//! and can reach ONLY the `/v1/shares/self*` read endpoints; it can neither read
//! arbitrary projects nor write anything.

use super::helpers::{load_blocked_by, row_to_ticket, TICKET_COLS};
use super::model::{ShareRow, Ticket};
use super::Store;
use crate::error::ApiResult;
use crate::ids::{now_ms, share_id, share_token_plaintext, token_hash};
use rusqlite::{params, OptionalExtension, Row};

/// Default share lifetime when the caller omits `ttl_seconds`: 24 hours.
pub const DEFAULT_SHARE_TTL_SECONDS: i64 = 86_400;
/// Hard cap on share lifetime: 30 days.
pub const MAX_SHARE_TTL_SECONDS: i64 = 30 * 86_400;

/// The kind of scope a share grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareKind {
    /// All tickets in a project.
    Project,
    /// A root ticket plus its full recursive descendant subtree.
    Subtree,
}

impl ShareKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShareKind::Project => "project",
            ShareKind::Subtree => "subtree",
        }
    }

    /// Parse the POST body `kind`. `epic` is the caller-facing spelling for a
    /// subtree share (an epic is the common subtree root); `subtree` is accepted
    /// as its explicit synonym.
    pub fn parse(raw: &str) -> Option<ShareKind> {
        match raw {
            "project" => Some(ShareKind::Project),
            "epic" | "subtree" => Some(ShareKind::Subtree),
            _ => None,
        }
    }
}

const SHARE_COLS: &str =
    "id, kind, \"ref\" AS ref_id, project, expires_at, created_by, created_at, revoked_at";

fn row_to_share(row: &Row) -> rusqlite::Result<ShareRow> {
    Ok(ShareRow {
        id: row.get("id")?,
        kind: row.get("kind")?,
        ref_id: row.get("ref_id")?,
        project: row.get("project")?,
        expires_at: row.get("expires_at")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        revoked_at: row.get("revoked_at")?,
    })
}

impl Store {
    /// Mint a share. `project` is the already-resolved and validated scope (the
    /// project id for a project share, or the root ticket's project for a
    /// subtree share). Returns (row, plaintext) — the plaintext is shown once.
    pub fn create_share(
        &self,
        kind: ShareKind,
        ref_id: &str,
        project: &str,
        expires_at: i64,
        created_by: &str,
    ) -> ApiResult<(ShareRow, String)> {
        let plaintext = share_token_plaintext();
        let hash = token_hash(&plaintext);
        let id = share_id();
        let now = now_ms();
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO shares (id, token_hash, kind, \"ref\", project, expires_at, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, hash, kind.as_str(), ref_id, project, expires_at, created_by, now],
            )?;
            Ok(())
        })?;
        let row = ShareRow {
            id,
            kind: kind.as_str().to_string(),
            ref_id: ref_id.to_string(),
            project: project.to_string(),
            expires_at,
            created_by: created_by.to_string(),
            created_at: now,
            revoked_at: None,
        };
        Ok((row, plaintext))
    }

    /// Look up a share by its token's SHA-256 hash. Returns the row regardless of
    /// expiry/revocation — the caller (share auth) decides how to respond so it
    /// can distinguish an unknown token (401) from an expired/revoked one (410).
    pub fn lookup_share_by_hash(&self, hash: &str) -> ApiResult<Option<ShareRow>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {SHARE_COLS} FROM shares WHERE token_hash = ?1"),
                    params![hash],
                    row_to_share,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// List share metadata. `created_by` filters to one creator (None = all).
    /// Never returns the plaintext or hash.
    pub fn list_shares(&self, created_by: Option<&str>) -> ApiResult<Vec<ShareRow>> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {SHARE_COLS} FROM shares");
            if created_by.is_some() {
                sql.push_str(" WHERE created_by = ?1");
            }
            sql.push_str(" ORDER BY created_at DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows: Result<Vec<_>, _> = match created_by {
                Some(c) => stmt.query_map(params![c], row_to_share)?.collect(),
                None => stmt.query_map([], row_to_share)?.collect(),
            };
            Ok(rows?)
        })
    }

    /// Load one share by its public id (for the revoke authorization check).
    pub fn get_share(&self, id: &str) -> ApiResult<Option<ShareRow>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {SHARE_COLS} FROM shares WHERE id = ?1"),
                    params![id],
                    row_to_share,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Revoke a share by its id. Returns false if no such (not-yet-revoked) share.
    pub fn revoke_share(&self, id: &str) -> ApiResult<bool> {
        self.with_tx(|tx| {
            let n = tx.execute(
                "UPDATE shares SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now_ms()],
            )?;
            Ok(n > 0)
        })
    }

    /// The tickets in a share's scope. For a project share (`kind` = "project"):
    /// every ticket in `project`. For a subtree share (`kind` = "subtree"): the
    /// `ref_id` root ticket plus every recursive descendant (via `parent`).
    /// Archived tickets are excluded unless `include_archived` is set. Each
    /// ticket carries its `blocked_by` edges.
    pub fn share_tickets(
        &self,
        kind: &str,
        ref_id: &str,
        project: &str,
        include_archived: bool,
    ) -> ApiResult<Vec<Ticket>> {
        let is_subtree = kind == "subtree";
        self.with_conn(|conn| {
            let archived_clause = if include_archived {
                ""
            } else {
                " AND t.archived_at IS NULL"
            };
            let sql = if is_subtree {
                format!(
                    r#"
                    WITH RECURSIVE sub(id) AS (
                        SELECT ?1
                        UNION
                        SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
                    )
                    SELECT {TICKET_COLS} FROM tickets t JOIN sub ON t.id = sub.id
                    WHERE 1=1{archived_clause}
                    ORDER BY t.created_at ASC, t.rowid ASC
                    "#
                )
            } else {
                format!(
                    "SELECT {TICKET_COLS} FROM tickets t WHERE t.project = ?1{archived_clause} \
                     ORDER BY t.created_at ASC, t.rowid ASC"
                )
            };
            let bind = if is_subtree { ref_id } else { project };
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt
                .query_map(params![bind], row_to_ticket)?
                .collect::<Result<Vec<_>, _>>()?;
            for t in &mut rows {
                load_blocked_by(conn, t)?;
            }
            Ok(rows)
        })
    }

    /// True when `ticket_id` is inside the share's scope. A project share covers
    /// every ticket whose project matches; a subtree share covers the root and
    /// its recursive descendants. Used to bound the per-ticket detail endpoint.
    pub fn ticket_in_share_scope(
        &self,
        kind: &str,
        ref_id: &str,
        project: &str,
        ticket_id: &str,
    ) -> ApiResult<bool> {
        self.with_conn(|conn| {
            if kind == "subtree" {
                let found: Option<i64> = conn
                    .query_row(
                        r#"
                        WITH RECURSIVE sub(id) AS (
                            SELECT ?1
                            UNION
                            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
                        )
                        SELECT 1 FROM sub WHERE id = ?2 LIMIT 1
                        "#,
                        params![ref_id, ticket_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(found.is_some())
            } else {
                let found: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM tickets WHERE id = ?1 AND project = ?2 LIMIT 1",
                        params![ticket_id, project],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(found.is_some())
            }
        })
    }
}
