//! Small, explicitly initiated imports populate the existing document/map for review.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::mindmapdoc::{self, NodeAdd},
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use yrs::{Map, Transact};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    request_id: String,
    revision: String,
    scope: Scope,
    draft: Draft,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    include: Vec<String>,
    exclude: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    title: String,
    summary: String,
    sections: Vec<Section>,
    gaps: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    key: String,
    parent: Option<String>,
    title: String,
    notes: String,
    sources: Vec<Source>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    path: String,
    start_line: u64,
    end_line: u64,
}
fn invalid(message: &str) -> ApiError {
    ApiError::validation("validation.spec_import", message)
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.spec_import", message)
}
fn under(path: &str, parent: &str) -> bool {
    parent == "." || path == parent || path.starts_with(&format!("{parent}/"))
}
fn path_ok(path: &str) -> bool {
    path == "."
        || (!path.is_empty()
            && path.len() <= 1024
            && !path.starts_with('/')
            && !path.contains('\\')
            && !path.chars().any(char::is_control)
            && !(path.as_bytes().get(1) == Some(&b':'))
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}
impl Import {
    fn validate(&self) -> ApiResult<()> {
        if self.request_id.is_empty()
            || self.request_id.len() > 120
            || self.request_id.chars().any(char::is_control)
        {
            return Err(invalid("request_id must contain 1–120 printable bytes."));
        }
        if ![40, 64].contains(&self.revision.len())
            || !self.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("revision must be an exact Git commit hash."));
        }
        if self.scope.include.is_empty()
            || self.scope.include.len() > 64
            || self.scope.exclude.len() > 64
            || !self
                .scope
                .include
                .iter()
                .chain(&self.scope.exclude)
                .all(|p| path_ok(p))
        {
            return Err(invalid(
                "Use explicit literal include paths and optional exclude paths.",
            ));
        }
        if self.draft.sections.is_empty()
            || self.draft.sections.len() > 12
            || self.draft.gaps.len() > 12
        {
            return Err(invalid(
                "MVP imports contain 1–12 sections and at most 12 gaps.",
            ));
        }
        mindmapdoc::validate_title(&self.draft.title)?;
        if self.draft.summary.len() > 2000 || self.draft.gaps.iter().any(|g| g.len() > 1000) {
            return Err(invalid("Import summary or gaps exceed their limit."));
        }
        let mut depths = HashMap::new();
        for section in &self.draft.sections {
            if section.key.is_empty() || section.key.len() > 40 || depths.contains_key(&section.key)
            {
                return Err(invalid("Section keys must be unique and at most 40 bytes."));
            }
            let depth = match &section.parent {
                None => 1,
                Some(p) => {
                    depths
                        .get(p)
                        .copied()
                        .ok_or_else(|| invalid("Parents must precede children."))?
                        + 1
                }
            };
            if depth > 4 {
                return Err(invalid("MVP imports allow four section levels."));
            }
            depths.insert(section.key.clone(), depth);
            mindmapdoc::validate_title(&section.title)?;
            if section.notes.trim().is_empty()
                || section.notes.len() > 4000
                || section.sources.is_empty()
                || section.sources.len() > 5
            {
                return Err(invalid(
                    "Sections require prose (at most 4000 bytes) and 1–5 source ranges.",
                ));
            }
            for source in &section.sources {
                if !path_ok(&source.path)
                    || source.path == "."
                    || source.start_line == 0
                    || source.end_line < source.start_line
                    || !self.scope.include.iter().any(|p| under(&source.path, p))
                    || self.scope.exclude.iter().any(|p| under(&source.path, p))
                {
                    return Err(invalid("Source ranges must lie within the declared scope."));
                }
            }
        }
        Ok(())
    }
}
fn add(
    doc: &yrs::Doc,
    parent: Option<String>,
    title: String,
    notes: String,
    actor: &str,
) -> ApiResult<String> {
    let result = mindmapdoc::add_nodes(
        doc,
        &[NodeAdd {
            parent,
            title,
            notes: Some(notes),
            position: None,
            kind: None,
            origin: Some("agent".into()),
            edge_label: None,
            by_user: None,
        }],
        actor,
    )?;
    Ok(result[0].0.clone())
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_scope("human")?;
    if body.to_string().len() > 64_000 {
        return Err(invalid("Import payload exceeds 64 KB."));
    }
    let request: Import =
        serde_json::from_value(body).map_err(|_| invalid("Invalid import payload fields."))?;
    request.validate()?;
    let map = state
        .store
        .get_mindmap(&id)?
        .ok_or_else(|| ApiError::not_found("mindmap", &id))?;
    ctx.require_project(&map.project)?;
    state.store.ensure_collab_writable(&id)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&request).unwrap())
    );
    let receipt_key = format!(
        "{:x}",
        Sha256::digest(format!("{}:{}", ctx.actor, request.request_id))
    );
    let room = super::docsync::open_room(&state, &id).await?;
    let result = room.mutate_durable(&state.store, &ctx.actor, |doc| {
        let receipts = doc.get_or_insert_map("spec_import_receipts");
        if let Some(yrs::Out::Any(yrs::Any::String(raw))) = receipts.get(&doc.transact(), &receipt_key) {
            let receipt: Value = serde_json::from_str(&raw).map_err(|_| conflict("Import receipt is invalid; inspect the document before retrying."))?;
            if receipt["digest"] != digest { return Err(conflict("This request_id was used with another draft. Use a new request_id.")); }
            return Ok(receipt["result"].clone());
        }
        if !mindmapdoc::snapshot(doc, &id).2.is_empty() { return Err(conflict("Import requires an empty specification. Existing sections are never replaced.")); }
        let root_notes = format!("Unreviewed codebase draft.\n{}\nSource commit: {}\nIncluded paths: {}\nExcluded paths: {}\nCoverage is limited to the selected code; runtime behavior was not tested.\n{}", request.draft.summary, request.revision, request.scope.include.join(", "), request.scope.exclude.join(", "), request.draft.gaps.iter().map(|g| format!("Open question: {g}")).collect::<Vec<_>>().join("\n"));
        let root = add(doc, None, request.draft.title.clone(), root_notes, &ctx.actor)?;
        let mut ids = HashMap::new();
        for section in &request.draft.sections {
            let parent = section.parent.as_ref().and_then(|p| ids.get(p)).cloned().unwrap_or_else(|| root.clone());
            let sources = section.sources.iter().map(|s| format!("Source: {}:{}–{} @ {}",s.path,s.start_line,s.end_line,request.revision)).collect::<Vec<_>>().join("\n");
            let node = add(doc, Some(parent), section.title.clone(), format!("{}\n{}", section.notes, sources), &ctx.actor)?;
            ids.insert(section.key.clone(), node);
        }
        let result = json!({"mindmap":id,"root":root,"sections":ids,"reviewed":false,"revision":request.revision});
        receipts.insert(&mut doc.transact_mut(),receipt_key.clone(),json!({"digest":digest,"result":result}).to_string());
        Ok(result)
    })?;
    let size = room.read(|doc| mindmapdoc::snapshot(doc, &id).2.len() as i64);
    state.store.note_mindmap_size(&id, size)?;
    state.wake();
    Ok(Json(result))
}
