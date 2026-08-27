//! Collaborative documents: prose humans and agents edit at the same time.
//!
//! This is the storage half. The live half — the y-protocols sync session over a
//! WebSocket — is `src/api/docs.rs`; everything that touches SQLite is here,
//! because the layering rule holds for this surface like every other.
//!
//! ## Why the prose is not a column
//!
//! An agent asked to tighten a paragraph spends several seconds thinking. If it
//! then writes a whole document back, everything typed during those seconds is
//! gone — and a one-word fix arrives as a whole-document diff nobody can review.
//! That is what a `body TEXT` column buys, and it is the failure this surface
//! exists to remove. So the prose is a Yjs CRDT, every participant is an ordinary
//! peer holding a replica, and merges are the data structure's problem rather
//! than a policy anyone has to enforce.
//!
//! What is stored here is therefore an **update log**: opaque Yjs blobs, replayed
//! in `seq` order to rebuild the document. Takomo does not parse them, and does
//! not need to — the same "Takomo stores, the agent computes" division Checklist
//! draws.
//!
//! ## The debounce is load-bearing, not a nicety
//!
//! Every mutation in this store runs as one `IMMEDIATE` transaction behind a
//! process-wide `Mutex<Connection>`, and that serialization *is* the
//! exactly-one-claimant guarantee for the ready queue. Persisting per keystroke
//! would therefore put every claim, transition and heartbeat in the process
//! behind somebody's typing — the same hazard the initiative attachment caps
//! already exist to prevent, arriving continuously instead of once.
//!
//! So the split is: **broadcast is memory, persistence is batched.** A room
//! accumulates updates and flushes them as one merged blob on a timer or on the
//! last peer leaving. [`Store::append_doc_update`] is the only write, and it is
//! two statements.
//!
//! ## Compaction needs no second table
//!
//! A Yjs document's whole state serializes as a single, ordinary update. So
//! compacting is: read the log, merge, `DELETE` the rows, `INSERT` one blob —
//! same format, same table. There is no snapshot format to keep in step, which
//! is the kind of thing that only stays true if someone writes down that it was
//! deliberate.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::{Document, DOCUMENT_STATUSES, MAX_METADATA};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{document_id, now_ms};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};

/// Cap on live documents per project.
pub const MAX_DOCUMENTS_PER_PROJECT: i64 = 2_000;

/// Default and maximum page size when listing documents.
pub const MAX_DOCUMENTS_PAGE: i64 = 200;

/// Cap on ONE flush, on the decoded bytes.
///
/// A flush is a merge of a few seconds of typing, which is kilobytes. A megabyte
/// is far past any honest one and small enough that holding the write mutex for
/// it is not felt. A paste of a large document arrives as several flushes.
pub const MAX_DOC_UPDATE_BYTES: usize = 1024 * 1024;

/// Cap on a document's whole update log, post-compaction.
///
/// 32 MiB of CRDT state is an enormous document — Yjs keeps deleted content as
/// tombstones, so this is generous for the prose it represents. The cap exists
/// because the log is replayed into memory on every room open, and an unbounded
/// one would be an unbounded allocation triggered by a `GET`.
pub const MAX_DOC_BYTES: i64 = 32 * 1024 * 1024;

/// Compact once the log passes this many rows.
///
/// Chosen against replay cost rather than storage: rebuilding a room applies
/// every row, so the number that matters is how long that takes on open. A few
/// hundred small updates is milliseconds.
pub const COMPACT_AFTER_UPDATES: i64 = 256;

const MAX_DOC_TITLE: usize = 200;
const MAX_DOC_PATH: usize = 400;

const DOC_COLS: &str = "id, project, title, path, status, initiative, metadata, version, \
    created_by, created_at, updated_at, archived_at";

#[derive(Debug, Clone, Default)]
pub struct DocumentCreate {
    pub project: String,
    pub title: String,
    pub path: Option<String>,
    pub status: Option<String>,
    pub initiative: Option<String>,
    pub metadata: Option<Value>,
}

/// A partial update. `initiative` uses an override slot — absent leaves it
/// alone, explicit null clears it — for the same reason an environment's
/// nullable fields do: once set, "unset it again" has to be expressible.
#[derive(Debug, Clone, Default)]
pub struct DocumentPatch {
    pub title: Option<String>,
    pub path: Option<String>,
    pub status: Option<String>,
    pub initiative: Option<Option<String>>,
    pub metadata_merge: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFilter {
    pub project: String,
    pub status: Option<String>,
    pub initiative: Option<String>,
    pub q: Option<String>,
    pub include_archived: bool,
    pub limit: Option<i64>,
}

// One validator per field rather than a shared helper or a macro, matching
// environments and checklist: the error-code scan in `tests/api.rs` reads source
// text, so a code reached through a helper is invisible to it — and a code the
// scan cannot see can drift out of the documented vocabulary silently.

fn validate_document_title(value: &str) -> ApiResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_DOC_TITLE {
        return Err(ApiError::validation(
            "validation.document_title",
            format!(
                "A document title must be 1..={MAX_DOC_TITLE} characters; got {}.",
                trimmed.chars().count()
            ),
        )
        .remedy("Send a short title; the prose belongs in the document itself.".to_string()));
    }
    Ok(())
}

/// A folder path: `/`-separated segments, no leading or trailing slash, no empty
/// segment and no `.`/`..`.
///
/// The last two are not path-traversal defence — nothing here touches a
/// filesystem — but tree-rendering defence. `a//b` and `a/../b` would each draw a
/// folder no one can name or navigate back out of.
fn validate_document_path(value: &str) -> ApiResult<()> {
    if value.is_empty() {
        return Ok(());
    }
    let bad = value.len() > MAX_DOC_PATH
        || value.starts_with('/')
        || value.ends_with('/')
        || value
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        return Err(ApiError::validation(
            "validation.document_path",
            format!(
                "'{value}' is not a folder path: use `/`-separated segments with no leading \
                 or trailing slash, no empty segment, and no '.' or '..' (max {MAX_DOC_PATH} \
                 characters)."
            ),
        )
        .remedy(
            "Send something like \"product/billing\", or \"\" for the top level.".to_string(),
        ));
    }
    Ok(())
}

fn validate_document_status(value: &str) -> ApiResult<()> {
    if !DOCUMENT_STATUSES.contains(&value) {
        return Err(ApiError::validation(
            "validation.document_status",
            format!(
                "'{value}' is not a document status. Use one of: {}.",
                DOCUMENT_STATUSES.join(", ")
            ),
        )
        .remedy(
            "Status is a label, not a state machine — nothing enforces an order between \
             them."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_document_metadata(raw: &str) -> ApiResult<()> {
    if raw.len() > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.document_metadata",
            format!(
                "metadata serializes to {} bytes; the maximum is {MAX_METADATA}.",
                raw.len()
            ),
        )
        .remedy("Keep metadata small; long prose belongs in the document.".to_string()));
    }
    Ok(())
}

/// The initiative a document is distilled from must exist and live in the same
/// project. Same rule, same reasoning as `checks.initiative`: the column is
/// validated on write so a reader never meets a reference naming nothing.
fn validate_document_initiative(
    conn: &Connection,
    project: &str,
    initiative: &str,
) -> ApiResult<()> {
    let found: Option<String> = conn
        .query_row(
            "SELECT project FROM initiatives WHERE id = ?1",
            params![initiative],
            |r| r.get(0),
        )
        .optional()?;
    match found {
        Some(p) if p == project => Ok(()),
        Some(p) => Err(ApiError::validation(
            "validation.document_initiative",
            format!("Initiative '{initiative}' belongs to project '{p}', not '{project}'."),
        )
        .remedy("Pick an initiative in this project, or omit 'initiative'.".to_string())),
        None => Err(ApiError::not_found("initiative", initiative).remedy(
            "List them with GET /v1/initiatives?project=<project>, or omit 'initiative' — a \
             document need not come from one."
                .to_string(),
        )),
    }
}

fn project_exists(conn: &Connection, project: &str) -> ApiResult<()> {
    let found: Option<String> = conn
        .query_row(
            "SELECT id FROM projects WHERE id = ?1",
            params![project],
            |r| r.get(0),
        )
        .optional()?;
    if found.is_none() {
        return Err(ApiError::not_found("project", project)
            .remedy("List projects with GET /v1/projects.".to_string()));
    }
    Ok(())
}

fn row_to_document(row: &Row) -> rusqlite::Result<Document> {
    let metadata_raw: String = row.get("metadata")?;
    Ok(Document {
        id: row.get("id")?,
        project: row.get("project")?,
        title: row.get("title")?,
        path: row.get("path")?,
        status: row.get("status")?,
        initiative: row.get("initiative")?,
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        version: row.get("version")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
        // Filled by `attach_log_stats`; a row read alone reports zero rather than
        // a wrong number.
        bytes: 0,
        updates: 0,
    })
}

/// Add the update-log size to already-loaded documents, in ONE query.
///
/// Separate from `row_to_document` and deliberately not a correlated subquery in
/// the main SELECT: a list of 200 documents would otherwise run 200 aggregate
/// scans of `doc_updates`. Here it is one grouped scan for the whole page.
fn attach_log_stats(conn: &Connection, docs: &mut [Document]) -> ApiResult<()> {
    if docs.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", docs.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT document, COUNT(*) AS n, COALESCE(SUM(bytes), 0) AS b
           FROM doc_updates WHERE document IN ({placeholders}) GROUP BY document"
    );
    let ids: Vec<&dyn rusqlite::ToSql> =
        docs.iter().map(|d| &d.id as &dyn rusqlite::ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(ids.as_slice())?;
    let mut stats: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        stats.insert(row.get("document")?, (row.get("n")?, row.get("b")?));
    }
    for doc in docs.iter_mut() {
        if let Some((n, b)) = stats.get(&doc.id) {
            doc.updates = *n;
            doc.bytes = *b;
        }
    }
    Ok(())
}

fn get_document_row(conn: &Connection, id: &str) -> ApiResult<Document> {
    let mut stmt = conn.prepare(&format!("SELECT {DOC_COLS} FROM documents WHERE id = ?1"))?;
    let doc = stmt
        .query_row(params![id], row_to_document)
        .optional()?
        .ok_or_else(|| {
            ApiError::not_found("document", id)
                .remedy("List documents with GET /v1/documents?project=<project>.".to_string())
        })?;
    Ok(doc)
}

impl Store {
    pub fn create_document(&self, req: &DocumentCreate, actor: &str) -> ApiResult<Document> {
        let title = req.title.trim().to_string();
        validate_document_title(&title)?;
        let path = req.path.clone().unwrap_or_default().trim().to_string();
        validate_document_path(&path)?;
        let status = req.status.clone().unwrap_or_else(|| "draft".to_string());
        validate_document_status(&status)?;
        let metadata = req.metadata.clone().unwrap_or(Value::Null);
        let metadata_raw = metadata.to_string();
        validate_document_metadata(&metadata_raw)?;

        let now = now_ms();
        let id = document_id();
        let project = req.project.clone();
        let initiative = req.initiative.clone();

        self.with_tx(|tx| {
            project_exists(tx, &project)?;
            ensure_project_writable(tx, &project)?;
            if let Some(i) = &initiative {
                validate_document_initiative(tx, &project, i)?;
            }

            let live: i64 = tx.query_row(
                "SELECT COUNT(*) FROM documents WHERE project = ?1 AND archived_at IS NULL",
                params![project],
                |r| r.get(0),
            )?;
            if live >= MAX_DOCUMENTS_PER_PROJECT {
                return Err(ApiError::validation(
                    "validation.document_count",
                    format!(
                        "Project '{project}' already has {live} live documents; the maximum \
                         is {MAX_DOCUMENTS_PER_PROJECT}."
                    ),
                )
                .remedy("Archive one with DELETE /v1/documents/{id}.".to_string()));
            }

            tx.execute(
                "INSERT INTO documents (id, project, title, path, status, initiative, metadata,
                    version, created_by, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?9)",
                params![
                    id,
                    project,
                    title,
                    path,
                    status,
                    initiative,
                    metadata_raw,
                    actor,
                    now
                ],
            )?;
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "document.created",
                json!({ "document": id, "title": title, "path": path }),
                now,
            )?;
            get_document_row(tx, &id)
        })
    }

    pub fn list_documents(&self, filter: &DocumentFilter) -> ApiResult<(Vec<Document>, i64)> {
        let limit = filter
            .limit
            .unwrap_or(MAX_DOCUMENTS_PAGE)
            .clamp(1, MAX_DOCUMENTS_PAGE);

        self.with_conn(|conn| {
            let mut where_sql = String::from("project = ?1");
            let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(filter.project.clone())];
            if !filter.include_archived {
                where_sql.push_str(" AND archived_at IS NULL");
            }
            if let Some(s) = &filter.status {
                args.push(Box::new(s.clone()));
                where_sql.push_str(&format!(" AND status = ?{}", args.len()));
            }
            if let Some(i) = &filter.initiative {
                // "" is the `?initiative=none` convention, the same shape
                // `?epic=none` uses on tickets: documents no initiative claims.
                if i.is_empty() {
                    where_sql.push_str(" AND initiative IS NULL");
                } else {
                    args.push(Box::new(i.clone()));
                    where_sql.push_str(&format!(" AND initiative = ?{}", args.len()));
                }
            }
            if let Some(q) = &filter.q {
                let needle = format!("%{}%", super::helpers::escape_like_literal(q));
                args.push(Box::new(needle));
                where_sql.push_str(&format!(
                    " AND (title LIKE ?{n} ESCAPE '\\' OR path LIKE ?{n} ESCAPE '\\')",
                    n = args.len()
                ));
            }

            let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();

            // Counted with the SAME predicate that selected the page: a `total`
            // from a different WHERE is how a reader comes to treat a fraction of
            // the work as all of it.
            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM documents WHERE {where_sql}"),
                refs.as_slice(),
                |r| r.get(0),
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {DOC_COLS} FROM documents WHERE {where_sql}
                 ORDER BY path ASC, title ASC, id ASC LIMIT {limit}"
            ))?;
            let mut docs = stmt
                .query_map(refs.as_slice(), row_to_document)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            attach_log_stats(conn, &mut docs)?;
            Ok((docs, total))
        })
    }

    pub fn get_document(&self, id: &str) -> ApiResult<Document> {
        self.with_conn(|conn| {
            let mut doc = get_document_row(conn, id)?;
            attach_log_stats(conn, std::slice::from_mut(&mut doc))?;
            Ok(doc)
        })
    }

    pub fn patch_document(
        &self,
        id: &str,
        patch: &DocumentPatch,
        actor: &str,
    ) -> ApiResult<Document> {
        if let Some(t) = &patch.title {
            validate_document_title(t)?;
        }
        if let Some(p) = &patch.path {
            validate_document_path(p.trim())?;
        }
        if let Some(s) = &patch.status {
            validate_document_status(s)?;
        }
        let now = now_ms();

        self.with_tx(|tx| {
            let current = get_document_row(tx, id)?;
            ensure_project_writable(tx, &current.project)?;
            if let Some(Some(i)) = &patch.initiative {
                validate_document_initiative(tx, &current.project, i)?;
            }

            let title = patch
                .title
                .clone()
                .map(|t| t.trim().to_string())
                .unwrap_or(current.title);
            let path = patch
                .path
                .clone()
                .map(|p| p.trim().to_string())
                .unwrap_or(current.path);
            let status = patch.status.clone().unwrap_or(current.status);
            let initiative = match &patch.initiative {
                Some(v) => v.clone(),
                None => current.initiative,
            };
            let metadata = match &patch.metadata_merge {
                Some(merge) => merge_metadata(&current.metadata, merge),
                None => current.metadata,
            };
            let metadata_raw = metadata.to_string();
            validate_document_metadata(&metadata_raw)?;

            tx.execute(
                "UPDATE documents SET title = ?2, path = ?3, status = ?4, initiative = ?5,
                    metadata = ?6, version = version + 1, updated_at = ?7 WHERE id = ?1",
                params![id, title, path, status, initiative, metadata_raw, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&current.project),
                actor,
                "document.updated",
                json!({ "document": id, "title": title, "path": path, "status": status }),
                now,
            )?;
            get_document_row(tx, id)
        })
    }

    /// Archive a document. Reversible, and the prose is untouched — the update
    /// log stays exactly as it was, which is what makes unarchiving honest
    /// rather than a restore from something lossy.
    pub fn archive_document(&self, id: &str, actor: &str) -> ApiResult<Document> {
        let now = now_ms();
        self.with_tx(|tx| {
            let current = get_document_row(tx, id)?;
            ensure_project_writable(tx, &current.project)?;
            if current.archived_at.is_some() {
                return Ok(current);
            }
            tx.execute(
                "UPDATE documents SET archived_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&current.project),
                actor,
                "document.archived",
                json!({ "document": id }),
                now,
            )?;
            get_document_row(tx, id)
        })
    }

    pub fn unarchive_document(&self, id: &str, actor: &str) -> ApiResult<Document> {
        let now = now_ms();
        self.with_tx(|tx| {
            let current = get_document_row(tx, id)?;
            ensure_project_writable(tx, &current.project)?;
            if current.archived_at.is_none() {
                return Ok(current);
            }
            tx.execute(
                "UPDATE documents SET archived_at = NULL, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&current.project),
                actor,
                "document.unarchived",
                json!({ "document": id }),
                now,
            )?;
            get_document_row(tx, id)
        })
    }

    /// Refuse a write to a document that cannot take one.
    ///
    /// Two gates, and the second is the one worth naming: the project's archive
    /// flag (which every other project-scoped mutation checks) **and** the
    /// document's own. An archived document is reversible and still readable —
    /// but accepting an agent's proposal into one would be work filed against
    /// something somebody deliberately set aside, and it would only be noticed
    /// when the document came back.
    pub fn ensure_document_writable(&self, id: &str) -> ApiResult<()> {
        self.with_conn(|conn| {
            let doc = get_document_row(conn, id)?;
            ensure_project_writable(conn, &doc.project)?;
            if doc.archived_at.is_some() {
                return Err(ApiError::conflict(
                    "conflict.document_archived",
                    format!("Document '{id}' is archived; it cannot be written to."),
                )
                .remedy(
                    "Restore it with POST /v1/documents/{id}/unarchive if this work still \
                     belongs to it."
                        .to_string(),
                ));
            }
            Ok(())
        })
    }

    /// Every update blob for a document, in `seq` order.
    ///
    /// Replayed into a fresh Yjs doc when a room opens. Reads go through a READER
    /// connection, so opening a large document cannot stall a claim.
    pub fn load_doc_updates(&self, id: &str) -> ApiResult<Vec<Vec<u8>>> {
        self.with_conn(|conn| {
            // Prove the document exists first, so an unknown id is a 404 rather
            // than an empty document a caller would happily start typing into.
            get_document_row(conn, id)?;
            let mut stmt =
                conn.prepare("SELECT blob FROM doc_updates WHERE document = ?1 ORDER BY seq ASC")?;
            let blobs = stmt
                .query_map(params![id], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(blobs)
        })
    }

    /// Append one flushed, merged update.
    ///
    /// The ONLY write on the hot path, and deliberately two statements: an
    /// INSERT and a timestamp touch. It runs inside the same single-writer
    /// transaction as every claim in the process, which is exactly why the caller
    /// must have debounced — see this module's header.
    ///
    /// Returns the log's row count afterwards, so the caller can decide to
    /// compact without a second query.
    pub fn append_doc_update(&self, id: &str, blob: &[u8], actor: &str) -> ApiResult<i64> {
        if blob.is_empty() {
            return Err(ApiError::validation(
                "validation.doc_update_empty",
                "An update must carry bytes; an empty one records nothing.".to_string(),
            )
            .remedy("Skip the flush when there is nothing to persist.".to_string()));
        }
        if blob.len() > MAX_DOC_UPDATE_BYTES {
            return Err(ApiError::validation(
                "validation.doc_update_too_large",
                format!(
                    "This update is {} bytes; the maximum for one flush is \
                     {MAX_DOC_UPDATE_BYTES}.",
                    blob.len()
                ),
            )
            .remedy(
                "Flush more often. One update is meant to be a few seconds of typing, and \
                 the whole write path in this process is serialized behind it."
                    .to_string(),
            ));
        }
        let now = now_ms();
        let bytes = blob.len() as i64;

        self.with_tx(|tx| {
            let doc = get_document_row(tx, id)?;
            ensure_project_writable(tx, &doc.project)?;

            let held: i64 = tx.query_row(
                "SELECT COALESCE(SUM(bytes), 0) FROM doc_updates WHERE document = ?1",
                params![id],
                |r| r.get(0),
            )?;
            if held + bytes > MAX_DOC_BYTES {
                return Err(ApiError::validation(
                    "validation.document_too_large",
                    format!(
                        "Document '{id}' holds {held} bytes of history and this update would \
                         take it past the {MAX_DOC_BYTES}-byte maximum."
                    ),
                )
                .remedy(
                    "Split the document — the whole log is replayed into memory when the \
                     document is opened, so the cap bounds that allocation."
                        .to_string(),
                ));
            }

            tx.execute(
                "INSERT INTO doc_updates (document, blob, bytes, created_by, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![id, blob, bytes, actor, now],
            )?;
            tx.execute(
                "UPDATE documents SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            // Deliberately NO event per flush. The event log is read by
            // long-pollers and by `/v1/events`; a row per few seconds of typing
            // per document would drown every other event in the project, and
            // nothing downstream wants keystroke granularity. Collaborators learn
            // about edits through the CRDT itself, which is faster and finer than
            // an event could be.
            let rows: i64 = tx.query_row(
                "SELECT COUNT(*) FROM doc_updates WHERE document = ?1",
                params![id],
                |r| r.get(0),
            )?;
            Ok(rows)
        })
    }

    /// Replace a document's whole update log with one merged blob.
    ///
    /// The caller does the merging, because merging is a Yjs operation and this
    /// module does not parse blobs. It passes the serialized state of the doc it
    /// already holds in memory — which is, by construction, every update in the
    /// log plus anything newer, so the delete-then-insert cannot lose an edit
    /// that arrived mid-compaction.
    pub fn compact_doc(&self, id: &str, state: &[u8], actor: &str) -> ApiResult<()> {
        if state.is_empty() {
            return Err(ApiError::validation(
                "validation.doc_update_empty",
                "A compaction must carry the document's serialized state.".to_string(),
            )
            .remedy("Skip compaction for an empty document.".to_string()));
        }
        let now = now_ms();
        let bytes = state.len() as i64;

        self.with_tx(|tx| {
            let doc = get_document_row(tx, id)?;
            ensure_project_writable(tx, &doc.project)?;
            // One transaction, so a reader either sees the old log or the new
            // single row — never an empty document between the two.
            tx.execute("DELETE FROM doc_updates WHERE document = ?1", params![id])?;
            tx.execute(
                "INSERT INTO doc_updates (document, blob, bytes, created_by, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![id, state, bytes, actor, now],
            )?;
            Ok(())
        })
    }
}

/// Shallow key merge, the same shape `metadata_merge` has elsewhere: an explicit
/// null deletes a key, any other value replaces it.
fn merge_metadata(current: &Value, merge: &Value) -> Value {
    let mut base = match current {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    if let Value::Object(patch) = merge {
        for (k, v) in patch {
            if v.is_null() {
                base.remove(k);
            } else {
                base.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(base)
}

/// A live sync session's resolved identity.
///
/// Returned by [`Store::lookup_doc_session_by_hash`] regardless of expiry or
/// revocation — the same split the answer-grant path uses, so the caller can
/// tell an unknown ticket from a dead one and say which.
#[derive(Debug, Clone)]
pub struct DocSession {
    pub id: String,
    pub document: String,
    pub project: String,
    pub actor: String,
    pub user: Option<String>,
    /// The name collaborators see next to a caret.
    pub display: String,
    pub can_write: bool,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

/// How long a sync ticket lives.
///
/// Long enough to cover a working session including the reconnects a flaky
/// network produces — `y-websocket` retries with the URL it was given, so a
/// ticket that died in ninety seconds would drop a writer mid-paragraph. Short
/// enough that a ticket recovered from an access log is worth little by the time
/// anyone reads it.
pub const DOC_SESSION_TTL_SECONDS: i64 = 12 * 3600;

impl Store {
    /// Mint a sync ticket for one document.
    ///
    /// `can_write` is decided by the caller from the MINTING token's scopes, not
    /// re-derived here: a `read`-only reader joins as a read-only peer, and the
    /// ticket cannot grant more than the credential that asked for it.
    pub fn create_doc_session(
        &self,
        document: &str,
        actor: &str,
        display: &str,
        user: Option<&str>,
        can_write: bool,
        expires_at: i64,
    ) -> ApiResult<(DocSession, String)> {
        let plaintext = crate::ids::doc_session_token_plaintext();
        let hash = crate::ids::token_hash(&plaintext);
        let id = crate::ids::doc_session_id();
        let now = now_ms();
        let document = document.to_string();

        let session = self.with_tx(|tx| {
            let doc = get_document_row(tx, &document)?;
            // Refused on an archived project for the reason an answer link is:
            // a session that could only ever fail to save is worse than none,
            // because the failure surfaces after the typing.
            ensure_project_writable(tx, &doc.project)?;
            tx.execute(
                "INSERT INTO doc_sessions (id, token_hash, document, project, actor, \"user\",
                    display, can_write, expires_at, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    id,
                    hash,
                    document,
                    doc.project,
                    actor,
                    user,
                    display,
                    i64::from(can_write),
                    expires_at,
                    now
                ],
            )?;
            Ok(DocSession {
                id: id.clone(),
                document: document.clone(),
                project: doc.project,
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

    pub fn lookup_doc_session_by_hash(&self, hash: &str) -> ApiResult<Option<DocSession>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, document, project, actor, \"user\", display, can_write,
                        expires_at, revoked_at FROM doc_sessions WHERE token_hash = ?1",
                    params![hash],
                    |r| {
                        Ok(DocSession {
                            id: r.get(0)?,
                            document: r.get(1)?,
                            project: r.get(2)?,
                            actor: r.get(3)?,
                            user: r.get(4)?,
                            display: r.get(5)?,
                            can_write: r.get::<_, i64>(6)? != 0,
                            expires_at: r.get(7)?,
                            revoked_at: r.get(8)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Drop sessions long past expiry. Called from the background sweeper.
    ///
    /// Deliberately keeps them for a grace period past `expires_at` rather than
    /// deleting on the tick they die: a client reconnecting with a just-expired
    /// ticket should be told it expired, which needs the row to still be there.
    pub fn sweep_expired_doc_sessions(&self) -> ApiResult<usize> {
        let cutoff = now_ms() - 24 * 3600 * 1000;
        self.with_tx(|tx| {
            let n = tx.execute(
                "DELETE FROM doc_sessions WHERE expires_at < ?1",
                params![cutoff],
            )?;
            Ok(n)
        })
    }
}
