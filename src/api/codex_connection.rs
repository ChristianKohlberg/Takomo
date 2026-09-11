//! Administrator-managed worker authentication. Provider credentials never cross this API.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    server::AppState,
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub auth_mode: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub verification_url: String,
    pub user_code: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub used_percent: u8,
    pub resets_at: Option<u64>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub status: String,
    pub account: Option<Account>,
    pub device: Option<Device>,
    pub limits: Option<Limits>,
    pub error: Option<String>,
}
impl Report {
    pub fn validate(&self) -> ApiResult<()> {
        let bad = || {
            ApiError::validation("validation.codex_connection","Invalid connection report. Send only supported account metadata, device codes and quota windows.")
        };
        if !matches!(
            self.status.as_str(),
            "unknown" | "connected" | "disconnected" | "login_pending" | "error"
        ) {
            return Err(bad());
        }
        if let Some(a) = &self.account {
            if a.email.as_ref().is_some_and(|s| s.len() > 254)
                || a.plan.as_ref().is_some_and(|s| s.len() > 80)
                || !matches!(a.auth_mode.as_str(), "chatgpt" | "apiKey" | "other")
            {
                return Err(bad());
            }
        }
        if self.error.as_ref().is_some_and(|s| s.len() > 500) {
            return Err(bad());
        }
        if let Some(d) = &self.device {
            if self.status != "login_pending"
                || d.verification_url != "https://auth.openai.com/codex/device"
                || d.user_code.is_empty()
                || d.user_code.len() > 32
                || !d
                    .user_code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(bad());
            }
        }
        if self.status == "login_pending" && self.device.is_none() {
            return Err(bad());
        }
        if let Some(l) = &self.limits {
            for w in [&l.primary, &l.secondary].into_iter().flatten() {
                if w.used_percent > 100 || w.resets_at.is_some_and(|v| v > 9_007_199_254_740_991) {
                    return Err(bad());
                }
            }
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    pub service_id: String,
    pub command_id: Option<String>,
    pub report: Option<Report>,
    #[serde(default)]
    pub busy: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub action: String,
    pub request_id: String,
}
fn admin(ctx: &AuthCtx) -> ApiResult<()> {
    ctx.require_scope("admin")?;
    ctx.require_scope("human")?;
    if ctx.projects.is_some() {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "auth.scope",
            "Codex connection management requires an unrestricted administrator.",
        ));
    }
    Ok(())
}
pub async fn list(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<impl axum::response::IntoResponse> {
    admin(&ctx)?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(s.store.codex_connections()?),
    ))
}
pub async fn command(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<Command>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let value = s.store.codex_command(&id, &req, &ctx.actor)?;
    s.wake();
    Ok(Json(value))
}
pub async fn poll(
    State(s): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(req): ApiJson<Poll>,
) -> ApiResult<impl axum::response::IntoResponse> {
    ctx.require_scope("agent:run")?;
    if let Some(report) = &req.report {
        report.validate()?;
    }
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(s.store.codex_poll(&ctx, &req)?),
    ))
}
