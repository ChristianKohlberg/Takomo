//! Document-scoped discussion. The client selects IDs, never supplies source.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::{document_chat::Send, mindmapdoc},
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
    let mut req: Send = serde_json::from_value(body)
        .map_err(|e| ApiError::validation("validation.document_chat", e.to_string()))?;
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
        let chosen:HashSet<&str>=req.section_ids.iter().map(String::as_str).collect();
        for id in &req.section_ids {
            if !nodes.iter().any(|n|&n.id==id) { return Err(ApiError::not_found("section",id)); }
        }
        let ordered=mindmapdoc::tree_order(&nodes);
        let sections:Vec<Value>=ordered.into_iter().filter(|n|req.whole_document||chosen.contains(n.id.as_str())).map(|n| {
            let prose_xml=mindmapdoc::read_section_prose(doc,&n.id).map(|frag|frag.get_string(&doc.transact())).unwrap_or_default();
            json!({"id":n.id,"parent_id":n.parent,"title":n.title,"notes":n.notes,"prose_xml":prose_xml})
        }).collect();
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
