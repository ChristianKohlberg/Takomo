//! Bug queue and research controls, shared by browser, CLI and MCP callers.
use super::{first, parse_i64_param, query_pairs, ApiJson};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
    store::bugs::{BugPatch, ResearchConfig, ResearchStart, Steering},
};
use axum::{
    extract::{Path, RawQuery, State},
    Extension, Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
fn decode<T: DeserializeOwned>(v: Value) -> ApiResult<T> {
    serde_json::from_value(v).map_err(|e| ApiError::validation("validation.bug", e.to_string()))
}
pub async fn list(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    let p = query_pairs(raw.as_deref());
    Ok(Json(s.store.list_bugs_advanced(
        &c,
        first(&p, "project"),
        first(&p, "triage"),
        first(&p, "severity"),
        parse_i64_param(&p, "limit")?.unwrap_or(50),
        parse_i64_param(&p, "offset")?.unwrap_or(0),
        first(&p, "view").unwrap_or(if first(&p, "all") == Some("true") {
            "all"
        } else {
            "open"
        }),
        first(&p, "q").or(first(&p, "search")),
        first(&p, "state"),
        first(&p, "assignee").or(first(&p, "claimed_by")),
        first(&p, "research_status"),
    )?))
}
pub async fn get(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(s.store.get_bug(&c, &id)?))
}
pub async fn patch(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(v): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let r = s.store.patch_bug(&c, &id, &decode::<BugPatch>(v)?)?;
    s.wake();
    Ok(Json(r))
}
pub async fn config(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(s.store.bug_research_config(&c, &id)?))
}
pub async fn set_config(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(v): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let r = s
        .store
        .set_bug_research_config(&c, &id, &decode::<ResearchConfig>(v)?)?;
    s.wake();
    Ok(Json(r))
}
pub async fn research(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(s.store.bug_research(&c, &id)?))
}
pub async fn start(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(v): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let r = s
        .store
        .start_bug_research(&c, &id, &decode::<ResearchStart>(v)?)?;
    s.wake();
    Ok(Json(r))
}
pub async fn steer(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(v): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let r = s.store.steer_agent_job(&c, &id, &decode::<Steering>(v)?)?;
    s.wake();
    Ok(Json(r))
}
pub async fn cancel(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(v): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    let _: Empty = decode(v)?;
    let r = s.store.cancel_agent_job(&c, &id)?;
    s.wake();
    Ok(Json(r))
}
