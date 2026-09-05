use super::{
    body_object, first, get_i64, parse_i64_param, query_pairs, reject_unknown, require_str, ApiJson,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
};
use axum::{
    extract::{Path, RawQuery, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use serde_json::Value;
use std::sync::Arc;
fn authorize(state: &AppState, ctx: &AuthCtx, id: &str, scope: &str) -> ApiResult<()> {
    ctx.require_scope(scope)?;
    let map = state
        .store
        .get_mindmap(id)?
        .ok_or_else(|| ApiError::not_found("mindmap", id))?;
    ctx.require_project(&map.project)
}
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &id, "read")?;
    let q = query_pairs(raw.as_deref());
    let before = parse_i64_param(&q, "before")?;
    let limit = parse_i64_param(&q, "limit")?.unwrap_or(30).clamp(1, 100);
    let checkpoints = match first(&q, "checkpoints") {
        None | Some("false") => false,
        Some("true") => true,
        _ => {
            return Err(ApiError::validation(
                "validation.specification_history",
                "checkpoints must be true or false.",
            ))
        }
    };
    Ok(Json(state.store.specification_history(
        &id,
        before,
        limit,
        checkpoints,
    )?))
}
pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, version)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    authorize(&state, &ctx, &id, "read")?;
    Ok(Json(
        tokio::task::spawn_blocking(move || state.store.specification_version(&id, version))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))??,
    ))
}
pub async fn download(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, version)): Path<(String, i64)>,
) -> ApiResult<impl IntoResponse> {
    authorize(&state, &ctx, &id, "read")?;
    let bytes =
        tokio::task::spawn_blocking(move || state.store.specification_version_state(&id, version))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=specification.yjs",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    ))
}
pub async fn checkpoint(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<impl IntoResponse> {
    authorize(&state, &ctx, &id, "write")?;
    let obj = body_object(&body)?;
    reject_unknown(obj, &["expected_version", "name"])?;
    let expected = get_i64(obj, "expected_version")?.ok_or_else(|| {
        ApiError::validation(
            "validation.specification_history",
            "expected_version is required. Read the history head first.",
        )
    })?;
    let name = require_str(obj, "name")?;
    let worker = Arc::clone(&state);
    let result = tokio::task::spawn_blocking(move || {
        worker
            .store
            .checkpoint_specification(&id, expected, &name, &ctx.actor, ctx.user.as_deref())
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    state.wake();
    Ok((StatusCode::CREATED, Json(result)))
}
