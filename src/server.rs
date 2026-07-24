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
    /// token id -> last time last_used_at was persisted.
    pub last_touch: Mutex<HashMap<String, i64>>,
}

impl AppState {
    pub fn new(store: Store) -> Arc<Self> {
        Arc::new(AppState {
            store,
            notify: Notify::new(),
            rate: Mutex::new(HashMap::new()),
            last_touch: Mutex::new(HashMap::new()),
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

    Router::new()
        .route("/healthz", get(crate::api::healthz))
        .route("/board", get(crate::api::board))
        .route("/inbox", get(crate::api::inbox))
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
    let store = Store::open(db_path).map_err(|e| e.into_message())?;
    let state = AppState::new(store);
    spawn_sweeper(state.clone(), std::time::Duration::from_secs(sweep_secs));
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    println!("takomo v{VERSION} listening on http://{addr} (db: {db_path})");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}
