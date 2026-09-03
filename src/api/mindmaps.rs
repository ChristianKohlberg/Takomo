//! /v1/mindmaps — brainstorming, before any of it is an idea.
//!
//! The read that matters is `GET /v1/mindmaps/{id}`: it returns the map **and
//! every node on it** in one request, because a canvas cannot draw half a tree and
//! a map is capped at 500 nodes precisely so this is affordable.
//!
//! The write that matters is `POST /v1/mindmaps/{id}/nodes`, which takes a
//! **batch**. One node is what a person typing sends; ten is what an agent
//! brainstorming with them sends, and that is the shape the surface is built
//! around rather than a special case bolted on.
//!
//! The nodes are not rows. Each map is one Yjs document (`store::mindmapdoc`),
//! so a person dragging a node, a second person typing into another, and an
//! agent adding a whole branch are all writing to ONE replica — and each of them
//! sees the others without reloading. Every write below therefore goes through
//! `open_room` + `mutate`, exactly as `/documents` does, rather than through SQL.
//!
//! See `src/store/mindmaps.rs` for what a mindmap deliberately is not.

use super::{
    body_object, first, get_f64, get_i64, get_str, paged, parse_i64_param, query_pairs,
    reject_unknown, require_str, ApiJson,
};
use crate::api::docsync::open_room;
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::mindmapdoc::{self, NodeAdd, NodePatch};
use crate::store::{
    BranchPromotion, MindmapCreate, MindmapListFilter, MindmapPatch, MAX_MINDMAPS_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 4] = ["project", "title", "summary", "metadata"];
const PATCH_FIELDS: [&str; 4] = ["title", "summary", "status", "metadata_merge"];
const NODE_FIELDS: [&str; 7] = [
    "parent",
    "text",
    "title",
    "notes",
    "position",
    "kind",
    "edge_label",
];
const NODE_PATCH_FIELDS: [&str; 11] = [
    "text",
    "title",
    "notes",
    "parent",
    "position",
    "at",
    "kind",
    "edge_label",
    "color",
    "shape",
    "icons",
];
const RELATIONSHIP_FIELDS: [&str; 3] = ["from", "to", "label"];
const ATTACHMENT_FIELDS: [&str; 4] = ["kind", "name", "gist", "ref"];

const DEFAULT_LIMIT: i64 = 50;

/// GET /v1/mindmaps?project=&status=&q=&limit=&offset= (read).
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    if let Some(project) = first(&pairs, "project") {
        ctx.require_project(project)?;
    }
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_MINDMAPS_PAGE);
    let offset = parse_i64_param(&pairs, "offset")?.unwrap_or(0).max(0);
    let filter = MindmapListFilter {
        project: first(&pairs, "project").map(str::to_string),
        allowed_projects: ctx.allowed_projects_vec(),
        status: first(&pairs, "status").map(str::to_string),
        q: first(&pairs, "q").map(str::to_string),
        limit,
        offset,
    };
    let (maps, total) = state.store.list_mindmaps(&filter)?;
    Ok(Json(paged(
        maps.iter().map(|m| m.to_json()).collect(),
        total,
        limit,
        "Raise 'limit' (max 200) or page with 'offset'; the newest-touched map is first.",
    )))
}

/// POST /v1/mindmaps (write) — start one. `{"project":"tp","title":"Payments rebuild"}`.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &CREATE_FIELDS)?;
    let project = require_str(obj, "project")?;
    ctx.require_project(&project)?;
    let req = MindmapCreate {
        title: require_str(obj, "title")?,
        summary: get_str(obj, "summary")?,
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
    };
    let map = state.store.create_mindmap(&project, &req, &ctx.actor)?;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "mindmap": map.to_json(),
            "nodes": [],
            "note": format!(
                "Grow it with POST /v1/mindmaps/{}/nodes {{\"nodes\":[{{\"text\":\"…\"}}]}} — a batch, so an agent can add a whole branch in one call. A node is a sentence or two; when one is worth keeping, promote it to an epic or an initiative.",
                map.id
            ),
        })),
    ))
}

/// PATCH /v1/mindmaps/{id} (write) — title, summary, status, metadata.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;
    let existing = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&existing.project)?;
    let patch = MindmapPatch {
        title: get_str(obj, "title")?,
        summary: get_str(obj, "summary")?,
        status: get_str(obj, "status")?,
        metadata_merge: obj.get("metadata_merge").filter(|v| !v.is_null()).cloned(),
    };
    let map = state.store.patch_mindmap(&id, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "mindmap": map.to_json() })))
}

/// DELETE /v1/mindmaps/{id} (write) — throw it away, nodes and all.
///
/// An ordinary thing to do, and the clearest statement of what a mindmap is. What
/// its branches *became* is untouched: those graduated and are work in their own
/// right. The response says how many nodes went, so a caller that deleted the
/// wrong map knows immediately.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&existing.project)?;
    let nodes = state.store.delete_mindmap(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "ok": true, "removed_nodes": nodes })))
}

/// Write the replica's pending edits to the log before answering.
///
/// **An API call that returns 201 has to have happened.** The debounced flusher
/// exists for somebody typing in a browser, where two seconds of batching is the
/// difference between a smooth canvas and every keystroke queued behind the
/// process-wide write mutex. A request is not typing: it makes one change and
/// then says it did.
///
/// Without this the room can be torn down — the last peer leaves when this
/// handler's guard drops — while its edits are still only in memory, and the
/// next request rebuilds the replica from a log that never received them. The
/// change is not lost for long, but it is missing from the read that follows,
/// which is the shape of bug that looks like a flaky test until somebody loses
/// a node.
async fn persist(state: &Arc<AppState>, room: &crate::api::docsync::RoomGuard, ctx: &AuthCtx) {
    crate::api::docsync::flush(state, room, &ctx.actor).await;
}

/// Fetch the map for its project check, then join its live document.
///
/// The row is read first and the room second, so an unknown map or a project the
/// caller cannot reach is a 404 or a 403 rather than an empty canvas.
async fn join(
    state: &Arc<AppState>,
    ctx: &AuthCtx,
    id: &str,
) -> ApiResult<(crate::store::Mindmap, crate::api::docsync::RoomGuard)> {
    let map = state
        .store
        .get_mindmap(id)?
        .ok_or_else(|| ApiError::not_found("mindmap", id))?;
    ctx.require_project(&map.project)?;
    let room = open_room(state, id).await?;
    Ok((map, room))
}

/// GET /v1/mindmaps/{id} (read) — the map and everything on it.
///
/// One request, because a canvas cannot draw half a tree — affordable precisely
/// because a map is capped, which is a better contract than paging a shape.
///
/// Read from the LIVE replica, not from the log, so what comes back includes the
/// edits somebody is making right now.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (mut map, room) = join(&state, &ctx, &id).await?;
    let (nodes, relationships, _) = room.read(|doc| mindmapdoc::snapshot(doc, &id));
    // Counted from the live replica, so this response is always exact. It is
    // NOT written back to the row: this is a `read`-scope route, and taking the
    // process-wide write mutex — the one that makes the ready queue's
    // exactly-one-claimant guarantee work — to refresh a cached number would be
    // a write nobody asked for. The flusher keeps the row current instead.
    map.nodes = nodes.len() as i64;
    let total = nodes.len();
    Ok(Json(json!({
        "mindmap": map.to_json(),
        "nodes": nodes,
        "relationships": relationships,
        "total": total,
    })))
}

/// GET /v1/mindmaps/{id}/outline?node= (read) — the map as indented text.
///
/// The cheapest shape for a model to reason about, and the one to read before
/// adding to a map you did not build.
pub async fn outline(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (map, room) = join(&state, &ctx, &id).await?;
    let pairs = query_pairs(raw.as_deref());
    let node = first(&pairs, "node").map(str::to_string);
    let text = room.read(|doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, &id);
        match node.as_deref() {
            // An unknown node reads as empty rather than 404. It has always
            // behaved this way, and a reader asking for a branch that has just
            // been pruned wants "nothing there", not an error.
            Some(node) => mindmapdoc::outline(&nodes, node),
            None => mindmapdoc::full_outline(&nodes, &map.title),
        }
    });
    Ok(Json(json!({ "mindmap": id, "outline": text })))
}

/// Read one node's JSON back out of the document after a write.
fn node_json(room: &crate::api::docsync::RoomGuard, map_id: &str, node_id: &str) -> Option<Value> {
    let (nodes, _, _) = room.read(|doc| mindmapdoc::snapshot(doc, map_id));
    nodes
        .into_iter()
        .find(|n| n["id"].as_str() == Some(node_id))
}

/// Who is writing, as far as the credential is concerned.
///
/// **Derived, never accepted from the body.** `origin` exists so a map can
/// eventually show which thoughts a person actually had, and a field a caller
/// can simply claim would say nothing. The `human` scope is what a person's
/// token carries; everything else is something automated, which is the same
/// distinction `case_verdicts.actor_kind` already draws.
fn origin_of(ctx: &AuthCtx) -> String {
    if ctx.require_scope("human").is_ok() {
        "human".to_string()
    } else {
        "agent".to_string()
    }
}

fn parse_node_add(obj: &serde_json::Map<String, Value>, origin: &str) -> ApiResult<NodeAdd> {
    reject_unknown(obj, &NODE_FIELDS)?;
    // `title` is the field's name now; `text` is what every existing caller
    // sends. Both are accepted and mean the same thing — renaming a field is not
    // a good enough reason to break somebody's script.
    let title = match get_str(obj, "title")? {
        Some(title) => title,
        None => require_str(obj, "text")?,
    };
    Ok(NodeAdd {
        parent: get_str(obj, "parent")?,
        title,
        notes: get_str(obj, "notes")?,
        position: get_i64(obj, "position")?.map(|p| p.max(0) as usize),
        kind: get_str(obj, "kind")?,
        origin: Some(origin.to_string()),
        edge_label: get_str(obj, "edge_label")?,
    })
}

/// POST /v1/mindmaps/{id}/nodes (write) — add a batch.
///
/// A batch, because that is what an agent adding a branch sends while somebody
/// is still talking. It lands whole or not at all: half a branch would leave a
/// map nobody asked for and no way to tell which half.
pub async fn add_nodes(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    let origin = origin_of(&ctx);

    let adds: Vec<NodeAdd> = match obj.get("nodes") {
        Some(Value::Array(items)) => {
            reject_unknown(obj, &["nodes"])?;
            let mut adds = Vec::with_capacity(items.len());
            for item in items {
                let entry = item.as_object().ok_or_else(|| {
                    ApiError::validation(
                        "validation.mindmap_nodes",
                        "Every entry in 'nodes' must be an object like {\"text\":\"…\",\"parent\":\"mn-…\"}.",
                    )
                })?;
                adds.push(parse_node_add(entry, &origin)?);
            }
            adds
        }
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_nodes",
                "Field 'nodes' must be an array of {text, parent?, position?} objects.",
            ))
        }
        None => vec![parse_node_add(obj, &origin)?],
    };

    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let actor = ctx.actor.clone();
    let created = room.mutate(|doc| mindmapdoc::add_nodes(doc, &adds, &actor))?;
    persist(&state, &room, &ctx).await;

    let (all, _, _) = room.read(|doc| mindmapdoc::snapshot(doc, &id));
    let nodes: Vec<Value> = created
        .iter()
        .filter_map(|(node_id, _)| {
            all.iter()
                .find(|n| n["id"].as_str() == Some(node_id.as_str()))
                .cloned()
        })
        .collect();

    state.store.note_mindmap_size(&id, all.len() as i64)?;
    // One event for the batch, not one per node: ten nodes from an agent turn
    // are one act of brainstorming.
    state.store.note_mindmap_event(
        &id,
        crate::store::MindmapChange::Grown,
        json!({ "mindmap": id, "nodes": nodes.len() }),
        &ctx.actor,
    )?;
    state.wake();
    Ok((StatusCode::CREATED, Json(json!({ "nodes": nodes }))))
}

/// PATCH /v1/mindmaps/{id}/nodes/{node} (write).
pub async fn patch_node(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &NODE_PATCH_FIELDS)?;

    let parent = match obj.get("parent") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_parent",
                "Field 'parent' must be a node id, or null to hang it off the root.",
            ))
        }
    };

    let at = match obj.get("at") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Object(point)) => {
            let x = get_f64(point, "x")?;
            let y = get_f64(point, "y")?;
            match (x, y) {
                (Some(x), Some(y)) => Some(Some((x, y))),
                _ => {
                    return Err(ApiError::validation(
                        "validation.mindmap_at",
                        "Field 'at' needs both 'x' and 'y' numbers, or null to let the layout place the node.",
                    ))
                }
            }
        }
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_at",
                "Field 'at' must be {\"x\":…,\"y\":…} or null.",
            ))
        }
    };

    let icons = match obj.get("icons") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect::<Vec<String>>(),
        ),
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_icons",
                "Field 'icons' must be an array of strings.",
            ))
        }
    };

    let patch = NodePatch {
        title: match get_str(obj, "title")? {
            Some(title) => Some(title),
            None => get_str(obj, "text")?,
        },
        notes: get_str(obj, "notes")?,
        parent,
        position: get_i64(obj, "position")?.map(|p| p.max(0) as usize),
        at,
        kind: get_str(obj, "kind")?,
        edge_label: get_str(obj, "edge_label")?,
        color: get_str(obj, "color")?,
        shape: get_str(obj, "shape")?,
        icons,
        reviewed: obj.get("reviewed").and_then(Value::as_bool),
    };

    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let moved = patch.parent.is_some();
    let actor = ctx.actor.clone();
    room.mutate(|doc| mindmapdoc::patch_node(doc, &node, &patch, &actor))?;
    persist(&state, &room, &ctx).await;

    if moved {
        // A reparent is the only node edit that reaches the event log. Text and
        // placement change constantly while somebody is thinking.
        state.store.note_mindmap_event(
            &id,
            crate::store::MindmapChange::Moved,
            json!({ "mindmap": id, "node": node }),
            &ctx.actor,
        )?;
    }
    state.wake();
    let json =
        node_json(&room, &id, &node).ok_or_else(|| ApiError::not_found("mindmap_node", &node))?;
    Ok(Json(json!({ "node": json })))
}

/// DELETE /v1/mindmaps/{id}/nodes/{node} (write) — prune a branch.
pub async fn delete_node(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let removed = room.mutate(|doc| mindmapdoc::delete_node(doc, &node))?;
    persist(&state, &room, &ctx).await;
    let (all, _, _) = room.read(|doc| mindmapdoc::snapshot(doc, &id));
    state.store.note_mindmap_size(&id, all.len() as i64)?;
    state.store.note_mindmap_event(
        &id,
        crate::store::MindmapChange::Pruned,
        json!({ "mindmap": id, "node": node, "removed": removed }),
        &ctx.actor,
    )?;
    state.wake();
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

/// POST /v1/mindmaps/{id}/nodes/{node}/promote (write) — graduate a branch.
///
/// The node STAYS and keeps a link to what it became. Promotion is not a move:
/// the map is the record of how the thinking got there, and a branch that
/// vanished the moment it mattered would make the map worthless as the thing you
/// read afterwards.
pub async fn promote(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["target"])?;
    let target = require_str(obj, "target")?;
    // Before anything is read: a caller who named a target that does not exist
    // got the verb wrong, and telling them the branch is already promoted would
    // send them to fix the wrong thing.
    crate::store::validate_promotion_target(&target)?;

    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let store = state.clone();
    let actor = ctx.actor.clone();
    let node_id = node.clone();
    let map_id = id.clone();

    // All of it inside ONE mutation, and in this order: read the branch, make
    // the work, then write the link. A link written first would point at nothing
    // if the work behind it failed, and no link at all would let the same
    // thought become a second, indistinguishable epic on the next attempt.
    let created = room.mutate(move |doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, &map_id);
        let ordered = mindmapdoc::tree_order(&nodes);
        let branch = ordered
            .iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| ApiError::not_found("mindmap_node", &node_id))?;

        if let (Some(kind), Some(existing)) = (&branch.promoted_kind, &branch.promoted_id) {
            return Err(ApiError::conflict(
                "mindmap.already_promoted",
                format!(
                    "That branch already became {kind} '{existing}'. Promoting it again would make a second one from the same thought, indistinguishable from the first."
                ),
            ));
        }

        let title = branch.title.clone();
        let branch_outline = mindmapdoc::outline(&nodes, &node_id);
        let children: Vec<(String, String)> = ordered
            .iter()
            .filter(|n| n.parent.as_deref() == Some(node_id.as_str()))
            .map(|child| (child.title.clone(), mindmapdoc::outline(&nodes, &child.id)))
            .collect();

        let created = store.store.promote_branch(
            &BranchPromotion {
                map_id: &map_id,
                node_id: &node_id,
                target: &target,
                title: &title,
                branch_outline: &branch_outline,
                children: &children,
            },
            &actor,
        )?;

        let kind = created["kind"].as_str().unwrap_or_default();
        let created_id = created["id"].as_str().unwrap_or_default();
        mindmapdoc::set_promoted(doc, &node_id, kind, created_id)?;
        Ok(created)
    })?;

    state.wake();
    let json =
        node_json(&room, &id, &node).ok_or_else(|| ApiError::not_found("mindmap_node", &node))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "node": json, "created": created })),
    ))
}

/// POST /v1/mindmaps/{id}/relationships (write) — link two nodes.
///
/// An edge that is NOT part of the hierarchy. The tree answers "what is this
/// part of"; this answers everything else — a question hanging off the thing it
/// questions, a screen that navigates to another, a plain "see also".
pub async fn add_relationship(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &RELATIONSHIP_FIELDS)?;
    let from = require_str(obj, "from")?;
    let to = require_str(obj, "to")?;
    let label = get_str(obj, "label")?.unwrap_or_default();

    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let created = room.mutate(|doc| mindmapdoc::add_relationship(doc, &from, &to, &label))?;
    persist(&state, &room, &ctx).await;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "relationship": {
                "id": created.id,
                "from": created.from,
                "to": created.to,
                "label": created.label,
            }
        })),
    ))
}

/// DELETE /v1/mindmaps/{id}/relationships/{relationship} (write).
pub async fn delete_relationship(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, relationship)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;
    room.mutate(|doc| mindmapdoc::delete_relationship(doc, &relationship))?;
    persist(&state, &room, &ctx).await;
    state.wake();
    Ok(Json(json!({ "ok": true })))
}

/// POST /v1/mindmaps/{id}/nodes/{node}/attachments (write) — point at something.
///
/// A POINTER, never the bytes. Files in a CRDT log are replayed by every peer
/// that joins, so a map with a PDF inside it would get slower to open for
/// everybody, forever.
pub async fn add_attachment(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &ATTACHMENT_FIELDS)?;
    let kind = require_str(obj, "kind")?;
    let name = require_str(obj, "name")?;
    let gist = get_str(obj, "gist")?.unwrap_or_default();
    let reference = get_str(obj, "ref")?.unwrap_or_default();

    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let created =
        room.mutate(|doc| mindmapdoc::add_attachment(doc, &node, &kind, &name, &gist, &reference))?;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "attachment": {
                "id": created.id,
                "kind": created.kind,
                "name": created.name,
                "gist": created.gist,
                "ref": created.reference,
            }
        })),
    ))
}

/// DELETE /v1/mindmaps/{id}/nodes/{node}/attachments/{attachment} (write).
pub async fn delete_attachment(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node, attachment)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let (_, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;
    room.mutate(|doc| mindmapdoc::delete_attachment(doc, &node, &attachment))?;
    persist(&state, &room, &ctx).await;
    state.wake();
    Ok(Json(json!({ "ok": true })))
}

/// POST /v1/mindmaps/{id}/documents (write) — write the map up.
///
/// The map becomes a TREE OF DOCUMENTS, one per thought that has something to
/// say, filed under folders that mirror the branches. `/documents` already
/// builds its tree from paths, so a map needs no section model inside a single
/// document to convert into something navigable — the folder tree is the
/// outline.
///
/// The rule that makes the result readable: a node becomes a document when it
/// has children or notes; a bare leaf becomes a **bullet** in its parent's
/// document. Without it, every six-word thought is its own page and a map of
/// forty converts into forty documents nobody opens.
///
/// **Running it again refiles, it never rewrites.** A node that already made a
/// document keeps it: the title and folder are brought back in line with the
/// map, and the prose is not touched, because somebody has probably been
/// writing in there and that is the entire point of turning a map into
/// documents.
pub async fn to_documents(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["path"])?;

    let (map, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    // The map's own title is the folder everything lands under, unless the
    // caller says otherwise. `""` puts it at the top level.
    let root = match get_str(obj, "path")? {
        Some(path) => path,
        None => map.title.clone(),
    };

    let plan = room.read(|doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, &id);
        mindmapdoc::plan_documents(&nodes, &root)
    });

    let mut made = Vec::new();
    let mut created = 0usize;
    let mut refiled = 0usize;

    // The plan's own front page.
    //
    // Without it the top of the outline is a FOLDER — a row you cannot open,
    // sitting above your whole plan. The map's title already names that folder,
    // so the document belongs at its parent with that name, which is exactly the
    // fold `/documents` performs to turn a folder and its namesake into one
    // section. Its id lives in the map's metadata, because a map has no root
    // node to hang it from: every first-ring branch has `parent: null`.
    if !root.is_empty() {
        let (parent, name) = match root.rsplit_once('/') {
            Some((parent, name)) => (parent.to_string(), name.to_string()),
            None => (String::new(), root.clone()),
        };
        let existing = map
            .metadata
            .get("document")
            .and_then(Value::as_str)
            .and_then(|doc_id| state.store.get_document(doc_id).ok());

        let front = match existing {
            Some(document) => {
                refiled += 1;
                state.store.patch_document(
                    &document.id,
                    &crate::store::DocumentPatch {
                        title: Some(name.clone()),
                        path: Some(parent.clone()),
                        ..Default::default()
                    },
                    &ctx.actor,
                )?
            }
            None => {
                let document = state.store.create_document(
                    &crate::store::DocumentCreate {
                        project: map.project.clone(),
                        title: name.clone(),
                        path: Some(parent.clone()),
                        ..Default::default()
                    },
                    &ctx.actor,
                )?;
                if !map.summary.trim().is_empty() {
                    let update = crate::store::prose::initial_update(&[
                        crate::store::prose::Block::Paragraph(map.summary.trim().to_string()),
                    ]);
                    state
                        .store
                        .append_collab_update(&document.id, &update, &ctx.actor)?;
                }
                state.store.patch_mindmap(
                    &id,
                    &MindmapPatch {
                        metadata_merge: Some(json!({ "document": document.id })),
                        ..Default::default()
                    },
                    &ctx.actor,
                )?;
                created += 1;
                document
            }
        };
        made.push(json!({
            "node": Value::Null,
            "document": front.id,
            "title": front.title,
            "path": front.path,
        }));
    }

    for entry in &plan {
        let existing = entry
            .existing
            .as_deref()
            .and_then(|doc_id| state.store.get_document(doc_id).ok());

        let document = match existing {
            Some(document) => {
                // Refile only. Its prose belongs to whoever has been writing it.
                refiled += 1;
                state.store.patch_document(
                    &document.id,
                    &crate::store::DocumentPatch {
                        title: Some(entry.title.clone()),
                        path: Some(entry.path.clone()),
                        ..Default::default()
                    },
                    &ctx.actor,
                )?
            }
            None => {
                let document = state.store.create_document(
                    &crate::store::DocumentCreate {
                        project: map.project.clone(),
                        title: entry.title.clone(),
                        path: Some(entry.path.clone()),
                        ..Default::default()
                    },
                    &ctx.actor,
                )?;
                if !entry.blocks.is_empty() {
                    let update = crate::store::prose::initial_update(&entry.blocks);
                    state
                        .store
                        .append_collab_update(&document.id, &update, &ctx.actor)?;
                }
                let node = entry.node.clone();
                let doc_id = document.id.clone();
                room.mutate(move |doc| mindmapdoc::set_document(doc, &node, &doc_id))?;
                created += 1;
                document
            }
        };

        made.push(json!({
            "node": entry.node,
            "document": document.id,
            "title": document.title,
            "path": document.path,
        }));
    }

    // The links back into the map would otherwise sit in memory, and a
    // conversion that lost them would make a second set of documents next time.
    persist(&state, &room, &ctx).await;
    state.wake();

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "documents": made,
            "created": created,
            "refiled": refiled,
            "note": "One document per thought that had something to say; a bare leaf became a bullet in its parent. Run it again after reshaping the map and the documents are refiled, never rewritten.",
        })),
    ))
}
