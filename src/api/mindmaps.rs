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
//! See `src/store/mindmaps.rs` for what a mindmap deliberately is not.

use super::{
    body_object, first, get_f64, get_i64, get_str, paged, parse_i64_param, query_pairs,
    reject_unknown, require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{
    MindmapCreate, MindmapListFilter, MindmapPatch, NodeAdd, NodePatch, MAX_MINDMAPS_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 4] = ["project", "title", "summary", "metadata"];
const PATCH_FIELDS: [&str; 4] = ["title", "summary", "status", "metadata_merge"];
const NODE_FIELDS: [&str; 3] = ["parent", "text", "position"];
const NODE_PATCH_FIELDS: [&str; 4] = ["text", "parent", "position", "at"];

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

/// GET /v1/mindmaps/{id} (read) — the map and every node on it, in one request.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (map, nodes) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    Ok(Json(json!({
        "mindmap": map.to_json(),
        "nodes": nodes.iter().map(|n| n.to_json()).collect::<Vec<_>>(),
        // The whole tree is here by construction — a map is capped well below any
        // page size — so this listing carries no cursor and needs none.
        "total": nodes.len(),
    })))
}

/// GET /v1/mindmaps/{id}/outline (read) — the tree as indented text.
///
/// Its own route because it is the shape a *model* reads and writes cheapest, and
/// the one a person pastes into a document. `?node=` narrows it to one branch.
pub async fn outline(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let (map, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    let pairs = query_pairs(raw.as_deref());
    let text = state.store.mindmap_outline(&id, first(&pairs, "node"))?;
    Ok(Json(json!({ "mindmap": id, "outline": text })))
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
    let (existing, _) = state
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
    let (existing, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&existing.project)?;
    let nodes = state.store.delete_mindmap(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "ok": true, "removed_nodes": nodes })))
}

/// POST /v1/mindmaps/{id}/nodes (write) — add thoughts, one or many.
///
/// Accepts `{"nodes":[…]}` for a batch and a bare `{"text":…}` for one, because
/// the one-node case is what a person's keystroke sends and making them wrap it in
/// an array is friction on the fastest path in the whole feature.
pub async fn add_nodes(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    let (map, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;

    let adds: Vec<NodeAdd> = match obj.get("nodes") {
        Some(Value::Array(items)) => {
            reject_unknown(obj, &["nodes"])?;
            items
                .iter()
                .map(|item| {
                    let node = item.as_object().ok_or_else(|| {
                        ApiError::validation(
                            "validation.mindmap_nodes",
                            "Every entry in 'nodes' must be an object like {\"text\":\"…\",\"parent\":\"mn-…\"}.",
                        )
                    })?;
                    reject_unknown(node, &NODE_FIELDS)?;
                    Ok(NodeAdd {
                        parent: get_str(node, "parent")?,
                        text: require_str(node, "text")?,
                        position: get_i64(node, "position")?,
                    })
                })
                .collect::<ApiResult<Vec<_>>>()?
        }
        Some(_) => {
            return Err(ApiError::validation(
                "validation.mindmap_nodes",
                "Field 'nodes' must be an array of {text, parent?, position?} objects.",
            ))
        }
        None => {
            reject_unknown(obj, &NODE_FIELDS)?;
            vec![NodeAdd {
                parent: get_str(obj, "parent")?,
                text: require_str(obj, "text")?,
                position: get_i64(obj, "position")?,
            }]
        }
    };

    let nodes = state.store.grow_mindmap(&id, &adds, &ctx.actor)?;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({ "nodes": nodes.iter().map(|n| n.to_json()).collect::<Vec<_>>() })),
    ))
}

/// PATCH /v1/mindmaps/{id}/nodes/{node} (write) — retype it, move it, place it.
///
/// `parent: null` lifts a node to the first ring and `at: null` returns it to the
/// layout; absent leaves either alone. Absent and null differ here for the same
/// reason they do on a checklist policy — one means "not my business", the other
/// is an instruction.
pub async fn patch_node(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &NODE_PATCH_FIELDS)?;
    let (map, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;

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
                // Half a coordinate places nothing, so it is refused rather than
                // guessed at from the node's current position.
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

    let patch = NodePatch {
        text: get_str(obj, "text")?,
        parent,
        position: get_i64(obj, "position")?,
        at,
    };
    let node = state
        .store
        .patch_mindmap_node(&id, &node, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "node": node.to_json() })))
}

/// DELETE /v1/mindmaps/{id}/nodes/{node} (write) — the node and its subtree.
pub async fn delete_node(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, node)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let (map, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    let removed = state.store.delete_mindmap_node(&id, &node, &ctx.actor)?;
    state.wake();
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

/// POST /v1/mindmaps/{id}/nodes/{node}/promote (write) — graduate a branch.
///
/// `{"target":"epic"}` makes an epic with its direct children as tickets;
/// `{"target":"initiative"}` makes an initiative seeded with the subtree. The node
/// stays either way and keeps a link to what it became — which is what lets a map
/// go on being useful once the brainstorming is over.
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
    let (map, _) = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    let (node, created) = state
        .store
        .promote_mindmap_node(&id, &node, &target, &ctx.actor)?;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({ "node": node.to_json(), "created": created })),
    ))
}
