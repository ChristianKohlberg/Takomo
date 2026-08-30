//! Mindmaps: the ten minutes before any of it is an idea.
//!
//! A tree grown at conversation speed — a project idea fanning out into API,
//! integrations, workflows, ideas; six words a node; a branch split in two the
//! moment it turns out to be two thoughts. Then, when a branch is worth keeping,
//! it graduates into an epic or an initiative and the node keeps the link.
//!
//! **It is a brainstorming method and nothing more.** No workflow, no claim, no
//! lease, no ready queue, no assignment, no comments, no attachments. Two rules
//! follow from that and shape everything here:
//!
//! - **Deleting one is ordinary.** An initiative is nurtured; a mindmap is
//!   scratch. `DELETE` cascades its nodes, and the epics and initiatives its
//!   branches became are untouched, because those left the map when they
//!   graduated.
//!
//! - **A node is capped short** (`mindmapdoc::MAX_TITLE`). That is the method, not a
//!   limitation: a thought needing more than a sentence or two has stopped being a
//!   brainstorm node and wants to be an initiative, and the refusal says so.
//!
//! Growth is bounded everywhere it could be unbounded, because every write here
//! runs under the process-wide write mutex that serializes the ready queue: a
//! batch is capped ([`MAX_GROW`]), a map is capped ([`MAX_NODES`]), and depth is
//! capped ([`MAX_DEPTH`]) — the same ceiling the initiative folder tree uses.
//!
//! **The nodes are not here.** They live in one Yjs document per map
//! ([`super::mindmapdoc`]), because a brainstorm is something several people and
//! an agent grow at the same time, and rows where the last writer wins throw one
//! of them away without saying so. What stays in SQL is everything a list has to
//! answer without opening a document: the map's own row.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::Mindmap;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{mindmap_id, now_ms};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// Page ceiling for the map listing.
pub const MAX_MINDMAPS_PAGE: i64 = 200;

const MAX_TITLE: usize = 300;
const MAX_SUMMARY: usize = 2000;

/// The three labels a map can carry — the same vocabulary an initiative uses,
/// because they answer the same question and a second spelling would be a second
/// thing to learn. `distilled` = its branches have graduated.
pub const MINDMAP_STATUSES: [&str; 3] = ["open", "parked", "distilled"];

/// What a branch can graduate into.
pub const PROMOTION_TARGETS: [&str; 2] = ["epic", "initiative"];

#[derive(Debug, Clone, Default)]
pub struct MindmapCreate {
    pub title: String,
    pub summary: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct MindmapPatch {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub metadata_merge: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct MindmapListFilter {
    pub project: Option<String>,
    pub allowed_projects: Option<Vec<String>>,
    pub status: Option<String>,
    /// Case-insensitive substring over title and summary.
    pub q: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

const MAP_COLS: &str = "id, project, title, summary, status, metadata, created_by, created_at, \
     updated_at, version, nodes";

fn row_to_map(r: &rusqlite::Row) -> rusqlite::Result<Mindmap> {
    let metadata_raw: String = r.get("metadata")?;
    Ok(Mindmap {
        id: r.get("id")?,
        project: r.get("project")?,
        title: r.get("title")?,
        summary: r.get("summary")?,
        status: r.get("status")?,
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        version: r.get("version")?,
        // Denormalised, written by whoever last read the map's document. See
        // `Store::note_mindmap_size` for why it is a column and not a count.
        nodes: r.get("nodes")?,
    })
}

fn validate_title(title: &str) -> ApiResult<String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::validation(
            "validation.mindmap_title",
            "A mindmap needs a title — it is the root everything else hangs off.",
        ));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(ApiError::validation(
            "validation.mindmap_title",
            format!("A mindmap title must be at most {MAX_TITLE} characters."),
        ));
    }
    Ok(title)
}

fn validate_status(status: &str) -> ApiResult<()> {
    if MINDMAP_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.mindmap_status",
        format!(
            "Unknown status '{status}'. Use one of: {}. It is a label an owner sets, not a workflow state, so nothing gates the order.",
            MINDMAP_STATUSES.join(", ")
        ),
    ))
}

fn require_map(conn: &Connection, id: &str) -> ApiResult<Mindmap> {
    conn.query_row(
        &format!("SELECT {MAP_COLS} FROM mindmaps WHERE id = ?1"),
        params![id],
        row_to_map,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("mindmap", id))
}

/// Insert one ticket inside the caller's transaction, in its project's initial
/// state, and emit the ordinary `ticket_created` so the board and the event
/// stream need no special case for a ticket a mindmap made.
///
/// Written here rather than through [`Store::create_ticket`] because the write
/// mutex is not reentrant: promotion is one transaction, and a nested `with_tx`
/// would deadlock. `schedules::materialize_one` inserts its ticket the same way
/// and for the same reason.
///
/// The id retry is the collision handling `create_ticket` does — a suffix widened
/// on each attempt — and nothing else about ticket creation is duplicated: a
/// promoted ticket carries no claim, no deps and no metadata beyond what is
/// written here.
#[allow(clippy::too_many_arguments)]
fn insert_ticket(
    conn: &Connection,
    project: &str,
    ty: &str,
    parent: Option<&str>,
    title: &str,
    body: &str,
    actor: &str,
    now: i64,
) -> ApiResult<String> {
    let initial: String = conn.query_row(
        "SELECT json_extract(workflow_json, '$.initial') FROM projects WHERE id = ?1",
        params![project],
        |r| r.get(0),
    )?;
    let title = title
        .chars()
        .take(super::model::MAX_TITLE)
        .collect::<String>();
    for attempt in 0..8 {
        let candidate = format!("{project}-{}", crate::ids::ticket_suffix(4 + attempt / 4));
        let res = conn.execute(
            "INSERT INTO tickets (id, project, type, parent, title, body, state, priority, labels, tags, metadata, links, created_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'normal', '[]', '[]', '{}', '{}', ?8, ?9, ?9)",
            params![candidate, project, ty, parent, title, body, initial, actor, now],
        );
        match res {
            Ok(_) => {
                emit_event(
                    conn,
                    Some(&candidate),
                    Some(project),
                    actor,
                    "ticket_created",
                    json!({ "title": title, "state": initial, "type": ty }),
                    now,
                )?;
                return Ok(candidate);
            }
            Err(e) if is_primary_key_conflict(&e) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError::internal(
        "could not allocate a ticket id for a promoted mindmap branch",
    ))
}

/// Whether a rusqlite error is the primary-key collision an id retry should
/// swallow, rather than any other constraint failure — which must surface.
fn is_primary_key_conflict(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == rusqlite::ErrorCode::ConstraintViolation
    ) && err.to_string().contains("tickets.id")
}

impl Store {
    pub fn create_mindmap(
        &self,
        project: &str,
        req: &MindmapCreate,
        actor: &str,
    ) -> ApiResult<Mindmap> {
        let title = validate_title(&req.title)?;
        let summary = req.summary.clone().unwrap_or_default();
        if summary.chars().count() > MAX_SUMMARY {
            return Err(ApiError::validation(
                "validation.mindmap_summary",
                format!("A mindmap summary must be at most {MAX_SUMMARY} characters."),
            ));
        }
        let metadata = req.metadata.clone().unwrap_or_else(|| json!({}));
        let now = now_ms();
        self.with_tx(|tx| {
            ensure_project_writable(tx, project)?;
            let exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }
            let id = mindmap_id();
            tx.execute(
                "INSERT INTO mindmaps (id, project, title, summary, status, metadata, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?7)",
                params![id, project, title, summary, metadata.to_string(), actor, now],
            )?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "mindmap_created",
                json!({ "mindmap": id, "title": title }),
                now,
            )?;
            Ok(Mindmap {
                id,
                project: project.to_string(),
                title,
                summary,
                status: "open".to_string(),
                metadata,
                created_by: actor.to_string(),
                created_at: now,
                updated_at: now,
                version: 1,
                nodes: 0,
            })
        })
    }

    /// One page of maps, plus how many matched — the same envelope contract every
    /// listing here owes its reader.
    pub fn list_mindmaps(&self, filter: &MindmapListFilter) -> ApiResult<(Vec<Mindmap>, i64)> {
        self.with_conn(|conn| {
            let mut scope = String::from(" FROM mindmaps WHERE 1=1");
            let mut binds: Vec<rusqlite::types::Value> = Vec::new();
            if let Some(project) = &filter.project {
                scope.push_str(" AND project = ?");
                binds.push(rusqlite::types::Value::Text(project.clone()));
            }
            if let Some(allowed) = &filter.allowed_projects {
                scope.push_str(" AND project IN (");
                for (i, p) in allowed.iter().enumerate() {
                    if i > 0 {
                        scope.push(',');
                    }
                    scope.push('?');
                    binds.push(rusqlite::types::Value::Text(p.clone()));
                }
                scope.push(')');
            }
            if let Some(status) = &filter.status {
                scope.push_str(" AND status = ?");
                binds.push(rusqlite::types::Value::Text(status.clone()));
            }
            if let Some(q) = &filter.q {
                scope.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ?)");
                let needle = format!("%{}%", q.to_lowercase());
                binds.push(rusqlite::types::Value::Text(needle.clone()));
                binds.push(rusqlite::types::Value::Text(needle));
            }
            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*){scope}"),
                rusqlite::params_from_iter(binds.clone()),
                |r| r.get(0),
            )?;

            let mut page = format!("SELECT {MAP_COLS}{scope} ORDER BY updated_at DESC, id");
            page.push_str(" LIMIT ? OFFSET ?");
            binds.push(rusqlite::types::Value::Integer(filter.limit));
            binds.push(rusqlite::types::Value::Integer(filter.offset));
            let mut stmt = conn.prepare(&page)?;
            let mut maps = stmt
                .query_map(rusqlite::params_from_iter(binds), row_to_map)?
                .collect::<Result<Vec<_>, _>>()?;
            // The count is what tells a reader whether a map is worth opening, so
            // the listing pays for it — once per row of the page, never per row of
            // the table.
            for map in &mut maps {
                // Read straight off the row rather than opened and counted:
                // a list of 200 maps would otherwise hydrate 200 documents. The
                // column is written by the flusher, so it lags a live edit by at
                // most one flush interval; `get_mindmap_with_nodes` on the
                // single-map read counts the open replica exactly.
                map.nodes = map.nodes.max(0);
            }
            Ok((maps, total))
        })
    }

    /// The map's own row. The nodes come from its document — see
    /// `src/api/mindmapdoc.rs`.
    pub fn get_mindmap(&self, id: &str) -> ApiResult<Option<Mindmap>> {
        self.with_conn(|conn| {
            let found = conn
                .query_row(
                    &format!("SELECT {MAP_COLS} FROM mindmaps WHERE id = ?1"),
                    params![id],
                    row_to_map,
                )
                .optional()?;
            Ok(found)
        })
    }

    /// Record how many nodes a map currently holds.
    ///
    /// A denormalised count, and it earns its keep: `GET /v1/mindmaps` reports
    /// a node count per map, and computing it honestly would mean replaying
    /// every listed map's document on every list. Written by whoever last saw
    /// the replica — the flusher every couple of seconds, and each API write
    /// immediately — so the list is never more than a flush behind, and the
    /// single-map read does not use it at all.
    pub fn note_mindmap_size(&self, id: &str, nodes: i64) -> ApiResult<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE mindmaps SET nodes = ?2 WHERE id = ?1",
                params![id, nodes],
            )?;
            Ok(())
        })
    }

    pub fn patch_mindmap(&self, id: &str, patch: &MindmapPatch, actor: &str) -> ApiResult<Mindmap> {
        if let Some(status) = &patch.status {
            validate_status(status)?;
        }
        let now = now_ms();
        self.with_tx(|tx| {
            let mut map = require_map(tx, id)?;
            ensure_project_writable(tx, &map.project)?;
            if let Some(title) = &patch.title {
                map.title = validate_title(title)?;
            }
            if let Some(summary) = &patch.summary {
                map.summary = summary.clone();
            }
            if let Some(status) = &patch.status {
                map.status = status.clone();
            }
            if let Some(m) = &patch.metadata_merge {
                if !m.is_object() {
                    return Err(ApiError::validation(
                        "validation.mindmap_metadata",
                        "metadata_merge must be a JSON object (keys set to null are removed).",
                    ));
                }
                super::merge_patch(&mut map.metadata, m);
            }
            tx.execute(
                "UPDATE mindmaps SET title = ?2, summary = ?3, status = ?4, metadata = ?5, \
                 version = version + 1, updated_at = ?6 WHERE id = ?1",
                params![
                    id,
                    map.title,
                    map.summary,
                    map.status,
                    map.metadata.to_string(),
                    now
                ],
            )?;
            map.updated_at = now;
            map.version += 1;
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_updated",
                json!({ "mindmap": id }),
                now,
            )?;
            Ok(map)
        })
    }

    /// Throw a map away. Ordinary, and the clearest statement of what a mindmap
    /// is: scratch.
    ///
    /// Nodes cascade. What a branch *became* does not: an epic or initiative that
    /// graduated is work in its own right, and it left the map when it graduated.
    pub fn delete_mindmap(&self, id: &str, actor: &str) -> ApiResult<i64> {
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, id)?;
            ensure_project_writable(tx, &map.project)?;
            let nodes: i64 = tx.query_row(
                "SELECT nodes FROM mindmaps WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            // The document and its tickets carry no foreign key back to the map
            // — `crdt_updates.object_id` points at one of two tables — so the
            // cascade is this call.
            Store::purge_collab(tx, id)?;
            tx.execute("DELETE FROM mindmaps WHERE id = ?1", params![id])?;
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_deleted",
                json!({ "mindmap": id, "title": map.title, "nodes": nodes }),
                now,
            )?;
            Ok(nodes)
        })
    }
}

/// Convert maps that predate the shared document into one.
///
/// Runs once per map at startup, and only for a map that has legacy rows and no
/// document yet — so it is idempotent, and a no-op on a fresh database and on
/// every start after the first.
///
/// **`mindmap_nodes` is deliberately left in place and simply stops being
/// read.** Dropping it in the same release that stops using it would remove the
/// only way to check afterwards that a conversion was faithful; a later release
/// drops it, once nobody needs to look.
///
/// A Yjs document's whole state serialises as one ordinary update, so the
/// conversion writes exactly one row — the same shape a compaction writes.
pub(crate) fn adopt_legacy_nodes(conn: &Connection) -> ApiResult<()> {
    if !super::has_table(conn, "mindmap_nodes")? {
        return Ok(());
    }

    let pending: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.mindmap FROM mindmap_nodes n \
             WHERE NOT EXISTS (SELECT 1 FROM crdt_updates u WHERE u.object_id = n.mindmap)",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };

    for map_id in pending {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            Option<String>,
            String,
            Option<f64>,
            Option<f64>,
            Option<String>,
            Option<String>,
            String,
            i64,
            i64,
        )> = {
            let mut stmt = conn.prepare(
                "SELECT id, parent, text, x, y, promoted_kind, promoted_id, created_by, \
                 created_at, updated_at FROM mindmap_nodes WHERE mindmap = ?1 \
                 ORDER BY position, created_at, id",
            )?;
            let mapped = stmt.query_map(params![map_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            continue;
        }

        let blob = super::mindmapdoc::build_from_legacy(&rows);
        let bytes = blob.len() as i64;
        let now = now_ms();
        conn.execute(
            "INSERT INTO crdt_updates (object_kind, object_id, blob, bytes, created_by, created_at) \
             VALUES ('mindmap', ?1, ?2, ?3, 'migration', ?4)",
            params![map_id, blob, bytes, now],
        )?;
        conn.execute(
            "UPDATE mindmaps SET nodes = ?2 WHERE id = ?1",
            params![map_id, rows.len() as i64],
        )?;
    }

    Ok(())
}

/// Everything the SQL side of a promotion needs, read out of the map's document
/// by the caller.
///
/// The node text arrives as an argument rather than being looked up here,
/// because the nodes are no longer rows: the caller has the replica open, reads
/// the branch from it, and hands over the finished strings. That split is also
/// what makes the ordering safe — see [`Store::promote_branch`].
/// Refuse an unknown promotion target.
///
/// Called before the branch is even looked at, because "that is not a thing you
/// can promote to" is a better answer than "that branch is already promoted" —
/// the caller got the verb wrong, and telling them about the branch instead
/// sends them to fix the wrong thing.
pub fn validate_promotion_target(target: &str) -> ApiResult<()> {
    if PROMOTION_TARGETS.contains(&target) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.mindmap_target",
        format!(
            "Unknown promotion target '{target}'. Use 'epic' (an epic with its children as tickets — the fastest path to work) or 'initiative' (a direction that needs nurturing first)."
        ),
    ))
}

/// The three shape changes worth an event.
///
/// A brainstorm generates edits constantly; only these three are things another
/// reader of the project would want to know about.
#[derive(Debug, Clone, Copy)]
pub enum MindmapChange {
    /// Nodes were added — one event for the batch, because ten nodes from an
    /// agent turn are one act of brainstorming.
    Grown,
    /// A node was reparented. Placement and text changes emit nothing.
    Moved,
    /// A branch was pruned.
    Pruned,
}

pub struct BranchPromotion<'a> {
    pub map_id: &'a str,
    pub node_id: &'a str,
    pub target: &'a str,
    /// The node's own title, which becomes the epic's or initiative's title.
    pub title: &'a str,
    /// The whole branch as indented text.
    pub branch_outline: &'a str,
    /// Direct children only, as `(title, outline)`.
    ///
    /// Direct children only, and that is the rule rather than a simplification:
    /// a deeper subtree would arrive as a flat pile of tickets whose shape
    /// nobody could recover, and the map keeps that shape for whoever wants it.
    pub children: &'a [(String, String)],
}

impl Store {
    /// Graduate a branch into an epic or an initiative.
    ///
    /// Called from inside the map's document mutation, and the ORDER matters:
    /// the caller checks the branch is not already promoted, calls this, and
    /// only writes the promotion link back into the document once this has
    /// returned an id. A link written first would point at nothing if the work
    /// behind it failed; a link never written would let the same thought become
    /// a second, indistinguishable epic on the next attempt.
    pub fn promote_branch(&self, promotion: &BranchPromotion, actor: &str) -> ApiResult<Value> {
        validate_promotion_target(promotion.target)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, promotion.map_id)?;
            ensure_project_writable(tx, &map.project)?;

            let created = match promotion.target {
                "epic" => {
                    let epic = insert_ticket(
                        tx,
                        &map.project,
                        "epic",
                        None,
                        promotion.title,
                        &format!(
                            "From the mindmap “{}” ({}).\n\n{}",
                            map.title, promotion.map_id, promotion.branch_outline
                        ),
                        actor,
                        now,
                    )?;
                    let mut children = Vec::new();
                    for (title, body) in promotion.children {
                        let id = insert_ticket(
                            tx,
                            &map.project,
                            "task",
                            Some(&epic),
                            title,
                            body,
                            actor,
                            now,
                        )?;
                        children.push(id);
                    }
                    json!({ "kind": "epic", "id": epic, "children": children })
                }
                _ => {
                    let id = crate::ids::initiative_id();
                    tx.execute(
                        "INSERT INTO initiatives (id, project, title, summary, status, labels, tags, metadata, created_by, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, '', 'open', '[]', '[]', ?4, ?5, ?6, ?6)",
                        params![
                            id,
                            map.project,
                            promotion.title,
                            json!({ "mindmap": promotion.map_id, "node": promotion.node_id })
                                .to_string(),
                            actor,
                            now
                        ],
                    )?;
                    // The subtree becomes the first entry rather than the summary:
                    // an entry carries provenance, and where an idea came from is
                    // exactly what a collection is supposed to remember.
                    let entry = crate::ids::initiative_entry_id();
                    let text = promotion.branch_outline;
                    tx.execute(
                        "INSERT INTO initiative_entries (id, initiative, project, kind, title, text, chars, text_bytes, source, meta, author, created_at) \
                         VALUES (?1, ?2, ?3, 'note', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            entry,
                            id,
                            map.project,
                            format!("From the mindmap “{}”", map.title),
                            text,
                            text.chars().count() as i64,
                            text.len() as i64,
                            format!("mindmap:{}", promotion.map_id),
                            json!({ "mindmap": promotion.map_id, "node": promotion.node_id })
                                .to_string(),
                            actor,
                            now
                        ],
                    )?;
                    emit_event(
                        tx,
                        None,
                        Some(&map.project),
                        actor,
                        "initiative_created",
                        json!({ "initiative": id, "title": promotion.title, "mindmap": promotion.map_id }),
                        now,
                    )?;
                    json!({ "kind": "initiative", "id": id })
                }
            };

            let kind = created["kind"].as_str().unwrap_or(promotion.target);
            let created_id = created["id"].as_str().unwrap_or_default();
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![promotion.map_id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_promoted",
                json!({ "mindmap": promotion.map_id, "node": promotion.node_id, "kind": kind, "id": created_id }),
                now,
            )?;
            Ok(created)
        })
    }

    /// Record that a map's shape changed, for the event log.
    ///
    /// The document layer does the changing; this is only the announcement, and
    /// it stays sparse on purpose. Text and placement edits emit nothing — they
    /// change constantly while somebody is thinking, and an event per
    /// keystroke-batch would bury every other event in the project under one
    /// person's typing.
    ///
    /// The kind is an enum with a literal per arm rather than a string
    /// parameter, so the event-kind guard in `tests/api.rs` — which reads the
    /// argument to `emit_event` out of the source — can still see what this
    /// emits.
    pub fn note_mindmap_event(
        &self,
        map_id: &str,
        change: MindmapChange,
        payload: Value,
        actor: &str,
    ) -> ApiResult<()> {
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, map_id)?;
            let project = Some(map.project.as_str());
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![map_id, now],
            )?;
            match change {
                MindmapChange::Grown => {
                    emit_event(tx, None, project, actor, "mindmap_grown", payload, now)?;
                }
                MindmapChange::Moved => {
                    emit_event(tx, None, project, actor, "mindmap_moved", payload, now)?;
                }
                MindmapChange::Pruned => {
                    emit_event(tx, None, project, actor, "mindmap_pruned", payload, now)?;
                }
            }
            Ok(())
        })
    }
}

impl Store {
    /// Change a map's document without a live room.
    ///
    /// The room layer is the normal path and it is asynchronous, shared, and
    /// broadcasts what it changes. The seeder and the CLI have none of that:
    /// they are one process, alone with a database, so they load the log,
    /// change the replica, and write the whole state back as a single update —
    /// which is exactly the shape a compaction writes, because a Yjs document's
    /// entire state serialises as one ordinary update.
    ///
    /// It **appends** its result rather than replacing the log. Replacing would
    /// be tidier — the state it computed is complete — but the CLI runs against
    /// a database file a server may be serving at the same time, and deleting
    /// the rows a live room's replica was built from throws that room's work
    /// away with no way for either side to notice. An append merges; Yjs is
    /// built for exactly that. Every live path goes through `open_room` instead.
    pub fn edit_mindmap_document<T>(
        &self,
        map_id: &str,
        f: impl FnOnce(&yrs::Doc) -> ApiResult<T>,
    ) -> ApiResult<T> {
        use yrs::updates::decoder::Decode;
        use yrs::{ReadTxn, Transact};

        let updates = self.load_collab_updates(map_id)?;
        let doc = yrs::Doc::new();
        {
            let mut txn = doc.transact_mut();
            for blob in &updates {
                if let Ok(update) = yrs::Update::decode_v1(blob) {
                    let _ = txn.apply_update(update);
                }
            }
        }

        let out = f(&doc)?;

        let (state, nodes) = {
            let txn = doc.transact();
            let state = txn.encode_state_as_update_v1(&yrs::StateVector::default());
            drop(txn);
            let (all, _, _) = super::mindmapdoc::snapshot(&doc, map_id);
            (state, all.len() as i64)
        };
        self.append_collab_update(map_id, &state, "seed")?;
        self.note_mindmap_size(map_id, nodes)?;
        Ok(out)
    }
}
