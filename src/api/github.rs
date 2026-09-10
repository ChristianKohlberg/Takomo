use super::ApiJson;
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    github::Github,
    server::AppState,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
pub(super) fn admin(ctx: &AuthCtx) -> ApiResult<()> {
    ctx.require_scope("admin")?;
    ctx.require_scope("human")?;
    if ctx.projects.is_some() {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "auth.scope",
            "GitHub connections require an administrator with access to all projects.",
        ));
    }
    Ok(())
}
fn invalid() -> ApiError {
    ApiError::validation("validation.github","Choose an installation and repository from the connected GitHub App, and an explicit relative source path.")
}
pub async fn status(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let configured = Github::configured();
    let slug = if configured {
        Some(Github::load()?.slug)
    } else {
        None
    };
    Ok(Json(
        json!({"configured":configured,"app_slug":slug,"connections":state.store.github_connections()?,"permissions":{"contents":"read"},"note":"Deployment-owned GitHub App. Only unrestricted administrators can connect or use its repositories."}),
    ))
}
pub async fn installations(Extension(ctx): Extension<AuthCtx>) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let app = Github::load()?;
    let raw = app.installations().await?;
    let rows = raw.as_array().ok_or_else(invalid)?;
    let items:Vec<_>=rows.iter().map(|r|json!({"id":r["id"],"account":r["account"]["login"],"suspended":!r["suspended_at"].is_null(),"permissions":r["permissions"]})).collect();
    Ok(Json(
        json!({"items":items,"limit":100,"has_more":rows.len()==100,"note":"First 100 installations of this deployment's app. Use a dedicated GitHub App for this deployment."}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    installation: u64,
}
pub async fn connect(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(req): ApiJson<Connect>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    let raw = Github::load()?.installations().await?;
    let found = raw
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|v| v["id"].as_u64() == Some(req.installation) && v["suspended_at"].is_null())
        })
        .ok_or_else(invalid)?;
    if ![Some("read"), Some("write")].contains(&found["permissions"]["contents"].as_str()) {
        return Err(invalid());
    }
    state.store.github_connect(
        req.installation,
        found["account"]["login"].as_str().ok_or_else(invalid)?,
        &crate::github::installation_management_url(found)?,
    )?;
    Ok(Json(json!({"connected":true})))
}
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<u64>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    state.store.github_disconnect(id)?;
    Ok(Json(json!({"connected":false})))
}
#[derive(Deserialize)]
pub struct Page {
    page: Option<u64>,
}
pub async fn repositories(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<u64>,
    Query(q): Query<Page>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    connected(&state, id)?;
    let page = q.page.unwrap_or(1);
    if !(1..=1000).contains(&page) {
        return Err(invalid());
    }
    let raw = Github::load()?.repositories(id, page).await?;
    let rows = raw["repositories"].as_array().ok_or_else(invalid)?;
    let items: Vec<_> = rows
        .iter()
        .map(|r| json!({"id":r["id"],"full_name":r["full_name"],"private":r["private"]}))
        .collect();
    Ok(Json(
        json!({"items":items,"total":raw["total_count"],"page":page,"limit":100,"note":"Only repositories currently granted to this installation."}),
    ))
}
pub(super) fn connected(state: &AppState, id: u64) -> ApiResult<()> {
    if !state
        .store
        .github_connections()?
        .iter()
        .any(|c| c["id"].as_u64() == Some(id))
    {
        return Err(invalid());
    }
    Ok(())
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub installation: u64,
    pub repository: u64,
    pub full_name: String,
    pub scope: Scope,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}
impl Scope {
    pub fn validate(&self) -> ApiResult<()> {
        let valid = |s: &String| {
            s == "."
                || (!s.is_empty()
                    && s.len() <= 512
                    && !s.contains(['\\', ':'])
                    && !s.chars().any(char::is_control)
                    && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".."))
        };
        if self.include.is_empty()
            || self.include.len() > 8
            || self.exclude.len() > 8
            || !self.include.iter().chain(&self.exclude).all(valid)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
pub async fn project_repository(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    Ok(Json(
        json!({"repository":state.store.project_repository(&project)?}),
    ))
}
pub async fn set_repository(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(req): ApiJson<Repository>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    ctx.require_project(&project)?;
    state.store.codebase_project_writable(&project)?;
    req.scope.validate()?;
    connected(&state, req.installation)?;
    // Verify the exact repository identity and current read permission before saving.
    Github::load()?
        .revision(req.installation, req.repository, &req.full_name)
        .await?;
    state
        .store
        .set_project_repository(&project, &serde_json::to_value(req).unwrap())?;
    Ok(Json(json!({"saved":true})))
}
