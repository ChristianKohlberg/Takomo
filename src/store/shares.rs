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
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, share_id, share_token_plaintext, token_hash};
use axum::http::StatusCode;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{params, OptionalExtension, Row};

/// Default share lifetime when the caller omits `ttl_seconds`: 24 hours.
pub const DEFAULT_SHARE_TTL_SECONDS: i64 = 86_400;
/// Hard cap on share lifetime: 30 days.
pub const MAX_SHARE_TTL_SECONDS: i64 = 30 * 86_400;

/// How many tickets one `GET /v1/shares/self/tickets` returns, and the ceiling an
/// explicit `limit` may ask for. One number for both, matching the `limit=200` the
/// board already uses to page `/v1/tickets`: the scope of a share is not something
/// the viewer chose, so there is no reason to hand out a smaller default page and
/// make every client page twice.
///
/// The point is that the query is **bounded at all** (takomo-fgca / takomo-vlpm).
/// A share link is designed to be pasted around, so an unbounded full-project scan
/// behind it is work an unauthenticated-in-practice reader can trigger at will. A
/// page never truncates silently: the response carries `next_cursor` and, when
/// there is more, a `warning` naming the exact call that fetches it.
pub const SHARE_TICKETS_PAGE: i64 = 200;

/// The kind of scope a share grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareKind {
    /// All tickets in a project.
    Project,
    /// A root ticket plus its full recursive descendant subtree.
    Subtree,
}

impl ShareKind {
    /// The canonical stored/echoed spelling — the ONLY thing ever written to the
    /// `shares.kind` column, and the only thing [`ShareKind::from_stored`] reads
    /// back.
    pub fn as_str(&self) -> &'static str {
        match self {
            ShareKind::Project => "project",
            ShareKind::Subtree => "subtree",
        }
    }

    /// Parse the `kind` field of a `POST /v1/shares` body — the *request*
    /// vocabulary, which is deliberately wider than the stored one: `epic` is the
    /// caller-facing spelling for a subtree share (an epic is the common subtree
    /// root) and `subtree` is accepted as its explicit synonym. Both normalize to
    /// [`ShareKind::Subtree`], so `epic` never reaches the database.
    ///
    /// Named apart from [`ShareKind::from_stored`] on purpose: mixing the two
    /// vocabularies up is exactly how a subtree share would turn into a
    /// whole-project one.
    pub fn parse_request(raw: &str) -> Option<ShareKind> {
        match raw {
            "project" => Some(ShareKind::Project),
            "epic" | "subtree" => Some(ShareKind::Subtree),
            _ => None,
        }
    }

    /// Read a `shares.kind` column value back. Strictly the inverse of
    /// [`ShareKind::as_str`]: request-only synonyms such as `epic` are **not**
    /// accepted here, and anything unrecognised is `None` so the read fails
    /// closed rather than resolving to a scope the share may not cover.
    fn from_stored(raw: &str) -> Option<ShareKind> {
        match raw {
            "project" => Some(ShareKind::Project),
            "subtree" => Some(ShareKind::Subtree),
            _ => None,
        }
    }
}

/// A `shares.kind` value that is not a [`ShareKind`]. Carried as the source of a
/// `rusqlite` conversion failure so a row that cannot be interpreted produces an
/// error instead of a `ShareRow` — there is deliberately no fallback variant to
/// resolve to, because every candidate fallback is a *wider* scope than the
/// share was minted for.
#[derive(Debug)]
struct UnknownShareKind(String);

impl std::fmt::Display for UnknownShareKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "share scope kind '{}' is not 'project' or 'subtree'",
            self.0
        )
    }
}

impl std::error::Error for UnknownShareKind {}

/// The column type *is* the enum: `row.get("kind")` yields a `ShareKind` or
/// fails. That is what keeps the scope decision out of `&str` comparisons, where
/// an unmatched string silently means "the other branch".
impl FromSql for ShareKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let raw = value.as_str()?;
        ShareKind::from_stored(raw)
            .ok_or_else(|| FromSqlError::Other(Box::new(UnknownShareKind(raw.to_string()))))
    }
}

/// Map a `shares` row read failure. Everything but an uninterpretable `kind`
/// takes the generic database-error mapping (opaque 500, detail logged); that one
/// case gets its own teaching error, because it is the one an operator can act
/// on — and because "this share is refused" must not look like a transient blip.
fn share_read_err(e: rusqlite::Error) -> ApiError {
    if let rusqlite::Error::FromSqlConversionFailure(_, _, src) = &e {
        if let Some(bad) = src.downcast_ref::<UnknownShareKind>() {
            eprintln!("share read refused: {bad}");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "share.kind_unrecognized",
                "This share's stored scope kind is neither 'project' nor 'subtree', so it cannot \
                 be served: the share is refused rather than widened to a scope it may not cover. \
                 Nothing is readable through this link until it is replaced.",
            )
            .remedy(
                "Such a share cannot be repaired over HTTP — every endpoint that reads it, \
                 including revoke, refuses it. Correct or delete the row in the store (shell \
                 access is the root of trust here), then mint a replacement with \
                 POST /v1/shares {kind: \"project\"|\"epic\", ref}.",
            );
        }
    }
    ApiError::from(e)
}

const SHARE_COLS: &str =
    "id, kind, \"ref\" AS ref_id, project, expires_at, created_by, created_at, revoked_at, last_used_at";

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
        last_used_at: row.get("last_used_at")?,
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
            kind,
            ref_id: ref_id.to_string(),
            project: project.to_string(),
            expires_at,
            created_by: created_by.to_string(),
            created_at: now,
            revoked_at: None,
            last_used_at: None,
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
                .optional()
                .map_err(share_read_err)?;
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
            rows.map_err(share_read_err)
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
                .optional()
                .map_err(share_read_err)?;
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

    /// Update last_used_at (called at most ~once a minute per share).
    pub fn touch_share(&self, id: &str) -> ApiResult<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE shares SET last_used_at = ?2 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            Ok(())
        })
    }

    /// One page of the tickets in a share's scope, oldest first, as
    /// `(tickets, next_cursor)`. For [`ShareKind::Project`]: every ticket in
    /// `project`. For [`ShareKind::Subtree`]: the `ref_id` root ticket plus every
    /// recursive descendant (via `parent`). Archived tickets are excluded unless
    /// `include_archived` is set. Each ticket carries its `blocked_by` edges.
    ///
    /// **Bounded**, which is the whole point of the signature (takomo-fgca /
    /// takomo-vlpm): this used to return the entire project with no `LIMIT`, and it
    /// runs behind a credential built to be pasted into a chat. `limit` is the
    /// caller's page size (the route clamps it to [`SHARE_TICKETS_PAGE`]) and
    /// `next_cursor` is `Some` exactly when more rows exist, so the route can say so
    /// instead of truncating quietly.
    ///
    /// Paged on `rowid`, and ordered by it too — a cursor needs a total order over
    /// a single column, and rowid *is* the insertion order that `created_at`
    /// approximates. `LIMIT + 1` rows are read to learn whether a next page exists
    /// without a second `COUNT(*)` over the same scan, exactly as
    /// [`Store::list_tickets`] does.
    ///
    /// `kind` is the enum, not a string, so the scope is chosen by an exhaustive
    /// `match`: there is no "everything else" arm that a mis-spelled kind could
    /// fall into, and a third variant would fail to compile here rather than
    /// quietly inherit the widest query.
    pub fn share_tickets(
        &self,
        kind: ShareKind,
        ref_id: &str,
        project: &str,
        include_archived: bool,
        cursor: Option<i64>,
        limit: i64,
    ) -> ApiResult<(Vec<Ticket>, Option<String>)> {
        self.with_conn(|conn| {
            let archived_clause = if include_archived {
                ""
            } else {
                " AND t.archived_at IS NULL"
            };
            // `?2` is the cursor: rows strictly after it, or all rows when the
            // caller is on the first page. Bound as a parameter rather than spliced
            // into the SQL so the statement text (and SQLite's plan cache entry) is
            // the same for every page.
            let after = cursor.unwrap_or(0);
            let (sql, bind) = match kind {
                ShareKind::Subtree => (
                    format!(
                        r#"
                    WITH RECURSIVE sub(id) AS (
                        SELECT ?1
                        UNION
                        SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
                    )
                    SELECT {TICKET_COLS}, t.rowid AS rid FROM tickets t JOIN sub ON t.id = sub.id
                    WHERE t.rowid > ?2{archived_clause}
                    ORDER BY t.rowid ASC LIMIT ?3
                    "#
                    ),
                    ref_id,
                ),
                ShareKind::Project => (
                    format!(
                        "SELECT {TICKET_COLS}, t.rowid AS rid FROM tickets t \
                         WHERE t.project = ?1 AND t.rowid > ?2{archived_clause} \
                         ORDER BY t.rowid ASC LIMIT ?3"
                    ),
                    project,
                ),
            };
            let mut stmt = conn.prepare(&sql)?;
            let mut rows: Vec<(Ticket, i64)> = stmt
                .query_map(params![bind, after, limit + 1], |r| {
                    Ok((row_to_ticket(r)?, r.get::<_, i64>("rid")?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = rows.len() as i64 > limit;
            rows.truncate(limit as usize);
            let next_cursor = if has_more {
                rows.last().map(|(_, rid)| rid.to_string())
            } else {
                None
            };
            let mut tickets = Vec::with_capacity(rows.len());
            for (mut t, _) in rows {
                load_blocked_by(conn, &mut t)?;
                tickets.push(t);
            }
            Ok((tickets, next_cursor))
        })
    }

    /// True when `ticket_id` is inside the share's scope. A project share covers
    /// every ticket whose project matches; a subtree share covers the root and
    /// its recursive descendants. Used to bound the per-ticket detail endpoint.
    ///
    /// Takes the enum for the same reason [`Store::share_tickets`] does: the
    /// membership test must be the share's own scope, never a default one.
    pub fn ticket_in_share_scope(
        &self,
        kind: ShareKind,
        ref_id: &str,
        project: &str,
        ticket_id: &str,
    ) -> ApiResult<bool> {
        self.with_conn(|conn| match kind {
            ShareKind::Subtree => {
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
            }
            ShareKind::Project => {
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
