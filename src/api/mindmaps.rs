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
use yrs::Transact;

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
    // A map written before sections had prose is moved into it here, once, on
    // the first open. Cheap, idempotent, and it broadcasts only if it changed
    // something.
    room.mutate(|doc| Ok(mindmapdoc::ensure_prose(doc)))?;
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
    // Where each section stands, in one grouped query rather than one per node,
    // because a map is drawn all at once. It is a reading and not a stored flag:
    // a section confirmed BEFORE its last edit is not confirmed any more.
    let standing = state.store.plan_standing(&id)?;
    Ok(Json(json!({
        "mindmap": map.to_json(),
        "nodes": nodes,
        "relationships": relationships,
        "standing": standing,
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

fn parse_node_add(
    obj: &serde_json::Map<String, Value>,
    origin: &str,
    by_user: Option<&str>,
) -> ApiResult<NodeAdd> {
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
        by_user: by_user.map(str::to_string),
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
                adds.push(parse_node_add(entry, &origin, ctx.user.as_deref())?);
            }
            adds
        }
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_nodes",
                "Field 'nodes' must be an array of {text, parent?, position?} objects.",
            ))
        }
        None => vec![parse_node_add(obj, &origin, ctx.user.as_deref())?],
    };

    let (map, room) = join(&state, &ctx, &id).await?;
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
    // One trace entry PER node, unlike the event log's one per batch: the event
    // log answers "what is happening in this project", where ten nodes from one
    // agent turn are one act; the trace answers "what happened to this section",
    // and a section was either written or it was not.
    for (node_id, _) in &created {
        state.store.record_trace(&crate::store::trace::Record {
            project: &map.project,
            mindmap: &id,
            node: Some(node_id),
            kind: "authored",
            actor: &ctx.actor,
            user: ctx.user.as_deref(),
            note: None,
        })?;
    }
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

    let (map, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let moved = patch.parent.is_some();
    let renamed = patch.title.is_some();
    let edited = patch.notes.is_some();
    let actor = ctx.actor.clone();
    room.mutate(|doc| mindmapdoc::patch_node(doc, &node, &patch, &actor))?;
    persist(&state, &room, &ctx).await;

    for (happened, kind) in [(renamed, "renamed"), (edited, "edited"), (moved, "moved")] {
        if happened {
            state.store.record_trace(&crate::store::trace::Record {
                project: &map.project,
                mindmap: &id,
                node: Some(&node),
                kind,
                actor: &ctx.actor,
                user: ctx.user.as_deref(),
                note: None,
            })?;
        }
    }

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
    let (map, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let removed = room.mutate(|doc| mindmapdoc::delete_node(doc, &node))?;
    state.store.record_trace(&crate::store::trace::Record {
        project: &map.project,
        mindmap: &id,
        node: Some(&node),
        kind: "pruned",
        actor: &ctx.actor,
        user: ctx.user.as_deref(),
        note: None,
    })?;
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

/// GET /v1/mindmaps/{id}/trace?node=&limit= (read) — the plan's history.
///
/// What happened to the plan, newest first, or to one section of it. This is
/// NOT the CRDT update log: that is the mechanism that rebuilds the text, is
/// written per flush, and is rewritten by compaction. This is the record of
/// acts somebody would name, each carrying the person behind the credential.
pub async fn trace(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let map = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;

    let pairs = query_pairs(raw.as_deref());
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(crate::store::MAX_TRACE_PAGE)
        .clamp(1, crate::store::MAX_TRACE_PAGE);
    let node = first(&pairs, "node");
    let (entries, total) = state.store.plan_trace(&id, node, limit)?;

    Ok(Json(paged(
        entries.iter().map(|e| e.to_json()).collect(),
        total,
        limit,
        "Raise 'limit' (max 500) or narrow with 'node'; the newest act is first.",
    )))
}

/// POST /v1/mindmaps/{id}/trace (write) — record an act the server cannot see.
///
/// Prose is edited over the sync socket, which never reaches the server as a
/// request, and a review is somebody saying so. Those two are the only kinds a
/// caller may write: everything else is recorded by the path that performs it,
/// so nobody can claim to have moved a node they did not move.
pub async fn add_trace(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["node", "kind", "note"])?;
    let kind = require_str(obj, "kind")?;
    if !crate::store::CLIENT_TRACE_KINDS.contains(&kind.as_str()) {
        return Err(ApiError::validation(
            "validation.trace_kind",
            format!(
                "'{kind}' is not a kind a caller may record. Use one of: {}. The rest are written by the paths that perform them, so nobody can claim to have done something they did not.",
                crate::store::CLIENT_TRACE_KINDS.join(", ")
            ),
        ));
    }
    let node = get_str(obj, "node")?;
    let note = get_str(obj, "note")?;

    let map = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    state.store.ensure_collab_writable(&id)?;

    state.store.record_trace(&crate::store::trace::Record {
        project: &map.project,
        mindmap: &id,
        node: node.as_deref(),
        kind: &kind,
        actor: &ctx.actor,
        user: ctx.user.as_deref(),
        note: note.as_deref(),
    })?;
    state.wake();
    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}

/// GET /v1/mindmaps/{id}/prose?node= (read) — the plan as an agent reads it.
///
/// Annotated with block ids, because that is what makes a reply addressable: an
/// agent answers with operations against block ids, never with a document. One
/// section with `node`, or the whole plan — headings from the tree, each
/// section's blocks beneath its own.
pub async fn prose(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (_, room) = join(&state, &ctx, &id).await?;
    let pairs = query_pairs(raw.as_deref());
    let node = first(&pairs, "node").map(str::to_string);

    let markdown = room.read(|doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, &id);
        match node.as_deref() {
            Some(node) => mindmapdoc::section_prose(doc, node).map(|frag| {
                let txn = doc.transact();
                let blocks = crate::api::docprops::read_blocks(&txn, &frag);
                crate::api::docprops::annotate(&blocks)
            }),
            None => {
                let ordered = mindmapdoc::tree_order(&nodes);
                let mut out = String::new();
                for section in ordered {
                    let level = mindmapdoc::depth_of(&nodes, &section.id).min(6);
                    out.push_str(&format!("{} {}\n\n", "#".repeat(level), section.title));
                    if let Ok(frag) = mindmapdoc::section_prose(doc, &section.id) {
                        let txn = doc.transact();
                        let blocks = crate::api::docprops::read_blocks(&txn, &frag);
                        let body = crate::api::docprops::annotate(&blocks);
                        if !body.trim().is_empty() {
                            out.push_str(&body);
                            out.push_str("\n\n");
                        }
                    }
                }
                Ok(out.trim_end().to_string())
            }
        }
    })?;

    Ok(Json(
        json!({ "mindmap": id, "node": node, "markdown": markdown }),
    ))
}

/// POST /v1/mindmaps/{id}/proposals (write) — an agent proposes to a section.
///
/// The rule is unchanged and is the whole point: an agent returns OPERATIONS
/// against block ids and never a document, so a person's concurrent typing
/// survives, and nothing is live until somebody accepts it.
pub async fn propose(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(
        obj,
        &["node", "operations", "instruction", "summary", "scope"],
    )?;
    let node = require_str(obj, "node")?;
    let operations = obj
        .get("operations")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let instruction = get_str(obj, "instruction")?.unwrap_or_default();
    let summary = get_str(obj, "summary")?.unwrap_or_default();
    let scope = match obj.get("scope") {
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect::<Vec<String>>(),
        ),
        _ => None,
    };

    let (map, room) = join(&state, &ctx, &id).await?;
    state.store.ensure_collab_writable(&id)?;

    let actor = ctx.actor.clone();
    let now = crate::ids::now_ms();
    let target = node.clone();
    let why = summary.clone();
    let (proposal, applied, skipped) = room.mutate(move |doc| {
        let frag = mindmapdoc::section_prose(doc, &target)?;
        let txn = doc.transact();
        let blocks = crate::api::docprops::read_blocks(&txn, &frag);
        drop(txn);
        let validated = crate::api::docprops::validate_ops(&operations, &blocks, scope.as_deref())?;
        let pid = crate::api::docprops::write_proposal(
            doc,
            Some(&target),
            &actor,
            &instruction,
            &why,
            &validated.ops,
            &validated.skipped,
            now,
        )?;
        Ok((pid, validated.ops.len(), validated.skipped))
    })?;

    persist(&state, &room, &ctx).await;
    state.store.record_trace(&crate::store::trace::Record {
        project: &map.project,
        mindmap: &id,
        node: Some(&node),
        kind: "proposed",
        actor: &ctx.actor,
        user: ctx.user.as_deref(),
        note: (!summary.is_empty()).then_some(summary.as_str()),
    })?;
    state.wake();

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "proposal": proposal,
            "mindmap": id,
            "node": node,
            "status": "pending",
            "operations": applied,
            "skipped": skipped,
            "note": "Offered, not applied. A person accepts or rejects it in the document view.",
        })),
    ))
}

/// GET /v1/mindmaps/{id}/proposals?node=&status= (read).
pub async fn proposals(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (_, room) = join(&state, &ctx, &id).await?;
    let pairs = query_pairs(raw.as_deref());
    let node = first(&pairs, "node");
    let status = first(&pairs, "status");

    let mut items = room.read(crate::api::docprops::read_proposals);
    if let Some(node) = node {
        items.retain(|p| p.get("node").and_then(Value::as_str) == Some(node));
    }
    if let Some(status) = status {
        items.retain(|p| p.get("status").and_then(Value::as_str) == Some(status));
    }
    let total = items.len() as i64;
    Ok(Json(json!({ "items": items, "total": total })))
}
