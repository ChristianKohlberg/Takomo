//! /v1/initiatives — initiatives and their entries.
//!
//! MCP came first here (`takomo_initiative_*` in `src/mcp.rs`), because the thing
//! that produces an initiative is an agent in a conversation, not a form. The write
//! routes below exist because `/initiatives` — the page — needs them: a person
//! reading a collection wants to start one, retitle it, park it, and add the
//! feedback they just got, and a browser cannot call an MCP tool. Both surfaces go
//! through the same `Store` methods and the same validators, so neither can drift
//! into accepting what the other refuses.
//!
//! The attachment route is the only endpoint in the API that returns something
//! other than JSON, and it is the reason `content` is never selected by any other
//! query: a document is fetched by itself, once, by the reader that wants it.

use super::{
    body_object, first, get_str, get_string_array, parse_i64_param, query_pairs, reject_unknown,
    require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{
    EntryCreate, InitiativeCreate, InitiativeListFilter, InitiativePatch, MAX_ENTRIES_PAGE,
    MAX_INITIATIVES_PAGE,
};
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

const CREATE_FIELDS: [&str; 7] = [
    "project", "title", "summary", "status", "labels", "tags", "metadata",
];
const PATCH_FIELDS: [&str; 6] = [
    "title",
    "summary",
    "status",
    "labels",
    "tags",
    "metadata_merge",
];
const ENTRY_FIELDS: [&str; 10] = [
    "kind",
    "title",
    "text",
    "source",
    "source_uri",
    "origin_at",
    "content_base64",
    "mime",
    "filename",
    "meta",
];

/// Decode a base64 attachment from the wire.
///
/// Strict (`STANDARD`, padded, no trailing garbage) on purpose: a truncated or
/// mangled upload has to be a validation error the caller can see, never bytes
/// silently stored short. The decoded length is what the store's caps are checked
/// against — the encoding is 4/3 larger and bounding that instead would let a
/// caller past the real limit.
///
/// Shared with the MCP tool rather than reimplemented there: this is the one place
/// that decides what "a valid upload" means, so the two surfaces cannot disagree
/// about it.
pub fn decode_attachment(encoded: &str) -> ApiResult<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| {
            ApiError::validation(
                "validation.entry_content_base64",
                format!(
                    "'content_base64' is not valid base64 ({e}). Use the standard alphabet with padding — the whole document, in one string."
                ),
            )
        })
}

/// Parse an RFC 3339 timestamp to unix milliseconds.
///
/// Only used for `origin_at`, where the caller is stating when something was
/// written. An unparseable value is refused rather than dropped: a wrong
/// provenance date is worse than a missing one. Shared with the MCP tool for the
/// same reason [`decode_attachment`] is.
pub fn parse_rfc3339_ms(raw: &str) -> ApiResult<i64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .map(|dt| dt.timestamp_millis())
        .map_err(|e| {
            ApiError::validation(
                "validation.origin_at",
                format!(
                    "'origin_at' must be an RFC 3339 timestamp such as '2026-07-01T09:00:00Z' ({e}). Omit it when the input originated now — that is already recorded."
                ),
            )
        })
}

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

/// POST /v1/initiatives (write) — open an initiative. Body:
/// `{"project":"tp","title":"Name the thing","summary":"…","tags":["person:ada"]}`.
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
    let req = InitiativeCreate {
        title: require_str(obj, "title")?,
        summary: get_str(obj, "summary")?,
        status: get_str(obj, "status")?,
        labels: get_string_array(obj, "labels")?.unwrap_or_default(),
        tags: get_string_array(obj, "tags")?.unwrap_or_default(),
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
    };
    let ini = state.store.create_initiative(&project, &req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(ini.to_json())))
}

/// PATCH /v1/initiatives/{id} (write) — edit the description: title, summary,
/// status, labels, tags, and/or merge into `metadata`.
///
/// Entries are deliberately not reachable from here. They are append-only — the
/// accumulated record IS the initiative — so the only editable part is how it is
/// described and filed.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    // Scope is checked against the initiative's own project, so naming an id never
    // reaches one the token may not see.
    let existing = state
        .store
        .get_initiative(&id)?
        .ok_or_else(|| ApiError::not_found("initiative", &id))?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &PATCH_FIELDS)?;
    let patch = InitiativePatch {
        title: get_str(obj, "title")?,
        summary: get_str(obj, "summary")?,
        status: get_str(obj, "status")?,
        labels: get_string_array(obj, "labels")?,
        tags: get_string_array(obj, "tags")?,
        metadata_merge: obj.get("metadata_merge").filter(|v| !v.is_null()).cloned(),
    };
    if patch.is_empty() {
        return Err(ApiError::bad_request(
            "validation.no_changes",
            "The patch contains no changes. Provide at least one of 'title', 'summary', 'status', 'labels', 'tags', 'metadata_merge'.",
        ));
    }
    let updated = state.store.patch_initiative(&id, &patch, &ctx.actor)?;
    state.wake();
    Ok(Json(updated.to_json()))
}

/// POST /v1/initiatives/{id}/entries (write) — append one contribution: a note,
/// a research finding, a colleague's feedback, a transcript, a document.
///
/// `source` is required, as it is on the MCP tool: an entry whose origin nobody
/// recorded is text nobody can weigh later. An attachment arrives as
/// `content_base64` because JSON cannot carry bytes, and is decoded here so the
/// store's caps bound the real payload rather than its encoding.
pub async fn create_entry(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let ini = state
        .store
        .get_initiative(&id)?
        .ok_or_else(|| ApiError::not_found("initiative", &id))?;
    ctx.require_project(&ini.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &ENTRY_FIELDS)?;
    let content = match get_str(obj, "content_base64")? {
        None => None,
        Some(encoded) => Some(decode_attachment(&encoded)?),
    };
    let origin_at = match get_str(obj, "origin_at")? {
        None => None,
        Some(raw) => Some(parse_rfc3339_ms(&raw)?),
    };
    let req = EntryCreate {
        kind: require_str(obj, "kind")?,
        title: get_str(obj, "title")?,
        text: get_str(obj, "text")?.unwrap_or_default(),
        content,
        mime: get_str(obj, "mime")?,
        filename: get_str(obj, "filename")?,
        source: require_str(obj, "source")?,
        source_uri: get_str(obj, "source_uri")?,
        origin_at,
        meta: obj.get("meta").filter(|v| !v.is_null()).cloned(),
    };
    let (entry, updated) = state.store.append_initiative_entry(&id, &req, &ctx.actor)?;
    state.wake();
    // The refreshed initiative rides along, so a UI that just appended can update
    // the rollup it is showing without a second request.
    Ok((
        StatusCode::CREATED,
        Json(json!({ "entry": entry.to_json(), "initiative": updated.to_json() })),
    ))
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
