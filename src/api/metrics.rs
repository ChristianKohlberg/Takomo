//! GET /v1/metrics — store observability. Read scope; scoped to the caller's
//! readable projects. Returns ticket counts by state and category per project,
//! open claim counts, and the total event count.

use super::blocking_read;
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
    // Aggregates over every ticket and every event row: a scan, off the runtime.
    let allowed = ctx.allowed_projects_vec();
    let state = state.clone();
    let out = blocking_read(move || state.store.metrics(allowed.as_deref())).await?;
    Ok(Json(out))
}
