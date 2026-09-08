use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::search::EmbeddingConfig,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

fn authorize(state: &AppState, ctx: &AuthCtx, map: &str, write: bool) -> ApiResult<()> {
    ctx.require_scope("read")?;
    if write {
        ctx.require_scope("write")?;
    }
    let row = state
        .store
        .get_mindmap(map)?
        .ok_or_else(|| ApiError::not_found("mindmap", map))?;
    ctx.require_project(&row.project)?;
    if write
        && state
            .store
            .get_project(&row.project)?
            .is_some_and(|p| p.archived_at.is_some())
    {
        return Err(ApiError::conflict(
            "conflict.project_archived",
            "Project is archived",
        ));
    }
    Ok(())
}
fn admin(ctx: &AuthCtx) -> ApiResult<()> {
    ctx.require_scope("admin")?;
    if ctx.projects.is_some() {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "embeddings.global",
            "Global embedding configuration requires an unrestricted admin token",
        ));
    }
    Ok(())
}
pub async fn settings(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let (c, key) = state.store.embedding_config()?;
    let mut value = serde_json::to_value(c).unwrap();
    value["configured"] = json!(!key.is_empty());
    Ok(Json(value))
}
pub async fn save_settings(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(mut body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let key = body
        .as_object_mut()
        .and_then(|b| b.remove("api_key"))
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| ApiError::validation("embeddings.key", "API key must be a string"))
        })
        .transpose()?;
    let config: EmbeddingConfig = serde_json::from_value(body)
        .map_err(|e| ApiError::validation("embeddings.config", e.to_string()))?;
    Ok(Json(state.store.save_embedding_config(config, key)?))
}
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &map, false)?;
    if query.q.chars().count() > 500 {
        return Err(ApiError::validation(
            "search.query",
            "Use at most 500 characters",
        ));
    }
    state
        .store
        .refresh_search(&map, false, crate::ids::now_ms())?;
    let (config, key) = state.store.embedding_config()?;
    let status = state.store.search_status(&map)?;
    let mut semantic_status = if key.is_empty() {
        "unconfigured"
    } else if status["indexed"].as_i64().unwrap_or(0) == 0 {
        "indexing"
    } else {
        "ready"
    };
    let mut vector = None;
    if semantic_status == "ready" && !query.q.trim().is_empty() {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::embeddings::embed(&config, &key, std::slice::from_ref(&query.q), true),
        )
        .await
        {
            Ok(Ok(mut v)) => vector = v.pop(),
            _ => semantic_status = "unavailable",
        }
    }
    // A map can move projects while the provider request is in flight.
    authorize(&state, &ctx, &map, false)?;
    let (results, used_vectors) = state.store.search_document(
        &map,
        &query.q,
        vector.as_deref().map(|v| (v, config.fingerprint())),
    )?;
    if vector.is_some() && !used_vectors {
        semantic_status = if state.store.embedding_config()?.1.is_empty() {
            "unconfigured"
        } else {
            "indexing"
        };
    }
    Ok(Json(
        json!({"results":results,"mode":if used_vectors{"hybrid"}else{"keyword"},"semantic_status":semantic_status}),
    ))
}
pub async fn status(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &map, false)?;
    state
        .store
        .refresh_search(&map, false, crate::ids::now_ms())?;
    Ok(Json(state.store.search_status(&map)?))
}
pub async fn sync(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    ApiJson(_): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &map, true)?;
    let room = crate::api::docsync::open_room(&state, &map).await?;
    crate::api::docsync::flush(&state, &room, &ctx.actor).await;
    state
        .store
        .refresh_search(&map, true, crate::ids::now_ms())?;
    Ok(Json(state.store.search_status(&map)?))
}
