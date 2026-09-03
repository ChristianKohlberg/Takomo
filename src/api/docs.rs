//! Collaborative documents — `/v1/projects/{project}/documents` and
//! `/v1/documents/{id}`.
//!
//! These are the ordinary JSON routes: create one, list them, rename it, file it
//! in a folder, archive it. **They never carry the prose.** The prose is a Yjs
//! CRDT and moves over the WebSocket session in `crate::api::docsync`, because
//! the whole point of this surface is that two people typing at once both keep
//! their words — and a `PUT body` is last-write-wins by construction.
//!
//! So a caller sees a document's title, folder, status and how much history it
//! holds; to read or change what it *says*, join the session.
//!
//! Writes take `write`, not `human`, matching environments: an agent distilling a
//! conversation into a document is exactly the caller this serves, and gating it
//! on a person would push that back out of band.

use super::{
    body_object, first, get_str, parse_i64_param, query_pairs, reject_unknown, require_str, ApiJson,
};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::{DocumentCreate, DocumentFilter, DocumentPatch, MAX_DOCUMENTS_PAGE};
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

const DOC_CREATE_FIELDS: [&str; 5] = ["title", "path", "status", "initiative", "metadata"];

const DOC_PATCH_FIELDS: [&str; 5] = ["title", "path", "status", "initiative", "metadata_merge"];

/// Read a field that is present-but-null distinctly from absent, so "clear this
/// again" stays expressible once a value has been set.
fn override_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> ApiResult<Option<Option<String>>> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a string or null."),
        )),
    }
}

/// POST /v1/projects/{project}/documents (write) — open a new document.
///
/// The document starts empty. There is no `body` field and deliberately so: the
/// first paragraph is typed into the session like every later one, which keeps
/// exactly one write path into the prose.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &DOC_CREATE_FIELDS)?;
    let req = DocumentCreate {
        project: project.clone(),
        title: require_str(obj, "title")?,
        path: get_str(obj, "path")?,
        status: get_str(obj, "status")?,
        initiative: get_str(obj, "initiative")?,
        metadata: obj.get("metadata").filter(|v| !v.is_null()).cloned(),
    };
    let doc = state.store.create_document(&req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(doc.to_json())))
}

/// GET /v1/projects/{project}/documents?status=&initiative=&q=&archived=include&limit= (read).
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let filter = DocumentFilter {
        project: project.clone(),
        status: first(&pairs, "status").map(str::to_string),
        // `?initiative=none` narrows to documents no initiative claims, the same
        // shape `?epic=none` uses on tickets.
        initiative: first(&pairs, "initiative").map(|v| {
            if v == "none" {
                String::new()
            } else {
                v.to_string()
            }
        }),
        q: first(&pairs, "q").map(str::to_string),
        include_archived: first(&pairs, "archived") == Some("include"),
        limit: parse_i64_param(&pairs, "limit")?,
    };
    let limit = filter
        .limit
        .unwrap_or(MAX_DOCUMENTS_PAGE)
        .clamp(1, MAX_DOCUMENTS_PAGE);
    let (docs, total) = state.store.list_documents(&filter)?;
    Ok(Json(super::paged(
        docs.iter().map(|d| d.to_json()).collect::<Vec<_>>(),
        total,
        limit,
        "Raise the page size with ?limit=N (max 200), or narrow with ?status=, \
         ?initiative= or ?q=.",
    )))
}

/// GET /v1/documents/{id} (read) — the document's filing, not its prose.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let doc = state.store.get_document(&id)?;
    ctx.require_project(&doc.project)?;
    Ok(Json(doc.to_json()))
}

/// PATCH /v1/documents/{id} (write). `initiative` accepts an explicit null to
/// clear it.
pub async fn patch(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_document(&id)?;
    ctx.require_project(&existing.project)?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &DOC_PATCH_FIELDS)?;
    let req = DocumentPatch {
        title: get_str(obj, "title")?,
        path: get_str(obj, "path")?,
        status: get_str(obj, "status")?,
        initiative: override_str(obj, "initiative")?,
        metadata_merge: obj.get("metadata_merge").cloned(),
    };
    let doc = state.store.patch_document(&id, &req, &ctx.actor)?;
    state.wake();
    Ok(Json(doc.to_json()))
}

/// DELETE /v1/documents/{id} (write) — archive it.
///
/// Archive rather than delete, and the update log is left exactly as it was. That
/// is what makes unarchiving honest: the prose comes back, not a copy of it that
/// lost the history of how it was argued into shape.
pub async fn archive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_document(&id)?;
    ctx.require_project(&existing.project)?;
    let doc = state.store.archive_document(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(doc.to_json()))
}

/// POST /v1/documents/{id}/unarchive (write) — bring it back.
pub async fn unarchive(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let existing = state.store.get_document(&id)?;
    ctx.require_project(&existing.project)?;
    let doc = state.store.unarchive_document(&id, &ctx.actor)?;
    state.wake();
    Ok(Json(doc.to_json()))
}

/// POST /v1/documents/{id}/run (write) — the prompt bar.
///
/// The one route in this server that calls a language model. `src/docagent.rs`
/// carries the argument for why that exception exists; what matters here is what
/// it does NOT change: the model's answer goes through exactly the same
/// `validate_ops` a fleet agent's does, and it lands as a **proposal** a person
/// still has to accept. It has no privileged path into the prose.
pub async fn run_agent(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let doc = state.store.get_document(&id)?;
    ctx.require_project(&doc.project)?;
    state.store.ensure_collab_writable(&id)?;

    let cfg = state
        .doc_agent
        .as_ref()
        .ok_or_else(crate::docagent::not_configured)?;

    let obj = body_object(&body)?;
    reject_unknown(obj, &["instruction", "scope", "model"])?;
    let instruction = require_str(obj, "instruction")?;
    let trimmed = instruction.trim();
    if trimmed.is_empty() || trimmed.len() > crate::docagent::MAX_INSTRUCTION {
        return Err(ApiError::validation(
            "validation.document_instruction",
            format!(
                "An instruction must be 1..={} characters; got {}.",
                crate::docagent::MAX_INSTRUCTION,
                trimmed.len()
            ),
        )
        .remedy("Say what you want changed in a sentence.".to_string()));
    }
    let scope: Option<Vec<String>> = match obj.get("scope") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        Some(_) => {
            return Err(ApiError::bad_request(
                "validation.field_type",
                "Field 'scope' must be an array of block ids or null.".to_string(),
            ))
        }
    };
    let model = get_str(obj, "model")?;

    // Read the LIVE replica, not the persisted log: the log is up to a flush
    // behind, and block ids the reader has already moved past would make every
    // op the model wrote get dropped as stale.
    let room = crate::api::docsync::open_room(&state, &id).await?;
    let annotated = room.read(|d| {
        let frag = d.get_or_insert_xml_fragment(crate::api::docprops::PROSE_FIELD);
        let txn = yrs::Transact::transact(d);
        let blocks = crate::api::docprops::read_blocks(&txn, &frag);
        drop(txn);
        crate::api::docprops::annotate(&blocks)
    });

    let plan =
        crate::docagent::run(cfg, trimmed, &annotated, scope.as_deref(), model.as_deref()).await?;

    // Re-read inside the mutation: the document may have moved while the model
    // was thinking, which is the ordinary case this whole surface is built for.
    let actor = format!("{} (prompt)", ctx.actor);
    let now = crate::ids::now_ms();
    let summary = plan.summary.clone();
    let (proposal, applied, skipped) = room.mutate(|d| {
        let frag = d.get_or_insert_xml_fragment(crate::api::docprops::PROSE_FIELD);
        let txn = yrs::Transact::transact(d);
        let blocks = crate::api::docprops::read_blocks(&txn, &frag);
        drop(txn);

        let validated = crate::api::docprops::validate_ops(
            &plan.ops,
            &blocks,
            scope.as_deref(),
            "takomo_document_read",
        )?;
        let pid = crate::api::docprops::write_proposal(
            d,
            None,
            &actor,
            trimmed,
            &summary,
            &validated.ops,
            &validated.skipped,
            now,
        )?;
        Ok((pid, validated.ops.len(), validated.skipped))
    })?;

    Ok(Json(json!({
        "proposal": proposal,
        "document": id,
        "status": "pending",
        "summary": plan.summary,
        "operations": applied,
        "skipped": skipped,
        "model": model.unwrap_or_else(|| cfg.model.clone()),
    })))
}
