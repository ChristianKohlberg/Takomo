//! GET /v1/export — stream a project's tickets (with comments and deps) as
//! JSONL (one JSON object per line). Read scope. The output round-trips through
//! `takomo import --from takomo`.

use super::{first, query_pairs};
use crate::auth::AuthCtx;
use crate::error::ApiResult;
use crate::ids::now_ms;
use crate::server::AppState;
use axum::extract::{RawQuery, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Extension;
use serde_json::Value;
use std::sync::Arc;

/// One JSONL line per ticket: the full ticket JSON (which already carries
/// `blocked_by` = its deps) plus a `comments` array. `metadata` and `links`
/// ride along verbatim, so an export is a faithful snapshot.
pub async fn export(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    let project = first(&pairs, "project");
    if let Some(p) = project {
        ctx.require_project(p)?;
    }

    let rows = state
        .store
        .export_tickets(project, ctx.allowed_projects_vec().as_deref())?;
    let now = now_ms();
    let mut body = String::new();
    for (ticket, comments) in &rows {
        let mut line = ticket.to_json(now);
        line["comments"] = Value::Array(comments.iter().map(|c| c.to_json()).collect());
        body.push_str(&line.to_string());
        body.push('\n');
    }

    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], body))
}
