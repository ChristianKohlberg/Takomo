//! Two exports, deliberately different in kind — do not collapse them.
//!
//! - `GET /v1/export` — ONE project's tickets (with comments and deps) as JSONL,
//!   read scope. A *logical* export: it round-trips through
//!   `takomo import --from takomo`, and it is the one an agent or a migration
//!   wants.
//! - `GET /v1/export/sqlite` — the WHOLE database as one SQLite file, admin
//!   scope and an unrestricted token. An *operator backup*.
//!
//! There is no per-project SQLite export and there should not be one. Tokens,
//! OAuth clients, shares, answer grants and events do not hang off a project at
//! all, so filtering the file by project would mean authoring a second
//! serializer that diverges from the schema the day someone adds a table. The
//! project-shaped export already exists above; this one is the whole file or
//! nothing.

use super::{blocking_read, first, query_pairs};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use crate::server::AppState;
use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
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

    // Unfiltered, this walks every ticket in the database and serializes the lot
    // into one String — the scan the read connections and the blocking pool
    // exist for. Both the query and the JSON building happen off the runtime.
    let project = project.map(str::to_string);
    let allowed = ctx.allowed_projects_vec();
    let state = state.clone();
    let body = blocking_read(move || {
        let rows = state
            .store
            .export_tickets(project.as_deref(), allowed.as_deref())?;
        let now = now_ms();
        let mut body = String::new();
        for (ticket, comments) in &rows {
            let mut line = ticket.to_json(now);
            line["comments"] = Value::Array(comments.iter().map(|c| c.to_json()).collect());
            body.push_str(&line.to_string());
            body.push('\n');
        }
        Ok(body)
    })
    .await?;

    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], body))
}

/// GET /v1/export/sqlite — the whole database as one SQLite file.
///
/// Gated on `admin` AND an unrestricted token. The scope alone is not enough:
/// this file carries every project's rows plus token hashes, OAuth client
/// secrets, share tokens and answer grants, so handing it to a token whose
/// allowlist names one project would be an escalation from that project to the
/// entire store — the exact boundary the allowlist exists to draw. An admin who
/// wants a backup can use an unrestricted token, or `takomo` on the box, which
/// is the root of trust anyway.
pub async fn export_sqlite(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    if ctx.projects.is_some() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "auth.project",
            format!(
                "A whole-database export needs a token with no project allowlist; yours ('{}') is limited to: {}. \
                 The file contains every project plus token hashes and OAuth client secrets, so it cannot be scoped down. \
                 Use an unrestricted admin token, or export this project's tickets with GET /v1/export?project=<id>.",
                ctx.actor,
                ctx.allowed_projects_vec().unwrap_or_default().join(", "),
            ),
        ));
    }

    // Staged to a file rather than built in memory, because VACUUM INTO's only
    // output is a file. The name carries the token id and a timestamp so two
    // concurrent dumps cannot collide on it — SQLite refuses to write a
    // destination that already exists, so a collision would be an error rather
    // than corruption, but an error the caller did nothing to deserve.
    let stem = format!("takomo-snapshot-{}-{}.tmp", ctx.token_id, now_ms());
    let state = state.clone();
    let bytes = blocking_read(move || {
        let dest = state.store.snapshot_dir().join(&stem);
        state.store.snapshot_into(&dest)?;
        // Read it back and unlink it in the same blocking task, so the file
        // cannot outlive the request even if the client hangs up: nothing else
        // knows the path, so nothing else would ever clean it up.
        let bytes = std::fs::read(&dest).map_err(|e| {
            let _ = std::fs::remove_file(&dest);
            ApiError::internal(format!("cannot read the snapshot back: {e}"))
        })?;
        let _ = std::fs::remove_file(&dest);
        Ok(bytes)
    })
    .await?;

    let filename = format!("takomo-{}.sqlite", iso(now_ms()).replace(':', "-"));
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.sqlite3".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    ))
}
