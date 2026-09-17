//! Human review routing over the existing shared document comment threads.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::document_reviews::{self, ReviewAction, SendReview},
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
#[derive(Deserialize)]
pub struct List {
    project: Option<String>,
    mindmap: Option<String>,
    queue: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}
pub async fn list(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Query(q): Query<List>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let limit = q.limit.unwrap_or(30);
    let offset = q.offset.unwrap_or(0);
    if limit == 0 || limit > 100 || offset > 1_000_000 {
        return Err(document_reviews::invalid(
            "Use limit 1–100 and offset up to 1000000.",
        ));
    }
    let mut result = s.store.review_list(
        &ctx,
        q.project.as_deref(),
        q.mindmap.as_deref(),
        q.queue.as_deref().unwrap_or("needs_me"),
        limit,
        offset,
    )?;
    for review in result["items"].as_array_mut().unwrap() {
        hydrate(&s, review).await?;
    }
    Ok(Json(result))
}
async fn hydrate(s: &Arc<AppState>, v: &mut Value) -> ApiResult<()> {
    let map = v["mindmap"].as_str().unwrap();
    let ids = serde_json::from_value::<Vec<String>>(v["thread_ids"].clone()).unwrap();
    let room = super::docsync::open_room(s, map).await?;
    match room.read(|doc| document_reviews::threads(doc, &ids)) {
        Ok(threads) => {
            v["snapshot"] = threads;
            v["source_missing"] = json!(false);
        }
        Err(_) => v["source_missing"] = json!(true),
    }
    Ok(())
}
pub async fn get(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    view(&s, &ctx, &id).await
}
async fn view(s: &Arc<AppState>, ctx: &AuthCtx, id: &str) -> ApiResult<Json<Value>> {
    let mut v = s.store.review(ctx, id)?;
    hydrate(s, &mut v).await?;
    Ok(Json(v))
}
pub async fn send(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    ApiJson(req): ApiJson<SendReview>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    req.validate()?;
    let row = s
        .store
        .get_mindmap(&map)?
        .ok_or_else(|| ApiError::not_found("mindmap", &map))?;
    ctx.require_project(&row.project)?;
    if let Some(id) = s.store.review_retry(&ctx, &map, &req)? {
        return view(&s, &ctx, &id).await;
    }
    let id = format!("rev-{}", crate::ids::ticket_suffix(16));
    let room = super::docsync::open_room(&s, &map).await?;
    let result = room
        .mutate_durable_with(
            |doc| document_reviews::publish(doc, &ctx, &req),
            false,
            |blob, threads| s.store.save_review(&ctx, &map, &req, &id, threads, blob),
        )
        .await;
    // Concurrent identical sends converge on the committed receipt.
    if result.is_err() {
        if let Some(previous) = s.store.review_retry(&ctx, &map, &req)? {
            return view(&s, &ctx, &previous).await;
        }
    }
    result?;
    s.wake();
    view(&s, &ctx, &id).await
}
pub async fn action(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<ReviewAction>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    let review = s.store.review(&ctx, &id)?;
    if s.store.review_action_retry(&ctx, &id, &req)? {
        return view(&s, &ctx, &id).await;
    }
    let map = review["mindmap"].as_str().unwrap();
    let room = super::docsync::open_room(&s, map).await?;
    let result = room
        .mutate_durable_with(
            |doc| document_reviews::apply_action(doc, &ctx, &review, &req),
            false,
            |blob, threads| s.store.save_review_action(&ctx, &id, &req, threads, blob),
        )
        .await;
    if result.is_err() && s.store.review_action_retry(&ctx, &id, &req)? {
        return view(&s, &ctx, &id).await;
    }
    result?;
    s.wake();
    view(&s, &ctx, &id).await
}
