//! POST /v1/tickets/{id}/transition — the only way state changes.

use super::tickets::load_visible;
use super::{body_object, get_i64, get_str, require_str};
use crate::auth::AuthCtx;
use crate::error::ApiResult;
use crate::ids::now_ms;
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde_json::Value;
use std::sync::Arc;

pub async fn transition(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("write")?;
    load_visible(&state, &ctx, &id)?;
    let obj = body_object(&body)?;
    let to = require_str(obj, "to")?;
    let reason = get_str(obj, "reason")?;
    let fence = get_i64(obj, "fence")?;

    let ticket =
        state
            .store
            .transition(&id, &to, reason.as_deref(), fence, &ctx.actor, &ctx.scopes)?;
    state.wake();
    Ok(Json(ticket.to_json(now_ms())))
}
