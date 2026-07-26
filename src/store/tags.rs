//! Project tag registry: named entities of a free-form `kind` (person,
//! component, team, …) that tickets reference by `kind:handle`. Generic by
//! design — a new kind is just a new string, no schema change — with per-kind
//! attributes carried in the free-form `meta` object. Tagging never affects
//! claims, leases, or question routing; a tag is reference metadata only.

use super::helpers::emit_event;
use super::merge_patch;
use super::model::{Tag, MAX_METADATA};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, tag_id};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

const MAX_LABEL: usize = 200;
const MAX_KIND: usize = 32;
const MAX_HANDLE: usize = 64;

#[derive(Debug, Clone, Default)]
pub struct TagCreate {
    pub kind: String,
    pub handle: String,
    pub label: Option<String>,
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct TagPatch {
    pub label: Option<String>,
    pub meta_merge: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct TagListFilter {
    pub project: String,
    /// Restrict to one kind (e.g. only people).
    pub kind: Option<String>,
    /// Case-insensitive substring match on handle or label.
    pub q: Option<String>,
}

/// Validate a tag `kind` slug: 1-32 chars, `^[a-z][a-z0-9-]*$`. Kept free of
/// `%`/`_` so a `kind:%` LIKE filter over ticket tags stays a literal prefix.
pub fn validate_tag_kind(kind: &str) -> ApiResult<()> {
    let b = kind.as_bytes();
    let ok = (1..=MAX_KIND).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
    if ok {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.tag_kind",
        format!(
            "Tag kind '{kind}' is invalid. Use 1-{MAX_KIND} chars matching ^[a-z][a-z0-9-]*$ (e.g. 'person', 'component', 'team')."
        ),
    ))
}

/// Validate a tag `handle` slug: 1-64 chars, `^[a-z0-9][a-z0-9._-]*$`. The
/// handle is the free-form identity — two tags are the same iff their
/// (project, kind, handle) match exactly.
fn validate_tag_handle(handle: &str) -> ApiResult<()> {
    let b = handle.as_bytes();
    let ok = (1..=MAX_HANDLE).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b[1..].iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(*c, b'.' | b'_' | b'-')
        });
    if ok {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.tag_handle",
        format!(
            "Tag handle '{handle}' is invalid. Use 1-{MAX_HANDLE} lowercase chars matching ^[a-z0-9][a-z0-9._-]*$ (e.g. 'ada', 'billing-core'). Put the display name in 'label'."
        ),
    ))
}

/// Parse and validate a `kind:handle` reference, returning the canonical form.
/// Used both for registry addressing and for tag references stored on tickets.
pub fn normalize_tag_ref(raw: &str) -> ApiResult<String> {
    let (kind, handle) = raw.split_once(':').ok_or_else(|| {
        ApiError::validation(
            "validation.tag_ref",
            format!(
                "Tag reference '{raw}' must be 'kind:handle' (e.g. 'person:ada'). It names an entity in this project's tag registry."
            ),
        )
    })?;
    validate_tag_kind(kind)?;
    validate_tag_handle(handle)?;
    Ok(format!("{kind}:{handle}"))
}

fn validate_label(label: &str) -> ApiResult<()> {
    if label.len() > MAX_LABEL {
        return Err(ApiError::validation(
            "validation.tag_label",
            format!("Tag label must be at most {MAX_LABEL} characters."),
        ));
    }
    Ok(())
}

fn validate_meta(meta: &Value) -> ApiResult<()> {
    if !meta.is_object() {
        return Err(ApiError::validation(
            "validation.tag_meta",
            "Tag meta must be a JSON object (per-kind attributes like {\"email\": \"...\"}).",
        ));
    }
    let size = serde_json::to_string(meta).map(|s| s.len()).unwrap_or(0);
    if size > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.tag_meta_size",
            format!("Tag meta is {size} bytes serialized; the cap is {MAX_METADATA}."),
        ));
    }
    Ok(())
}

fn row_to_tag(r: &rusqlite::Row) -> rusqlite::Result<Tag> {
    let meta_raw: String = r.get("meta")?;
    Ok(Tag {
        id: r.get("id")?,
        project: r.get("project")?,
        kind: r.get("kind")?,
        handle: r.get("handle")?,
        label: r.get("label")?,
        meta: serde_json::from_str(&meta_raw).unwrap_or(Value::Null),
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

const TAG_COLS: &str = "id, project, kind, handle, label, meta, created_by, created_at, updated_at";

/// Ensure a registry row exists for every `kind:handle` in `refs` under
/// `project`, creating a stub (label defaults to the handle) for any that are
/// missing. This is the lazy-create behind tagging a ticket: a not-yet-declared
/// handle just works, and can be enriched (label/meta) later. Runs inside the
/// caller's write transaction. `refs` must already be normalized.
pub(crate) fn ensure_tags_exist(
    conn: &Connection,
    project: &str,
    refs: &[String],
    actor: &str,
    now: i64,
) -> ApiResult<()> {
    for r in refs {
        let (kind, handle) = r.split_once(':').expect("normalized ref has a colon");
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM tags WHERE project = ?1 AND kind = ?2 AND handle = ?3",
                params![project, kind, handle],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            continue;
        }
        let id = tag_id();
        conn.execute(
            "INSERT INTO tags (id, project, kind, handle, label, meta, created_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, '{}', ?6, ?7, ?7)",
            params![id, project, kind, handle, handle, actor, now],
        )?;
        emit_event(
            conn,
            None,
            Some(project),
            actor,
            "tag_created",
            json!({ "kind": kind, "handle": handle, "auto": true }),
            now,
        )?;
    }
    Ok(())
}

impl Store {
    pub fn create_tag(&self, project: &str, req: &TagCreate, actor: &str) -> ApiResult<Tag> {
        validate_tag_kind(&req.kind)?;
        validate_tag_handle(&req.handle)?;
        let label = req.label.clone().unwrap_or_else(|| req.handle.clone());
        validate_label(&label)?;
        let meta = req.meta.clone().unwrap_or_else(|| json!({}));
        validate_meta(&meta)?;
        let now = now_ms();
        self.with_tx(|tx| {
            // 404 if the project does not exist (FK would otherwise fail opaquely).
            let project_exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![project], |r| r.get(0))
                .optional()?;
            if project_exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }
            let exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM tags WHERE project = ?1 AND kind = ?2 AND handle = ?3",
                    params![project, req.kind, req.handle],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_some() {
                return Err(ApiError::conflict(
                    "tag.exists",
                    format!(
                        "Tag '{}:{}' already exists in project '{project}'. PATCH /v1/projects/{project}/tags/{}/{} to change its label or meta.",
                        req.kind, req.handle, req.kind, req.handle
                    ),
                ));
            }
            let id = tag_id();
            tx.execute(
                "INSERT INTO tags (id, project, kind, handle, label, meta, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![id, project, req.kind, req.handle, label, meta.to_string(), actor, now],
            )?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "tag_created",
                json!({ "kind": req.kind, "handle": req.handle }),
                now,
            )?;
            Ok(Tag {
                id,
                project: project.to_string(),
                kind: req.kind.clone(),
                handle: req.handle.clone(),
                label,
                meta,
                created_by: actor.to_string(),
                created_at: now,
                updated_at: now,
            })
        })
    }

    pub fn list_tags(&self, filter: &TagListFilter) -> ApiResult<Vec<Tag>> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {TAG_COLS} FROM tags WHERE project = ?1");
            let mut params_vec: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Text(filter.project.clone())];
            if let Some(kind) = &filter.kind {
                sql.push_str(" AND kind = ?");
                params_vec.push(rusqlite::types::Value::Text(kind.clone()));
            }
            if let Some(q) = &filter.q {
                sql.push_str(" AND (LOWER(handle) LIKE ? OR LOWER(label) LIKE ?)");
                let needle = format!("%{}%", q.to_lowercase());
                params_vec.push(rusqlite::types::Value::Text(needle.clone()));
                params_vec.push(rusqlite::types::Value::Text(needle));
            }
            sql.push_str(" ORDER BY kind, handle");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params_vec), row_to_tag)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_tag(&self, project: &str, kind: &str, handle: &str) -> ApiResult<Option<Tag>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {TAG_COLS} FROM tags WHERE project = ?1 AND kind = ?2 AND handle = ?3"
            );
            let tag = conn
                .query_row(&sql, params![project, kind, handle], row_to_tag)
                .optional()?;
            Ok(tag)
        })
    }

    pub fn patch_tag(
        &self,
        project: &str,
        kind: &str,
        handle: &str,
        patch: &TagPatch,
        actor: &str,
    ) -> ApiResult<Tag> {
        if let Some(label) = &patch.label {
            validate_label(label)?;
        }
        let now = now_ms();
        self.with_tx(|tx| {
            let sql = format!(
                "SELECT {TAG_COLS} FROM tags WHERE project = ?1 AND kind = ?2 AND handle = ?3"
            );
            let mut tag = tx
                .query_row(&sql, params![project, kind, handle], row_to_tag)
                .optional()?
                .ok_or_else(|| ApiError::not_found("tag", &format!("{kind}:{handle}")))?;
            if let Some(label) = &patch.label {
                tag.label = label.clone();
            }
            if let Some(m) = &patch.meta_merge {
                if !m.is_object() {
                    return Err(ApiError::validation(
                        "validation.tag_meta",
                        "meta_merge must be a JSON object (keys set to null are removed).",
                    ));
                }
                merge_patch(&mut tag.meta, m);
                validate_meta(&tag.meta)?;
            }
            tx.execute(
                "UPDATE tags SET label = ?4, meta = ?5, updated_at = ?6 WHERE project = ?1 AND kind = ?2 AND handle = ?3",
                params![project, kind, handle, tag.label, tag.meta.to_string(), now],
            )?;
            tag.updated_at = now;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "tag_updated",
                json!({ "kind": kind, "handle": handle }),
                now,
            )?;
            Ok(tag)
        })
    }

    /// Delete a registry entry. Ticket references (`kind:handle` strings) are
    /// left as-is — the reference is the source of truth, the registry row an
    /// enrichment — so a later re-create picks the display name back up. Returns
    /// the number of ticket references still pointing at the deleted tag, for a
    /// caller "N tickets still reference this" note.
    pub fn delete_tag(
        &self,
        project: &str,
        kind: &str,
        handle: &str,
        actor: &str,
    ) -> ApiResult<i64> {
        let now = now_ms();
        let reference = format!("{kind}:{handle}");
        self.with_tx(|tx| {
            let existed = tx.execute(
                "DELETE FROM tags WHERE project = ?1 AND kind = ?2 AND handle = ?3",
                params![project, kind, handle],
            )?;
            if existed == 0 {
                return Err(ApiError::not_found("tag", &reference));
            }
            let still_referenced: i64 = tx.query_row(
                "SELECT COUNT(*) FROM tickets WHERE project = ?1 AND EXISTS (SELECT 1 FROM json_each(tickets.tags) WHERE json_each.value = ?2)",
                params![project, reference],
                |r| r.get(0),
            )?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "tag_deleted",
                json!({ "kind": kind, "handle": handle, "still_referenced": still_referenced }),
                now,
            )?;
            Ok(still_referenced)
        })
    }
}
