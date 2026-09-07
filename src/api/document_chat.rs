//! Document-scoped discussion. The client selects IDs, never supplies source.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::{
        document_chat::{Context, Send},
        mindmapdoc,
    },
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use yrs::{GetString, Transact};

pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let row = state
        .store
        .get_mindmap(&map)?
        .ok_or_else(|| ApiError::not_found("mindmap", &map))?;
    ctx.require_project(&row.project)?;
    Ok(Json(state.store.document_conversation(&map)?))
}
pub async fn send(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let new_context = body.get("context").is_some()
        || (body.get("section_ids").is_none() && body.get("whole_document").is_none());
    if body.get("context").is_some()
        && (body.get("section_ids").is_some() || body.get("whole_document").is_some())
    {
        return Err(ApiError::validation(
            "validation.document_chat",
            "Do not combine context with legacy scope fields",
        ));
    }
    let mut req: Send = serde_json::from_value(body)
        .map_err(|e| ApiError::validation("validation.document_chat", e.to_string()))?;
    if new_context && req.context.is_none() {
        req.context = Some(Context::default());
    }
    req.validate()?;
    let row = state
        .store
        .get_mindmap(&map)?
        .ok_or_else(|| ApiError::not_found("mindmap", &map))?;
    ctx.require_project(&row.project)?;
    if let Some(previous) = state
        .store
        .retry_document_message(&ctx, &map, &row.project, &req)?
    {
        return Ok(Json(previous));
    }
    let room = crate::api::docsync::open_room(&state, &map).await?;
    let snapshot=room.read(|doc| {
        let (_,_,nodes)=mindmapdoc::snapshot(doc,&map);
        if req.context.is_some() && nodes.len()>500 { return Err(ApiError::validation("validation.document_chat","Document workspace supports at most 500 total sections. Split this document before asking Codex.")); }
        let chosen:HashSet<&str>=req.section_ids.iter().map(String::as_str).collect();
        let all_ids:Vec<_>=if let Some(c)=&req.context { c.section_ids.iter().chain(&c.pinned_section_ids).collect() } else { req.section_ids.iter().collect() };
        for id in all_ids {
            if !nodes.iter().any(|n|&n.id==id) { return Err(ApiError::not_found("section",id)); }
        }
        let ordered=mindmapdoc::tree_order(&nodes);
        let sections:Vec<Value>=ordered.into_iter().filter(|n|req.context.is_some()||req.whole_document||chosen.contains(n.id.as_str())).map(|n| {
            let prose_xml=mindmapdoc::read_section_prose(doc,&n.id).map(|frag|frag.get_string(&doc.transact())).unwrap_or_default();
            let mut section=json!({"id":n.id,"parent_id":n.parent,"title":n.title,"notes":n.notes,"prose_xml":prose_xml});
            if req.context.is_some() { section["version"]=json!(crate::ids::sha256_hex(section.to_string().as_bytes())); }
            section
        }).collect();
        if let Some(context)=&req.context {
            if let Some(quote)=&context.quote {
                let source=sections.iter().find(|s|s["id"].as_str()==Some(&quote.section_id)).unwrap()["notes"].as_str().unwrap();
                let normalized=|s:&str|s.split_whitespace().collect::<Vec<_>>().join(" ");
                if !normalized(source).contains(&normalized(&quote.text)) { return Err(ApiError::conflict("conflict.agent_job","The selected text changed. Select it again before sending.")); }
            }
            return Ok((json!({"kind":"document_workspace","schema_version":2,"mindmap_id":map,"title":row.title,"context":context,"sections":sections,"action":req.action}).to_string(),context.section_ids.clone()));
        }
        let ids=if req.whole_document {vec![]} else {sections.iter().map(|s|s["id"].as_str().unwrap().to_owned()).collect()};
        Ok((json!({"kind":"document_chat","mindmap_id":map,"title":row.title,"scope":{"whole_document":req.whole_document,"section_ids":ids},"sections":sections,"action":req.action}).to_string(),ids))
    })?;
    req.section_ids = snapshot.1;
    let result = state
        .store
        .send_document_message(&ctx, &map, &row.project, &snapshot.0, &req)?;
    state.wake();
    Ok(Json(result))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pins {
    pinned_section_ids: Vec<String>,
}
pub async fn pins(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    ApiJson(body): ApiJson<Pins>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let row = state
        .store
        .get_mindmap(&map)?
        .ok_or_else(|| ApiError::not_found("mindmap", &map))?;
    ctx.require_project(&row.project)?;
    let room = crate::api::docsync::open_room(&state, &map).await?;
    room.read(|doc| {
        let (_, _, nodes) = mindmapdoc::snapshot(doc, &map);
        for id in &body.pinned_section_ids {
            if !nodes.iter().any(|n| &n.id == id) {
                return Err(ApiError::not_found("section", id));
            }
        }
        state
            .store
            .set_document_pins(&ctx, &map, &row.project, &body.pinned_section_ids)
    })?;
    Ok(Json(state.store.document_conversation(&map)?))
}
