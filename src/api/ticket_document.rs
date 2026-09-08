use super::{first, query_pairs, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::{ticket_document, MindmapListFilter},
};
use axum::{
    extract::{Path, RawQuery, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
fn project(state: &AppState, ctx: &AuthCtx, id: &str) -> ApiResult<String> {
    let t = state
        .store
        .get_ticket(id)?
        .ok_or_else(|| ApiError::not_found("ticket", id))?;
    ctx.require_project(&t.project)?;
    Ok(t.project)
}
pub async fn with_document<F>(state: &Arc<AppState>, project: &str, f: F) -> ApiResult<Value>
where
    F: FnOnce(Option<&Value>) -> ApiResult<Value>,
{
    let (maps, _) = state.store.list_mindmaps(&MindmapListFilter {
        project: Some(project.into()),
        limit: 1,
        ..Default::default()
    })?;
    if let Some(map) = maps.first() {
        let room = super::docsync::open_room(state, &map.id).await?;
        room.read(|doc| {
            let snapshot = ticket_document::document_from_doc(doc, &map.id, &map.title)?;
            f(Some(&snapshot))
        })
    } else {
        f(None)
    }
}
pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let p = project(&state, &ctx, &id)?;
    Ok(Json(
        with_document(&state, &p, |doc| {
            state.store.ticket_document_view(&ctx, &id, doc)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    section_id: String,
    #[serde(default)]
    primary: bool,
    reason: Option<String>,
}
pub async fn add(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Add>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let p = project(&state, &ctx, &id)?;
    let value = with_document(&state, &p, |doc| {
        state.store.add_ticket_document_link(
            &ctx,
            &id,
            &body.section_id,
            body.primary,
            body.reason
                .as_deref()
                .unwrap_or("Attached by a project member"),
            doc.ok_or_else(|| {
                ApiError::validation("validation.ticket_document", "This project has no document")
            })?,
        )
    })
    .await?;
    state.wake();
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    state: Option<String>,
    primary: Option<bool>,
}
pub async fn change(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, link)): Path<(String, String)>,
    ApiJson(body): ApiJson<Change>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let p = project(&state, &ctx, &id)?;
    let result = with_document(&state, &p, |doc| {
        state.store.change_ticket_document_link(
            &ctx,
            &id,
            &link,
            body.state.as_deref(),
            body.primary,
            doc,
        )
    })
    .await?;
    state.wake();
    Ok(Json(result))
}
pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, link)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let p = project(&state, &ctx, &id)?;
    let result = with_document(&state, &p, |doc| {
        state
            .store
            .change_ticket_document_link(&ctx, &id, &link, Some("removed"), None, doc)
    })
    .await?;
    state.wake();
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    request_id: String,
}
pub async fn classify(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Request>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let p = project(&state, &ctx, &id)?;
    let result = with_document(&state, &p, |doc| {
        state
            .store
            .request_ticket_classification(&ctx, &id, &body.request_id, doc)
    })
    .await?;
    state.wake();
    Ok(Json(result))
}
pub async fn config(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.store.ticket_document_config(&ctx, &id)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    mode: String,
}
pub async fn set_config(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Config>,
) -> ApiResult<Json<Value>> {
    let value = state
        .store
        .set_ticket_document_config(&ctx, &id, &body.mode)?;
    state.wake();
    Ok(Json(value))
}
pub async fn backfill(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Request>,
) -> ApiResult<Json<Value>> {
    let value = state
        .store
        .backfill_ticket_classification(&ctx, &id, &body.request_id)?;
    state.wake();
    Ok(Json(value))
}
pub async fn reverse(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    let pairs = query_pairs(raw.as_deref());
    let parse = |key: &str, default: usize, max: usize| -> ApiResult<usize> {
        match first(&pairs, key) {
            None => Ok(default),
            Some(v) => v
                .parse::<usize>()
                .ok()
                .filter(|v| *v <= max)
                .ok_or_else(|| {
                    ApiError::validation("validation.ticket_document", format!("Invalid {key}"))
                }),
        }
    };
    let limit = parse("limit", 100, 500)?;
    let offset = parse("offset", 0, 100000)?;
    if limit == 0 {
        return Err(ApiError::validation(
            "validation.ticket_document",
            "limit must be positive",
        ));
    }
    Ok(Json(state.store.project_document_links(
        &ctx,
        &id,
        first(&pairs, "section_id"),
        limit,
        offset,
    )?))
}
