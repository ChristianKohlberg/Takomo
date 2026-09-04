//! The one route dictation needs.
//!
//! It hands the browser a short-lived token and nothing else. Audio never
//! reaches this server: the page streams it straight to the provider, which
//! keeps a recording out of the request log and off this disk. What the page
//! must not hold is the account key, so it never gets one.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::json;
use std::sync::Arc;

use crate::auth::AuthCtx;
use crate::error::ApiResult;
use crate::server::AppState;

/// POST /v1/speech/token (write) — a token for one dictation session.
///
/// `write`, not `read`: dictation exists to put nodes on a map, and a credential
/// that cannot change the map has no use for it. It also costs the operator
/// money per minute, which is not something a read token should be able to spend.
pub async fn mint_token(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("write")?;
    let cfg = state
        .speech
        .as_ref()
        .ok_or_else(crate::speech::not_configured)?;
    let token = cfg.mint().await?;
    Ok(Json(json!({
        "token": token,
        "expires_in": crate::speech::TOKEN_TTL_SECONDS,
    })))
}
