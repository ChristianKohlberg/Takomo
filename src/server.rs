//! Server assembly: shared state, router, bind guard, lease sweeper.

use crate::auth::{answer_auth_middleware, auth_middleware, share_auth_middleware};
use crate::store::Store;
use axum::routing::{get, patch, post, put};
use axum::Router;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct AppState {
    pub store: Store,
    /// Woken after every committed mutation; long-pollers (ready/claim,
    /// events wait, SSE) re-check on each wake.
    pub notify: Notify,
    /// token id -> unix-ms timestamps of writes in the sliding window.
    pub rate: Mutex<HashMap<String, VecDeque<i64>>>,
    /// share id -> unix-ms timestamps of *requests* in the sliding window. A
    /// separate map from `rate`, not a shared one keyed by credential, because the
    /// two count different things: writes for a `tk_` token, every request for a
    /// read-only `tks_` share link (see `auth::SHARE_REQUESTS_PER_MINUTE`). One map
    /// would make "how much has this credential spent" ambiguous.
    pub share_rate: Mutex<HashMap<String, VecDeque<i64>>>,
    /// token id -> last time last_used_at was persisted.
    pub last_touch: Mutex<HashMap<String, i64>>,
    /// One global window for `POST /oauth/register`. A third map rather than a
    /// key in `rate`, for the same reason `share_rate` is separate: it counts a
    /// different thing (registrations by nobody in particular, since dynamic
    /// registration is unauthenticated by specification), and merging it would
    /// make "what has this credential spent" unanswerable.
    pub oauth_register_rate: Mutex<HashMap<String, VecDeque<i64>>>,
    /// The OAuth authorization server's identity, from `TAKOMO_PUBLIC_URL`.
    /// `None` disables the OAuth endpoints and the `WWW-Authenticate` challenge
    /// on `/mcp` — a server that cannot state its own issuer identity cannot run
    /// the flow, and half-running it produces connection failures with no
    /// diagnostics on the client side.
    pub oauth: Option<crate::api::oauth::OauthConfig>,
}

impl AppState {
    pub fn new(store: Store) -> Arc<Self> {
        AppState::new_with_oauth(store, None)
    }

    /// [`AppState::new`] with an OAuth authorization server configured. Separate
    /// constructor rather than a field assignment because `new` hands back an
    /// `Arc` that is immediately shared.
    pub fn new_with_oauth(
        store: Store,
        oauth: Option<crate::api::oauth::OauthConfig>,
    ) -> Arc<Self> {
        Arc::new(AppState {
            store,
            notify: Notify::new(),
            rate: Mutex::new(HashMap::new()),
            share_rate: Mutex::new(HashMap::new()),
            last_touch: Mutex::new(HashMap::new()),
            oauth_register_rate: Mutex::new(HashMap::new()),
            oauth,
        })
    }

    /// Call after any successful mutation so long-pollers re-check.
    pub fn wake(&self) {
        self.notify.notify_waiters();
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let authed = Router::new()
        .route("/v1/whoami", get(crate::api::tokens::whoami))
        .route(
            "/v1/tokens",
            get(crate::api::tokens::list).post(crate::api::tokens::create),
        )
        .route(
            "/v1/tokens/{id}",
            axum::routing::delete(crate::api::tokens::revoke),
        )
        .route(
            "/v1/projects",
            get(crate::api::projects::list).post(crate::api::projects::create),
        )
        .route(
            "/v1/projects/{project}",
            axum::routing::delete(crate::api::projects::delete),
        )
        .route(
            "/v1/projects/{project}/workflow",
            get(crate::api::projects::get_workflow).merge(put(crate::api::projects::put_workflow)),
        )
        .route(
            "/v1/projects/{project}/roadmap",
            get(crate::api::projects::roadmap),
        )
        .route(
            "/v1/projects/{project}/language",
            put(crate::api::projects::put_language),
        )
        .route(
            "/v1/projects/{project}/tags",
            get(crate::api::tags::list).post(crate::api::tags::create),
        )
        .route(
            "/v1/projects/{project}/tags/{kind}/{handle}",
            get(crate::api::tags::get_one)
                .merge(patch(crate::api::tags::patch))
                .merge(axum::routing::delete(crate::api::tags::delete)),
        )
        .route(
            "/v1/projects/{project}/style",
            put(crate::api::projects::put_style),
        )
        .route(
            "/v1/projects/{project}/answer-link-ttl",
            put(crate::api::projects::put_answer_link_ttl),
        )
        .route(
            "/v1/projects/{project}/claim-ttl",
            put(crate::api::projects::put_claim_ttl),
        )
        .route(
            "/v1/tickets",
            post(crate::api::tickets::create).get(crate::api::tickets::list),
        )
        .route(
            "/v1/tickets/{id}",
            get(crate::api::tickets::get_one).merge(patch(crate::api::tickets::patch_one)),
        )
        .route(
            "/v1/tickets/{id}/transition",
            post(crate::api::transition::transition),
        )
        .route(
            "/v1/tickets/{id}/comments",
            post(crate::api::tickets::add_comment),
        )
        .route(
            "/v1/tickets/{id}/archive",
            post(crate::api::tickets::archive),
        )
        .route(
            "/v1/tickets/{id}/promote",
            post(crate::api::tickets::promote),
        )
        .route(
            "/v1/tickets/{id}/promotions",
            get(crate::api::tickets::list_promotions),
        )
        .route("/v1/promotions", get(crate::api::tickets::promotions_index))
        .route(
            "/v1/tickets/{id}/unarchive",
            post(crate::api::tickets::unarchive),
        )
        .route(
            "/v1/tickets/{id}/deps",
            get(crate::api::tickets::deps_graph)
                .post(crate::api::tickets::add_dep)
                .delete(crate::api::tickets::remove_dep),
        )
        .route("/v1/tickets/{id}/claim", post(crate::api::claims::claim))
        .route(
            "/v1/tickets/{id}/heartbeat",
            post(crate::api::claims::heartbeat),
        )
        .route(
            "/v1/tickets/{id}/release",
            post(crate::api::claims::release),
        )
        .route(
            "/v1/tickets/{id}/force-release",
            post(crate::api::claims::force_release),
        )
        .route("/v1/ready", get(crate::api::claims::ready_peek))
        .route("/v1/ready/claim", post(crate::api::claims::ready_claim))
        .route(
            "/v1/questions",
            get(crate::api::questions::list).post(crate::api::questions::create),
        )
        .route("/v1/questions/{id}", get(crate::api::questions::get_one))
        .route(
            "/v1/questions/{id}/answer",
            post(crate::api::questions::answer),
        )
        .route(
            "/v1/questions/{id}/withdraw",
            post(crate::api::questions::withdraw),
        )
        .route(
            "/v1/questions/{id}/reopen",
            post(crate::api::questions::reopen),
        )
        .route(
            "/v1/questions/{id}/followup",
            post(crate::api::questions::followup),
        )
        .route(
            "/v1/questions/{id}/reply",
            post(crate::api::questions::reply),
        )
        .route(
            "/v1/questions/{id}/options",
            post(crate::api::questions::revise_options),
        )
        .route(
            "/v1/questions/{id}/answer-link",
            post(crate::api::questions::create_link),
        )
        .route(
            "/v1/answer-links/{id}",
            axum::routing::delete(crate::api::questions::revoke_link),
        )
        .route("/v1/events", get(crate::api::events::list))
        .route("/v1/events/stream", get(crate::api::events::stream))
        .route("/v1/export", get(crate::api::export::export))
        .route("/v1/metrics", get(crate::api::metrics::metrics))
        .route(
            "/v1/shares",
            get(crate::api::shares::list).post(crate::api::shares::create),
        )
        .route(
            "/v1/shares/{id}",
            axum::routing::delete(crate::api::shares::revoke),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Share-token-scoped read endpoints run on a DISTINCT auth path: a share
    // token resolves only against the `shares` table and reaches only these
    // routes, so it can neither hit a normal endpoint nor carry write access.
    let share_authed = Router::new()
        .route("/v1/shares/self", get(crate::api::shares::self_meta))
        .route(
            "/v1/shares/self/tickets",
            get(crate::api::shares::self_tickets),
        )
        .route(
            "/v1/shares/self/tickets/{id}",
            get(crate::api::shares::self_ticket_detail),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            share_auth_middleware,
        ));

    // Answer-grant-scoped endpoints run on their OWN auth path: a `tka_` answer
    // link resolves only against the answer_grants table and reaches only these
    // two routes — it can read and answer exactly one question, nothing else.
    let answer_authed = Router::new()
        .route(
            "/v1/answer/self",
            get(crate::api::questions::self_get).post(crate::api::questions::self_answer),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            answer_auth_middleware,
        ));

    // Hosted MCP endpoint: rmcp's streamable-HTTP transport at /mcp, behind the
    // same bearer auth as the REST API. Lives in the same binary and calls the
    // internal store directly (see crate::mcp).
    let mcp = crate::mcp::mcp_router(state.clone());

    // The OAuth authorization server. Mounted at the top level, i.e. OUTSIDE
    // every bearer middleware, and that is not incidental: discovery is what a
    // client reads *in order to* obtain a credential, so requiring one would make
    // the flow unstartable. (These paths previously fell through to the `/v1`
    // middleware's fallback and answered 401, which is exactly the dead end a
    // hosted client cannot get past.)
    //
    // Each handler reports its own "not configured" state instead of the route
    // being absent, so an operator who set TAKOMO_PUBLIC_URL wrong gets a
    // sentence explaining it rather than a bare 404.
    let oauth = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(crate::api::oauth::protected_resource_metadata),
        )
        // The suffixed form, which a client probes when the protected resource
        // lives at a path rather than at the origin (RFC 9728 §3.1).
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(crate::api::oauth::protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(crate::api::oauth::authorization_server_metadata),
        )
        .route("/oauth/register", post(crate::api::oauth::register))
        .route(
            "/oauth/authorize",
            get(crate::api::oauth::authorize_get).post(crate::api::oauth::authorize_post),
        )
        .route("/oauth/token", post(crate::api::oauth::token));

    Router::new()
        .route("/healthz", get(crate::api::healthz))
        .route("/board", get(crate::api::board))
        .route("/inbox", get(crate::api::inbox))
        .route("/favicon.svg", get(crate::api::favicon))
        .route("/favicon.ico", get(crate::api::favicon))
        .merge(oauth)
        .merge(authed)
        .merge(share_authed)
        .merge(answer_authed)
        .merge(mcp)
        .with_state(state)
}

/// Background sweep: clear expired leases (emitting lease_expired) and wake
/// long-pollers so freed tickets are re-dispatched promptly.
pub fn spawn_sweeper(state: Arc<AppState>, interval: std::time::Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let mut woke = false;
            match state.store.sweep_expired() {
                Ok(n) if n > 0 => woke = true,
                Ok(_) => {}
                Err(e) => eprintln!("lease sweep failed: {}", e.body.message),
            }
            match state.store.sweep_expired_questions() {
                Ok(n) if n > 0 => woke = true,
                Ok(_) => {}
                Err(e) => eprintln!("question sweep failed: {}", e.body.message),
            }
            // Spent authorization codes, retired refresh tokens, and OAuth-issued
            // access tokens long past expiry. Deliberately does NOT set `woke`:
            // nothing long-polls on OAuth state, and waking every poller for a
            // routine garbage collection would be pure churn.
            if let Err(e) = state.store.sweep_expired_oauth() {
                eprintln!("oauth sweep failed: {}", e.body.message);
            }
            if woke {
                state.wake();
            }
        }
    });
}

/// Refuse to bind non-loopback addresses unless explicitly allowed — the
/// server terminates plain HTTP; TLS is the deployment's job (see auth.md).
pub fn check_bind_guard(addr: &SocketAddr) -> Result<(), String> {
    if addr.ip().is_loopback() {
        return Ok(());
    }
    if std::env::var("TAKOMO_ALLOW_PUBLIC_BIND").as_deref() == Ok("1") {
        return Ok(());
    }
    Err(format!(
        "refusing to bind non-loopback address {addr}: takomo terminates plain HTTP and expects a \
         loopback/tailnet deployment behind TLS. Set TAKOMO_ALLOW_PUBLIC_BIND=1 to bind anyway \
         (make sure a reverse proxy, Tailscale, or platform TLS fronts it)."
    ))
}

pub async fn serve(bind: &str, db_path: &str, sweep_secs: u64) -> Result<(), String> {
    let addr: SocketAddr = bind
        .parse()
        .map_err(|e| format!("invalid bind address '{bind}': {e}"))?;
    check_bind_guard(&addr)?;
    // Validated here, at startup, so a malformed public URL is a refusal to boot
    // rather than a connector that fails to attach for reasons only visible in
    // another vendor's logs.
    let oauth = match std::env::var("TAKOMO_PUBLIC_URL") {
        Ok(raw) if !raw.trim().is_empty() => {
            Some(crate::api::oauth::OauthConfig::from_public_url(&raw)?)
        }
        _ => None,
    };
    let store = Store::open(db_path).map_err(|e| e.into_message())?;
    let state = AppState::new_with_oauth(store, oauth);
    spawn_sweeper(state.clone(), std::time::Duration::from_secs(sweep_secs));
    let oauth_line = match state.oauth.as_ref() {
        Some(cfg) => format!(
            "OAuth issuer {} (resource {})",
            cfg.issuer(),
            cfg.resource()
        ),
        None => "OAuth off (set TAKOMO_PUBLIC_URL to let hosted MCP clients connect)".to_string(),
    };
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    println!("takomo v{VERSION} listening on http://{addr} (db: {db_path})");
    println!("  {oauth_line}");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}
