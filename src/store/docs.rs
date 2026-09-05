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
//! last peer leaving. [`Store::append_collab_update`] is the only write, and it is
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
/// scans of `crdt_updates`. Here it is one grouped scan for the whole page.
fn attach_log_stats(conn: &Connection, docs: &mut [Document]) -> ApiResult<()> {
    if docs.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", docs.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT object_id, COUNT(*) AS n, COALESCE(SUM(bytes), 0) AS b
           FROM crdt_updates WHERE object_id IN ({placeholders}) GROUP BY object_id"
    );
    let ids: Vec<&dyn rusqlite::ToSql> =
        docs.iter().map(|d| &d.id as &dyn rusqlite::ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(ids.as_slice())?;
    let mut stats: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        stats.insert(row.get("object_id")?, (row.get("n")?, row.get("b")?));
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

impl Store {}
