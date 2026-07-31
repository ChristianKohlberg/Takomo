//! Initiatives: a home for an idea before it is work.
//!
//! A ticket is a unit of work — it has a workflow, it gets claimed, it closes. An
//! initiative is the other thing a fleet produces: a product idea, a direction,
//! what is left over after a good conversation. It is nurtured rather than
//! completed, and several contributors — a colleague asked for feedback, a few
//! agents told to go research — add to it over time.
//!
//! So there is deliberately **no** workflow here, no claim, no lease, no fence and
//! no ready queue. Nothing races for an initiative, because appending is not
//! exclusive: two agents adding findings at the same time both belong, and the
//! append-only entry log keeps both. `status` is a plain label
//! ([`INITIATIVE_STATUSES`]) an owner sets, not a state machine that gates
//! transitions.
//!
//! What *is* enforced is size, because an initiative is the one thing in this store
//! that accepts binary uploads, and the two bounds exist for two different reasons.
//!
//! [`MAX_ENTRY_CONTENT_BYTES`] is a **latency** bound. Every write here happens
//! inside the single `IMMEDIATE` transaction behind the process-wide write mutex —
//! the mutex whose serialization is the exactly-one-claimant guarantee for the
//! ready queue — so the time one attachment takes to land is time every claim,
//! transition and heartbeat in the process spends waiting. The point is not that
//! the number is small; it is that it is bounded at all.
//!
//! [`MAX_INITIATIVE_BYTES`] and [`MAX_INITIATIVE_ENTRIES`] are **accumulation**
//! bounds, and cost nothing per request: they stop one initiative from growing
//! without limit, which matters because the store is a single SQLite file someone
//! has to back up. Both are checked, along with the per-attachment cap, before any
//! bytes are written.

use super::helpers::emit_event;
use super::merge_patch;
use super::model::{Initiative, InitiativeEntry, InitiativeRollup, MAX_BODY, MAX_METADATA};
use super::tags::{ensure_tags_exist, normalize_tag_set};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{initiative_entry_id, initiative_id, now_ms};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// The lifecycle labels an initiative can carry.
///
/// A label, not a workflow: nothing enforces an order and any of them can be set
/// at any time. `open` is being fed, `parked` is deliberately set aside (still
/// readable, still appendable), `distilled` records that its substance has been
/// turned into tickets — which is the only outcome an initiative has, since it is
/// never "done" on its own terms.
pub const INITIATIVE_STATUSES: [&str; 3] = ["open", "parked", "distilled"];

/// Cap on a single attachment's bytes (5 MiB).
///
/// The bytes are written with the process-wide write mutex held, so this number is
/// how long one upload may make every other writer in the process wait. It is
/// deliberately a latency bound rather than a storage one: 5 MiB of blob is a
/// single-digit-to-tens-of-milliseconds write on local disk, which is a real stall
/// but a bounded one, and it is enough for the documents an initiative actually
/// collects — a spec, a slide export, a scanned contract, a long transcript.
///
/// A video or a design bundle still does not belong here. Point `source_uri` at it
/// and put a summary in `text`; the collection stays weighable and the store stays
/// something a person can back up.
pub const MAX_ENTRY_CONTENT_BYTES: usize = 5_242_880;

/// Cap on how many entries one initiative may accumulate.
///
/// Generous, because accumulating is the point — but finite, because every read of
/// the initiative sums over all of them. At the cap the rollup is still one indexed
/// scan of 1000 rows.
///
/// Note which bound binds first: 1000 entries at the attachment cap is far past
/// [`MAX_INITIATIVE_BYTES`], so a document-heavy initiative hits the byte total
/// long before this. This one is what bounds an initiative fed thousands of short
/// notes, where no single entry is large and the total never approaches 1 GiB.
pub const MAX_INITIATIVE_ENTRIES: i64 = 1_000;

/// Cap on one initiative's total stored bytes (1 GiB), text and attachments
/// together.
///
/// Unlike the per-attachment cap this costs no request any latency — it is a
/// ceiling on accumulation, not on the work of one write. It is set where a single
/// initiative stops being something the rest of the deployment can ignore: the
/// store is one SQLite file, and one idea's collection should not be the reason
/// that file is hard to copy.
pub const MAX_INITIATIVE_BYTES: i64 = 1_073_741_824;

/// Render a byte count the way the caps are written, so an error message never
/// says "1024 MiB". Derived from the value rather than kept beside it as a label,
/// which would be free to drift from the number it describes.
fn human_bytes(bytes: i64) -> String {
    const MIB: i64 = 1_048_576;
    const GIB: i64 = 1_073_741_824;
    if bytes >= GIB && bytes % GIB == 0 {
        format!("{} GiB", bytes / GIB)
    } else if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// Default/max page size when listing initiatives.
pub const MAX_INITIATIVES_PAGE: i64 = 200;

/// Default/max page size when listing an initiative's entries.
pub const MAX_ENTRIES_PAGE: i64 = 200;

const MAX_INITIATIVE_TITLE: usize = 300;
/// The "very short description" cap. Well under a ticket body on purpose: the long
/// form belongs in an entry, where it arrives with provenance attached.
const MAX_SUMMARY: usize = 1_000;
const MAX_ENTRY_KIND: usize = 32;
const MAX_ENTRY_TITLE: usize = 300;
const MAX_SOURCE: usize = 200;
const MAX_SOURCE_URI: usize = 2_048;
const MAX_FILENAME: usize = 255;
const MAX_MIME: usize = 128;
const MAX_LABELS: usize = 50;

#[derive(Debug, Clone, Default)]
pub struct InitiativeCreate {
    pub title: String,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub labels: Vec<String>,
    pub tags: Vec<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct InitiativePatch {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub labels: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub metadata_merge: Option<Value>,
}

impl InitiativePatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.summary.is_none()
            && self.status.is_none()
            && self.labels.is_none()
            && self.tags.is_none()
            && self.metadata_merge.is_none()
    }
}

/// One contribution to append. `text`, `content`, or both — an entry with neither
/// is refused, because it would record provenance for nothing.
#[derive(Debug, Clone, Default)]
pub struct EntryCreate {
    pub kind: String,
    pub title: Option<String>,
    pub text: String,
    /// Raw attachment bytes, already decoded from whatever the wire carried.
    pub content: Option<Vec<u8>>,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub source: String,
    pub source_uri: Option<String>,
    pub origin_at: Option<i64>,
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct InitiativeListFilter {
    pub project: Option<String>,
    /// Restrict to projects this token may see (None = all).
    pub allowed_projects: Option<Vec<String>>,
    pub status: Option<String>,
    /// Case-insensitive substring match on title or summary.
    pub q: Option<String>,
    /// Match initiatives carrying this exact `kind:handle` tag reference.
    pub tag: Option<String>,
    pub label: Option<String>,
}

fn validate_status(status: &str) -> ApiResult<()> {
    if INITIATIVE_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.initiative_status",
        format!(
            "Initiative status '{status}' is not one of {}. This is a lifecycle label, not a workflow state: 'open' is being fed, 'parked' is set aside, 'distilled' means its substance became tickets.",
            INITIATIVE_STATUSES.join(", ")
        ),
    ))
}

/// Validate a free-form entry `kind` slug: 1-32 chars, `^[a-z][a-z0-9-]*$` — the
/// same shape as a tag kind, so the two read alike wherever they appear together.
fn validate_entry_kind(kind: &str) -> ApiResult<()> {
    let b = kind.as_bytes();
    let ok = (1..=MAX_ENTRY_KIND).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
    if ok {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.entry_kind",
        format!(
            "Entry kind '{kind}' is invalid. Use 1-{MAX_ENTRY_KIND} chars matching ^[a-z][a-z0-9-]*$ — e.g. 'note', 'research', 'feedback', 'transcript', 'document'. The vocabulary is open; pick a word that says what kind of input this is."
        ),
    ))
}

/// Validate an attachment's media type as bare `type/subtype`.
///
/// This is a security check, not tidiness. The value is echoed straight into the
/// `Content-Type` header of `GET …/entries/{id}/content`, so anything permitted
/// here reaches a response header verbatim. Restricting it to RFC 9110 token
/// characters with exactly one `/` — no parameters, no whitespace, no control
/// characters — means a caller cannot smuggle header syntax through it, and the
/// read path can serve the stored value without sanitizing it again.
fn validate_mime(mime: &str) -> ApiResult<()> {
    fn is_token(part: &str) -> bool {
        !part.is_empty()
            && part.bytes().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
            })
    }
    if let Some((ty, sub)) = mime.split_once('/') {
        if is_token(ty) && is_token(sub) {
            return Ok(());
        }
    }
    Err(ApiError::validation(
        "validation.entry_mime",
        format!(
            "Media type '{mime}' is invalid. Use a bare 'type/subtype' (e.g. 'text/markdown', 'application/pdf') — no parameters, no whitespace. This value is served back as the attachment's Content-Type, so it is restricted to what a header may hold."
        ),
    ))
}

/// Refuse a field longer than its cap.
///
/// One code for every length overrun on this surface rather than one per field,
/// and deliberately: the code is what a caller branches on, and "a field is too
/// long" is a single decision — send less text. Which field, and what the cap was,
/// belong in the message and `details`, where they inform without multiplying the
/// contract. It also keeps the code a literal the error-code scan can read; a
/// `format!`-assembled one would be invisible to it.
fn check_len(field: &str, value: &str, max: usize) -> ApiResult<()> {
    let count = value.chars().count();
    if count <= max {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.initiative_field_length",
        format!("'{field}' is {count} characters; the cap is {max}."),
    )
    .details(json!({ "field": field, "chars": count, "max_chars": max })))
}

fn validate_metadata(meta: &Value) -> ApiResult<()> {
    if !meta.is_object() {
        return Err(ApiError::validation(
            "validation.initiative_metadata",
            "metadata must be a JSON object.",
        ));
    }
    let size = serde_json::to_string(meta).map(|s| s.len()).unwrap_or(0);
    if size > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.initiative_metadata_size",
            format!("metadata is {size} bytes serialized; the cap is {MAX_METADATA}."),
        ));
    }
    Ok(())
}

fn normalize_labels(labels: &[String]) -> ApiResult<Vec<String>> {
    if labels.len() > MAX_LABELS {
        return Err(ApiError::validation(
            "validation.initiative_labels",
            format!("{} labels is over the cap of {MAX_LABELS}.", labels.len()),
        ));
    }
    let mut out: Vec<String> = Vec::with_capacity(labels.len());
    for l in labels {
        let trimmed = l.trim();
        if trimmed.is_empty() {
            return Err(ApiError::validation(
                "validation.initiative_labels",
                "A label may not be empty or whitespace-only.",
            ));
        }
        if !out.iter().any(|k| k == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

const INITIATIVE_COLS: &str =
    "id, project, title, summary, status, labels, tags, metadata, created_by, created_at, \
     updated_at, version";

/// Entry columns for every list/detail read — note the absence of `content`. The
/// blob is fetched on its own, by id, so paging entries never drags documents
/// through memory.
const ENTRY_COLS: &str = "id, initiative, project, kind, title, text, mime, filename, chars, \
     text_bytes, content_bytes, source, source_uri, origin_at, meta, author, created_at";

fn row_to_initiative(r: &rusqlite::Row) -> rusqlite::Result<Initiative> {
    let labels_raw: String = r.get("labels")?;
    let tags_raw: String = r.get("tags")?;
    let metadata_raw: String = r.get("metadata")?;
    Ok(Initiative {
        id: r.get("id")?,
        project: r.get("project")?,
        title: r.get("title")?,
        summary: r.get("summary")?,
        status: r.get("status")?,
        labels: serde_json::from_str(&labels_raw).unwrap_or_default(),
        tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        version: r.get("version")?,
        // Filled by load_rollup; a row read alone has no counts yet.
        rollup: InitiativeRollup::default(),
    })
}

fn row_to_entry(r: &rusqlite::Row) -> rusqlite::Result<InitiativeEntry> {
    let meta_raw: String = r.get("meta")?;
    Ok(InitiativeEntry {
        id: r.get("id")?,
        initiative: r.get("initiative")?,
        project: r.get("project")?,
        kind: r.get("kind")?,
        title: r.get("title")?,
        text: r.get("text")?,
        mime: r.get("mime")?,
        filename: r.get("filename")?,
        chars: r.get("chars")?,
        text_bytes: r.get("text_bytes")?,
        content_bytes: r.get("content_bytes")?,
        source: r.get("source")?,
        source_uri: r.get("source_uri")?,
        origin_at: r.get("origin_at")?,
        meta: serde_json::from_str(&meta_raw).unwrap_or(Value::Null),
        author: r.get("author")?,
        created_at: r.get("created_at")?,
    })
}

/// Recompute an initiative's counts from its entries. One indexed aggregate; runs
/// on both the read and the write path so a caller never sees stale numbers.
fn load_rollup(conn: &Connection, initiative: &str) -> ApiResult<InitiativeRollup> {
    let rollup = conn.query_row(
        "SELECT COUNT(*) AS entries, \
                COALESCE(SUM(content_bytes > 0), 0) AS attachments, \
                COALESCE(SUM(chars), 0) AS chars, \
                COALESCE(SUM(text_bytes), 0) AS text_bytes, \
                COALESCE(SUM(content_bytes), 0) AS attachment_bytes, \
                MAX(created_at) AS last_entry_at \
         FROM initiative_entries WHERE initiative = ?1",
        params![initiative],
        |r| {
            let text_bytes: i64 = r.get("text_bytes")?;
            let attachment_bytes: i64 = r.get("attachment_bytes")?;
            Ok(InitiativeRollup {
                entries: r.get("entries")?,
                attachments: r.get("attachments")?,
                chars: r.get("chars")?,
                bytes: text_bytes + attachment_bytes,
                attachment_bytes,
                last_entry_at: r.get("last_entry_at")?,
            })
        },
    )?;
    Ok(rollup)
}

fn get_initiative_row(conn: &Connection, id: &str) -> ApiResult<Option<Initiative>> {
    let sql = format!("SELECT {INITIATIVE_COLS} FROM initiatives WHERE id = ?1");
    let found = conn
        .query_row(&sql, params![id], row_to_initiative)
        .optional()?;
    match found {
        None => Ok(None),
        Some(mut ini) => {
            ini.rollup = load_rollup(conn, &ini.id)?;
            Ok(Some(ini))
        }
    }
}

impl Store {
    pub fn create_initiative(
        &self,
        project: &str,
        req: &InitiativeCreate,
        actor: &str,
    ) -> ApiResult<Initiative> {
        let title = req.title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::validation(
                "validation.initiative_title",
                "An initiative needs a title — the quick name the idea goes by.",
            ));
        }
        check_len("title", &title, MAX_INITIATIVE_TITLE)?;
        let summary = req.summary.clone().unwrap_or_default();
        check_len("summary", &summary, MAX_SUMMARY)?;
        let status = req.status.clone().unwrap_or_else(|| "open".to_string());
        validate_status(&status)?;
        let labels = normalize_labels(&req.labels)?;
        let tags = normalize_tag_set(&req.tags, "tags")?;
        let metadata = req.metadata.clone().unwrap_or_else(|| json!({}));
        validate_metadata(&metadata)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let project_exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .optional()?;
            if project_exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }
            ensure_tags_exist(tx, project, &tags, actor, now)?;
            let id = initiative_id();
            tx.execute(
                "INSERT INTO initiatives (id, project, title, summary, status, labels, tags, \
                 metadata, created_by, created_at, updated_at, version) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1)",
                params![
                    id,
                    project,
                    title,
                    summary,
                    status,
                    json!(labels).to_string(),
                    json!(tags).to_string(),
                    metadata.to_string(),
                    actor,
                    now,
                ],
            )?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "initiative_created",
                json!({ "initiative": id, "title": title }),
                now,
            )?;
            Ok(Initiative {
                id,
                project: project.to_string(),
                title,
                summary,
                status,
                labels,
                tags,
                metadata,
                created_by: actor.to_string(),
                created_at: now,
                updated_at: now,
                version: 1,
                rollup: InitiativeRollup::default(),
            })
        })
    }

    pub fn get_initiative(&self, id: &str) -> ApiResult<Option<Initiative>> {
        self.with_conn(|conn| get_initiative_row(conn, id))
    }

    /// Update the initiative's own metadata. Entries are append-only and are not
    /// reachable from here — nurturing an initiative means adding to it, and the
    /// only editable part is how it is described and filed.
    pub fn patch_initiative(
        &self,
        id: &str,
        patch: &InitiativePatch,
        actor: &str,
    ) -> ApiResult<Initiative> {
        if let Some(title) = &patch.title {
            if title.trim().is_empty() {
                return Err(ApiError::validation(
                    "validation.initiative_title",
                    "An initiative's title may not be cleared.",
                ));
            }
            check_len("title", title.trim(), MAX_INITIATIVE_TITLE)?;
        }
        if let Some(summary) = &patch.summary {
            check_len("summary", summary, MAX_SUMMARY)?;
        }
        if let Some(status) = &patch.status {
            validate_status(status)?;
        }
        let labels = match &patch.labels {
            Some(l) => Some(normalize_labels(l)?),
            None => None,
        };
        let tags = match &patch.tags {
            Some(t) => Some(normalize_tag_set(t, "tags")?),
            None => None,
        };
        let now = now_ms();
        self.with_tx(|tx| {
            let mut ini =
                get_initiative_row(tx, id)?.ok_or_else(|| ApiError::not_found("initiative", id))?;
            if let Some(t) = &patch.title {
                ini.title = t.trim().to_string();
            }
            if let Some(s) = &patch.summary {
                ini.summary = s.clone();
            }
            if let Some(s) = &patch.status {
                ini.status = s.clone();
            }
            if let Some(l) = labels {
                ini.labels = l;
            }
            if let Some(t) = tags {
                ensure_tags_exist(tx, &ini.project, &t, actor, now)?;
                ini.tags = t;
            }
            if let Some(m) = &patch.metadata_merge {
                if !m.is_object() {
                    return Err(ApiError::validation(
                        "validation.initiative_metadata",
                        "metadata_merge must be a JSON object (keys set to null are removed).",
                    ));
                }
                merge_patch(&mut ini.metadata, m);
                validate_metadata(&ini.metadata)?;
            }
            ini.version += 1;
            ini.updated_at = now;
            tx.execute(
                "UPDATE initiatives SET title = ?2, summary = ?3, status = ?4, labels = ?5, \
                 tags = ?6, metadata = ?7, updated_at = ?8, version = ?9 WHERE id = ?1",
                params![
                    id,
                    ini.title,
                    ini.summary,
                    ini.status,
                    json!(ini.labels).to_string(),
                    json!(ini.tags).to_string(),
                    ini.metadata.to_string(),
                    now,
                    ini.version,
                ],
            )?;
            emit_event(
                tx,
                None,
                Some(&ini.project),
                actor,
                "initiative_updated",
                json!({ "initiative": id, "status": ini.status }),
                now,
            )?;
            Ok(ini)
        })
    }

    /// List initiatives, newest first, with cursor pagination. Returns
    /// (initiatives, next_cursor).
    pub fn list_initiatives(
        &self,
        filter: &InitiativeListFilter,
        cursor: Option<i64>,
        limit: i64,
    ) -> ApiResult<(Vec<Initiative>, Option<String>)> {
        self.with_conn(|conn| {
            let mut sql =
                format!("SELECT {INITIATIVE_COLS}, rowid AS rid FROM initiatives WHERE 1=1");
            let mut params_vec: Vec<SqlValue> = Vec::new();
            if let Some(c) = cursor {
                sql.push_str(" AND rowid < ?");
                params_vec.push(SqlValue::Integer(c));
            }
            if let Some(p) = &filter.project {
                sql.push_str(" AND project = ?");
                params_vec.push(SqlValue::Text(p.clone()));
            }
            if let Some(allowed) = &filter.allowed_projects {
                sql.push_str(" AND project IN (");
                for (i, p) in allowed.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    params_vec.push(SqlValue::Text(p.clone()));
                }
                sql.push(')');
            }
            if let Some(s) = &filter.status {
                sql.push_str(" AND status = ?");
                params_vec.push(SqlValue::Text(s.clone()));
            }
            if let Some(l) = &filter.label {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM json_each(initiatives.labels) WHERE json_each.value = ?)",
                );
                params_vec.push(SqlValue::Text(l.clone()));
            }
            if let Some(t) = &filter.tag {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM json_each(initiatives.tags) WHERE json_each.value = ?)",
                );
                params_vec.push(SqlValue::Text(t.clone()));
            }
            if let Some(q) = &filter.q {
                // Same tokenized AND-of-terms search as tickets: every term must
                // match, each against title OR summary.
                for term in q.split_whitespace() {
                    sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ?)");
                    let needle = format!("%{}%", term.to_lowercase());
                    params_vec.push(SqlValue::Text(needle.clone()));
                    params_vec.push(SqlValue::Text(needle));
                }
            }
            sql.push_str(" ORDER BY rowid DESC LIMIT ?");
            params_vec.push(SqlValue::Integer(limit + 1));

            let mut stmt = conn.prepare(&sql)?;
            let mapped = stmt.query_map(rusqlite::params_from_iter(params_vec), |r| {
                let rid: i64 = r.get("rid")?;
                Ok((row_to_initiative(r)?, rid))
            })?;
            let mut rows: Vec<(Initiative, i64)> = Vec::new();
            for row in mapped {
                rows.push(row?);
            }
            let has_more = rows.len() as i64 > limit;
            rows.truncate(limit as usize);
            let next_cursor = if has_more {
                rows.last().map(|(_, rid)| rid.to_string())
            } else {
                None
            };
            let mut out = Vec::with_capacity(rows.len());
            for (mut ini, _) in rows {
                ini.rollup = load_rollup(conn, &ini.id)?;
                out.push(ini);
            }
            Ok((out, next_cursor))
        })
    }

    /// Append one contribution. Returns the entry plus the initiative with its
    /// recomputed rollup, so a caller sees immediately what the collection now
    /// weighs.
    pub fn append_initiative_entry(
        &self,
        initiative: &str,
        req: &EntryCreate,
        actor: &str,
    ) -> ApiResult<(InitiativeEntry, Initiative)> {
        validate_entry_kind(&req.kind)?;
        let source = req.source.trim().to_string();
        if source.is_empty() {
            return Err(ApiError::validation(
                "validation.entry_source",
                "'source' is required: an entry records where an input came from — an agent id, a person, a conversation ('claude:chat', 'person:ada', 'agent:w1'). Without it the collection is a pile of text nobody can attribute.",
            ));
        }
        check_len("source", &source, MAX_SOURCE)?;
        if let Some(uri) = &req.source_uri {
            check_len("source_uri", uri, MAX_SOURCE_URI)?;
        }
        if let Some(t) = &req.title {
            check_len("entry_title", t, MAX_ENTRY_TITLE)?;
        }
        if let Some(f) = &req.filename {
            check_len("filename", f, MAX_FILENAME)?;
        }
        if let Some(m) = &req.mime {
            check_len("mime", m, MAX_MIME)?;
            validate_mime(m)?;
        }
        if req.text.len() > MAX_BODY {
            return Err(ApiError::validation(
                "validation.entry_text",
                format!(
                    "Entry text is {} bytes; the cap is {MAX_BODY}. Split it across entries, or attach it as a document.",
                    req.text.len()
                ),
            ));
        }
        // Borrowed, never cloned. At the attachment cap a clone here would copy
        // 5 MiB for nothing — the bytes are handed straight to `params!` further
        // down and are never mutated or kept.
        let content = req.content.as_ref().filter(|c| !c.is_empty());
        if req.text.trim().is_empty() && content.is_none() {
            return Err(ApiError::validation(
                "validation.entry_empty",
                "An entry needs 'text', an attachment, or both. An entry with neither would record provenance for nothing.",
            ));
        }
        let content_bytes = content.as_ref().map(|c| c.len()).unwrap_or(0);
        if content_bytes > MAX_ENTRY_CONTENT_BYTES {
            return Err(ApiError::validation(
                "initiative.attachment_too_large",
                format!(
                    "The attachment is {content_bytes} bytes; the cap is {MAX_ENTRY_CONTENT_BYTES} ({}). The bytes are written with the process-wide write mutex held, so this cap is how long one upload may make every claim and transition in the store wait.",
                    human_bytes(MAX_ENTRY_CONTENT_BYTES as i64)
                ),
            )
            .remedy(
                "Keep the document under the cap, or host it elsewhere and put the URL in 'source_uri' with a summary in 'text'.",
            )
            .details(json!({
                "bytes": content_bytes,
                "max_bytes": MAX_ENTRY_CONTENT_BYTES,
            })));
        }
        if content.is_some() && req.filename.is_none() && req.mime.is_none() {
            return Err(ApiError::validation(
                "validation.entry_attachment_unlabeled",
                "An attachment needs 'filename' and/or 'mime' — without either, nothing downstream can tell a reader (or a browser) what the bytes are.",
            ));
        }
        let meta = req.meta.clone().unwrap_or_else(|| json!({}));
        validate_metadata(&meta)?;
        let chars = req.text.chars().count() as i64;
        let text_bytes = req.text.len() as i64;
        let now = now_ms();
        self.with_tx(|tx| {
            let ini = get_initiative_row(tx, initiative)?
                .ok_or_else(|| ApiError::not_found("initiative", initiative))?;
            // Both caps are checked here, against the rollup just read inside this
            // transaction, so two concurrent appends cannot each see room for the
            // last entry: the write mutex serializes them.
            if ini.rollup.entries >= MAX_INITIATIVE_ENTRIES {
                return Err(ApiError::conflict(
                    "initiative.too_many_entries",
                    format!(
                        "Initiative '{initiative}' already holds {} entries, the cap is {MAX_INITIATIVE_ENTRIES}. An initiative this full has outgrown being one idea.",
                        ini.rollup.entries
                    ),
                )
                .remedy(
                    "Distil it: turn what it says into tickets, set its status to 'distilled', and open a fresh initiative for what is still an idea.",
                ));
            }
            let would_be = ini.rollup.bytes + text_bytes + content_bytes as i64;
            if would_be > MAX_INITIATIVE_BYTES {
                return Err(ApiError::conflict(
                    "initiative.too_large",
                    format!(
                        "This entry would take initiative '{initiative}' to {would_be} bytes; the cap is {MAX_INITIATIVE_BYTES} ({}).",
                        human_bytes(MAX_INITIATIVE_BYTES)
                    ),
                )
                .remedy(
                    "Host large documents elsewhere and reference them with 'source_uri', or distil this initiative and start a fresh one.",
                )
                .details(json!({
                    "bytes": ini.rollup.bytes,
                    "would_be": would_be,
                    "max_bytes": MAX_INITIATIVE_BYTES,
                })));
            }
            let id = initiative_entry_id();
            tx.execute(
                "INSERT INTO initiative_entries (id, initiative, project, kind, title, text, \
                 content, mime, filename, chars, text_bytes, content_bytes, source, source_uri, \
                 origin_at, meta, author, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    id,
                    initiative,
                    ini.project,
                    req.kind,
                    req.title,
                    req.text,
                    content,
                    req.mime,
                    req.filename,
                    chars,
                    text_bytes,
                    content_bytes as i64,
                    source,
                    req.source_uri,
                    req.origin_at,
                    meta.to_string(),
                    actor,
                    now,
                ],
            )?;
            // The initiative's own updated_at tracks its collection: an appended
            // entry is a change to the initiative, even though nothing in its own
            // row changed. `version` deliberately does NOT move — it guards the
            // description against a concurrent edit, and appending never conflicts
            // with one.
            tx.execute(
                "UPDATE initiatives SET updated_at = ?2 WHERE id = ?1",
                params![initiative, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&ini.project),
                actor,
                "initiative_entry_added",
                json!({
                    "initiative": initiative,
                    "entry": id,
                    "kind": req.kind,
                    "source": source,
                    "chars": chars,
                    "content_bytes": content_bytes,
                }),
                now,
            )?;
            let entry = InitiativeEntry {
                id,
                initiative: initiative.to_string(),
                project: ini.project.clone(),
                kind: req.kind.clone(),
                title: req.title.clone(),
                text: req.text.clone(),
                mime: req.mime.clone(),
                filename: req.filename.clone(),
                chars,
                text_bytes,
                content_bytes: content_bytes as i64,
                source,
                source_uri: req.source_uri.clone(),
                origin_at: req.origin_at,
                meta,
                author: actor.to_string(),
                created_at: now,
            };
            let mut updated = ini;
            updated.updated_at = now;
            updated.rollup = load_rollup(tx, initiative)?;
            Ok((entry, updated))
        })
    }

    /// List an initiative's entries, newest first, with cursor pagination. Never
    /// loads attachment bytes.
    pub fn list_initiative_entries(
        &self,
        initiative: &str,
        cursor: Option<i64>,
        limit: i64,
    ) -> ApiResult<(Vec<InitiativeEntry>, Option<String>)> {
        self.with_conn(|conn| {
            let mut sql = format!(
                "SELECT {ENTRY_COLS}, rowid AS rid FROM initiative_entries WHERE initiative = ?1"
            );
            let mut params_vec: Vec<SqlValue> = vec![SqlValue::Text(initiative.to_string())];
            if let Some(c) = cursor {
                sql.push_str(" AND rowid < ?");
                params_vec.push(SqlValue::Integer(c));
            }
            sql.push_str(" ORDER BY rowid DESC LIMIT ?");
            params_vec.push(SqlValue::Integer(limit + 1));
            let mut stmt = conn.prepare(&sql)?;
            let mapped = stmt.query_map(rusqlite::params_from_iter(params_vec), |r| {
                let rid: i64 = r.get("rid")?;
                Ok((row_to_entry(r)?, rid))
            })?;
            let mut rows: Vec<(InitiativeEntry, i64)> = Vec::new();
            for row in mapped {
                rows.push(row?);
            }
            let has_more = rows.len() as i64 > limit;
            rows.truncate(limit as usize);
            let next_cursor = if has_more {
                rows.last().map(|(_, rid)| rid.to_string())
            } else {
                None
            };
            Ok((rows.into_iter().map(|(e, _)| e).collect(), next_cursor))
        })
    }

    /// One entry's metadata (no bytes), scoped to its initiative so a mismatched
    /// pair is a 404 rather than a cross-initiative read.
    pub fn get_initiative_entry(
        &self,
        initiative: &str,
        entry: &str,
    ) -> ApiResult<Option<InitiativeEntry>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ENTRY_COLS} FROM initiative_entries WHERE initiative = ?1 AND id = ?2"
            );
            Ok(conn
                .query_row(&sql, params![initiative, entry], row_to_entry)
                .optional()?)
        })
    }

    /// An entry's attachment bytes, with the mime and filename to serve them
    /// under. `Ok(None)` means no such entry; `Ok(Some((None, ..)))` means the
    /// entry exists but is text-only.
    #[allow(clippy::type_complexity)]
    pub fn initiative_entry_content(
        &self,
        initiative: &str,
        entry: &str,
    ) -> ApiResult<Option<(Option<Vec<u8>>, Option<String>, Option<String>)>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT content, mime, filename FROM initiative_entries \
                     WHERE initiative = ?1 AND id = ?2",
                    params![initiative, entry],
                    |r| {
                        Ok((
                            r.get::<_, Option<Vec<u8>>>("content")?,
                            r.get::<_, Option<String>>("mime")?,
                            r.get::<_, Option<String>>("filename")?,
                        ))
                    },
                )
                .optional()?)
        })
    }
}
