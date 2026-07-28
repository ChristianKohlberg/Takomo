//! /v1/tickets: create, list/search, get, patch, comments, deps.

use super::{
    all, attach_conventions, blocking_read, body_object, first, get_i64, get_str, get_string_array,
    parse_i64_param, query_pairs, require_str, ApiJson,
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

/// Every field `POST /v1/tickets` accepts. Anything else is a teaching 400
/// (`reject_unknown_fields`), so a field the store understands but this list
/// omits is unreachable over HTTP. `pub` so the field-list guard in
/// `tests/api.rs` can check it against `store::TicketCreate` and the OpenAPI
/// `TicketCreate` schema — see the comment above those tests for the six lists
/// one ticket field lives in.
pub const CREATE_FIELDS: [&str; 11] = [
    "project",
    "type",
    "parent",
    "title",
    "body",
    "priority",
    "labels",
    "tags",
    "metadata",
    "blocked_by",
    "state",
];
/// Every field `PATCH /v1/tickets/{id}` accepts. `state` is deliberately absent
/// (it is workflow-controlled; see the teaching 409 in [`patch_one`]). `pub` for
/// the same guard as [`CREATE_FIELDS`].
pub const PATCH_FIELDS: [&str; 13] = [
    "title",
    "body",
    "priority",
    "labels",
    "labels_add",
    "labels_remove",
    "tags",
    "tags_add",
    "tags_remove",
    "parent",
    "links",
    "metadata_merge",
    "fence",
];

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
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
        tags: get_string_array(obj, "tags")?.unwrap_or_default(),
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
    // Echo the project's conventions back on create: the ticket text was just
    // written, so this is the moment an agent can still fix it.
    attach_conventions(&state, &mut out, &ticket.project);
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
        tags: all(&pairs, "tag"),
        tag_kinds: all(&pairs, "tag_kind"),
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
                        // Deps can cross projects; only reveal title/state for a
                        // dependency the token is allowed to see, else a bare id.
                        match state.store.get_ticket(dep_id)? {
                            Some(d) if ctx.can_project(&d.project) => {
                                blocked_by_detail.push(json!({
                                    "id": d.id, "title": d.title, "state": d.state,
                                    "state_category": d.state_category,
                                }))
                            }
                            _ => blocked_by_detail
                                .push(json!({ "id": dep_id, "out_of_scope": true })),
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
                "promotions" => {
                    let promos = state.store.promotions_for(&id)?;
                    out["promotions"] =
                        Value::Array(promos.iter().map(|p| p.to_json()).collect());
                }
                other => {
                    return Err(ApiError::bad_request(
                        "validation.include",
                        format!(
                            "Unknown include '{other}'. Valid values: comments, children, deps, events, promotions (comma-separated)."
                        ),
                    ))
                }
            }
        }
    }

    // Single-ticket read: one project row, no per-item cost. The list endpoint
    // above builds straight from `Ticket::to_json`, so arrays stay untouched.
    attach_conventions(&state, &mut out, &ticket.project);

    let etag = format!("\"{}\"", ticket.version);
    Ok(([("ETag", etag)], Json(out)))
}

pub async fn patch_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
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
        tags: get_string_array(obj, "tags")?,
        tags_add: get_string_array(obj, "tags_add")?.unwrap_or_default(),
        tags_remove: get_string_array(obj, "tags_remove")?.unwrap_or_default(),
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown_fields(obj, &["body"], "Comment")?;
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

    // With `transitive`, the walk chases the whole chain — a query per node, for
    // as many nodes as the graph reaches. Off the runtime (see `blocking_read`);
    // the direct case rides along rather than branching on cost.
    let allowed = ctx.allowed_projects_vec();
    let state = state.clone();
    let out = blocking_read(move || {
        state
            .store
            .dep_graph(&id, direction, transitive, allowed.as_deref())
    })
    .await?;
    Ok(Json(out))
}

const PROMOTE_FIELDS: [&str; 4] = ["target", "url", "ref", "note"];

/// POST /v1/tickets/{id}/promote (write scope). Records that the ticket's work
/// reached a named target/stage — free-form ("staging", "production",
/// "published", …), so it fits any workflow, not just software. Append-only.
pub async fn promote(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown_fields(obj, &PROMOTE_FIELDS, "Promotion")?;
    let target = require_str(obj, "target")?;
    let url = get_str(obj, "url")?;
    let ref_ = get_str(obj, "ref")?;
    let note = get_str(obj, "note")?;
    let promo = state.store.promote_ticket(
        &id,
        &ctx.actor,
        &target,
        url.as_deref(),
        ref_.as_deref(),
        note.as_deref(),
    )?;
    state.wake();
    Ok((StatusCode::CREATED, Json(promo.to_json())))
}

/// GET /v1/tickets/{id}/promotions (read scope) — the ticket's promotion history,
/// newest first.
pub async fn list_promotions(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    load_visible(&state, &ctx, &id)?;
    let promos = state.store.promotions_for(&id)?;
    Ok(Json(json!({
        "items": promos.iter().map(|p| p.to_json()).collect::<Vec<_>>(),
    })))
}

/// GET /v1/promotions?project=<id> (read scope) — the latest promotion per
/// ticket across a project, so the board can badge cards in one call.
pub async fn promotions_index(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    let project = first(&pairs, "project").ok_or_else(|| {
        ApiError::bad_request(
            "validation.project_required",
            "Query parameter 'project' is required: GET /v1/promotions?project=<id>.",
        )
    })?;
    ctx.require_project(project)?;
    let promos = state
        .store
        .latest_promotions_for_project(project, ctx.allowed_projects_vec().as_deref())?;
    Ok(Json(json!({
        "items": promos.iter().map(|p| p.to_json()).collect::<Vec<_>>(),
    })))
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
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    reject_unknown_fields(obj, &["blocked_by", "fence"], "Dependency")?;
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
