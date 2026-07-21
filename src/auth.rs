//! Bearer-token auth middleware: `tk_` tokens hashed at rest, scopes,
//! per-project access, and a per-token sliding-window write rate limit.

use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, token_hash};
use crate::server::AppState;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AuthCtx {
    pub token_id: String,
    pub actor: String,
    pub scopes: HashSet<String>,
    /// None = all projects.
    pub projects: Option<HashSet<String>>,
}

impl AuthCtx {
    pub fn require_scope(&self, scope: &str) -> ApiResult<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "auth.scope",
            format!(
                "This operation requires the '{scope}' scope; your token ('{}') carries: {}. Have an operator mint a token with the needed scope: takomo token create --actor {} --scopes {scope},...",
                self.actor,
                if self.scopes.is_empty() {
                    "none".to_string()
                } else {
                    let mut v: Vec<_> = self.scopes.iter().cloned().collect();
                    v.sort();
                    v.join(",")
                },
                self.actor
            ),
        ))
    }

    pub fn can_project(&self, project: &str) -> bool {
        match &self.projects {
            None => true,
            Some(set) => set.contains(project),
        }
    }

    pub fn require_project(&self, project: &str) -> ApiResult<()> {
        if self.can_project(project) {
            return Ok(());
        }
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "auth.project",
            format!(
                "Your token is not scoped to project '{project}'. It covers: {}. Use a token minted for this project.",
                self.projects
                    .as_ref()
                    .map(|s| {
                        let mut v: Vec<_> = s.iter().cloned().collect();
                        v.sort();
                        v.join(",")
                    })
                    .unwrap_or_else(|| "*".to_string())
            ),
        ))
    }

    /// Projects to restrict list queries to (None = unrestricted).
    pub fn allowed_projects_vec(&self) -> Option<Vec<String>> {
        self.projects.as_ref().map(|s| {
            let mut v: Vec<_> = s.iter().cloned().collect();
            v.sort();
            v
        })
    }
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = header.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "auth.missing",
            "Missing bearer token. Send 'Authorization: Bearer tk_...' on every request; only /healthz is open. Tokens are minted on the server with: takomo token create.",
        ));
    }

    let row = state
        .store
        .lookup_token(&token_hash(token))?
        .ok_or_else(|| invalid_token("unknown token"))?;
    let now = now_ms();
    if row.revoked_at.is_some() {
        return Err(invalid_token("the token has been revoked"));
    }
    if let Some(exp) = row.expires_at {
        if exp <= now {
            return Err(invalid_token("the token has expired"));
        }
    }

    // Per-token sliding-window write rate limit (contains runaway agent loops).
    let is_write = !matches!(*request.method(), Method::GET | Method::HEAD);
    if is_write {
        let mut windows = state.rate.lock().expect("rate lock");
        let window = windows.entry(row.id.clone()).or_default();
        let cutoff = now - 60_000;
        while window.front().is_some_and(|t| *t <= cutoff) {
            window.pop_front();
        }
        if window.len() as i64 >= row.rate_limit {
            let oldest = *window.front().expect("non-empty window");
            let retry_after_secs = ((oldest + 60_000 - now) / 1000).max(1);
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate.limited",
                format!(
                    "Token '{}' exceeded its write budget of {} writes/minute. If you are retrying a rejected call in a loop, stop and re-read the error's remedy instead. Wait {retry_after_secs}s before the next write.",
                    row.actor, row.rate_limit
                ),
            )
            .header("Retry-After", retry_after_secs.to_string()));
        }
        window.push_back(now);
    }

    // Touch last_used_at at most once a minute per token.
    {
        let mut touched = state.last_touch.lock().expect("touch lock");
        let due = touched
            .get(&row.id)
            .map(|t| now - *t >= 60_000)
            .unwrap_or(true);
        if due {
            touched.insert(row.id.clone(), now);
            drop(touched);
            let _ = state.store.touch_token(&row.id);
        }
    }

    let ctx = AuthCtx {
        token_id: row.id,
        actor: row.actor,
        scopes: row.scopes.into_iter().collect(),
        projects: row.projects.map(|p| p.into_iter().collect()),
    };
    request.extensions_mut().insert(ctx);
    Ok(next.run(request).await)
}

fn invalid_token(why: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "auth.invalid",
        format!("The bearer token was rejected: {why}. Mint a fresh one on the server with: takomo token create."),
    )
}

/// The resolved context for a share-scoped request. A share token authorizes
/// ONLY the read-only `/v1/shares/self*` endpoints, bounded to this scope.
#[derive(Debug, Clone)]
pub struct ShareCtx {
    pub share_id: String,
    /// "project" or "subtree".
    pub kind: String,
    /// Project id (project share) or root ticket id (subtree share).
    pub ref_id: String,
    /// Denormalized project the share is scoped to.
    pub project: String,
    pub expires_at: i64,
}

/// Distinct auth path for share tokens. It resolves the bearer token against the
/// `shares` table only — a normal `tk_` token is not there and is rejected, and
/// this middleware guards ONLY the `/v1/shares/self*` routes, so a share token
/// can never reach a normal endpoint. Expired or revoked shares return 410 Gone
/// so the board can show a clear "this link has expired" page.
pub async fn share_auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = header.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "share.missing",
            "Missing share token. Open the shared link, which carries its token in the URL fragment (#s=...).",
        ));
    }

    let share = state
        .store
        .lookup_share_by_hash(&token_hash(token))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "share.invalid",
                "This share token is not recognized. The shared link may be mistyped or already deleted.",
            )
        })?;

    let now = now_ms();
    if share.revoked_at.is_some() {
        return Err(share_gone("this shared link has been revoked"));
    }
    if share.expires_at <= now {
        return Err(share_gone("this shared link has expired"));
    }

    let ctx = ShareCtx {
        share_id: share.id,
        kind: share.kind,
        ref_id: share.ref_id,
        project: share.project,
        expires_at: share.expires_at,
    };
    request.extensions_mut().insert(ctx);
    Ok(next.run(request).await)
}

fn share_gone(why: &str) -> ApiError {
    ApiError::new(StatusCode::GONE, "share.expired", why)
}
