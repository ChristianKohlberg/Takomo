//! Token administration over HTTP: mint, list, revoke, plus `whoami`.
//!
//! These endpoints reuse the exact store logic the `token` CLI subcommand
//! drives (hash at rest, plaintext shown once). Minting, listing, and revoking
//! all require the `admin` scope; `whoami` needs only a valid token. See
//! spec/auth.md for the deliberate posture shift this represents (token minting
//! is no longer SSH-only — admin scope can mint over HTTP).

use super::{body_object, get_i64, get_string_array, require_str};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

/// Default per-minute write budget when the caller omits `rate_limit`
/// (matches the `token create` CLI default).
const DEFAULT_RATE_LIMIT: i64 = 120;

/// GET /v1/whoami — echo the caller's own identity (actor, scopes, projects).
/// Any valid token may call this; it removes the need for a TAKOMO_ACTOR env
/// var since a client can just ask the store who it is.
pub async fn whoami(Extension(ctx): Extension<AuthCtx>) -> Json<Value> {
    let mut scopes: Vec<String> = ctx.scopes.iter().cloned().collect();
    scopes.sort();
    let projects = match ctx.allowed_projects_vec() {
        None => json!("*"),
        Some(list) => json!(list),
    };
    Json(json!({
        "token_id": ctx.token_id,
        "actor": ctx.actor,
        "scopes": scopes,
        "projects": projects,
    }))
}

/// POST /v1/tokens (admin) — mint a token; the plaintext is returned ONCE and
/// stored only as a SHA-256 hash, exactly as the CLI path does.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let obj = body_object(&body)?;

    let actor = require_str(obj, "actor")?;

    let scopes = match get_string_array(obj, "scopes")? {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(ApiError::validation(
                "token.scopes",
                "Field 'scopes' is required and must be a non-empty array of strings, e.g. [\"read\",\"write\"].",
            ))
        }
    };

    // projects: absent/null/"*" = all projects; otherwise a non-empty array of ids.
    let projects: Option<Vec<String>> = match obj.get("projects") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "*" => None,
        Some(Value::String(_)) => {
            return Err(ApiError::validation(
                "token.projects",
                "Field 'projects' must be an array of project ids, or the string \"*\" for all projects.",
            ))
        }
        Some(Value::Array(_)) => match get_string_array(obj, "projects")? {
            Some(list) if !list.is_empty() => Some(list),
            _ => {
                return Err(ApiError::validation(
                    "token.projects",
                    "Field 'projects' must not be an empty array; use \"*\" to grant all projects.",
                ))
            }
        },
        Some(_) => {
            return Err(ApiError::validation(
                "token.projects",
                "Field 'projects' must be an array of project ids, or the string \"*\" for all projects.",
            ))
        }
    };

    let rate_limit = get_i64(obj, "rate_limit")?.unwrap_or(DEFAULT_RATE_LIMIT);
    if rate_limit < 1 {
        return Err(ApiError::validation(
            "token.rate_limit",
            "Field 'rate_limit' (writes/minute) must be a positive integer.",
        ));
    }

    let expires_at = match get_i64(obj, "expires_seconds")? {
        None => None,
        Some(secs) if secs > 0 => Some(now_ms() + secs * 1000),
        Some(_) => {
            return Err(ApiError::validation(
                "token.expires_seconds",
                "Field 'expires_seconds' must be a positive integer number of seconds.",
            ))
        }
    };

    let (row, plaintext) =
        state
            .store
            .create_token(&actor, &scopes, projects.as_deref(), rate_limit, expires_at)?;

    let mut out = row.to_json();
    if let Value::Object(map) = &mut out {
        map.insert("token".to_string(), Value::String(plaintext));
        map.insert(
            "warning".to_string(),
            Value::String(
                "This plaintext token is shown ONCE; only its SHA-256 is stored. Save it now."
                    .to_string(),
            ),
        );
    }
    Ok((StatusCode::CREATED, Json(out)))
}

/// GET /v1/tokens (admin) — list token metadata. Never returns the plaintext or
/// the hash.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("admin")?;
    let tokens = state.store.list_tokens()?;
    let out: Vec<Value> = tokens.iter().map(|t| t.to_json()).collect();
    Ok(Json(Value::Array(out)))
}

/// DELETE /v1/tokens/{id} (admin) — revoke a token by its public id.
pub async fn revoke(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    ctx.require_scope("admin")?;
    let revoked = state.store.revoke_token(&id)?;
    if !revoked {
        return Err(ApiError::not_found("token", &id));
    }
    Ok(StatusCode::NO_CONTENT)
}
