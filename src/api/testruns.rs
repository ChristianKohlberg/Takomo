//! Definitions are editable; runs and results refer to immutable snapshots.
use super::{body_object, first, query_pairs, reject_unknown, require_str, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::testruns::{ResultCreate, RunCreate},
};
use axum::{
    extract::{Path, RawQuery, State},
    http::StatusCode,
    Extension, Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

fn decode<T: DeserializeOwned>(value: Value) -> ApiResult<T> {
    serde_json::from_value(value)
        .map_err(|error| ApiError::validation("validation.test_run", error.to_string()))
}
pub async fn definitions(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let offset = super::parse_i64_param(&pairs, "offset")?.unwrap_or(0);
    let limit = super::parse_i64_param(&pairs, "limit")?.unwrap_or(50);
    Ok(Json(
        super::blocking_read(move || state.store.list_test_definitions(&project, offset, limit))
            .await?,
    ))
}
pub async fn definition(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let value = state.store.test_definition(&id)?;
    ctx.require_project(value["project"].as_str().unwrap())?;
    Ok(Json(value))
}
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let pairs = query_pairs(raw.as_deref());
    let limit = super::parse_i64_param(&pairs, "limit")?.unwrap_or(30);
    let cursor = first(&pairs, "cursor").map(String::from);
    Ok(Json(
        super::blocking_read(move || {
            state
                .store
                .list_test_runs(&project, cursor.as_deref(), limit)
        })
        .await?,
    ))
}
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let req: RunCreate = decode(body)?;
    let result = state.store.create_test_run(&project, &req, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(result)))
}
pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let result = state.store.get_test_run(&id)?;
    ctx.require_project(result["project"].as_str().unwrap())?;
    Ok(Json(result))
}
pub async fn transition(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    let current = state.store.get_test_run(&id)?;
    ctx.require_project(current["project"].as_str().unwrap())?;
    let object = body_object(&body)?;
    reject_unknown(object, &["action"])?;
    let result =
        state
            .store
            .transition_test_run(&id, &require_str(object, "action")?, &ctx.actor)?;
    state.wake();
    Ok(Json(result))
}
pub async fn result(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    ctx.require_scope("write")?;
    let current = state.store.get_test_run(&id)?;
    ctx.require_project(current["project"].as_str().unwrap())?;
    let request: ResultCreate = decode(body)?;
    if request.actor_kind == "human" {
        ctx.require_scope("human")?;
    }
    let result = state
        .store
        .record_test_result(&id, &request, &ctx.actor, ctx.user.as_deref())?;
    state.wake();
    Ok((StatusCode::CREATED, Json(result)))
}
pub async fn retry(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    ctx.require_scope("write")?;
    let current = state.store.get_test_run(&id)?;
    ctx.require_project(current["project"].as_str().unwrap())?;
    let object = body_object(&body)?;
    reject_unknown(object, &["idempotency_key"])?;
    let result =
        state
            .store
            .retry_test_run(&id, &require_str(object, "idempotency_key")?, &ctx.actor)?;
    state.wake();
    Ok((StatusCode::CREATED, Json(result)))
}
