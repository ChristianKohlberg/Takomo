//! Server assembly: shared state, router, bind guard, lease/question/schedule sweeper.

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
    /// Every collaborative document currently open, keyed by id.
    ///
    /// In-memory and deliberately not a cache: a room exists only while somebody
    /// is editing, is hydrated from the update log on the first join, and is
    /// dropped after the last leave has flushed. Nothing survives between
    /// sessions, so it cannot become a second, divergent copy of the store.
    pub rooms: crate::api::docsync::Rooms,
    /// The document agent, from `TAKOMO_TENSORX_API_KEY`. `None` turns the
    /// `/documents` prompt bar off; everything else on that surface works
    /// without it, because an agent over MCP proposes through the same path.
    pub doc_agent: Option<crate::docagent::DocAgentConfig>,
    /// Dictation, from `TAKOMO_ASSEMBLYAI_API_KEY`. `None` leaves the map's mic
    /// button out rather than offering one that cannot work.
    pub speech: Option<crate::speech::SpeechConfig>,
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

    /// [`AppState::new_with_oauth`] plus a configured document agent.
    pub fn new_with_agent(
        store: Store,
        oauth: Option<crate::api::oauth::OauthConfig>,
        doc_agent: Option<crate::docagent::DocAgentConfig>,
        speech: Option<crate::speech::SpeechConfig>,
    ) -> Arc<Self> {
        let state = AppState::new_with_oauth(store, oauth);
        // `new_with_oauth` hands back an Arc that nothing else holds yet, so
        // this is the one moment the field can still be set.
        let mut state = Arc::try_unwrap(state).map_err(|_| ()).expect("sole owner");
        state.doc_agent = doc_agent;
        state.speech = speech;
        Arc::new(state)
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
            rooms: crate::api::docsync::Rooms::default(),
            doc_agent: None,
            speech: None,
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
        // The people directory. Global, like /v1/tokens and unlike the tag
        // registry: a person is not per-project, so these are not nested under
        // /v1/projects/{project}. Membership is what scopes them to one.
        .route(
            "/v1/users",
            get(crate::api::users::list).post(crate::api::users::create),
        )
        .route(
            "/v1/users/{handle}",
            get(crate::api::users::get_one).merge(patch(crate::api::users::patch)),
        )
        .route("/v1/mindmaps/{id}/nodes/{node}/conversation", get(crate::api::agent_chat::conversation))
        .route("/v1/mindmaps/{id}/nodes/{node}/conversation/messages", post(crate::api::agent_chat::send))
        .route("/v1/agent-jobs/claim", post(crate::api::agent_chat::claim))
        .route("/v1/agent-jobs/{id}/heartbeat", post(crate::api::agent_chat::heartbeat))
        .route("/v1/agent-jobs/{id}/result", post(crate::api::agent_chat::result))
        .route("/v1/users/{handle}/disable", post(crate::api::users::disable))
        .route("/v1/users/{handle}/enable", post(crate::api::users::enable))
        .route(
            "/v1/users/{handle}/projects",
            post(crate::api::users::add_member),
        )
        .route(
            "/v1/users/{handle}/projects/{project}",
            axum::routing::delete(crate::api::users::remove_member),
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
        // Dry-run: would this document be accepted? A distinct route rather than
        // a flag on the PUT, because a query parameter that turns a write into a
        // read is the kind of thing a proxy strips and a caller forgets.
        .route(
            "/v1/projects/{project}/workflow/validate",
            post(crate::api::projects::validate_workflow_dry_run),
        )
        .route(
            "/v1/projects/{project}/workflow-layout",
            get(crate::api::projects::get_workflow_layout)
                .merge(put(crate::api::projects::put_workflow_layout)),
        )
        // The workflow library: named state machines reusable across projects.
        // It stores documents and never applies one — applying stays the PUT
        // above, so the never-strand-a-ticket check has a single code path.
        .route(
            "/v1/workflows",
            get(crate::api::workflows::list).post(crate::api::workflows::create),
        )
        .route(
            "/v1/workflows/{id}",
            get(crate::api::workflows::get_one)
                .merge(patch(crate::api::workflows::patch))
                .merge(axum::routing::delete(crate::api::workflows::delete)),
        )
        .route(
            "/v1/projects/{project}/archive",
            post(crate::api::projects::archive),
        )
        .route(
            "/v1/projects/{project}/unarchive",
            post(crate::api::projects::unarchive),
        )
        .route(
            "/v1/projects/{project}/roadmap",
            get(crate::api::projects::roadmap),
        )
        .route(
            "/v1/projects/{project}/impact",
            get(crate::api::projects::impact),
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
            "/v1/projects/{project}/writing-instructions",
            get(crate::api::projects::get_writing_instructions)
                .put(crate::api::projects::put_writing_instructions),
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
        // Environments: where a check can be run. Writes take `write`, not
        // `human` — an agent that just leased an ephemeral instance is exactly
        // the caller this registry exists for.
        .route(
            "/v1/projects/{project}/environments",
            get(crate::api::environments::list).post(crate::api::environments::create),
        )
        .route(
            "/v1/environments/{id}",
            get(crate::api::environments::get_one)
                .merge(patch(crate::api::environments::patch))
                .merge(axum::routing::delete(crate::api::environments::archive)),
        )
        .route(
            "/v1/environments/{id}/unarchive",
            post(crate::api::environments::unarchive),
        )
        // Collaborative documents. These routes carry a document's FILING —
        // title, folder, status — and never its prose: the text is a Yjs CRDT
        // and moves over the WebSocket session instead, because a `PUT body`
        // is last-write-wins and losing what someone typed while an agent was
        // thinking is the exact failure this surface removes.
        .route(
            "/v1/projects/{project}/documents",
            get(crate::api::docs::list).post(crate::api::docs::create),
        )
        .route(
            "/v1/documents/{id}",
            get(crate::api::docs::get_one)
                .merge(patch(crate::api::docs::patch))
                .merge(axum::routing::delete(crate::api::docs::archive)),
        )
        .route(
            "/v1/documents/{id}/unarchive",
            post(crate::api::docs::unarchive),
        )
        // Mint a ticket for the sync socket. This one IS behind the bearer
        // middleware — it is the authenticated act that produces the credential
        // the socket then uses.
        .route(
            "/v1/documents/{id}/session",
            post(crate::api::docsync::create_document_session),
        )
        // The prompt bar. The ONE route in this server that calls a language
        // model — see the header of src/docagent.rs for why that exception
        // exists and what keeps it contained. Off unless a key is configured;
        // everything else on /documents works without it.
        .route("/v1/documents/{id}/run", post(crate::api::docs::run_agent))
        .route("/v1/projects/{project}/test-definitions", get(crate::api::testruns::definitions))
        .route("/v1/checks/{id}/definition", get(crate::api::testruns::definition))
        .route("/v1/projects/{project}/test-runs", get(crate::api::testruns::list).post(crate::api::testruns::create))
        .route("/v1/test-runs/{id}", get(crate::api::testruns::get).patch(crate::api::testruns::transition))
        .route("/v1/test-runs/{id}/results", post(crate::api::testruns::result))
        .route("/v1/test-runs/{id}/retry", post(crate::api::testruns::retry))
        // Checklist: releases, checks, cases, verdicts and the derived reports.
        .route(
            "/v1/projects/{project}/releases",
            get(crate::api::checklist::list_releases).post(crate::api::checklist::push_release),
        )
        .route(
            "/v1/projects/{project}/checks",
            get(crate::api::checklist::list_checks).post(crate::api::checklist::create_check),
        )
        .route(
            "/v1/initiatives/{id}/verification",
            get(crate::api::checklist::initiative_verification),
        )
        .route(
            "/v1/projects/{project}/checklist/policy",
            get(crate::api::checklist::get_policies).put(crate::api::checklist::put_policy),
        )
        .route(
            "/v1/projects/{project}/checklist/coverage",
            get(crate::api::checklist::coverage),
        )
        .route(
            "/v1/projects/{project}/checklist/worklist",
            get(crate::api::checklist::worklist),
        )
        .route(
            "/v1/projects/{project}/checklist/gate",
            get(crate::api::checklist::gate),
        )
        .route(
            "/v1/checks/{id}",
            get(crate::api::checklist::get_check)
                .merge(patch(crate::api::checklist::patch_check))
                .merge(axum::routing::delete(crate::api::checklist::archive_check)),
        )
        .route(
            "/v1/checks/{id}/cases",
            get(crate::api::checklist::list_cases).put(crate::api::checklist::file_cases),
        )
        .route("/v1/cases/{id}", get(crate::api::checklist::get_case))
        .route(
            "/v1/cases/{id}/verdict",
            post(crate::api::checklist::record_verdict),
        )
        .route(
            "/v1/projects/{project}/schedule-approval",
            put(crate::api::schedules::put_approval),
        )
        .route(
            "/v1/schedules",
            get(crate::api::schedules::list).post(crate::api::schedules::create),
        )
        .route(
            "/v1/schedules/{id}",
            get(crate::api::schedules::get_one)
                .merge(patch(crate::api::schedules::patch))
                .merge(axum::routing::delete(crate::api::schedules::delete)),
        )
        .route(
            "/v1/schedules/{id}/occurrences",
            get(crate::api::schedules::occurrences),
        )
        .route(
            "/v1/schedules/{id}/activate",
            post(crate::api::schedules::activate),
        )
        .route(
            "/v1/schedules/{id}/reject",
            post(crate::api::schedules::reject),
        )
        .route(
            "/v1/schedules/{id}/pause",
            post(crate::api::schedules::pause),
        )
        .route(
            "/v1/schedules/{id}/resume",
            post(crate::api::schedules::resume),
        )
        .route("/v1/schedules/{id}/run", post(crate::api::schedules::run_now))
        .route(
            "/v1/tickets",
            post(crate::api::tickets::create).get(crate::api::tickets::list),
        )
        // Before `/v1/tickets/{id}` for readability only — the router matches a
        // static segment ahead of a parameter either way, and no ticket id can
        // be the bare word `move` (ids are `<project>-<suffix>`).
        .route("/v1/tickets/move", post(crate::api::tickets::move_tickets))
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
        .route(
            "/v1/tickets/{id}/claim",
            get(crate::api::claims::claim_status).post(crate::api::claims::claim),
        )
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
            "/v1/questions/{id}/assign",
            post(crate::api::questions::assign),
        )
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
        // Initiatives. MCP came first (see crate::mcp), because an agent in a
        // conversation is what produces one; these writes exist because the
        // /initiatives page needs them and a browser cannot call an MCP tool.
        .route(
            "/v1/initiatives",
            get(crate::api::initiatives::list).post(crate::api::initiatives::create),
        )
        .route(
            "/v1/initiatives/{id}",
            get(crate::api::initiatives::get_one)
                .patch(crate::api::initiatives::patch)
                .delete(crate::api::initiatives::delete),
        )
        .route(
            "/v1/initiatives/{id}/entries",
            get(crate::api::initiatives::list_entries)
                .post(crate::api::initiatives::create_entry),
        )
        .route(
            "/v1/initiatives/{id}/entries/{entry}/content",
            get(crate::api::initiatives::entry_content),
        )
        // Mindmaps: brainstorming, before any of it is an idea. The detail read
        // returns the whole tree because a canvas cannot draw half of one, and the
        // node route takes a BATCH because that is what an agent adding a branch
        // sends.
        .route(
            "/v1/mindmaps",
            get(crate::api::mindmaps::list).post(crate::api::mindmaps::create),
        )
        .route(
            "/v1/mindmaps/{id}",
            get(crate::api::mindmaps::get_one)
                .merge(patch(crate::api::mindmaps::patch))
                .merge(axum::routing::delete(crate::api::mindmaps::delete)),
        )
        .route("/v1/mindmaps/{id}/outline", get(crate::api::mindmaps::outline))
        // The plan's sections, flat: what resolves a check's `node` to a title.
        .route(
            "/v1/projects/{project}/nodes",
            get(crate::api::mindmaps::project_nodes),
        )
        .route("/v1/mindmaps/{id}/nodes", post(crate::api::mindmaps::add_nodes))
        .route(
            "/v1/mindmaps/{id}/nodes/{node}",
            patch(crate::api::mindmaps::patch_node)
                .merge(axum::routing::delete(crate::api::mindmaps::delete_node)),
        )
        .route(
            "/v1/mindmaps/{id}/nodes/{node}/promote",
            post(crate::api::mindmaps::promote),
        )
        // Ask a model for a change to one section. The one route that calls one.
        .route("/v1/mindmaps/{id}/run", post(crate::api::mindmaps::run_agent))
        // Dictation's only route: a short-lived token, never the account key.
        .route("/v1/speech/token", post(crate::api::speech::mint_token))
        // The plan as an agent reads it, and what it may offer back.
        .route("/v1/mindmaps/{id}/prose", get(crate::api::mindmaps::prose))
        .route(
            "/v1/mindmaps/{id}/proposals",
            get(crate::api::mindmaps::proposals).post(crate::api::mindmaps::propose),
        )
        // What happened to the plan, and who did it.
        .route("/v1/mindmaps/{id}/versions", get(crate::api::spec_history::list))
        .route("/v1/mindmaps/{id}/versions/{version}", get(crate::api::spec_history::get))
        .route("/v1/mindmaps/{id}/versions/{version}/state", get(crate::api::spec_history::download))
        .route("/v1/mindmaps/{id}/checkpoints", post(crate::api::spec_history::checkpoint))
        .route(
            "/v1/mindmaps/{id}/trace",
            get(crate::api::mindmaps::trace).post(crate::api::mindmaps::add_trace),
        )
        // The sync ticket, minted exactly as a document's is — a browser
        // WebSocket cannot carry an Authorization header, so the credential has
        // to ride the handshake.
        .route("/v1/checks/{id}/session", post(crate::api::docsync::create_check_session))
        .route("/v1/projects/{id}/session", post(crate::api::docsync::create_project_session))
        .route(
            "/v1/mindmaps/{id}/session",
            post(crate::api::docsync::create_mindmap_session),
        )
        // An edge that is not part of the hierarchy.
        .route(
            "/v1/mindmaps/{id}/relationships",
            post(crate::api::mindmaps::add_relationship),
        )
        .route(
            "/v1/mindmaps/{id}/relationships/{relationship}",
            axum::routing::delete(crate::api::mindmaps::delete_relationship),
        )
        // A pointer to something a node refers to. Never the bytes.
        .route(
            "/v1/mindmaps/{id}/nodes/{node}/attachments",
            post(crate::api::mindmaps::add_attachment),
        )
        .route(
            "/v1/mindmaps/{id}/nodes/{node}/attachments/{attachment}",
            axum::routing::delete(crate::api::mindmaps::delete_attachment),
        )
        .route("/v1/events", get(crate::api::events::list))
        .route("/v1/events/stream", get(crate::api::events::stream))
        .route("/v1/export", get(crate::api::export::export))
        .route("/v1/export/sqlite", get(crate::api::export::export_sqlite))
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
        .route("/", get(crate::api::root_redirect))
        .route("/healthz", get(crate::api::healthz))
        // The document sync socket. Mounted OUTSIDE every bearer middleware, for
        // the reason `/oauth/*` is: it authenticates with a credential of its own
        // (a `tkd_` ticket in the query, because a browser WebSocket cannot set an
        // Authorization header), and running it through a middleware that demands
        // a different one would make it unreachable.
        //
        // The document id is the LAST segment, and that is a wire requirement
        // rather than a preference: `y-websocket` builds its URL as
        // `serverUrl + "/" + room + "?" + params`, so a path with anything after
        // the room — or a base that already carries a query string — comes out
        // mangled. `/v1/documents/{id}/sync` did exactly that.
        .route("/v1/docsync/{id}", get(crate::api::docsync::sync))
        .route("/v1/sync/{id}", get(crate::api::docsync::sync))
        .route("/board", get(crate::api::board))
        .route("/epics", get(crate::api::board))
        .route("/inbox", get(crate::api::inbox))
        .route("/documents", get(crate::api::documents_page))
        .route("/specification", get(crate::api::documents_page))
        .route("/projects/{project}/specification", get(crate::api::documents_page))
        .route("/initiatives", get(crate::api::initiatives_page))
        .route("/mindmaps", get(crate::api::mindmaps_page))
        .route("/schedules", get(crate::api::schedules_page))
        .route("/verification", get(crate::api::verification_page))
        .route("/environments", get(crate::api::environments_page))
        .route("/settings", get(crate::api::settings_page))
        // The app's assets. Still not a static-file handler: `build.rs` bakes the
        // generated `web/dist/assets/` into a compile-time name → bytes table and
        // this route is an exact lookup in it, so there is no directory to
        // traverse and no path to sanitize. One route rather than four names
        // because the editor route is code-split, and the bundler names those
        // chunks — see the ASSETS comment in src/api/mod.rs.
        .route("/assets/{file}", get(crate::api::asset_by_name))
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
            match state.store.sweep_expired_agent_jobs() {
                Ok(n) if n > 0 => woke = true,
                Ok(_) => {}
                Err(e) => eprintln!("agent job sweep failed: {}", e.body.message),
            }
            // The third pass: fire every schedule whose slot has come. Each one
            // is its own transaction, so a single corrupt cadence cannot stop
            // the rest — and a ticket that appears here is an ordinary ticket,
            // so waking the long-pollers is all the dispatch it needs.
            match state.store.materialize_due() {
                Ok(n) if n > 0 => woke = true,
                Ok(_) => {}
                Err(e) => eprintln!("schedule sweep failed: {}", e.body.message),
            }
            // Spent authorization codes, retired refresh tokens, and OAuth-issued
            // access tokens long past expiry. Deliberately does NOT set `woke`:
            // nothing long-polls on OAuth state, and waking every poller for a
            // routine garbage collection would be pure churn.
            if let Err(e) = state.store.sweep_expired_oauth() {
                eprintln!("oauth sweep failed: {}", e.body.message);
            }
            // Long-dead document sync tickets. Also deliberately does not set
            // `woke`, for the same reason: nothing long-polls on them.
            if let Err(e) = state.store.sweep_expired_collab_sessions() {
                eprintln!("doc session sweep failed: {}", e.body.message);
            }
            if woke {
                state.wake();
            }
        }
    });
}

/// Decide whether the OAuth authorization server is on, from the raw
/// `TAKOMO_PUBLIC_URL`, and produce the startup line that says which.
///
/// **A value this server cannot use as an OAuth issuer turns OAuth off and says so
/// loudly. It does not stop the server** — and that is a deliberate reversal of
/// how this first shipped (takomo-z919).
///
/// The reason is that `TAKOMO_PUBLIC_URL` is older than OAuth and has a second,
/// far more tolerant reader: `notify::board_link` and the answer links in
/// `api::questions` only ever needed a non-empty string to put in front of a path.
/// An operator who set it long ago for readable notification links — with a path
/// prefix, or as plain `http` on a tailnet host, both of which `docs/hosting.md`
/// describes as supported deployments — must not lose their server to an upgrade
/// that merely added a stricter reader of the same variable. They never asked for
/// OAuth at all.
///
/// [`check_bind_guard`] below *does* refuse to boot, and the asymmetry is the
/// point rather than an inconsistency: that guard stops an unencrypted service
/// from being exposed to a network, where continuing is the dangerous option.
/// Here the entire consequence of a bad value is that hosted MCP clients cannot
/// attach — which every `/oauth/*` route already reports per request, in a 404
/// that names the variable. So the failure is reported twice and costs nothing,
/// where refusing to boot would cost an outage.
///
/// Returns the line rather than printing it so the decision is testable without
/// starting a server, which is the gap that let the original mistake through: the
/// validator itself was well covered, and nothing pinned what `serve` did with it.
pub fn resolve_oauth(raw: Option<&str>) -> (Option<crate::api::oauth::OauthConfig>, String) {
    let Some(raw) = raw.filter(|v| !v.trim().is_empty()) else {
        return (
            None,
            "OAuth off (set TAKOMO_PUBLIC_URL to let hosted MCP clients connect)".to_string(),
        );
    };
    match crate::api::oauth::OauthConfig::from_public_url(raw) {
        Ok(cfg) => {
            let line = format!("OAuth issuer {} (resource {})", cfg.issuer(), cfg.resource());
            (Some(cfg), line)
        }
        Err(why) => (
            None,
            format!(
                "OAuth OFF — TAKOMO_PUBLIC_URL is set but unusable as an issuer: {why}\n  \
                 Hosted MCP clients (claude.ai, ChatGPT, the Gemini app) cannot connect until it is \
                 fixed. Everything else, including the absolute links in ask-a-human \
                 notifications, is unaffected and still uses this value as before."
            ),
        ),
    }
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
    let public_url = std::env::var("TAKOMO_PUBLIC_URL").ok();
    let (oauth, oauth_line) = resolve_oauth(public_url.as_deref());
    let (doc_agent, agent_line) = crate::docagent::resolve(
        std::env::var("TAKOMO_TENSORX_API_KEY").ok(),
        std::env::var("TAKOMO_TENSORX_BASE_URL").ok(),
        std::env::var("TAKOMO_DOC_MODEL").ok(),
    );
    let store = Store::open(db_path).map_err(|e| e.into_message())?;
    let state = AppState::new_with_agent(
        store,
        oauth,
        doc_agent,
        crate::speech::SpeechConfig::from_env(std::env::var("TAKOMO_ASSEMBLYAI_API_KEY").ok()),
    );
    spawn_sweeper(state.clone(), std::time::Duration::from_secs(sweep_secs));
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    println!("takomo v{VERSION} listening on http://{addr} (db: {db_path})");
    println!("  {oauth_line}");
    println!("  {agent_line}");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}

/// Unit tests for the one decision in this file that is pure. The rest of the
/// server surface is exercised over real HTTP from `tests/`, per this repo's
/// convention; this is here because the bug it guards against (takomo-z919) was
/// invisible to that surface — a server that refuses to start has no surface to
/// test, and the validator it delegates to was already fully covered.
#[cfg(test)]
mod oauth_env_tests {
    use super::resolve_oauth;

    #[test]
    fn a_usable_origin_turns_oauth_on() {
        let (cfg, line) = resolve_oauth(Some("https://takomo.example.com"));
        let cfg = cfg.expect("a bare https origin is usable");
        assert_eq!(cfg.issuer(), "https://takomo.example.com");
        assert_eq!(cfg.resource(), "https://takomo.example.com/mcp");
        assert!(line.contains("OAuth issuer"), "line: {line}");
    }

    #[test]
    fn unset_or_blank_turns_oauth_off_without_complaint() {
        for raw in [None, Some(""), Some("   ")] {
            let (cfg, line) = resolve_oauth(raw);
            assert!(cfg.is_none(), "{raw:?} must not configure OAuth");
            assert!(
                line.contains("OAuth off"),
                "an absent value is a normal state, not a warning ({raw:?}): {line}"
            );
        }
    }

    /// The regression this function exists for: a value that is useless to OAuth
    /// must leave the server running, because the same variable has a tolerant
    /// older reader (notification links) whose operator never asked for OAuth.
    #[test]
    fn an_unusable_value_turns_oauth_off_loudly_instead_of_failing() {
        for raw in [
            "http://takomo.internal",          // plain http on a tailnet host
            "https://takomo.example.com/path", // a path prefix
            "https://takomo.example.com?x=1",  // a query string
            "takomo.example.com",              // no scheme
        ] {
            let (cfg, line) = resolve_oauth(Some(raw));
            assert!(cfg.is_none(), "{raw} must not configure OAuth");
            assert!(
                line.contains("OAuth OFF"),
                "a rejected value must be reported loudly ({raw}): {line}"
            );
            // The operator needs to know two things: that hosted clients are the
            // only casualty, and that their notification links still work.
            assert!(
                line.contains("cannot connect") && line.contains("notifications"),
                "the line must scope the damage ({raw}): {line}"
            );
        }
    }
}
