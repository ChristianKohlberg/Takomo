//! GET /v1/metrics — store observability. Read scope; scoped to the caller's
//! readable projects. Returns ticket counts by state and category per project,
//! open claim counts, and the total event count.

use crate::auth::AuthCtx;
use crate::error::ApiResult;
use crate::server::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

pub async fn metrics(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let out = state.store.metrics(ctx.allowed_projects_vec().as_deref())?;
    Ok(Json(out))
}
