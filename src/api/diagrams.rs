//! Stateless render requests use the same read/project authorization as documents.
use super::ApiJson;
use crate::{
    auth::AuthCtx,
    diagrams::MAX_SOURCE_BYTES,
    error::{ApiError, ApiResult},
    server::AppState,
};
use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    project: String,
    engine: String,
    source: String,
}

pub async fn render(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    ApiJson(body): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let request: RenderRequest = serde_json::from_value(body).map_err(|_| {
        ApiError::validation(
            "validation.diagram",
            "Provide project, engine and source strings only.",
        )
    })?;
    ctx.require_project(&request.project)?;
    if state.store.get_project(&request.project)?.is_none() {
        return Err(ApiError::not_found("project", &request.project));
    }
    if !matches!(request.engine.as_str(), "mermaid" | "plantuml" | "d2") {
        return Err(ApiError::validation(
            "validation.diagram_engine",
            "Choose mermaid, plantuml or d2.",
        ));
    }
    if request.source.trim().is_empty() || request.source.len() > MAX_SOURCE_BYTES {
        return Err(ApiError::validation(
            "validation.diagram_source",
            "Diagram source must contain text and be at most 50 KB.",
        ));
    }
    let renderer = state.diagrams.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "diagram.not_configured",
            "Diagram rendering is not configured. Ask an operator to set TAKOMO_KROKI_URL.",
        )
    })?;
    Ok(Json(
        json!({"svg": renderer.render(&request.engine, &request.source).await?}),
    ))
}
