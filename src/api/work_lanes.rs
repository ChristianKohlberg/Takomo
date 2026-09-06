//! Lanes and explicitly authorized, leased agent handoffs.
use super::{
    body_object, first, get_str, parse_i64_param, query_pairs, reject_unknown, require_str, ApiJson,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
};
use axum::{
    extract::{Path, RawQuery, State},
    Extension, Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
fn access(ctx: &AuthCtx, v: &Value, scope: &str) -> ApiResult<()> {
    ctx.require_scope(scope)?;
    ctx.require_project(v["project"].as_str().unwrap())
}
fn paging(raw: Option<&str>) -> ApiResult<(i64, i64)> {
    let p = query_pairs(raw);
    let l = parse_i64_param(&p, "limit")?.unwrap_or(100).clamp(1, 200);
    let o = parse_i64_param(&p, "offset")?.unwrap_or(0);
    if o < 0 {
        return Err(ApiError::bad_request(
            "validation.field_type",
            "offset must be nonnegative.",
        ));
    }
    Ok((l, o))
}
pub async fn create(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    let o = body_object(&body)?;
    reject_unknown(o, &["title", "purpose", "context"])?;
    let v = s.store.work_lane_create(
        &project,
        &require_str(o, "title")?,
        &get_str(o, "purpose")?.unwrap_or_default(),
        &get_str(o, "context")?.unwrap_or_default(),
        &ctx.actor,
    )?;
    s.wake();
    Ok(Json(v))
}
pub async fn list(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let (l, o) = paging(raw.as_deref())?;
    let (items, total) = s.store.work_lane_list(&project, l, o)?;
    Ok(Json(super::paged(
        items,
        total,
        l,
        "Continue with ?offset=N&limit=N (maximum 200).",
    )))
}
pub async fn get(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let v = s.store.work_lane_get(&id)?;
    access(&ctx, &v, "read")?;
    Ok(Json(v))
}
pub async fn patch(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    access(&ctx, &s.store.work_lane_get(&id)?, "write")?;
    let v = s.store.work_lane_patch(&id, &body, &ctx.actor)?;
    s.wake();
    Ok(Json(v))
}
async fn membership(
    s: Arc<AppState>,
    ctx: AuthCtx,
    id: String,
    ticket: String,
    remove: bool,
) -> ApiResult<Json<Value>> {
    access(&ctx, &s.store.work_lane_get(&id)?, "write")?;
    let v = s.store.work_lane_ticket(&id, &ticket, remove, &ctx.actor)?;
    s.wake();
    Ok(Json(v))
}
pub async fn attach(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, ticket)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    membership(s, ctx, id, ticket, false).await
}
pub async fn detach(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, ticket)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    membership(s, ctx, id, ticket, true).await
}
pub async fn handoff_create(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    access(&ctx, &s.store.work_lane_get(&id)?, "write")?;
    let v = s.store.work_handoff_create(&id, &body, &ctx.actor)?;
    s.wake();
    Ok(Json(v))
}
pub async fn handoff_get(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let v = s.store.work_handoff_get(&id)?;
    access(&ctx, &v, "read")?;
    Ok(Json(v))
}
pub async fn handoff_list(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    let lane = s.store.work_lane_get(&id)?;
    access(&ctx, &lane, "read")?;
    let (l, o) = paging(raw.as_deref())?;
    let (items, total) =
        s.store
            .work_handoff_list(lane["project"].as_str().unwrap(), Some(&id), None, l, o)?;
    Ok(Json(super::paged(
        items,
        total,
        l,
        "Continue with ?offset=N&limit=N (maximum 200).",
    )))
}
pub async fn project_handoffs(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    let (l, o) = paging(raw.as_deref())?;
    let p = query_pairs(raw.as_deref());
    let (items, total) = s
        .store
        .work_handoff_list(&project, None, first(&p, "status"), l, o)?;
    Ok(Json(super::paged(
        items,
        total,
        l,
        "Continue with ?offset=N&limit=N (maximum 200).",
    )))
}
async fn action(
    s: Arc<AppState>,
    ctx: AuthCtx,
    id: String,
    action: &str,
    body: Value,
) -> ApiResult<Json<Value>> {
    access(&ctx, &s.store.work_handoff_get(&id)?, "write")?;
    if ["dispatch", "cancel"].contains(&action) && !ctx.scopes.contains("admin") {
        ctx.require_scope("human")?;
    }
    if ["claim", "heartbeat", "result"].contains(&action) {
        ctx.require_scope("agent:run")?;
    }
    let v = s
        .store
        .work_handoff_action(&id, action, &body, &ctx.actor, &ctx.token_id)?;
    s.wake();
    Ok(Json(v))
}
macro_rules! empty_action {
    ($name:ident) => {
        pub async fn $name(
            State(s): State<Arc<AppState>>,
            Extension(ctx): Extension<AuthCtx>,
            Path(id): Path<String>,
        ) -> ApiResult<Json<Value>> {
            action(s, ctx, id, stringify!($name), json!({})).await
        }
    };
}
empty_action!(dispatch);
empty_action!(cancel);
empty_action!(claim);
macro_rules! body_action {
    ($name:ident) => {
        pub async fn $name(
            State(s): State<Arc<AppState>>,
            Extension(ctx): Extension<AuthCtx>,
            Path(id): Path<String>,
            ApiJson(body): ApiJson<Value>,
        ) -> ApiResult<Json<Value>> {
            action(s, ctx, id, stringify!($name), body).await
        }
    };
}
body_action!(heartbeat);
body_action!(result);
