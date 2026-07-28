//! /v1/shares: mint, list, and revoke shareable read-only web links, plus the
//! share-token-scoped `/v1/shares/self*` read endpoints the board renders.
//!
//! The management endpoints (create/list/revoke) run on the normal token auth
//! path. The `self*` endpoints run on the distinct share-token auth path
//! (`auth::share_auth_middleware`): a share token can reach ONLY those, is
//! read-only, and is bounded to its scope. See spec/auth.md.

use super::{body_object, first, get_i64, parse_i64_param, query_pairs, require_str, ApiJson};
use crate::auth::{AuthCtx, ShareCtx};
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::server::AppState;
use crate::store::{
    ShareKind, DEFAULT_SHARE_TTL_SECONDS, MAX_SHARE_TTL_SECONDS, SHARE_TICKETS_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 3] = ["kind", "ref", "ttl_seconds"];

/// POST /v1/shares (write scope) — mint a shareable, read-only, expiring link.
/// Body: `{kind: "project"|"epic", ref: "<project-or-ticket-id>", ttl_seconds?}`.
/// Returns `{id, token, expires_at, kind, ref, project, path}` — `token` is
/// shown ONCE; `path` is the fragment URL to open (`/board#s=<token>`).
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let obj = body_object(&body)?;
    let unknown: Vec<&String> = obj
        .keys()
        .filter(|k| !CREATE_FIELDS.contains(&k.as_str()))
        .collect();
    if let Some(k) = unknown.first() {
        return Err(ApiError::bad_request(
            "validation.unknown_field",
            format!(
                "Unknown field '{k}' in ShareCreate. Accepted fields: {}.",
                CREATE_FIELDS.join(", ")
            ),
        ));
    }

    let kind_raw = require_str(obj, "kind")?;
    let kind = ShareKind::parse_request(&kind_raw).ok_or_else(|| {
        ApiError::validation(
            "share.kind",
            format!(
                "Field 'kind' must be 'project' (all tickets in a project) or 'epic' (a ticket plus its full descendant subtree); got '{kind_raw}'."
            ),
        )
    })?;
    let ref_id = require_str(obj, "ref")?;

    let ttl = match get_i64(obj, "ttl_seconds")? {
        None => DEFAULT_SHARE_TTL_SECONDS,
        Some(secs) if secs <= 0 => {
            return Err(ApiError::validation(
                "share.ttl",
                "Field 'ttl_seconds' must be a positive integer number of seconds.",
            ))
        }
        Some(secs) if secs > MAX_SHARE_TTL_SECONDS => {
            return Err(ApiError::validation(
                "share.ttl",
                format!(
                    "Field 'ttl_seconds' exceeds the maximum of {MAX_SHARE_TTL_SECONDS} seconds (30 days)."
                ),
            ))
        }
        Some(secs) => secs,
    };

    // Validate the referent exists and resolve the project the share is bound
    // to, enforcing the caller's own project scoping in the process.
    let project = match kind {
        ShareKind::Project => {
            ctx.require_project(&ref_id)?;
            state
                .store
                .get_project(&ref_id)?
                .ok_or_else(|| ApiError::not_found("project", &ref_id))?;
            ref_id.clone()
        }
        ShareKind::Subtree => {
            let ticket = state
                .store
                .get_ticket(&ref_id)?
                .ok_or_else(|| ApiError::not_found("ticket", &ref_id))?;
            ctx.require_project(&ticket.project)?;
            ticket.project
        }
    };

    let expires_at = now_ms() + ttl * 1000;
    let (row, plaintext) = state
        .store
        .create_share(kind, &ref_id, &project, expires_at, &ctx.actor)?;

    let mut out = row.to_json();
    if let Value::Object(map) = &mut out {
        map.insert("token".to_string(), Value::String(plaintext.clone()));
        map.insert(
            "path".to_string(),
            Value::String(format!("/board#s={plaintext}")),
        );
        map.insert(
            "warning".to_string(),
            Value::String(
                "This share token is shown ONCE; only its SHA-256 is stored. Anyone with the link can view the scoped board (read-only) until it expires."
                    .to_string(),
            ),
        );
    }
    Ok((StatusCode::CREATED, Json(out)))
}

/// GET /v1/shares (read scope) — list share metadata. An admin sees all shares;
/// any other reader sees only the shares they created. Never returns the token.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let filter = if ctx.scopes.contains("admin") {
        None
    } else {
        Some(ctx.actor.as_str())
    };
    let shares = state.store.list_shares(filter)?;
    let out: Vec<Value> = shares.iter().map(|s| s.to_json()).collect();
    Ok(Json(Value::Array(out)))
}

/// DELETE /v1/shares/{id} (write scope) — revoke a share. Allowed for the share's
/// creator or any admin. Revocation is immediate: the share token then returns
/// 410 Gone on every `self*` endpoint.
pub async fn revoke(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let share = state
        .store
        .get_share(&id)?
        .ok_or_else(|| ApiError::not_found("share", &id))?;
    if share.created_by != ctx.actor && !ctx.scopes.contains("admin") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "share.not_owner",
            "Only the share's creator or an admin can revoke it.",
        ));
    }
    let revoked = state.store.revoke_share(&id)?;
    if !revoked {
        // Existed but was already revoked — treat as idempotent success.
        return Ok(StatusCode::NO_CONTENT);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Share-token-scoped read endpoints (distinct auth path).

/// GET /v1/shares/self — the share's own scope plus the project workflow, so the
/// board can render the columns without any other token.
pub async fn self_meta(
    State(state): State<Arc<AppState>>,
    Extension(share): Extension<ShareCtx>,
) -> ApiResult<Json<Value>> {
    let project = state
        .store
        .get_project(&share.project)?
        .ok_or_else(|| ApiError::not_found("project", &share.project))?;
    Ok(Json(json!({
        "kind": share.kind.as_str(),
        "ref": share.ref_id,
        "project": share.project,
        "expires_at": iso(share.expires_at),
        "workflow": project.workflow,
    })))
}

/// GET /v1/shares/self/tickets — one page of the tickets in the share's scope
/// (read-only), oldest first. Archived tickets are excluded unless
/// `?include_archived=true`. `?limit=` (1..=SHARE_TICKETS_PAGE) and `?cursor=`
/// page it.
///
/// Paged rather than whole-scope because a share token is a credential designed to
/// be pasted around, and an unbounded full-project scan behind one is work anyone
/// holding the link can trigger repeatedly (takomo-fgca / takomo-vlpm). Truncation
/// is never silent: `next_cursor` is the machine answer and, when there is more, a
/// `warning` spells out the next call in words.
pub async fn self_tickets(
    State(state): State<Arc<AppState>>,
    Extension(share): Extension<ShareCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    let pairs = query_pairs(raw.as_deref());
    let include_archived = match first(&pairs, "include_archived") {
        Some("true" | "1") => true,
        None | Some("false" | "0") => false,
        Some(other) => {
            return Err(ApiError::bad_request(
                "validation.include_archived",
                format!("Query parameter 'include_archived' must be true or false, got '{other}'."),
            ))
        }
    };
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(SHARE_TICKETS_PAGE)
        .clamp(1, SHARE_TICKETS_PAGE);
    let cursor = match first(&pairs, "cursor") {
        None => None,
        Some(c) => Some(c.parse::<i64>().map_err(|_| {
            ApiError::bad_request(
                "validation.cursor",
                "Invalid cursor; pass the exact next_cursor value from the previous page.",
            )
        })?),
    };
    let (tickets, next_cursor) = state.store.share_tickets(
        share.kind,
        &share.ref_id,
        &share.project,
        include_archived,
        cursor,
        limit,
    )?;
    let now = now_ms();
    let items: Vec<Value> = tickets.iter().map(|t| t.to_json(now)).collect();
    let mut out = json!({ "items": items, "next_cursor": next_cursor });
    if let Some(c) = &next_cursor {
        // A page that leaves rows behind says so in words as well as in
        // `next_cursor` — a viewer must never be shown a partial board that looks
        // complete.
        out["warning"] = Value::String(format!(
            "This is a page of {} ticket(s), not the whole share: more remain. Fetch the next page with GET /v1/shares/self/tickets?cursor={c} (add &include_archived=true to keep including archived tickets) and repeat while next_cursor is set.",
            items_len(&out)
        ));
    }
    Ok(Json(out))
}

/// How many items the response actually carries — read back off the built JSON so
/// the number in the warning can never disagree with the array beside it.
fn items_len(out: &Value) -> usize {
    out["items"].as_array().map(Vec::len).unwrap_or(0)
}

/// GET /v1/shares/self/tickets/{id} — one in-scope ticket with comments and
/// dependency detail, for the board's detail panel. A ticket outside the share's
/// scope returns 404 (the share cannot reach it).
pub async fn self_ticket_detail(
    State(state): State<Arc<AppState>>,
    Extension(share): Extension<ShareCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !state
        .store
        .ticket_in_share_scope(share.kind, &share.ref_id, &share.project, &id)?
    {
        return Err(ApiError::not_found("ticket", &id));
    }
    let ticket = state
        .store
        .get_ticket(&id)?
        .ok_or_else(|| ApiError::not_found("ticket", &id))?;
    let now = now_ms();
    let mut out = ticket.to_json(now);

    let comments = state.store.comments_for(&id)?;
    out["comments"] = Value::Array(comments.iter().map(|c| c.to_json()).collect());

    let mut blocked_by_detail = Vec::new();
    for dep_id in &ticket.blocked_by {
        // Only reveal a dependency's title/state when it is itself inside the
        // share's scope; otherwise expose just the bare id so an outside viewer
        // can't learn about tickets in other projects/subtrees via edges.
        let in_scope =
            state
                .store
                .ticket_in_share_scope(share.kind, &share.ref_id, &share.project, dep_id)?;
        match (in_scope, state.store.get_ticket(dep_id)?) {
            (true, Some(d)) => blocked_by_detail.push(json!({
                "id": d.id, "title": d.title, "state": d.state,
                "state_category": d.state_category,
            })),
            _ => blocked_by_detail.push(json!({ "id": dep_id, "out_of_scope": true })),
        }
    }
    out["deps"] = json!({
        "blocked_by": blocked_by_detail,
        "blocks": state.store.blocks_of(&id)?,
    });

    Ok(Json(out))
}
