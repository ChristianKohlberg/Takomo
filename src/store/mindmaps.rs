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
//! - **A node is capped short** ([`MAX_NODE_TEXT`]). That is the method, not a
//!   limitation: a thought needing more than a sentence or two has stopped being a
//!   brainstorm node and wants to be an initiative, and the refusal says so.
//!
//! Growth is bounded everywhere it could be unbounded, because every write here
//! runs under the process-wide write mutex that serializes the ready queue: a
//! batch is capped ([`MAX_GROW`]), a map is capped ([`MAX_NODES`]), and depth is
//! capped ([`MAX_DEPTH`]) — the same ceiling the initiative folder tree uses.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::{Mindmap, MindmapNode};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{mindmap_id, mindmap_node_id, now_ms};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The most a node may say. A sentence or two — deliberately close to a tweet.
///
/// This is the one cap in the store that exists for the sake of the *method*
/// rather than for cost. A brainstorm where nodes grow into paragraphs has
/// quietly become a document, and the value of the map — that you can read a
/// whole branch at a glance — is gone by the time anyone notices. So the refusal
/// names the way out (promote it) instead of just reporting a length.
pub const MAX_NODE_TEXT: usize = 280;
/// How many thoughts one map may hold. A brainstorm, not a database.
pub const MAX_NODES: i64 = 500;
/// How deep it may nest — the ceiling `web/src/lib/initiative-tree.ts` uses for
/// folders, for the same reason: past this nobody can read the shape anyway.
pub const MAX_DEPTH: usize = 8;
/// Nodes one `grow` call may add: an agent turn's worth. The loop runs inside the
/// write transaction, so its length is time every other writer spends waiting.
pub const MAX_GROW: usize = 50;
/// Page ceiling for the map listing.
pub const MAX_MINDMAPS_PAGE: i64 = 200;

/// The gap between sibling positions. Wide enough that inserting between two
/// nodes is one write, and re-gapping is a rare repair rather than the norm.
const POSITION_GAP: i64 = 1000;

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

/// One thought to add. `parent` is None for a first-ring branch.
#[derive(Debug, Clone, Default)]
pub struct NodeAdd {
    pub parent: Option<String>,
    pub text: String,
    /// Where among its siblings. None appends to the end, which is what typing
    /// does; a value is what splitting a thought in two needs.
    pub position: Option<i64>,
}

/// What a node edit may change. Every field is optional and `None` means "leave
/// it", which is what lets the page write one field per keystroke-batch without
/// echoing the rest of the node back.
#[derive(Debug, Clone, Default)]
pub struct NodePatch {
    pub text: Option<String>,
    /// `Some(None)` moves the node up to the first ring; `Some(Some(id))`
    /// reparents it under that node. Absent and null differ here.
    pub parent: Option<Option<String>>,
    pub position: Option<i64>,
    /// `Some(None)` clears the hand placement and returns the node to the layout —
    /// what "tidy up" sends.
    pub at: Option<Option<(f64, f64)>>,
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
     updated_at, version";
const NODE_COLS: &str = "id, mindmap, parent, text, position, x, y, promoted_kind, promoted_id, \
     created_by, created_at, updated_at";

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
        // Filled by the caller: a count is one more statement, and the read that
        // wants it says so.
        nodes: 0,
    })
}

fn row_to_node(r: &rusqlite::Row) -> rusqlite::Result<MindmapNode> {
    Ok(MindmapNode {
        id: r.get("id")?,
        mindmap: r.get("mindmap")?,
        parent: r.get("parent")?,
        text: r.get("text")?,
        position: r.get("position")?,
        x: r.get("x")?,
        y: r.get("y")?,
        promoted_kind: r.get("promoted_kind")?,
        promoted_id: r.get("promoted_id")?,
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
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

/// Validate one node's text against the cap that *is* the method.
pub fn validate_node_text(text: &str) -> ApiResult<String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::validation(
            "validation.mindmap_node_text",
            "A node needs some text — an empty thought is not one.",
        ));
    }
    let count = text.chars().count();
    if count > MAX_NODE_TEXT {
        return Err(ApiError::validation(
            "validation.mindmap_node_text",
            format!(
                "That node is {count} characters and the cap is {MAX_NODE_TEXT}. A mindmap node is a sentence or two — that brevity is what makes a branch readable at a glance."
            ),
        )
        .remedy(
            "Shorten it, split it into two nodes, or promote the branch to an initiative (POST /v1/mindmaps/{id}/nodes/{node}/promote) where the long form belongs.",
        ));
    }
    Ok(text)
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

fn require_node(conn: &Connection, map: &str, node: &str) -> ApiResult<MindmapNode> {
    conn.query_row(
        &format!("SELECT {NODE_COLS} FROM mindmap_nodes WHERE id = ?1 AND mindmap = ?2"),
        params![node, map],
        row_to_node,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("mindmap_node", node))
}

fn count_nodes(conn: &Connection, map: &str) -> ApiResult<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM mindmap_nodes WHERE mindmap = ?1",
        params![map],
        |r| r.get(0),
    )?)
}

/// How deep a node sits, counting the first ring as 1. Walks parents, bounded by
/// [`MAX_DEPTH`] + 1 so a cycle in a hand-edited database cannot spin here.
fn depth_of(conn: &Connection, node: &str) -> ApiResult<usize> {
    let mut depth = 1;
    let mut cursor = node.to_string();
    while depth <= MAX_DEPTH + 1 {
        let parent: Option<Option<String>> = conn
            .query_row(
                "SELECT parent FROM mindmap_nodes WHERE id = ?1",
                params![cursor],
                |r| r.get(0),
            )
            .optional()?;
        match parent.flatten() {
            None => return Ok(depth),
            Some(p) => {
                depth += 1;
                cursor = p;
            }
        }
    }
    Ok(depth)
}

/// The next free position among `parent`'s children, gapped.
fn next_position(conn: &Connection, map: &str, parent: Option<&str>) -> ApiResult<i64> {
    let highest: Option<i64> = match parent {
        Some(p) => conn.query_row(
            "SELECT MAX(position) FROM mindmap_nodes WHERE mindmap = ?1 AND parent = ?2",
            params![map, p],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT MAX(position) FROM mindmap_nodes WHERE mindmap = ?1 AND parent IS NULL",
            params![map],
            |r| r.get(0),
        )?,
    };
    Ok(highest.unwrap_or(0) + POSITION_GAP)
}

/// Every node in `map`, ordered parent-first then by position — the order a
/// caller can build a tree from in one pass.
fn all_nodes(conn: &Connection, map: &str) -> ApiResult<Vec<MindmapNode>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NODE_COLS} FROM mindmap_nodes WHERE mindmap = ?1 ORDER BY position, created_at, id"
    ))?;
    let rows = stmt
        .query_map(params![map], row_to_node)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `node` and everything beneath it, depth-first in sibling order.
///
/// One read of the whole map rather than a query per level: a map is capped at
/// [`MAX_NODES`], so walking it in memory is cheaper than the recursion it saves
/// — and it is the same list the caller usually already holds.
pub(crate) fn subtree(nodes: &[MindmapNode], root: &str) -> Vec<MindmapNode> {
    let mut children: HashMap<Option<String>, Vec<&MindmapNode>> = HashMap::new();
    for n in nodes {
        children.entry(n.parent.clone()).or_default().push(n);
    }
    let mut out = Vec::new();
    let mut stack: Vec<String> = vec![root.to_string()];
    while let Some(id) = stack.pop() {
        if let Some(node) = nodes.iter().find(|n| n.id == id) {
            out.push(node.clone());
        }
        if let Some(kids) = children.get(&Some(id)) {
            // Reversed onto the stack so siblings come back in position order.
            for kid in kids.iter().rev() {
                stack.push(kid.id.clone());
            }
        }
    }
    out
}

/// A subtree as indented text — the shape a model reads and writes cheapest, and
/// what an initiative gets seeded with when a branch graduates.
pub(crate) fn outline(nodes: &[MindmapNode], root: &str) -> String {
    let mut depth: HashMap<String, usize> = HashMap::new();
    let mut out = String::new();
    for node in subtree(nodes, root) {
        let level = match &node.parent {
            Some(p) => depth.get(p).copied().map(|d| d + 1).unwrap_or(0),
            None => 0,
        };
        depth.insert(node.id.clone(), level);
        out.push_str(&"  ".repeat(level));
        out.push_str("- ");
        out.push_str(&node.text);
        out.push('\n');
    }
    out
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
                map.nodes = count_nodes(conn, &map.id)?;
            }
            Ok((maps, total))
        })
    }

    /// A map and every node on it — one request, because the canvas needs the
    /// whole tree to draw anything and the map is capped at [`MAX_NODES`].
    pub fn get_mindmap(&self, id: &str) -> ApiResult<Option<(Mindmap, Vec<MindmapNode>)>> {
        self.with_conn(|conn| {
            let found = conn
                .query_row(
                    &format!("SELECT {MAP_COLS} FROM mindmaps WHERE id = ?1"),
                    params![id],
                    row_to_map,
                )
                .optional()?;
            let Some(mut map) = found else {
                return Ok(None);
            };
            let nodes = all_nodes(conn, id)?;
            map.nodes = nodes.len() as i64;
            Ok(Some((map, nodes)))
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
            map.nodes = count_nodes(tx, id)?;
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
            let nodes = count_nodes(tx, id)?;
            tx.execute("DELETE FROM mindmap_nodes WHERE mindmap = ?1", params![id])?;
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

    /// Add thoughts — one when a person types, many when an agent is brainstorming
    /// with them. Returns them in the order they were asked for.
    ///
    /// One transaction for the batch, so ten nodes from one agent turn either all
    /// land or none do; a caller reading the map back never sees half a thought.
    pub fn grow_mindmap(
        &self,
        id: &str,
        adds: &[NodeAdd],
        actor: &str,
    ) -> ApiResult<Vec<MindmapNode>> {
        if adds.is_empty() {
            return Err(ApiError::validation(
                "validation.mindmap_nodes",
                "Send at least one node to add.",
            ));
        }
        if adds.len() > MAX_GROW {
            return Err(ApiError::validation(
                "validation.mindmap_nodes",
                format!(
                    "{} nodes is over the cap of {MAX_GROW} per call. Each one costs statements inside the transaction that serializes every claim and transition in the store, so a batch has to be bounded.",
                    adds.len()
                ),
            ));
        }
        // Validated before the transaction opens: a batch that cannot land should
        // not hold the write mutex while finding that out.
        let texts = adds
            .iter()
            .map(|a| validate_node_text(&a.text))
            .collect::<ApiResult<Vec<_>>>()?;
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, id)?;
            ensure_project_writable(tx, &map.project)?;
            let existing = count_nodes(tx, id)?;
            if existing + adds.len() as i64 > MAX_NODES {
                return Err(ApiError::conflict(
                    "mindmap.full",
                    format!(
                        "This map holds {existing} nodes and the cap is {MAX_NODES}. A brainstorm this big has stopped being one — promote its branches into initiatives or epics, or start a second map."
                    ),
                ));
            }
            let mut out = Vec::with_capacity(adds.len());
            for (add, text) in adds.iter().zip(texts) {
                // A parent must be on THIS map: a node reparented across maps
                // would put a branch somewhere its own map cannot see.
                if let Some(parent) = &add.parent {
                    require_node(tx, id, parent)?;
                    let depth = depth_of(tx, parent)?;
                    if depth >= MAX_DEPTH {
                        return Err(ApiError::conflict(
                            "mindmap.too_deep",
                            format!(
                                "That would nest {} levels deep and the cap is {MAX_DEPTH}. Past this nobody can read the shape — promote the branch instead.",
                                depth + 1
                            ),
                        ));
                    }
                }
                let position = match add.position {
                    Some(p) => p,
                    None => next_position(tx, id, add.parent.as_deref())?,
                };
                let node_id = mindmap_node_id();
                tx.execute(
                    "INSERT INTO mindmap_nodes (id, mindmap, parent, text, position, created_by, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![node_id, id, add.parent, text, position, actor, now],
                )?;
                out.push(MindmapNode {
                    id: node_id,
                    mindmap: id.to_string(),
                    parent: add.parent.clone(),
                    text,
                    position,
                    x: None,
                    y: None,
                    promoted_kind: None,
                    promoted_id: None,
                    created_by: actor.to_string(),
                    created_at: now,
                    updated_at: now,
                });
            }
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            // One event for the batch, not one per node: ten nodes from an agent
            // turn are one act of brainstorming, and a log that says so is
            // readable where a hundred lines are not.
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_grown",
                json!({ "mindmap": id, "nodes": out.len() }),
                now,
            )?;
            Ok(out)
        })
    }

    /// Edit one node: its text, where it hangs, or where it sits.
    ///
    /// Only a **reparent** reaches the event log. Text and placement change
    /// constantly while somebody is thinking — that is the whole point of the
    /// surface — and an event per keystroke-batch would bury every other event in
    /// the project under one person's typing. Moving a branch is different: it
    /// changes what the map says, and it is the edit somebody may want to find
    /// again.
    pub fn patch_mindmap_node(
        &self,
        map_id: &str,
        node_id: &str,
        patch: &NodePatch,
        actor: &str,
    ) -> ApiResult<MindmapNode> {
        let text = patch.text.as_deref().map(validate_node_text).transpose()?;
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, map_id)?;
            ensure_project_writable(tx, &map.project)?;
            let mut node = require_node(tx, map_id, node_id)?;
            let was_under = node.parent.clone();

            if let Some(parent) = &patch.parent {
                if let Some(parent_id) = parent {
                    if parent_id == node_id {
                        return Err(ApiError::conflict(
                            "mindmap.cycle",
                            "A node cannot hang off itself.",
                        ));
                    }
                    require_node(tx, map_id, parent_id)?;
                    // The guard that matters: dropping a node onto its own
                    // descendant would cut that whole branch off the map, and it
                    // is exactly what a drag makes easy to try.
                    let nodes = all_nodes(tx, map_id)?;
                    if subtree(&nodes, node_id).iter().any(|n| n.id == *parent_id) {
                        return Err(ApiError::conflict(
                            "mindmap.cycle",
                            "That would hang a node off one of its own children, which would cut the branch off the map.",
                        ));
                    }
                    let depth = depth_of(tx, parent_id)?;
                    if depth >= MAX_DEPTH {
                        return Err(ApiError::conflict(
                            "mindmap.too_deep",
                            format!(
                                "That would nest {} levels deep and the cap is {MAX_DEPTH}.",
                                depth + 1
                            ),
                        ));
                    }
                }
                node.parent = parent.clone();
                // A move without an explicit position lands at the end of its new
                // ring, which is where a dropped node visually goes.
                if patch.position.is_none() {
                    node.position = next_position(tx, map_id, node.parent.as_deref())?;
                }
            }
            if let Some(text) = text {
                node.text = text;
            }
            if let Some(position) = patch.position {
                node.position = position;
            }
            if let Some(at) = &patch.at {
                match at {
                    Some((x, y)) => {
                        node.x = Some(*x);
                        node.y = Some(*y);
                    }
                    // Back to the layout — what "tidy up" sends.
                    None => {
                        node.x = None;
                        node.y = None;
                    }
                }
            }
            tx.execute(
                "UPDATE mindmap_nodes SET parent = ?3, text = ?4, position = ?5, x = ?6, y = ?7, \
                 updated_at = ?8 WHERE id = ?1 AND mindmap = ?2",
                params![
                    node_id,
                    map_id,
                    node.parent,
                    node.text,
                    node.position,
                    node.x,
                    node.y,
                    now
                ],
            )?;
            node.updated_at = now;
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![map_id, now],
            )?;
            // Structural change only — see the note on this method.
            if was_under != node.parent {
                emit_event(
                    tx,
                    None,
                    Some(&map.project),
                    actor,
                    "mindmap_moved",
                    json!({
                        "mindmap": map_id,
                        "node": node_id,
                        "from": was_under,
                        "to": node.parent,
                    }),
                    now,
                )?;
            }
            Ok(node)
        })
    }

    /// Remove a node and everything under it. Returns how many went.
    pub fn delete_mindmap_node(&self, map_id: &str, node_id: &str, actor: &str) -> ApiResult<i64> {
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, map_id)?;
            ensure_project_writable(tx, &map.project)?;
            require_node(tx, map_id, node_id)?;
            let nodes = all_nodes(tx, map_id)?;
            let doomed = subtree(&nodes, node_id);
            // Deepest first. `subtree` returns pre-order (a parent before its
            // children), and deleting a parent while its children still point at
            // it trips the foreign key — so the delete walks it backwards.
            for node in doomed.iter().rev() {
                tx.execute("DELETE FROM mindmap_nodes WHERE id = ?1", params![node.id])?;
            }
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![map_id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_pruned",
                json!({ "mindmap": map_id, "node": node_id, "removed": doomed.len() }),
                now,
            )?;
            Ok(doomed.len() as i64)
        })
    }

    /// Graduate a branch: it becomes an epic with its children as tickets, or an
    /// initiative seeded with the subtree.
    ///
    /// **The node stays.** Promotion is not a move — the map is the record of how
    /// the thinking got there, and a branch that vanished the moment it mattered
    /// would make the map worthless as the thing you read afterwards. What the
    /// node gains is a link to what it became, which is what lets a map keep
    /// earning its place as navigation once the brainstorming is over.
    ///
    /// Refused if the node already graduated: promoting twice would silently make
    /// two epics from one thought, and the second would look exactly as legitimate
    /// as the first.
    ///
    /// Everything happens in one transaction, and the rows are written with SQL
    /// here rather than by calling `create_ticket` / `create_initiative` — the
    /// write mutex is not reentrant, so a nested `with_tx` would deadlock. This is
    /// the same reason `schedules::materialize_one` inserts its ticket inline.
    pub fn promote_mindmap_node(
        &self,
        map_id: &str,
        node_id: &str,
        target: &str,
        actor: &str,
    ) -> ApiResult<(MindmapNode, Value)> {
        if !PROMOTION_TARGETS.contains(&target) {
            return Err(ApiError::validation(
                "validation.mindmap_target",
                format!(
                    "Unknown promotion target '{target}'. Use 'epic' (an epic with its children as tickets — the fastest path to work) or 'initiative' (a direction that needs nurturing first)."
                ),
            ));
        }
        let now = now_ms();
        self.with_tx(|tx| {
            let map = require_map(tx, map_id)?;
            ensure_project_writable(tx, &map.project)?;
            let mut node = require_node(tx, map_id, node_id)?;
            if let (Some(kind), Some(id)) = (&node.promoted_kind, &node.promoted_id) {
                return Err(ApiError::conflict(
                    "mindmap.already_promoted",
                    format!(
                        "That branch already became {kind} '{id}'. Promoting it again would make a second one from the same thought, indistinguishable from the first."
                    ),
                ));
            }
            let nodes = all_nodes(tx, map_id)?;
            let created = match target {
                "epic" => {
                    let epic = insert_ticket(
                        tx,
                        &map.project,
                        "epic",
                        None,
                        &node.text,
                        &format!(
                            "From the mindmap “{}” ({map_id}).\n\n{}",
                            map.title,
                            outline(&nodes, node_id)
                        ),
                        actor,
                        now,
                    )?;
                    // Direct children only. A deeper subtree would arrive as a
                    // flat pile of tickets whose shape nobody could recover,
                    // whereas the map keeps that shape for whoever wants it.
                    let mut children = Vec::new();
                    for child in nodes.iter().filter(|n| n.parent.as_deref() == Some(node_id)) {
                        let body = outline(&nodes, &child.id);
                        let id = insert_ticket(
                            tx,
                            &map.project,
                            "task",
                            Some(&epic),
                            &child.text,
                            &body,
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
                            node.text,
                            json!({ "mindmap": map_id, "node": node_id }).to_string(),
                            actor,
                            now
                        ],
                    )?;
                    // The subtree becomes the first entry rather than the summary:
                    // an entry carries provenance, and where an idea came from is
                    // exactly what a collection is supposed to remember.
                    let entry = crate::ids::initiative_entry_id();
                    let text = outline(&nodes, node_id);
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
                            format!("mindmap:{map_id}"),
                            json!({ "mindmap": map_id, "node": node_id }).to_string(),
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
                        json!({ "initiative": id, "title": node.text, "mindmap": map_id }),
                        now,
                    )?;
                    json!({ "kind": "initiative", "id": id })
                }
            };

            let kind = created["kind"].as_str().unwrap_or(target).to_string();
            let created_id = created["id"].as_str().unwrap_or_default().to_string();
            tx.execute(
                "UPDATE mindmap_nodes SET promoted_kind = ?3, promoted_id = ?4, updated_at = ?5 \
                 WHERE id = ?1 AND mindmap = ?2",
                params![node_id, map_id, kind, created_id, now],
            )?;
            node.promoted_kind = Some(kind.clone());
            node.promoted_id = Some(created_id.clone());
            node.updated_at = now;
            tx.execute(
                "UPDATE mindmaps SET updated_at = ?2 WHERE id = ?1",
                params![map_id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&map.project),
                actor,
                "mindmap_promoted",
                json!({ "mindmap": map_id, "node": node_id, "kind": kind, "id": created_id }),
                now,
            )?;
            Ok((node, created))
        })
    }

    /// The subtree under `node`, as indented text. What an agent reads, and what
    /// seeds an initiative when a branch graduates.
    pub fn mindmap_outline(&self, map_id: &str, node_id: Option<&str>) -> ApiResult<String> {
        self.with_conn(|conn| {
            let map = require_map(conn, map_id)?;
            let nodes = all_nodes(conn, map_id)?;
            match node_id {
                Some(root) => Ok(outline(&nodes, root)),
                None => {
                    // The whole map, with its title as the root line — the shape a
                    // person recognises as "the map".
                    let mut out = format!("# {}\n", map.title);
                    for top in nodes.iter().filter(|n| n.parent.is_none()) {
                        out.push_str(&outline(&nodes, &top.id));
                    }
                    Ok(out)
                }
            }
        })
    }
}
