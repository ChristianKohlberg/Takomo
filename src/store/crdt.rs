//! One collaborative object, whatever kind it is.
//!
//! `/documents` shipped first, so the update log, the sync ticket and the room
//! layer were all written in terms of documents. A mindmap needs exactly the
//! same three things, and the honest way to give it them is to widen what
//! exists rather than to stand up a second copy that drifts.
//!
//! **The id carries the kind.** `src/ids.rs` mints `doc-…` and `mm-…`, so a
//! bare object id says which table it belongs to and no composite key is
//! needed. That is not a convenience — it is a requirement. `y-websocket`
//! composes its URL as `serverUrl + "/" + room + "?" + params`, so the room has
//! to survive as a single path segment; a `kind:id` room would come back
//! mangled, exactly as `/v1/documents/{id}/sync` did before it was reverted.
//!
//! What stays kind-specific and why:
//!
//! - **Where the object lives.** [`CollabKind::resolve`] reads `documents` or
//!   `mindmaps` for the project and the archive state.
//! - **Error codes.** A document keeps every code it already emits, because
//!   those are published in `spec/openapi.yaml` and somebody's client reads
//!   them. A mindmap gets its own parallel set. They are written out as literal
//!   match arms rather than composed with `format!`, because the error-code scan
//!   in `tests/api.rs` reads the source text and a composed code would be
//!   invisible to it.
//!
//! Everything else — the log, compaction, the ticket, the room — is shared.

use axum::http::StatusCode;
use rusqlite::{params, Connection, OptionalExtension};

use super::helpers::ensure_project_writable;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;

use super::Store;

/// The largest single flush, in decoded bytes.
pub const MAX_UPDATE_BYTES: usize = 1024 * 1024;

/// The largest an object's whole log may grow to. This bounds what a join has
/// to allocate when it replays, which is why it is a store-side cap and not a
/// UI one.
pub const MAX_OBJECT_BYTES: i64 = 32 * 1024 * 1024;

/// Rows in the log that trigger a compaction on the next flush.
pub const COMPACT_AFTER_UPDATES: i64 = 256;

/// How long a sync ticket is good for.
pub const SESSION_TTL_SECONDS: i64 = 12 * 3600;

/// A spent ticket is kept this long past expiry before the sweeper takes it, so
/// a client that reconnects late gets `session_expired` rather than the
/// indistinguishable `session_invalid`.
const SESSION_GRACE_MS: i64 = 24 * 3600 * 1000;

/// Which kind of thing is being edited together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabKind {
    Document,
    Mindmap,
}

impl CollabKind {
    /// Read the kind off the id, which is where it already lives.
    pub fn from_id(id: &str) -> Option<Self> {
        if id.starts_with("doc-") {
            Some(CollabKind::Document)
        } else if id.starts_with("mm-") {
            Some(CollabKind::Mindmap)
        } else {
            None
        }
    }

    /// The value stored in `object_kind`, and the noun in a message.
    pub fn as_str(&self) -> &'static str {
        match self {
            CollabKind::Document => "document",
            CollabKind::Mindmap => "mindmap",
        }
    }

    /// The route that mints a ticket for this kind.
    fn session_route(&self) -> &'static str {
        match self {
            CollabKind::Document => "POST /v1/documents/{id}/session",
            CollabKind::Mindmap => "POST /v1/mindmaps/{id}/session",
        }
    }
}

/// An object that can be edited together, and what a writer needs to know
/// about it.
#[derive(Debug, Clone)]
pub struct CollabObject {
    pub kind: CollabKind,
    pub id: String,
    pub project: String,
    pub archived: bool,
}

/// An id whose prefix names no collaborative kind.
///
/// 404 rather than 400: from the caller's side an id that reaches nothing is an
/// id that reaches nothing, and saying "that prefix is not one of mine" leaks
/// the id scheme without helping.
fn unknown_kind(id: &str) -> ApiError {
    ApiError::not_found("object", id)
}

/// Look up the object behind an id, whichever table it lives in.
pub(crate) fn resolve(conn: &Connection, id: &str) -> ApiResult<CollabObject> {
    let kind = CollabKind::from_id(id).ok_or_else(|| unknown_kind(id))?;
    match kind {
        CollabKind::Document => {
            let row: Option<(String, Option<i64>)> = conn
                .query_row(
                    "SELECT project, archived_at FROM documents WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (project, archived_at) = row.ok_or_else(|| ApiError::not_found("document", id))?;
            Ok(CollabObject {
                kind,
                id: id.to_string(),
                project,
                archived: archived_at.is_some(),
            })
        }
        CollabKind::Mindmap => {
            // A mindmap has no archive gate of its own. Throwing one away is
            // ordinary — that is what makes it safe to start one early — so
            // there is no state between "here" and "gone" to freeze.
            let project: Option<String> = conn
                .query_row(
                    "SELECT project FROM mindmaps WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            let project = project.ok_or_else(|| ApiError::not_found("mindmap", id))?;
            Ok(CollabObject {
                kind,
                id: id.to_string(),
                project,
                archived: false,
            })
        }
    }
}

/// The object is archived, so nothing may be written to it.
fn archived_error(object: &CollabObject) -> ApiError {
    match object.kind {
        CollabKind::Document => ApiError::conflict(
            "conflict.document_archived",
            format!(
                "Document '{}' is archived; it cannot be written to.",
                object.id
            ),
        )
        .remedy("Restore it with POST /v1/documents/{id}/unarchive, then try again.".to_string()),
        // Unreachable today — `resolve` never reports a mindmap as archived —
        // but written out rather than left to `unreachable!()`, so adding an
        // archive gate to mindmaps is a schema change and not also a panic.
        CollabKind::Mindmap => ApiError::conflict(
            "conflict.mindmap_archived",
            format!(
                "Mindmap '{}' is archived; it cannot be written to.",
                object.id
            ),
        )
        .remedy("Restore it, then try again.".to_string()),
    }
}

impl Store {
    /// Resolve an id to its object, or 404.
    pub fn collab_object(&self, id: &str) -> ApiResult<CollabObject> {
        self.with_conn(|conn| resolve(conn, id))
    }

    /// Refuse a write the object cannot take: gone, archived, or under an
    /// archived project.
    pub fn ensure_collab_writable(&self, id: &str) -> ApiResult<()> {
        self.with_conn(|conn| {
            let object = resolve(conn, id)?;
            ensure_project_writable(conn, &object.project)?;
            if object.archived {
                return Err(archived_error(&object));
            }
            Ok(())
        })
    }

    /// Every update ever written for this object, oldest first.
    ///
    /// Replayed in `seq` order to rebuild the replica when the first peer
    /// joins. Resolving the object first means an unknown id 404s here rather
    /// than opening an empty room that looks like a valid, empty object.
    pub fn load_collab_updates(&self, id: &str) -> ApiResult<Vec<Vec<u8>>> {
        self.with_conn(|conn| {
            resolve(conn, id)?;
            let mut stmt = conn
                .prepare("SELECT blob FROM crdt_updates WHERE object_id = ?1 ORDER BY seq ASC")?;
            let rows = stmt.query_map(params![id], |r| r.get::<_, Vec<u8>>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Append one merged flush and report how many rows the log now holds.
    ///
    /// Deliberately emits no event. These arrive every couple of seconds while
    /// somebody is typing, and one event per flush would bury every other event
    /// in the project under one person's keyboard.
    pub fn append_collab_update(&self, id: &str, blob: &[u8], actor: &str) -> ApiResult<i64> {
        let kind = CollabKind::from_id(id).ok_or_else(|| unknown_kind(id))?;
        if blob.is_empty() {
            return Err(update_empty(kind));
        }
        if blob.len() > MAX_UPDATE_BYTES {
            return Err(update_too_large(kind, blob.len()));
        }

        let now = now_ms();
        let bytes = blob.len() as i64;
        self.with_tx(|tx| {
            let object = resolve(tx, id)?;
            ensure_project_writable(tx, &object.project)?;
            if object.archived {
                return Err(archived_error(&object));
            }

            let held: i64 = tx.query_row(
                "SELECT COALESCE(SUM(bytes), 0) FROM crdt_updates WHERE object_id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            if held + bytes > MAX_OBJECT_BYTES {
                return Err(object_too_large(object.kind, id, held));
            }

            tx.execute(
                "INSERT INTO crdt_updates (object_kind, object_id, blob, bytes, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![object.kind.as_str(), id, blob, bytes, actor, now],
            )?;
            touch(tx, &object, now)?;

            let rows: i64 = tx.query_row(
                "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            Ok(rows)
        })
    }

    /// Replace the whole log with one update carrying the same state.
    ///
    /// No snapshot table is involved, and that is a property of Yjs rather than
    /// a shortcut: a document's entire state serialises as an ordinary update,
    /// so compaction is the same shape and the same format as an increment. One
    /// transaction, so no reader ever sees the empty moment between.
    pub fn compact_collab(&self, id: &str, state: &[u8], actor: &str) -> ApiResult<()> {
        let kind = CollabKind::from_id(id).ok_or_else(|| unknown_kind(id))?;
        if state.is_empty() {
            return Err(update_empty(kind));
        }
        let now = now_ms();
        let bytes = state.len() as i64;
        self.with_tx(|tx| {
            let object = resolve(tx, id)?;
            ensure_project_writable(tx, &object.project)?;
            // The same gate the append path applies. Compaction rewrites the
            // whole log, so letting it through on an archived object would be
            // the one write that gets past a freeze.
            if object.archived {
                return Err(archived_error(&object));
            }
            tx.execute("DELETE FROM crdt_updates WHERE object_id = ?1", params![id])?;
            tx.execute(
                "INSERT INTO crdt_updates (object_kind, object_id, blob, bytes, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![object.kind.as_str(), id, state, bytes, actor, now],
            )?;
            Ok(())
        })
    }

    /// Drop an object's log and tickets.
    ///
    /// `crdt_updates.object_id` carries no foreign key — it points at one of two
    /// tables — so the cascade the FK used to provide is this call instead, made
    /// from each kind's own delete path.
    pub(crate) fn purge_collab(tx: &Connection, id: &str) -> ApiResult<()> {
        tx.execute("DELETE FROM crdt_updates WHERE object_id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM crdt_sessions WHERE object_id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

/// Mark the owning row as touched, so a list ordered by `updated_at` reflects
/// that somebody is working in there.
fn touch(tx: &Connection, object: &CollabObject, now: i64) -> ApiResult<()> {
    match object.kind {
        CollabKind::Document => {
            tx.execute(
                "UPDATE documents SET updated_at = ?2 WHERE id = ?1",
                params![object.id, now],
            )?;
        }
        CollabKind::Mindmap => {
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![object.id, now],
            )?;
        }
    }
    Ok(())
}

/// A short-lived credential for one object's sync socket.
///
/// The reason it exists is narrow and unchanged: a browser `WebSocket` cannot
/// set an `Authorization` header, so the credential has to ride the handshake,
/// and a real `tk_` token in a query string would land in every access log on
/// the path. This one reaches exactly one object, expires, and is revocable.
///
/// Widened from documents to any collaborative object rather than given a
/// sixth token prefix of its own: it is the same credential doing the same job,
/// and a new prefix would imply a new auth path when there is not one.
///
/// **`revoked_at` is read but nothing writes it yet.** The column and the check
/// are here so revocation is a row update when it is built; until then a
/// ticket's only bound is [`SESSION_TTL_SECONDS`], and revoking the `tk_` token
/// it came from does not reach it. Said plainly here rather than left implied by
/// the column's existence.
#[derive(Debug, Clone)]
pub struct CollabSession {
    pub id: String,
    pub kind: CollabKind,
    pub object: String,
    pub project: String,
    pub actor: String,
    pub user: Option<String>,
    pub display: String,
    pub can_write: bool,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

impl Store {
    /// Mint a ticket for one object. The plaintext is returned once and never
    /// stored; only its hash is kept.
    pub fn create_collab_session(
        &self,
        object_id: &str,
        actor: &str,
        display: &str,
        user: Option<&str>,
        can_write: bool,
        expires_at: i64,
    ) -> ApiResult<(CollabSession, String)> {
        let plaintext = crate::ids::doc_session_token_plaintext();
        let hash = crate::ids::token_hash(&plaintext);
        let id = crate::ids::doc_session_id();
        let now = now_ms();

        let session = self.with_tx(|tx| {
            let object = resolve(tx, object_id)?;
            ensure_project_writable(tx, &object.project)?;
            tx.execute(
                "INSERT INTO crdt_sessions (id, token_hash, object_kind, object_id, project, \
                 actor, \"user\", display, can_write, expires_at, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    hash,
                    object.kind.as_str(),
                    object_id,
                    object.project,
                    actor,
                    user,
                    display,
                    i64::from(can_write),
                    expires_at,
                    now,
                ],
            )?;
            Ok(CollabSession {
                id: id.clone(),
                kind: object.kind,
                object: object_id.to_string(),
                project: object.project,
                actor: actor.to_string(),
                user: user.map(str::to_string),
                display: display.to_string(),
                can_write,
                expires_at,
                revoked_at: None,
            })
        })?;

        Ok((session, plaintext))
    }

    /// Find a ticket by the hash of its plaintext.
    ///
    /// Expired and revoked rows come back rather than reading as absent, so the
    /// caller can tell "this ticket is finished" from "this ticket was never
    /// real" — two different things to tell somebody.
    pub fn lookup_collab_session_by_hash(&self, hash: &str) -> ApiResult<Option<CollabSession>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, object_kind, object_id, project, actor, \"user\", display, \
                     can_write, expires_at, revoked_at FROM crdt_sessions WHERE token_hash = ?1",
                    params![hash],
                    |r| {
                        let kind: String = r.get(1)?;
                        Ok(CollabSession {
                            id: r.get(0)?,
                            kind: if kind == "mindmap" {
                                CollabKind::Mindmap
                            } else {
                                CollabKind::Document
                            },
                            object: r.get(2)?,
                            project: r.get(3)?,
                            actor: r.get(4)?,
                            user: r.get(5)?,
                            display: r.get(6)?,
                            can_write: r.get::<_, i64>(7)? != 0,
                            expires_at: r.get(8)?,
                            revoked_at: r.get(9)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Take tickets that expired long enough ago that nobody is coming back.
    pub fn sweep_expired_collab_sessions(&self) -> ApiResult<usize> {
        let cutoff = now_ms() - SESSION_GRACE_MS;
        self.with_tx(|tx| {
            let removed = tx.execute(
                "DELETE FROM crdt_sessions WHERE expires_at < ?1",
                params![cutoff],
            )?;
            Ok(removed)
        })
    }
}

// ---------------------------------------------------------------------------
// Errors the sync path raises.
//
// Each is written per kind with a literal code, because the error-code scan in
// `tests/api.rs` reads the source text: a code built with `format!` would be
// invisible to it, and the guard that keeps `x-error-codes` honest would stop
// covering this file.
// ---------------------------------------------------------------------------

/// An update with no bytes in it.
///
/// Per kind with a literal code, like every error below: `/documents` published
/// these codes before mindmaps existed, and widening the log underneath must not
/// change what a document client reads.
fn update_empty(kind: CollabKind) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::validation(
            "validation.doc_update_empty",
            "An update carries no bytes. Nothing changed, so there is nothing to store.",
        ),
        CollabKind::Mindmap => ApiError::validation(
            "validation.mindmap_update_empty",
            "An update carries no bytes. Nothing changed, so there is nothing to store.",
        ),
    }
}

/// One flush that is too big to be one flush.
fn update_too_large(kind: CollabKind, len: usize) -> ApiError {
    let message = format!(
        "That update is {len} bytes and the cap is {MAX_UPDATE_BYTES}. One flush should be a few seconds of edits, not a whole document pasted at once."
    );
    match kind {
        CollabKind::Document => ApiError::validation("validation.doc_update_too_large", message),
        CollabKind::Mindmap => ApiError::validation("validation.mindmap_update_too_large", message),
    }
}

/// The whole log has outgrown what a reader can be asked to replay.
fn object_too_large(kind: CollabKind, id: &str, held: i64) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::validation(
            "validation.document_too_large",
            format!("Document '{id}' already holds {held} bytes of history and the cap is {MAX_OBJECT_BYTES}. Every reader replays the whole log to open it, so it cannot grow without bound."),
        ),
        CollabKind::Mindmap => ApiError::validation(
            "validation.mindmap_too_large",
            format!("Mindmap '{id}' already holds {held} bytes of history and the cap is {MAX_OBJECT_BYTES}. Every reader replays the whole log to open it, so it cannot grow without bound."),
        ),
    }
}

/// The room is full.
pub fn too_many_peers(kind: CollabKind, id: &str) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "document.too_many_peers",
            format!("Document '{id}' already has the maximum number of live editors."),
        )
        .remedy("Wait for somebody to close the document, then reconnect.".to_string()),
        CollabKind::Mindmap => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "mindmap.too_many_peers",
            format!("Mindmap '{id}' already has the maximum number of live editors."),
        )
        .remedy("Wait for somebody to close the map, then reconnect.".to_string()),
    }
}

/// No ticket on the handshake at all.
pub fn session_missing(kind: CollabKind, id: &str) -> ApiError {
    let route = kind.session_route();
    match kind {
        CollabKind::Document => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "document.session_missing",
            "This socket needs a sync ticket; the handshake carried none.".to_string(),
        )
        .remedy(format!("Mint one with {route} and pass it as ?ticket=… — a browser WebSocket cannot send an Authorization header, which is why the credential rides the query string.")),
        CollabKind::Mindmap => {
            let _ = id;
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "mindmap.session_missing",
                "This socket needs a sync ticket; the handshake carried none.".to_string(),
            )
            .remedy(format!("Mint one with {route} and pass it as ?ticket=… — a browser WebSocket cannot send an Authorization header, which is why the credential rides the query string."))
        }
    }
}

/// A ticket that never existed.
pub fn session_invalid(kind: CollabKind) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "document.session_invalid",
            "That sync ticket is not one this server issued.".to_string(),
        )
        .remedy("Mint a fresh one with POST /v1/documents/{id}/session.".to_string()),
        CollabKind::Mindmap => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "mindmap.session_invalid",
            "That sync ticket is not one this server issued.".to_string(),
        )
        .remedy("Mint a fresh one with POST /v1/mindmaps/{id}/session.".to_string()),
    }
}

/// A ticket that has run out or been revoked.
pub fn session_expired(kind: CollabKind) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::new(
            StatusCode::GONE,
            "document.session_expired",
            "That sync ticket has expired or been revoked.".to_string(),
        )
        .remedy("Mint a fresh one with POST /v1/documents/{id}/session.".to_string()),
        CollabKind::Mindmap => ApiError::new(
            StatusCode::GONE,
            "mindmap.session_expired",
            "That sync ticket has expired or been revoked.".to_string(),
        )
        .remedy("Mint a fresh one with POST /v1/mindmaps/{id}/session.".to_string()),
    }
}

/// A real ticket, pointed at something else.
///
/// This is the guard that makes the ticket worth scoping at all: it reaches one
/// object, and presenting it at another is refused rather than quietly widened.
pub fn session_wrong_object(kind: CollabKind, wanted: &str, holds: &str) -> ApiError {
    match kind {
        CollabKind::Document => ApiError::new(
            StatusCode::FORBIDDEN,
            "document.session_wrong_document",
            format!("That sync ticket is for document '{holds}', not '{wanted}'."),
        )
        .remedy("Mint a ticket for the document you are opening.".to_string()),
        CollabKind::Mindmap => ApiError::new(
            StatusCode::FORBIDDEN,
            "mindmap.session_wrong_mindmap",
            format!("That sync ticket is for mindmap '{holds}', not '{wanted}'."),
        )
        .remedy("Mint a ticket for the map you are opening.".to_string()),
    }
}
