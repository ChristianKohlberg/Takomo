//! Bearer-token auth middleware: `tk_` tokens hashed at rest, scopes,
//! per-project access, and a per-token sliding-window write rate limit.

use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, token_hash};
use crate::server::AppState;
use crate::store::ShareKind;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct AuthCtx {
    pub token_id: String,
    pub actor: String,
    pub scopes: HashSet<String>,
    /// None = all projects.
    pub projects: Option<HashSet<String>>,
    /// This token's per-minute write budget. Carried on the context so a
    /// surface that can only classify a write *after* the middleware has run —
    /// MCP, where the operation's name lives inside the JSON-RPC body — can
    /// debit the same window (see [`debit_write_budget`]).
    pub rate_limit: i64,
    /// Which person in the directory holds this credential (a `usr-…` id), or
    /// None for a machine token. See `store::users`.
    ///
    /// **This is identity, and it is deliberately NOT a scope.** A named assignee
    /// may answer an `approve` question, so if identity were carried as a scope
    /// string it would be forgeable: scopes are free-form (`expert:<tag>` proves
    /// it), so an admin minting `user:usr-abc` would be minting the right to
    /// decide as that person. It lives here, set only from the token row's
    /// admin-written `user` column, and is passed explicitly to the store calls
    /// that need it.
    pub user: Option<String>,
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

/// Which surface a `tk_` request arrived on. It decides *where* a write is
/// classified, never whether the budget applies.
///
/// - [`Surface::Rest`]: one HTTP request is one operation, so the method is a
///   faithful classifier — `GET`/`HEAD` are reads and free, everything else
///   debits.
/// - [`Surface::Mcp`]: every MCP frame is `POST /mcp` and the operation's name
///   lives inside an opaque JSON-RPC body, so the method classifies nothing —
///   it would bill `takomo_show` and even `tools/list` as writes. The
///   middleware therefore only authenticates, and `crate::mcp` debits by tool
///   name at the point where the frame is already parsed and dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Rest,
    Mcp,
}

impl Surface {
    /// Does the middleware debit this request itself?
    fn debits_in_middleware(self, method: &Method) -> bool {
        match self {
            Surface::Rest => !matches!(*method, Method::GET | Method::HEAD),
            Surface::Mcp => false,
        }
    }
}

/// Bearer auth for the REST surface (`/v1/*`): writes are classified by method.
pub async fn auth_middleware(
    state: State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authenticate(state, request, next, Surface::Rest).await
}

/// Bearer auth for the MCP surface (`/mcp`). The same `tk_` token path — same
/// table, same scopes, same project allowlist, same rate-limit window — with
/// the write debit deferred to the tool dispatch in `crate::mcp`, which is the
/// first place that knows whether the frame is a read or a write.
pub async fn mcp_auth_middleware(
    state: State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authenticate(state, request, next, Surface::Mcp).await
}

async fn authenticate(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
    surface: Surface,
) -> Result<Response, ApiError> {
    // On the MCP surface a 401 is the start of a handshake, not just a refusal:
    // it is where a hosted client (claude.ai, ChatGPT, the Gemini app) learns that
    // this resource is OAuth-protected and where to find the authorization server.
    // The header goes on every 401 from this surface, because a client that gets a
    // bare 401 has nothing to discover from and reports an unexplained failure.
    // REST is left out deliberately: the advertised `resource` identifier is the
    // MCP endpoint, so pointing /v1 at it would name the wrong resource.
    let challenge = |err: ApiError| match (surface, state.oauth.as_ref()) {
        (Surface::Mcp, Some(cfg)) => err.header("WWW-Authenticate", cfg.www_authenticate()),
        _ => err,
    };

    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = header.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() {
        let mut message = String::from(
            "Missing bearer token. Send 'Authorization: Bearer tk_...' on every request; only /healthz is open. Tokens are minted on the server with: takomo token create.",
        );
        if surface == Surface::Mcp && state.oauth.is_some() {
            message.push_str(
                " This endpoint also accepts an OAuth-issued token: see the WWW-Authenticate header on this response for where to start the flow, which is what a hosted client (claude.ai, ChatGPT, the Gemini app) does automatically.",
            );
        }
        return Err(challenge(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "auth.missing",
            message,
        )));
    }

    let row = state
        .store
        .lookup_token(&token_hash(token))?
        .ok_or_else(|| challenge(invalid_token("unknown token")))?;
    let now = now_ms();
    if row.revoked_at.is_some() {
        return Err(challenge(invalid_token("the token has been revoked")));
    }
    if let Some(exp) = row.expires_at {
        if exp <= now {
            return Err(challenge(invalid_token("the token has expired")));
        }
    }

    let ctx = AuthCtx {
        token_id: row.id,
        actor: row.actor,
        scopes: row.scopes.into_iter().collect(),
        projects: row.projects.map(|p| p.into_iter().collect()),
        rate_limit: row.rate_limit,
        user: row.user,
    };

    // Per-token sliding-window write rate limit (contains runaway agent loops).
    if surface.debits_in_middleware(request.method()) {
        debit_write_budget(&state, &ctx)?;
    }

    // Touch last_used_at at most once a minute per token.
    {
        let mut touched = state.last_touch.lock().expect("touch lock");
        let due = touched
            .get(&ctx.token_id)
            .map(|t| now - *t >= 60_000)
            .unwrap_or(true);
        if due {
            touched.insert(ctx.token_id.clone(), now);
            drop(touched);
            let _ = state.store.touch_token(&ctx.token_id);
        }
    }

    request.extensions_mut().insert(ctx);
    Ok(next.run(request).await)
}

/// Charge one event to a per-minute sliding window, or report how long the caller
/// must wait. `Ok(())` means it was charged; `Err(secs)` means the window is full
/// and nothing was charged.
///
/// One implementation, two very different budgets on top of it (`tk_` writes and
/// `tks_` requests), because the window arithmetic is the part that is easy to get
/// subtly wrong — an off-by-one in the eviction loop turns a limiter into a
/// throttle nobody notices.
fn debit_window(
    windows: &Mutex<HashMap<String, VecDeque<i64>>>,
    key: &str,
    limit: i64,
    now: i64,
) -> Result<(), i64> {
    let mut windows = windows.lock().expect("rate lock");
    let window = windows.entry(key.to_string()).or_default();
    let cutoff = now - 60_000;
    while window.front().is_some_and(|t| *t <= cutoff) {
        window.pop_front();
    }
    if window.len() as i64 >= limit {
        // `unwrap_or(now)` covers a limit of 0, where the window is empty and
        // everything is refused: retry a full window later.
        let oldest = window.front().copied().unwrap_or(now);
        return Err(((oldest + 60_000 - now) / 1000).max(1));
    }
    window.push_back(now);
    Ok(())
}

/// Charge one event to a sliding window that is **not** keyed by a credential.
///
/// The one caller is dynamic client registration (`POST /oauth/register`), which
/// RFC 7591 requires to be unauthenticated: there is no token to charge and no
/// caller identity to key by, so the budget is global. Exposed here rather than
/// reimplemented there so all three budgets in this codebase share the one
/// window implementation whose arithmetic is easy to get subtly wrong.
///
/// `Err(secs)` is how long the caller must wait; nothing was charged.
pub fn debit_shared_window(
    windows: &Mutex<HashMap<String, VecDeque<i64>>>,
    key: &str,
    limit: i64,
) -> Result<(), i64> {
    debit_window(windows, key, limit, now_ms())
}

/// Charge one write to this token's sliding-window budget, or reject with the
/// teaching 429. Both surfaces call this, so a write costs exactly the same
/// whether it arrives as `POST /v1/tickets/{id}/comments` or as the
/// `takomo_comment` MCP tool — and a read costs nothing on either.
pub fn debit_write_budget(state: &AppState, ctx: &AuthCtx) -> ApiResult<()> {
    let now = now_ms();
    if let Err(retry_after_secs) = debit_window(&state.rate, &ctx.token_id, ctx.rate_limit, now) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate.limited",
            format!(
                "Token '{}' exceeded its write budget of {} writes/minute. Only writes are charged: reads are free and still work — GET on /v1, and the read-only MCP tools (takomo_show, takomo_list, takomo_ready, takomo_deps, takomo_questions, takomo_projects, takomo_workflow, takomo_roadmap, takomo_whoami). If you are retrying a rejected call in a loop, stop and re-read the error's remedy instead. Wait {retry_after_secs}s before the next write.",
                ctx.actor, ctx.rate_limit
            ),
        )
        .remedy(format!(
            "Wait {retry_after_secs}s, then retry this exact call; keep reading meanwhile if you need to."
        ))
        .header("Retry-After", retry_after_secs.to_string()));
    }
    Ok(())
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
    /// The scope this share grants, carried as the enum all the way from the row
    /// read to the store query — a share whose stored kind could not be
    /// interpreted never gets a `ShareCtx` at all.
    pub kind: ShareKind,
    /// Project id (project share) or root ticket id (subtree share).
    pub ref_id: String,
    /// Denormalized project the share is scoped to.
    pub project: String,
    pub expires_at: i64,
}

/// Requests per minute one share link may make, across all `/v1/shares/self*`
/// routes and every viewer holding it.
///
/// **Reads are charged here, and that is not an inconsistency with the `tk_` path
/// where reads are free.** The two budgets exist for different reasons. A `tk_`
/// token is a named actor, individually revocable, and the risk it carries is a
/// runaway agent loop writing; its reads are cheap and its holder is accountable. A
/// `tks_` share is a bearer capability *designed to be pasted around* — into a
/// chat, a ticket, an email — with no per-viewer identity, and it can only read. So
/// reads are the entire attack surface it has, and leaving them free left the share
/// path with no budget at all (takomo-fgca).
///
/// 120/minute is deliberately the same order as a token's default write budget: far
/// more than a board full of humans needs (a share view is two requests, plus one
/// per ticket opened, and the page does not poll in share mode), and a hard ceiling
/// of two requests a second for a link being hammered. It is a per-link budget, not
/// per viewer, because the link *is* the identity — and revoking it is the
/// mitigation an operator actually has.
pub const SHARE_REQUESTS_PER_MINUTE: i64 = 120;

/// Distinct auth path for share tokens. It resolves the bearer token against the
/// `shares` table only — a normal `tk_` token is not there and is rejected, and
/// this middleware guards ONLY the `/v1/shares/self*` routes, so a share token
/// can never reach a normal endpoint. Expired or revoked shares return 410 Gone
/// so the board can show a clear "this link has expired" page.
///
/// Every request through here is charged to the share's own sliding window
/// ([`SHARE_REQUESTS_PER_MINUTE`]). Charged **after** the share resolves, so the
/// budget is keyed by share id: an unrecognized token has no key to charge and is
/// bounded instead by what it costs, which is one indexed lookup by token hash.
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

    if let Err(retry_after_secs) =
        debit_window(&state.share_rate, &share.id, SHARE_REQUESTS_PER_MINUTE, now)
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "share.rate_limited",
            format!(
                "This shared link has made more than {SHARE_REQUESTS_PER_MINUTE} requests in the last minute, which is more than viewing a board takes, so it is being throttled. The link still works — wait {retry_after_secs}s and reload. The budget is per link and shared by everyone holding it, so a link being polled by a script slows it down for every reader."
            ),
        )
        // A viewer holds no token and no scope: there is nothing for them to
        // raise, so the remedy is the only two things they can actually do.
        .remedy(format!(
            "Wait {retry_after_secs}s, then reload. If a script or a dashboard is polling this link, stop it or ask for a normal read-only token instead; if the link may have leaked, have its owner revoke it (DELETE /v1/shares/{{id}}) and mint a replacement."
        ))
        .header("Retry-After", retry_after_secs.to_string()));
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

/// The resolved context for an answer-grant request. An answer grant (`tka_`)
/// authorizes ONLY the `/v1/answer/self*` endpoints — reading and answering the
/// one referenced question — and nothing else.
#[derive(Debug, Clone)]
pub struct AnswerCtx {
    pub grant_id: String,
    /// The single question this grant can answer.
    pub question: String,
    pub project: String,
    /// Actor recorded as the answerer.
    pub actor: String,
    /// The directory person this link was minted for, if any — the identity that
    /// lets a link satisfy an `approve` addressed to that person. `None` for a link
    /// handed to an outside expert, which still answers on the question's
    /// expertise.
    pub user: Option<String>,
    pub expires_at: i64,
}

/// Distinct auth path for answer-grant (`tka_`) tokens. It resolves the bearer
/// token against the `answer_grants` table only (a normal `tk_`/`tks_` token is
/// not there and is rejected), guards ONLY the `/v1/answer/self*` routes, and
/// returns 410 Gone for a grant that is expired, revoked, or already spent.
pub async fn answer_auth_middleware(
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
            "answer.missing",
            "Missing answer-link token. Open the answer link, which carries its token in the URL fragment (#a=...).",
        ));
    }

    let grant = state
        .store
        .lookup_answer_grant_by_hash(&token_hash(token))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "answer.invalid",
                "This answer-link token is not recognized. The link may be mistyped or already deleted.",
            )
        })?;

    let now = now_ms();
    if grant.revoked_at.is_some() {
        return Err(answer_gone("this answer link has been revoked"));
    }
    if grant.used_at.is_some() {
        return Err(answer_gone(
            "this answer link has already been used (it is single-use)",
        ));
    }
    if grant.expires_at <= now {
        return Err(answer_gone("this answer link has expired"));
    }

    let ctx = AnswerCtx {
        grant_id: grant.id,
        question: grant.question,
        project: grant.project,
        actor: grant.actor,
        user: grant.user,
        expires_at: grant.expires_at,
    };
    request.extensions_mut().insert(ctx);
    Ok(next.run(request).await)
}

fn answer_gone(why: &str) -> ApiError {
    ApiError::new(StatusCode::GONE, "answer.expired", why)
}
