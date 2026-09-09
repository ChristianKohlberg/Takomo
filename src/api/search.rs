use super::{first, query_pairs, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    query_embeddings::QueryEmbedding,
    server::AppState,
    store::search::{terms, EmbeddingConfig, RESULT_LIMIT},
};
use axum::{
    extract::{Path, RawQuery, State},
    Extension, Json,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub use crate::query_embeddings::QUERY_EMBEDDINGS_PER_MINUTE;

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
    body.as_object_mut().map(|b| b.remove("configured"));
    let key = body
        .as_object_mut()
        .and_then(|b| b.remove("api_key"))
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| ApiError::validation("embeddings.key", "API key must be a string"))
        })
        .transpose()?;
    let config: EmbeddingConfig = serde_json::from_value(body).map_err(|e| {
        ApiError::validation(
            "embeddings.config",
            format!(
                "Embedding settings are replaced whole, never merged: send provider, endpoint, model, dimensions, quiet_seconds and max_wait_seconds together, with api_key only when it changes ({e}). Nothing was changed."
            ),
        )
        .remedy("GET /v1/settings/embeddings, edit the fields you want, and PUT all six back.")
    })?;
    let saved = state.store.save_embedding_config(config, key)?;
    state.query_embeddings.invalidate();
    Ok(Json(saved))
}
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &map, false)?;
    let pairs = query_pairs(raw.as_deref());
    let Some(q) = first(&pairs, "q") else {
        return Err(ApiError::validation(
            "search.query",
            "Query parameter 'q' is required: the words or a description to search this document for, at most 500 characters.",
        )
        .remedy("Retry as GET /v1/mindmaps/{id}/search?q=<query>."));
    };
    let q = q.to_owned();
    if q.chars().count() > 500 {
        return Err(
            ApiError::validation("search.query", "Use at most 500 characters")
                .remedy("Shorten the query and retry."),
        );
    }
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
    if semantic_status == "ready" && !terms(&q).is_empty() {
        let (generation, result) = state
            .query_embeddings
            .get(&config, &key, &ctx.token_id, &q)
            .await;
        // A configuration change cannot make an older request current, including
        // switching away and back to the same provider while it was running.
        let (current, current_key) = state.store.embedding_config()?;
        if !state.query_embeddings.is_current(generation)
            || current.fingerprint() != config.fingerprint()
            || current_key != key
        {
            semantic_status = if current_key.is_empty() {
                "unconfigured"
            } else {
                "indexing"
            };
        } else {
            match result {
                QueryEmbedding::Ready(v) => vector = Some(v),
                QueryEmbedding::Throttled => semantic_status = "throttled",
                QueryEmbedding::Unavailable => semantic_status = "unavailable",
            }
        }
    }
    // A map can move projects while the provider request is in flight.
    authorize(&state, &ctx, &map, false)?;
    let had_vector = vector.is_some();
    let outcome = {
        let state = state.clone();
        let map = map.clone();
        let q = q.clone();
        let fingerprint = config.fingerprint();
        super::blocking_read(move || {
            state.store.search_document(
                &map,
                &q,
                vector.as_deref().map(|v| (v.as_slice(), fingerprint)),
            )
        })
        .await?
    };
    if had_vector && !outcome.used_vectors {
        semantic_status = if outcome.configured {
            "indexing"
        } else {
            "unconfigured"
        };
    }
    let shown = outcome.hits.len();
    let truncated = outcome.candidates > shown;
    let mut body = json!({
        "results": outcome.hits,
        "limit": RESULT_LIMIT,
        "candidates": outcome.candidates,
        "truncated": truncated,
        "mode": if outcome.used_vectors { "hybrid" } else { "keyword" },
        "semantic_status": semantic_status,
        "projection": if outcome.projection_error.is_some() { "stale" } else { "current" },
        "projection_error": outcome.projection_error,
    });
    if truncated {
        body["note"] = json!(format!(
            "Showing the {shown} best-ranked of {} candidate sections. Candidates come from a bounded retrieval (the top {} keyword and top {} semantic chunks), not a count of every section that could match; refine the query to narrow it.",
            outcome.candidates,
            crate::store::search::CANDIDATE_LIMIT,
            crate::store::search::CANDIDATE_LIMIT
        ));
    }
    Ok(Json(body))
}
pub async fn status(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(map): Path<String>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &map, false)?;
    let status = super::blocking_read(move || {
        state.store.project_search(&map, crate::ids::now_ms())?;
        state.store.search_status(&map)
    })
    .await?;
    Ok(Json(status))
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
    let (scheduled, status) = super::blocking_read(move || {
        let scheduled = state
            .store
            .refresh_search(&map, true, crate::ids::now_ms())?;
        Ok((scheduled, state.store.search_status(&map)?))
    })
    .await?;
    Ok(Json(sync_response(status, scheduled)))
}
/// The sync route's body: the status plus whether the manual bypass was applied.
/// A projection that kept being outrun by edits schedules nothing, and saying
/// "scheduled" then would promise work the store never queued.
pub fn sync_response(mut status: Value, scheduled: bool) -> Value {
    status["sync"] = json!(if scheduled { "scheduled" } else { "deferred" });
    if !scheduled {
        status["sync_note"] = json!(format!(
            "Nothing was scheduled: {}. Pending changes still follow the normal quiet delay; sync again once editing pauses to bypass it.",
            crate::store::search::PROJECTION_DEFERRED
        ));
    }
    status
}
