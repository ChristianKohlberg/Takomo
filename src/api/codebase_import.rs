use super::{
    github::{admin, connected, Repository},
    ApiJson,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    github::Github,
    server::AppState,
    store::mindmapdoc,
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
fn invalid() -> ApiError {
    ApiError::validation("validation.codebase_import","Use a valid request ID, an empty project specification and the current extraction attempt.")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    request_id: String,
    mindmap: String,
}
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    ctx.require_project(&project)?;
    Ok(Json(state.store.codebase_imports(&project)?))
}
pub async fn start(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(project): Path<String>,
    ApiJson(req): ApiJson<Start>,
) -> ApiResult<Json<Value>> {
    admin(&ctx)?;
    ctx.require_scope("write")?;
    ctx.require_project(&project)?;
    if req.request_id.is_empty() || req.request_id.len() > 120 {
        return Err(invalid());
    }
    state.store.codebase_project_writable(&project)?;
    let map = state.store.get_mindmap(&req.mindmap)?.ok_or_else(invalid)?;
    if map.project != project {
        return Err(invalid());
    }
    state.store.ensure_collab_writable(&map.id)?;
    if let Some(existing) =
        state
            .store
            .existing_codebase_import(&ctx, &project, &map.id, &req.request_id)?
    {
        return Ok(Json(existing));
    }
    let room = super::docsync::open_room(&state, &map.id).await?;
    if !room.read(|d| mindmapdoc::snapshot(d, &map.id).2.is_empty()) {
        return Err(invalid());
    }
    let repository = state
        .store
        .project_repository(&project)?
        .ok_or_else(invalid)?;
    let repo: Repository = serde_json::from_value(repository.clone()).map_err(|_| invalid())?;
    connected(&state, repo.installation)?;
    let revision = Github::load()?
        .revision(repo.installation, repo.repository, &repo.full_name)
        .await?;
    let mut source = repository;
    source["revision"] = json!(revision);
    source["limits"] = json!({"max_files":20,"max_source_bytes":100000,"max_tool_calls":12});
    source["max_sections"] = json!(3);
    let result =
        state
            .store
            .enqueue_codebase_import(&ctx, &project, &map.id, &req.request_id, &source)?;
    state.wake();
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    service_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    service_id: String,
    attempt_id: String,
}
fn identity(service: &str) -> ApiResult<()> {
    if service.is_empty() || service.len() > 120 {
        return Err(invalid());
    }
    Ok(())
}
pub async fn claim(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(req): ApiJson<Claim>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    identity(&req.service_id)?;
    Ok(Json(
        state.store.claim_codebase_import(&ctx, &req.service_id)?,
    ))
}
pub async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<Attempt>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    let job = state
        .store
        .codebase_lease(&ctx, &id, &req.service_id, &req.attempt_id, true)?;
    Ok(Json(json!({"status":job["status"]})))
}
pub async fn source_token(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<Attempt>,
) -> ApiResult<impl axum::response::IntoResponse> {
    ctx.require_scope("agent:run")?;
    let job = state
        .store
        .codebase_lease(&ctx, &id, &req.service_id, &req.attempt_id, false)?;
    if job["status"] != "running" {
        return Err(invalid());
    }
    let map = job["mindmap"].as_str().ok_or_else(invalid)?;
    state.store.ensure_collab_writable(map)?;
    let room = super::docsync::open_room(&state, map).await?;
    if !room.read(|doc| mindmapdoc::snapshot(doc, map).2.is_empty()) {
        return Err(invalid());
    }

    let repo:Repository=serde_json::from_value(json!({"installation":job["source"]["installation"],"repository":job["source"]["repository"],"full_name":job["source"]["full_name"],"scope":job["source"]["scope"]})).map_err(|_|invalid())?;
    connected(&state, repo.installation)?;
    let token = Github::load()?
        .token(repo.installation, Some(repo.repository))
        .await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"token":token})),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultInput {
    service_id: String,
    attempt_id: String,
    draft: Option<Value>,
    error: Option<String>,
}
pub async fn result(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<ResultInput>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("agent:run")?;
    let job = state
        .store
        .codebase_lease(&ctx, &id, &req.service_id, &req.attempt_id, false)?;
    if job["status"] == "completed" {
        return Ok(Json(job["result"].clone()));
    }
    if let Some(error) = req.error {
        if req.draft.is_some() || error.len() > 2000 {
            return Err(invalid());
        }
        state
            .store
            .finish_codebase_import(&id, &req.attempt_id, None, Some(&error))?;
        return Ok(Json(json!({"status":"failed"})));
    }
    let draft = req.draft.ok_or_else(invalid)?;
    if draft["sections"].as_array().is_none_or(|s| s.len() > 3) {
        return Err(invalid());
    }
    let installation = job["source"]["installation"].as_u64().ok_or_else(invalid)?;
    connected(&state, installation)?;
    // The human explicitly authorized this job to populate this empty target.
    // Workers cannot choose the target, scope, revision or originating actor.
    let mut publisher = ctx.clone();
    publisher.actor = job["actor"].as_str().ok_or_else(invalid)?.into();
    publisher.scopes.insert("human".into());
    publisher.scopes.insert("write".into());
    let body = json!({"request_id":id,"revision":job["source"]["revision"],"scope":job["source"]["scope"],"draft":draft});
    let result = super::spec_import::publish(
        State(state.clone()),
        Extension(publisher),
        Path(job["mindmap"].as_str().ok_or_else(invalid)?.into()),
        ApiJson(body),
    )
    .await?
    .0;
    state
        .store
        .finish_codebase_import(&id, &req.attempt_id, Some(&result), None)?;
    state.wake();
    Ok(Json(result))
}
