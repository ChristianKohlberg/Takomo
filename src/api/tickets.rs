//! /v1/tickets: create, list/search, get, patch, comments, deps.

use super::{
    all, body_object, first, get_i64, get_str, get_string_array, parse_i64_param, query_pairs,
    require_str,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use crate::store::{Ticket, TicketCreate, TicketListFilter, TicketPatch};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 10] = [
    "project",
    "type",
    "parent",
    "title",
    "body",
    "priority",
    "labels",
    "metadata",
    "blocked_by",
    "state",
];
const PATCH_FIELDS: [&str; 10] = [
    "title",
    "body",
    "priority",
    "labels",
    "labels_add",
    "labels_remove",
    "parent",
    "links",
    "metadata_merge",
    "fence",
];

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    reject_unknown_fields(obj, &CREATE_FIELDS, "TicketCreate")?;

    let req = TicketCreate {
        project: require_str(obj, "project")?,
        ty: get_str(obj, "type")?,
        parent: get_str(obj, "parent")?,
        title: require_str(obj, "title")?,
        body: get_str(obj, "body")?,
        priority: get_str(obj, "priority")?,
        labels: get_string_array(obj, "labels")?.unwrap_or_default(),
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
        blocked_by: get_string_array(obj, "blocked_by")?.unwrap_or_default(),
        state: get_str(obj, "state")?,
    };
    ctx.require_project(&req.project)?;

    let idem_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|k| !k.is_empty());
    if let Some(k) = idem_key {
        if k.len() > 128 {
            return Err(ApiError::bad_request(
                "validation.idempotency_key",
                "Idempotency-Key must be at most 128 characters.",
            ));
        }
    }

    let (ticket, similar, replayed) = state.store.create_ticket(&req, &ctx.actor, idem_key)?;
    state.wake();
    let mut out = ticket.to_json(now_ms());
    out["similar"] = Value::Array(similar);
    let status = if replayed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(out)))
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());

    if let Some(p) = first(&pairs, "project") {
        ctx.require_project(p)?;
    }
    let filter = TicketListFilter {
        project: first(&pairs, "project").map(str::to_string),
        state: first(&pairs, "state").map(str::to_string),
        ty: first(&pairs, "type").map(str::to_string),
        labels: all(&pairs, "label"),
        parent: first(&pairs, "parent").map(str::to_string),
        q: first(&pairs, "q").map(str::to_string),
        claimed_by: first(&pairs, "claimed_by").map(str::to_string),
        allowed_projects: ctx.allowed_projects_vec(),
        archived: parse_archived(&pairs)?,
    };
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(50)
        .clamp(1, 200);
    let cursor = match first(&pairs, "cursor") {
        None => None,
        Some(c) => Some(c.parse::<i64>().map_err(|_| {
            ApiError::bad_request(
                "validation.cursor",
                "Invalid cursor; pass the exact next_cursor value from the previous page.",
            )
        })?),
    };
    let fields = first(&pairs, "fields").map(parse_fields);

    let (tickets, next_cursor) = state.store.list_tickets(&filter, cursor, limit)?;
    let now = now_ms();
    let items: Vec<Value> = tickets
        .iter()
        .map(|t| project_fields(t.to_json(now), fields.as_deref()))
        .collect();
    Ok(Json(json!({ "items": items, "next_cursor": next_cursor })))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("read")?;
    let ticket = load_visible(&state, &ctx, &id)?;
    let pairs = query_pairs(raw.as_deref());
    let now = now_ms();
    let mut out = ticket.to_json(now);

    if let Some(include_raw) = first(&pairs, "include") {
        for inc in include_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match inc {
                "comments" => {
                    let comments = state.store.comments_for(&id)?;
                    out["comments"] =
                        Value::Array(comments.iter().map(|c| c.to_json()).collect());
                }
                "children" => {
                    let children = state.store.children_of(&id)?;
                    out["children"] = Value::Array(
                        children
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id, "title": c.title, "type": c.ty,
                                    "state": c.state, "state_category": c.state_category,
                                    "priority": c.priority,
                                })
                            })
                            .collect(),
                    );
                }
                "deps" => {
                    let mut blocked_by_detail = Vec::new();
                    for dep_id in &ticket.blocked_by {
                        if let Some(d) = state.store.get_ticket(dep_id)? {
                            blocked_by_detail.push(json!({
                                "id": d.id, "title": d.title, "state": d.state,
                                "state_category": d.state_category,
                            }));
                        }
                    }
                    out["deps"] = json!({
                        "blocked_by": blocked_by_detail,
                        "blocks": state.store.blocks_of(&id)?,
                    });
                }
                "events" => {
                    let events = state.store.events_for_ticket(&id, 200)?;
                    out["events"] = Value::Array(events.iter().map(|e| e.to_json()).collect());
                }
                other => {
                    return Err(ApiError::bad_request(
                        "validation.include",
                        format!(
                            "Unknown include '{other}'. Valid values: comments, children, deps, events (comma-separated)."
                        ),
                    ))
                }
            }
        }
    }

    let etag = format!("\"{}\"", ticket.version);
    Ok(([("ETag", etag)], Json(out)))
}

pub async fn patch_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;

    // State is not patchable — by design. Teach the right call.
    if obj.contains_key("state") {
        return Err(ApiError::conflict(
            "patch.state_not_patchable",
            format!(
                "State cannot be changed via PATCH; it is workflow-controlled. Use POST /v1/tickets/{id}/transition with {{\"to\": \"<state>\"}} — that call validates the move against the project workflow and tells you the allowed transitions if it is illegal."
            ),
        )
        .remedy(format!("POST /v1/tickets/{id}/transition")));
    }
    reject_unknown_fields(obj, &PATCH_FIELDS, "TicketPatch")?;

    let patch = TicketPatch {
        title: get_str(obj, "title")?,
        body: get_str(obj, "body")?,
        priority: get_str(obj, "priority")?,
        labels: get_string_array(obj, "labels")?,
        labels_add: get_string_array(obj, "labels_add")?.unwrap_or_default(),
        labels_remove: get_string_array(obj, "labels_remove")?.unwrap_or_default(),
        parent: match obj.get("parent") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(s)) => Some(Some(s.clone())),
            Some(_) => {
                return Err(ApiError::bad_request(
                    "validation.field_type",
                    "Field 'parent' must be a string ticket id, or null to clear the parent.",
                ))
            }
        },
        links: obj.get("links").filter(|v| !v.is_null()).cloned(),
        metadata_merge: obj.get("metadata_merge").filter(|v| !v.is_null()).cloned(),
        fence: get_i64(obj, "fence")?,
    };

    let if_match = parse_if_match(&headers)?;
    let ticket = state
        .store
        .patch_ticket(&id, &patch, &ctx.actor, if_match)?;
    state.wake();
    Ok(Json(ticket.to_json(now_ms())))
}

pub async fn add_comment(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    let text = require_str(obj, "body")?;
    let comment = state.store.add_comment(&id, &ctx.actor, &text)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(comment.to_json())))
}

/// GET /v1/tickets/{id}/deps — the dependency graph around a ticket.
/// `direction` = blocked_by (default) | blocks | both; `transitive` = false
/// (default) | true. Returns cycle-safe nodes + canonical `{ticket, blocked_by}`
/// edges.
pub async fn deps_graph(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    load_visible(&state, &ctx, &id)?;
    let pairs = query_pairs(raw.as_deref());

    let direction = match first(&pairs, "direction") {
        None => crate::store::DepDirection::BlockedBy,
        Some(raw) => crate::store::DepDirection::parse(raw).ok_or_else(|| {
            ApiError::bad_request(
                "validation.direction",
                format!(
                    "Unknown direction '{raw}'. Use one of: blocked_by (what blocks this ticket), blocks (what this ticket blocks), both."
                ),
            )
        })?,
    };
    let transitive = match first(&pairs, "transitive") {
        None => false,
        Some("true" | "1") => true,
        Some("false" | "0") => false,
        Some(other) => {
            return Err(ApiError::bad_request(
                "validation.transitive",
                format!("Query parameter 'transitive' must be true or false, got '{other}'."),
            ))
        }
    };

    let out = state.store.dep_graph(&id, direction, transitive)?;
    Ok(Json(out))
}

/// POST /v1/tickets/{id}/archive (write scope). Hides the ticket from default
/// list/ready/board/metrics views. Any state is allowed; terminal done/cancelled
/// is the typical case. Idempotent.
pub async fn archive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let ticket = state.store.archive_ticket(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(ticket.to_json(now_ms())))
}

/// POST /v1/tickets/{id}/unarchive (write scope). Returns the ticket to the
/// default views. Idempotent.
pub async fn unarchive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let ticket = state.store.unarchive_ticket(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(ticket.to_json(now_ms())))
}

pub async fn add_dep(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    let blocked_by = require_str(obj, "blocked_by")?;
    let fence = get_i64(obj, "fence")?;
    state.store.add_dep(&id, &blocked_by, &ctx.actor, fence)?;
    state.wake();
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ticket": id, "blocked_by": blocked_by })),
    ))
}

pub async fn remove_dep(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<StatusCode> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let pairs = query_pairs(raw.as_deref());
    let blocked_by = first(&pairs, "blocked_by").ok_or_else(|| {
        ApiError::bad_request(
            "validation.query",
            "Query parameter 'blocked_by' is required: DELETE /v1/tickets/{id}/deps?blocked_by=<ticket-id>.",
        )
    })?;
    let fence = parse_i64_param(&pairs, "fence")?;
    state.store.remove_dep(&id, blocked_by, &ctx.actor, fence)?;
    state.wake();
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------

/// Load a ticket, enforcing token project scoping (a scoped token reaching
/// outside its projects gets a teaching 403 naming the boundary).
pub fn load_visible(state: &AppState, ctx: &AuthCtx, id: &str) -> ApiResult<Ticket> {
    let ticket = state
        .store
        .get_ticket(id)?
        .ok_or_else(|| ApiError::not_found("ticket", id))?;
    ctx.require_project(&ticket.project)?;
    Ok(ticket)
}

fn reject_unknown_fields(
    obj: &serde_json::Map<String, Value>,
    known: &[&str],
    shape: &str,
) -> ApiResult<()> {
    let unknown: Vec<&String> = obj
        .keys()
        .filter(|k| !known.contains(&k.as_str()))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "validation.unknown_field",
        format!(
            "Unknown field(s) in {shape}: {}. Accepted fields: {}. If you are attaching custom data, put it under 'metadata' (create) or 'metadata_merge' (patch) with namespaced keys.",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            known.join(", ")
        ),
    ))
}

fn parse_if_match(headers: &HeaderMap) -> ApiResult<Option<i64>> {
    let Some(raw) = headers.get("If-Match").and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let cleaned = raw.trim().trim_start_matches("W/").trim_matches('"').trim();
    cleaned.parse::<i64>().map(Some).map_err(|_| {
        ApiError::bad_request(
            "validation.if_match",
            format!(
                "If-Match must be the ticket version as returned in the ETag header, e.g. If-Match: \"7\" (got '{raw}')."
            ),
        )
    })
}

/// Resolve archived-ticket visibility from `archived` (only|all|active) and the
/// `include_archived` (true|false) shorthand. Default is active-only.
fn parse_archived(pairs: &[(String, String)]) -> ApiResult<crate::store::ArchivedFilter> {
    use crate::store::ArchivedFilter;
    if let Some(a) = first(pairs, "archived") {
        return match a {
            "only" => Ok(ArchivedFilter::Only),
            "all" => Ok(ArchivedFilter::Include),
            "active" => Ok(ArchivedFilter::Exclude),
            other => Err(ApiError::bad_request(
                "validation.archived",
                format!(
                    "Query parameter 'archived' must be one of: only, all, active (got '{other}'). Or pass include_archived=true to include archived tickets."
                ),
            )),
        };
    }
    match first(pairs, "include_archived") {
        Some("true" | "1") => Ok(ArchivedFilter::Include),
        None | Some("false" | "0") => Ok(ArchivedFilter::Exclude),
        Some(other) => Err(ApiError::bad_request(
            "validation.include_archived",
            format!("Query parameter 'include_archived' must be true or false, got '{other}'."),
        )),
    }
}

fn parse_fields(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Sparse responses: keep only requested fields (id always included).
fn project_fields(full: Value, fields: Option<&[String]>) -> Value {
    let Some(fields) = fields else { return full };
    let Value::Object(map) = full else {
        return full;
    };
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        if k == "id" || fields.iter().any(|f| f == &k) {
            out.insert(k, v);
        }
    }
    Value::Object(out)
}
