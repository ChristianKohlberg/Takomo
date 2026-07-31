//! /v1/initiatives — read surface for initiatives and their entries.
//!
//! Reads only, deliberately. Initiatives are created and fed over MCP
//! (`takomo_initiative_*` in `src/mcp.rs`), because the thing that produces them is
//! an agent in a conversation, not a form. These routes are what a UI reads: list,
//! detail with the rollup, the entry collection, and the bytes of one attachment.
//!
//! The attachment route is the only endpoint in the API that returns something
//! other than JSON, and it is the reason `content` is never selected by any other
//! query: a document is fetched by itself, once, by the reader that wants it.

use super::{first, parse_i64_param, query_pairs};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{InitiativeListFilter, MAX_ENTRIES_PAGE, MAX_INITIATIVES_PAGE};
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

fn parse_cursor(pairs: &[(String, String)]) -> ApiResult<Option<i64>> {
    match first(pairs, "cursor") {
        None => Ok(None),
        Some(c) => Ok(Some(c.parse::<i64>().map_err(|_| {
            ApiError::bad_request(
                "validation.cursor",
                "Invalid cursor; pass the exact next_cursor value from the previous page.",
            )
        })?)),
    }
}

/// GET /v1/initiatives?project=&status=&q=&label=&tag=&limit=&cursor= (read).
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
    let filter = InitiativeListFilter {
        project: first(&pairs, "project").map(str::to_string),
        allowed_projects: ctx.allowed_projects_vec(),
        status: first(&pairs, "status").map(str::to_string),
        q: first(&pairs, "q").map(str::to_string),
        tag: first(&pairs, "tag").map(str::to_string),
        label: first(&pairs, "label").map(str::to_string),
    };
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(50)
        .clamp(1, MAX_INITIATIVES_PAGE);
    let cursor = parse_cursor(&pairs)?;
    let (items, next_cursor) = state.store.list_initiatives(&filter, cursor, limit)?;
    let items: Vec<Value> = items.iter().map(|i| i.to_json()).collect();
    Ok(Json(json!({ "items": items, "next_cursor": next_cursor })))
}

/// GET /v1/initiatives/{id} (read) — the initiative with its derived rollup.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let ini = state
        .store
        .get_initiative(&id)?
        .ok_or_else(|| ApiError::not_found("initiative", &id))?;
    ctx.require_project(&ini.project)?;
    Ok(Json(ini.to_json()))
}

/// GET /v1/initiatives/{id}/entries?limit=&cursor= (read) — the collection,
/// newest first. Entry text is included; attachment bytes are not (`has_content`
/// says whether there are any, and the content route serves them).
pub async fn list_entries(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let ini = state
        .store
        .get_initiative(&id)?
        .ok_or_else(|| ApiError::not_found("initiative", &id))?;
    ctx.require_project(&ini.project)?;
    let pairs = query_pairs(raw.as_deref());
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(50)
        .clamp(1, MAX_ENTRIES_PAGE);
    let cursor = parse_cursor(&pairs)?;
    let (entries, next_cursor) = state.store.list_initiative_entries(&id, cursor, limit)?;
    let items: Vec<Value> = entries.iter().map(|e| e.to_json()).collect();
    Ok(Json(json!({
        "items": items,
        "next_cursor": next_cursor,
        "rollup": ini.rollup.to_json(),
    })))
}

/// GET /v1/initiatives/{id}/entries/{entry}/content (read) — the raw attachment
/// bytes, served under the entry's own mime and filename.
///
/// `Content-Disposition: attachment` rather than `inline`: these bytes were
/// uploaded by an agent and are echoed back verbatim, so nothing about them should
/// ever be rendered as active content in the origin that serves the board.
pub async fn entry_content(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, entry)): Path<(String, String)>,
) -> ApiResult<Response> {
    ctx.require_scope("read")?;
    let ini = state
        .store
        .get_initiative(&id)?
        .ok_or_else(|| ApiError::not_found("initiative", &id))?;
    ctx.require_project(&ini.project)?;
    let found = state
        .store
        .initiative_entry_content(&id, &entry)?
        .ok_or_else(|| ApiError::not_found("initiative_entry", &entry))?;
    let (content, mime, filename) = found;
    // A distinct code from `notfound.initiative_entry`: the entry is there, it just
    // has no bytes. A caller that conflated the two would go looking for a missing
    // id when the answer is "read its text instead".
    let bytes = content.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "initiative.no_attachment",
            format!("Entry '{entry}' is text-only and has no attachment to download."),
        )
        .remedy(
            "Its text is already in GET /v1/initiatives/{id}/entries. Only entries with 'has_content': true have bytes to fetch.",
        )
    })?;
    // Safe to serve verbatim: `store::initiatives::validate_mime` accepted it as a
    // bare `type/subtype` of header token characters when the entry was appended.
    let mime = mime.unwrap_or_else(|| "application/octet-stream".to_string());
    let disposition = match filename {
        // Quoted, with quotes and control characters stripped: the filename comes
        // from an API caller, and an unescaped one could otherwise inject header
        // parameters.
        Some(name) => {
            let safe: String = name
                .chars()
                .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
                .collect();
            format!("attachment; filename=\"{safe}\"")
        }
        None => "attachment".to_string(),
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_DISPOSITION, disposition),
            // Belt to the disposition's braces: never let a sniffer decide these
            // agent-supplied bytes are HTML.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response())
}
