//! Hosted MCP server: the streamable-HTTP MCP transport, in the same binary.
//!
//! This exposes the tracker's work loop as native MCP tools over a remote HTTP
//! transport (rmcp's `StreamableHttpService`), so an agent can attach with
//! `claude mcp add --transport http https://<host>/mcp` instead of spawning the
//! Node stdio wrapper in `clients/mcp`. Tools call the internal `Store` directly
//! — no HTTP round-trip back to this process, no duplicated API logic.
//!
//! Auth: the `/mcp` endpoint is wrapped in the SAME bearer-token middleware as
//! the REST API (`crate::auth::auth_middleware`), so a missing/invalid token or
//! a share (`tks_`) token is rejected before any MCP frame is processed, and the
//! resolved [`AuthCtx`] rides in the request extensions. Every tool re-checks
//! scope and project access exactly like the matching REST handler.
//!
//! Fences: the Node wrapper remembers a claimed ticket's fencing token in
//! process memory; a hosted server cannot rely on session affinity, so instead
//! the fence is resolved from the store — when the caller holds the active claim
//! its valid fence IS the ticket's current `fence_seq`. An explicit `fence`
//! argument always overrides. Target states for the convenience verbs
//! (start/done/block/cancel) are resolved from the project workflow by category,
//! mirroring the CLI and the Node MCP.

use crate::api::tickets::load_visible;
use crate::auth::{auth_middleware, AuthCtx};
use crate::error::{AllowedTransition, ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use crate::store::{ReadyFilter, Ticket, TicketCreate, TicketListFilter, TicketPatch};
use crate::workflow::Workflow;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};

use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;

/// Mount the MCP streamable-HTTP transport at `/mcp`, behind the same bearer
/// auth as the REST API. Merged into the main router by `server::build_router`.
pub fn mcp_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let factory_state = state.clone();
    // Stateless, JSON-response mode: every POST /mcp is a self-contained,
    // independently bearer-authenticated request/response. The tracker tools are
    // pure request/response (no server-initiated messages), so no SSE session is
    // needed — and statelessness lets the endpoint scale horizontally behind a
    // load balancer with no session affinity. Spec-compliant per MCP Streamable
    // HTTP (2025-06-18), which lets the server answer with application/json.
    //
    // DNS-rebinding host allow-listing is a browser-cookie defense; this API is
    // bearer-token only (no ambient credentials a rebinding page could ride) and
    // is meant to be reachable at whatever public host fronts it, so the Host
    // allow-list is disabled. TLS + the token are the guard.
    let config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || Ok(TakomoMcp::new(factory_state.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );

    Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
}

/// The MCP tool surface. Cloned per session by the transport's service factory;
/// all real state lives behind the shared `Arc<AppState>`.
#[derive(Clone)]
pub struct TakomoMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<TakomoMcp>,
}

// ---- tool argument schemas --------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NewArgs {
    /// Project id the ticket belongs to.
    pub project: String,
    /// Short ticket title.
    pub title: String,
    /// Ticket type, e.g. task, bug, epic, spike (workflow-dependent).
    pub r#type: Option<String>,
    /// Priority, e.g. low, normal, high, critical.
    pub priority: Option<String>,
    /// Parent ticket id (for subtasks).
    pub parent: Option<String>,
    /// Labels to attach.
    pub labels: Option<Vec<String>>,
    /// Markdown body / description.
    pub body: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListArgs {
    /// Filter by project id.
    pub project: Option<String>,
    /// Filter by exact state, e.g. ready, done.
    pub state: Option<String>,
    /// Filter by type.
    pub r#type: Option<String>,
    /// Filter by a single label.
    pub label: Option<String>,
    /// Full-text query over title/body.
    pub q: Option<String>,
    /// Max items (1-200, default 50).
    pub limit: Option<i64>,
    /// Pagination cursor from a previous call's next_cursor.
    pub cursor: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadyArgs {
    /// Filter the ready queue by project id.
    pub project: Option<String>,
    /// Restrict to a ticket type.
    pub r#type: Option<String>,
    /// Restrict to a single label.
    pub label: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdArgs {
    /// Ticket id.
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NextArgs {
    /// Restrict to a project id.
    pub project: Option<String>,
    /// Restrict to a ticket type.
    pub r#type: Option<String>,
    /// Restrict to a single label.
    pub label: Option<String>,
    /// Seconds to long-poll for work before giving up (0-60, default 0).
    pub wait: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StartArgs {
    /// Ticket id.
    pub id: String,
    /// Explicit target state (defaults to the workflow's in-progress state).
    pub to: Option<String>,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TransitionArgs {
    /// Ticket id.
    pub id: String,
    /// Target state id.
    pub to: String,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FenceArgs {
    /// Ticket id.
    pub id: String,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BlockArgs {
    /// Ticket id.
    pub id: String,
    /// Optional note explaining the blocker (added as a comment first).
    pub comment: Option<String>,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CommentArgs {
    /// Ticket id.
    pub id: String,
    /// Comment text.
    pub body: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LinkArgs {
    /// Ticket id.
    pub id: String,
    /// Link name, e.g. 'pr', 'branch', 'design'.
    pub key: String,
    /// Link value (URL or ref).
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DepArgs {
    /// The dependent ticket id (the one that is blocked).
    pub id: String,
    /// The ticket id that must finish first.
    pub blocked_by: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DepsArgs {
    /// Ticket id to inspect.
    pub id: String,
    /// Direction: blocked_by (default), blocks, or both.
    pub direction: Option<String>,
    /// Follow edges transitively (default false).
    pub transitive: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectArgs {
    /// Project id.
    pub project: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AskArgs {
    /// Ticket id the question is about. For a blocking question it is parked in
    /// a blocked state and your lease is released (block-and-resume): end your
    /// run and pick it back up once a human has answered.
    pub id: String,
    /// "blocking" (default): parks + resumes this ticket. "advisory": a routed,
    /// recorded decision that does NOT change ticket state — use it for
    /// epic-level or strategic questions that shouldn't freeze the work.
    pub mode: Option<String>,
    /// Question kind: confirm (yes/no), choose (pick an option), clarify (free
    /// text), or approve (approve/reject an action).
    pub kind: String,
    /// The question, phrased for a human domain expert.
    pub title: String,
    /// Optional context: why you are asking and what you have tried.
    pub body: Option<String>,
    /// For kind=choose: the options to pick from (>= 2).
    pub options: Option<Vec<String>>,
    /// Your recommended answer (a hint for the human; also applied on timeout if
    /// on_timeout=recommended).
    pub recommended: Option<String>,
    /// Routing tags for the human queue, e.g. ["domain:billing"].
    pub expertise: Option<Vec<String>>,
    /// Urgency: critical, high, normal (default), or low.
    pub urgency: Option<String>,
    /// Auto-expire the question after this many seconds (see on_timeout).
    pub expires_in_seconds: Option<i64>,
    /// On timeout: recommended (apply your recommendation), escalate (open the
    /// pool), or cancel (cancel the ticket). Omit to just flag it expired.
    pub on_timeout: Option<String>,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnswerArgs {
    /// Question id (from takomo_questions or the question_asked event).
    pub id: String,
    /// The answer: "yes"/"no" for confirm/approve, the chosen option for choose,
    /// or the explanation text for clarify.
    pub answer: String,
    /// Optional note recorded alongside the answer.
    pub note: Option<String>,
    /// Override the workflow state the ticket resumes into (defaults to the
    /// workflow's human-gated resume state).
    pub resume_to: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QuestionsArgs {
    /// Filter by project id.
    pub project: Option<String>,
    /// Filter by ticket id.
    pub ticket: Option<String>,
    /// Statuses to include (comma-separated); default open.
    pub status: Option<String>,
    /// Only questions routed to your token's expert:<tag> scopes.
    pub mine: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WithdrawArgs {
    /// Question id to withdraw.
    pub id: String,
    /// Optional reason recorded on the withdrawal.
    pub reason: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnswerLinkArgs {
    /// Question id to mint an answer link for.
    pub id: String,
    /// Link lifetime in seconds (default 3 days, max 30 days).
    pub ttl_seconds: Option<i64>,
    /// Who a use of the link is attributed to (default human:link:<qid>).
    pub actor: Option<String>,
}

// ---- tools ------------------------------------------------------------------

#[tool_router]
impl TakomoMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Create a new ticket. Surfaces any `similar` existing tickets the store \
        detected (possible duplicates)."
    )]
    async fn takomo_new(
        &self,
        Parameters(a): Parameters<NewArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_new(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "List tickets with optional filters. Returns compact items plus a cursor \
        for pagination."
    )]
    async fn takomo_list(
        &self,
        Parameters(a): Parameters<ListArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_list(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "List tickets that are ready to be worked (unblocked, in a claimable \
        ready state)."
    )]
    async fn takomo_ready(
        &self,
        Parameters(a): Parameters<ReadyArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_ready(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Fetch one full ticket by id, including body, links, dependencies, and \
        any active claim."
    )]
    async fn takomo_show(
        &self,
        Parameters(a): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_show(&require_auth(&ctx)?, &a.id))
    }

    #[tool(
        description = "Claim a specific ticket by id, taking its lease. Later \
        start/transition/done/release calls resolve the fencing token automatically."
    )]
    async fn takomo_claim(
        &self,
        Parameters(a): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_claim(&require_auth(&ctx)?, &a.id))
    }

    #[tool(
        description = "Atomically pick and claim the next ready ticket (optionally filtered). \
        With `wait`, long-polls up to that many seconds for work to appear."
    )]
    async fn takomo_next(
        &self,
        Parameters(a): Parameters<NextArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_next(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "Begin work: claim the ticket if claimable and not already yours, then \
        move it into the workflow's in-progress state (override with `to`)."
    )]
    async fn takomo_start(
        &self,
        Parameters(a): Parameters<StartArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_start(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Move a ticket to an explicit state. Includes your fence automatically \
        when you hold the lease. On an illegal move the store's allowed_transitions are returned."
    )]
    async fn takomo_transition(
        &self,
        Parameters(a): Parameters<TransitionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_transition(&require_auth(&ctx)?, &a.id, &a.to, a.fence))
    }

    #[tool(
        description = "Move a ticket to the workflow's terminal done state. Fence resolved \
        automatically."
    )]
    async fn takomo_done(
        &self,
        Parameters(a): Parameters<FenceArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.advance(&require_auth(&ctx)?, &a.id, "done", a.fence))
    }

    #[tool(
        description = "Move a ticket to the workflow's blocked state. Optionally record a \
        comment explaining the blocker first."
    )]
    async fn takomo_block(
        &self,
        Parameters(a): Parameters<BlockArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_block(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Move a ticket to the workflow's cancelled terminal state. Fence \
        resolved automatically."
    )]
    async fn takomo_cancel(
        &self,
        Parameters(a): Parameters<FenceArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.advance(&require_auth(&ctx)?, &a.id, "cancelled", a.fence))
    }

    #[tool(description = "Add a comment to a ticket.")]
    async fn takomo_comment(
        &self,
        Parameters(a): Parameters<CommentArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_comment(&require_auth(&ctx)?, &a.id, &a.body))
    }

    #[tool(
        description = "Attach or update a named link on a ticket (e.g. key='pr'). Existing \
        links are merged, not replaced."
    )]
    async fn takomo_link(
        &self,
        Parameters(a): Parameters<LinkArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_link(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Record that a ticket is blocked by another ticket (adds a dependency \
        edge)."
    )]
    async fn takomo_dep(
        &self,
        Parameters(a): Parameters<DepArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_dep(&require_auth(&ctx)?, &a.id, &a.blocked_by))
    }

    #[tool(
        description = "Inspect a ticket's dependency graph (blocked_by / blocks / both, \
        optionally transitive)."
    )]
    async fn takomo_deps(
        &self,
        Parameters(a): Parameters<DepsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_deps(&require_auth(&ctx)?, a))
    }

    #[tool(description = "Release your claim/lease on a ticket, echoing the fencing token.")]
    async fn takomo_release(
        &self,
        Parameters(a): Parameters<FenceArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_release(&require_auth(&ctx)?, &a.id, a.fence))
    }

    #[tool(
        description = "Archive a ticket, hiding it from default list/ready/board views. \
        Idempotent."
    )]
    async fn takomo_archive(
        &self,
        Parameters(a): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_archive(&require_auth(&ctx)?, &a.id))
    }

    #[tool(description = "List all projects visible to your token and their workflow names.")]
    async fn takomo_projects(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_projects(&require_auth(&ctx)?))
    }

    #[tool(
        description = "Show a project's workflow definition (states, categories, and legal \
        transitions). Useful for self-correcting illegal moves."
    )]
    async fn takomo_workflow(
        &self,
        Parameters(a): Parameters<ProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_workflow(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "Show a project's roadmap: epics with their child tickets and progress, \
        each with `flags` for epics whose own state contradicts their children \
        (done_with_open_children, open_with_all_children_done, empty_epic), plus an \
        `unparented` rollup over the non-epic tickets no epic owns."
    )]
    async fn takomo_roadmap(
        &self,
        Parameters(a): Parameters<ProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_roadmap(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "Ask a human for a decision when you are blocked (confirmation, a choice, \
        a clarification, or approval). Parks the ticket in a blocked state and releases your \
        lease: end your run and resume the ticket after a human answers. Route to a domain \
        expert with `expertise` tags like [\"domain:billing\"]. Phrase the question (and any \
        options) in the project's expected human-facing language when one is set — see the \
        `language_hint` on takomo_show/next/start or takomo_workflow's `question_language`."
    )]
    async fn takomo_ask(
        &self,
        Parameters(a): Parameters<AskArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_ask(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Answer an open question (requires the human scope). Records the reply and \
        performs the ticket's human-gated transition to resume it."
    )]
    async fn takomo_answer(
        &self,
        Parameters(a): Parameters<AnswerArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_answer(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "List open questions on the ask-a-human board (the inbox). Filter by \
        project/ticket/status, or `mine` to see only questions routed to your expert:<tag> scopes."
    )]
    async fn takomo_questions(
        &self,
        Parameters(a): Parameters<QuestionsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_questions(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Withdraw an open question you no longer need answered (e.g. you resolved \
        the blocker yourself). The ticket stays parked; resume it with takomo_transition."
    )]
    async fn takomo_withdraw(
        &self,
        Parameters(a): Parameters<WithdrawArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_withdraw(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Mint a per-question answer link for an outside expert who shouldn't hold a \
        token. Requires the human scope (and, for an approve question, the matching expert:<tag>). \
        Returns a single-use, expiring tka_ token + a /board#a=<token> path — share it with the person."
    )]
    async fn takomo_answer_link(
        &self,
        Parameters(a): Parameters<AnswerLinkArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_answer_link(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Identify the caller behind the current token: actor, scopes, and \
        project access."
    )]
    async fn takomo_whoami(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        let mut scopes: Vec<String> = auth.scopes.iter().cloned().collect();
        scopes.sort();
        let projects = match auth.allowed_projects_vec() {
            None => json!("*"),
            Some(list) => json!(list),
        };
        respond(Ok(json!({
            "ok": true,
            "whoami": {
                "token_id": auth.token_id,
                "actor": auth.actor,
                "scopes": scopes,
                "projects": projects,
            }
        })))
    }
}

// ---- tool implementations (call the internal store directly) ----------------

impl TakomoMcp {
    fn do_new(&self, auth: &AuthCtx, a: NewArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let req = TicketCreate {
            project: a.project,
            ty: a.r#type,
            parent: a.parent,
            title: a.title,
            body: a.body,
            priority: a.priority,
            labels: a.labels.unwrap_or_default(),
            metadata: None,
            blocked_by: Vec::new(),
            state: None,
        };
        // A fresh idempotency key per call keeps a retried MCP frame from
        // double-creating, matching the Node wrapper's auto-key behaviour.
        let key = format!("mcp-{}", crate::ids::ticket_suffix(16));
        let (ticket, similar, _replayed) =
            self.state
                .store
                .create_ticket(&req, &auth.actor, Some(&key))?;
        self.state.wake();
        let mut out = json!({ "ok": true, "ticket": ticket.to_json(now_ms()) });
        if !similar.is_empty() {
            out["similar"] = Value::Array(similar.clone());
            out["note"] = json!(format!(
                "Store detected {} possibly-similar ticket(s); review before assuming this is new.",
                similar.len()
            ));
        }
        Ok(out)
    }

    fn do_list(&self, auth: &AuthCtx, a: ListArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let filter = TicketListFilter {
            project: a.project,
            state: a.state,
            ty: a.r#type,
            labels: a.label.into_iter().collect(),
            parent: None,
            q: a.q,
            claimed_by: None,
            allowed_projects: auth.allowed_projects_vec(),
            archived: crate::store::ArchivedFilter::Exclude,
        };
        let limit = a.limit.unwrap_or(50).clamp(1, 200);
        let cursor = match a.cursor {
            None => None,
            Some(c) => Some(c.parse::<i64>().map_err(|_| {
                ApiError::bad_request(
                    "validation.cursor",
                    "Invalid cursor; pass the exact next_cursor value from the previous page.",
                )
            })?),
        };
        let (tickets, next_cursor) = self.state.store.list_tickets(&filter, cursor, limit)?;
        let items: Vec<Value> = tickets.iter().map(brief).collect();
        Ok(json!({ "ok": true, "items": items, "next_cursor": next_cursor }))
    }

    fn do_ready(&self, auth: &AuthCtx, a: ReadyArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let filter = ReadyFilter {
            project: a.project,
            ty: a.r#type,
            labels: a.label.into_iter().collect(),
            allowed_projects: auth.allowed_projects_vec(),
        };
        let tickets = self.state.store.ready_peek(&filter, 20)?;
        let items: Vec<Value> = tickets.iter().map(brief).collect();
        Ok(json!({ "ok": true, "items": items }))
    }

    fn do_show(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let mut out = json!({ "ok": true, "ticket": ticket.to_json(now_ms()) });
        // Surface every open human question so a resuming agent sees the full
        // barrier (the ticket resumes only once all are answered) and, once
        // answered, reads the decisions on the ticket's comments.
        let open = self.state.store.open_questions_for_ticket(id)?;
        if !open.is_empty() {
            out["open_questions"] = json!(open.iter().map(|q| q.to_json()).collect::<Vec<_>>());
        }
        let hint = self.language_hint(&ticket.project);
        if !hint.is_null() {
            out["language_hint"] = hint;
        }
        Ok(out)
    }

    fn do_ask(&self, auth: &AuthCtx, a: AskArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let expires_at = match a.expires_in_seconds {
            Some(s) if s > 0 => Some(now_ms() + s * 1000),
            _ => None,
        };
        let on_timeout = match a.on_timeout.as_deref() {
            Some(raw) => Some(crate::store::TimeoutAction::parse(raw)?),
            None => None,
        };
        let req = crate::store::AskRequest {
            ticket: a.id.clone(),
            mode: a.mode,
            kind: a.kind,
            title: a.title,
            body: a.body.unwrap_or_default(),
            options: a.options.unwrap_or_default(),
            recommended: a.recommended.map(Value::String).unwrap_or(Value::Null),
            expertise: a.expertise.unwrap_or_default(),
            urgency: a.urgency,
            expires_at,
            on_timeout,
            fence: resolve_fence(&ticket, &auth.actor, a.fence),
        };
        let (question, updated) = self.state.store.ask_question(&req, &auth.actor)?;
        self.state.wake();
        crate::notify::question_asked(&self.state, &question);
        let mut note = if question.mode == "advisory" {
            format!(
                "Advisory question recorded on '{}' — it does not change the ticket's state or your lease. It's routed to the inbox for a human; keep working or end your run as you see fit.",
                updated.id
            )
        } else {
            format!(
                "Parked '{}' in '{}' and released your lease. End your run; resume once every open question on it is answered (the answers land on this ticket).",
                updated.id, updated.state
            )
        };
        // Soft language nudge: if the project expects a specific human-facing
        // language, remind the agent (re-ask correctly if this one wasn't).
        if let Ok(Some(p)) = self.state.store.get_project(&question.project) {
            if let Some(lang) = p.question_language.filter(|l| !l.trim().is_empty()) {
                note.push_str(&format!(
                    " This project expects the question (and any options) written in {lang} — re-ask in {lang} if this one wasn't.",
                ));
            }
        }
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
            "ticket": updated.to_json(now_ms()),
            "note": note,
        }))
    }

    fn do_answer(&self, auth: &AuthCtx, a: AnswerArgs) -> ApiResult<Value> {
        auth.require_scope("human")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        let answer = match a.note {
            Some(n) => json!({ "value": a.answer, "note": n }),
            None => json!({ "value": a.answer }),
        };
        let (question, ticket) = self.state.store.answer_question(
            &a.id,
            &auth.actor,
            &auth.scopes,
            &answer,
            a.resume_to.as_deref(),
        )?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
            "ticket": ticket.to_json(now_ms()),
        }))
    }

    fn do_questions(&self, auth: &AuthCtx, a: QuestionsArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let expertise = if a.mine.unwrap_or(false) {
            let tags: Vec<String> = auth
                .scopes
                .iter()
                .filter_map(|s| s.strip_prefix("expert:").map(str::to_string))
                .collect();
            if tags.is_empty() {
                return Ok(
                    json!({ "ok": true, "items": [], "note": "Your token carries no expert:<tag> scopes, so no questions route to you. Drop `mine` to see the whole queue." }),
                );
            }
            tags
        } else {
            Vec::new()
        };
        let filter = crate::store::QuestionFilter {
            project: a.project,
            ticket: a.ticket,
            statuses: a
                .status
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            expertise,
            allowed_projects: auth.allowed_projects_vec(),
        };
        let items = self.state.store.list_questions(&filter)?;
        Ok(json!({ "ok": true, "items": items.iter().map(|q| q.to_json()).collect::<Vec<_>>() }))
    }

    fn do_withdraw(&self, auth: &AuthCtx, a: WithdrawArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        let question =
            self.state
                .store
                .withdraw_question(&a.id, &auth.actor, a.reason.as_deref())?;
        self.state.wake();
        Ok(json!({ "ok": true, "question": question.to_json() }))
    }

    fn do_answer_link(&self, auth: &AuthCtx, a: AnswerLinkArgs) -> ApiResult<Value> {
        auth.require_scope("human")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        if q.status != "open" {
            return Err(ApiError::conflict(
                "question.not_open",
                format!("Question '{}' is '{}', not open.", a.id, q.status),
            ));
        }
        if q.kind == "approve" {
            let has_expert = q
                .expertise
                .iter()
                .any(|t| auth.scopes.contains(&format!("expert:{t}")));
            if !has_expert {
                return Err(ApiError::new(
                    axum::http::StatusCode::FORBIDDEN,
                    "question.approve_expertise",
                    "Minting an answer link for an 'approve' question needs the matching expert:<tag> scope — you can only delegate authority you hold.",
                ));
            }
        }
        let ttl = match a.ttl_seconds {
            None => crate::store::DEFAULT_ANSWER_TTL_SECONDS,
            Some(s) if s <= 0 || s > crate::store::MAX_ANSWER_TTL_SECONDS => {
                return Err(ApiError::validation(
                    "answer_link.ttl",
                    format!(
                        "ttl_seconds must be between 1 and {} (30 days).",
                        crate::store::MAX_ANSWER_TTL_SECONDS
                    ),
                ))
            }
            Some(s) => s,
        };
        let actor = a.actor.unwrap_or_else(|| format!("human:link:{}", a.id));
        let expires_at = now_ms() + ttl * 1000;
        let (row, plaintext) = self.state.store.create_answer_grant(
            &a.id,
            &q.project,
            &actor,
            expires_at,
            &auth.actor,
        )?;
        let mut out = row.to_json();
        out["token"] = json!(plaintext);
        out["path"] = json!(format!("/board#a={plaintext}"));
        if let Ok(base) = std::env::var("TAKOMO_PUBLIC_URL") {
            if !base.trim().is_empty() {
                out["url"] = json!(format!(
                    "{}/board#a={plaintext}",
                    base.trim_end_matches('/')
                ));
            }
        }
        out["note"] = json!("Single-use, expiring link. Share it only with the intended person; the token is shown once.");
        Ok(json!({ "ok": true, "answer_link": out }))
    }

    fn do_claim(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let (_ticket, lease) = self.state.store.claim_ticket(id, &auth.actor, None)?;
        self.state.wake();
        let mut out = json!({ "ok": true, "lease": lease.to_json() });
        let hint = self.language_hint(&ticket.project);
        if !hint.is_null() {
            out["language_hint"] = hint;
        }
        Ok(out)
    }

    async fn do_next(&self, auth: &AuthCtx, a: NextArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let filter = ReadyFilter {
            project: a.project,
            ty: a.r#type,
            labels: a.label.into_iter().collect(),
            allowed_projects: auth.allowed_projects_vec(),
        };
        let wait = a.wait.unwrap_or(0).clamp(0, 60);
        let deadline = now_ms() + wait * 1000;
        loop {
            if let Some((ticket, lease)) =
                self.state.store.ready_claim(&filter, &auth.actor, None)?
            {
                self.state.wake();
                let project = ticket.project.clone();
                let mut out = ticket.to_json(now_ms());
                out["lease"] = lease.to_json();
                let mut res = json!({ "ok": true, "claimed": true, "ticket": out });
                let hint = self.language_hint(&project);
                if !hint.is_null() {
                    res["language_hint"] = hint;
                }
                return Ok(res);
            }
            if now_ms() >= deadline {
                return Ok(
                    json!({ "ok": true, "claimed": false, "note": "No ready ticket to claim." }),
                );
            }
            // Poll at most every 2s, and never sleep past the deadline.
            let poll_ms = (deadline - now_ms()).clamp(1, 2000) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
        }
    }

    fn do_start(&self, auth: &AuthCtx, a: StartArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let wf = self.workflow_for(&ticket.project)?;

        let mut fence = resolve_fence(&ticket, &auth.actor, a.fence);
        // Claim if we do not already hold the lease and the state is claimable.
        if fence.is_none() && is_claimable(&wf, &ticket.state) {
            let (_t, lease) = self.state.store.claim_ticket(&a.id, &auth.actor, None)?;
            fence = Some(lease.fence);
        }

        let target = match a.to {
            Some(t) => t,
            None => {
                if category_of(&wf, &ticket.state) == Some("in_progress") {
                    let fresh = self.state.store.get_ticket(&a.id)?;
                    return Ok(json!({
                        "ok": true,
                        "note": format!("Already in an in-progress state ('{}').", ticket.state),
                        "ticket": fresh.map(|t| t.to_json(now_ms())),
                    }));
                }
                let cands = targets_in_category(&wf, &ticket.state, "in_progress");
                match cands.into_iter().next() {
                    Some(t) => t,
                    None => {
                        return Err(ApiError::conflict(
                            "transition.no_target",
                            format!(
                                "No in-progress transition available from '{}' in workflow '{}'. Pass an explicit `to`.",
                                ticket.state, wf.name
                            ),
                        )
                        .current_state(ticket.state.clone())
                        .allowed_transitions(allowed_transitions_from(&wf, &ticket.state)));
                    }
                }
            }
        };

        let updated =
            self.state
                .store
                .transition(&a.id, &target, None, fence, &auth.actor, &auth.scopes)?;
        self.state.wake();
        let mut out =
            json!({ "ok": true, "transitioned_to": target, "ticket": updated.to_json(now_ms()) });
        let hint = self.language_hint(&updated.project);
        if !hint.is_null() {
            out["language_hint"] = hint;
        }
        Ok(out)
    }

    fn do_transition(
        &self,
        auth: &AuthCtx,
        id: &str,
        to: &str,
        fence_override: Option<i64>,
    ) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let fence = resolve_fence(&ticket, &auth.actor, fence_override);
        let updated =
            self.state
                .store
                .transition(id, to, None, fence, &auth.actor, &auth.scopes)?;
        self.state.wake();
        Ok(json!({ "ok": true, "transitioned_to": to, "ticket": updated.to_json(now_ms()) }))
    }

    /// Advance to the first legal target in `category` (done/blocked/cancelled),
    /// resolving state names from the project workflow. Mirrors the Node MCP.
    fn advance(
        &self,
        auth: &AuthCtx,
        id: &str,
        category: &str,
        fence_override: Option<i64>,
    ) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let wf = self.workflow_for(&ticket.project)?;
        let target = match targets_in_category(&wf, &ticket.state, category)
            .into_iter()
            .next()
        {
            Some(t) => t,
            None => {
                return Err(ApiError::conflict(
                    "transition.no_target",
                    format!(
                        "No legal transition to a '{category}' state from '{}' in workflow '{}'.",
                        ticket.state, wf.name
                    ),
                )
                .current_state(ticket.state.clone())
                .allowed_transitions(allowed_transitions_from(&wf, &ticket.state)));
            }
        };
        let fence = resolve_fence(&ticket, &auth.actor, fence_override);
        let updated =
            self.state
                .store
                .transition(id, &target, None, fence, &auth.actor, &auth.scopes)?;
        self.state.wake();
        Ok(json!({ "ok": true, "transitioned_to": target, "ticket": updated.to_json(now_ms()) }))
    }

    fn do_block(&self, auth: &AuthCtx, a: BlockArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        load_visible(&self.state, auth, &a.id)?;
        if let Some(comment) = &a.comment {
            self.state.store.add_comment(&a.id, &auth.actor, comment)?;
            self.state.wake();
        }
        self.advance(auth, &a.id, "blocked", a.fence)
    }

    fn do_comment(&self, auth: &AuthCtx, id: &str, body: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        load_visible(&self.state, auth, id)?;
        let comment = self.state.store.add_comment(id, &auth.actor, body)?;
        self.state.wake();
        Ok(json!({ "ok": true, "comment": comment.to_json() }))
    }

    fn do_link(&self, auth: &AuthCtx, a: LinkArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let mut links = ticket.links.as_object().cloned().unwrap_or_default();
        links.insert(a.key, Value::String(a.value));
        let patch = TicketPatch {
            links: Some(Value::Object(links)),
            fence: resolve_fence(&ticket, &auth.actor, None),
            ..Default::default()
        };
        let updated = self
            .state
            .store
            .patch_ticket(&a.id, &patch, &auth.actor, None)?;
        self.state.wake();
        Ok(json!({ "ok": true, "links": updated.links }))
    }

    fn do_dep(&self, auth: &AuthCtx, id: &str, blocked_by: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let fence = resolve_fence(&ticket, &auth.actor, None);
        self.state
            .store
            .add_dep(id, blocked_by, &auth.actor, fence)?;
        self.state.wake();
        Ok(json!({ "ok": true, "dependency": { "ticket": id, "blocked_by": blocked_by } }))
    }

    fn do_deps(&self, auth: &AuthCtx, a: DepsArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        load_visible(&self.state, auth, &a.id)?;
        let direction = match a.direction.as_deref() {
            None => crate::store::DepDirection::BlockedBy,
            Some(raw) => crate::store::DepDirection::parse(raw).ok_or_else(|| {
                ApiError::bad_request(
                    "validation.direction",
                    format!("Unknown direction '{raw}'. Use one of: blocked_by, blocks, both."),
                )
            })?,
        };
        let graph = self
            .state
            .store
            .dep_graph(&a.id, direction, a.transitive.unwrap_or(false))?;
        Ok(json!({ "ok": true, "deps": graph }))
    }

    fn do_release(
        &self,
        auth: &AuthCtx,
        id: &str,
        fence_override: Option<i64>,
    ) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let fence = resolve_fence(&ticket, &auth.actor, fence_override).ok_or_else(|| {
            ApiError::conflict(
                "release.no_lease",
                format!(
                    "You do not hold an active lease on '{id}'. Pass an explicit fence to release."
                ),
            )
        })?;
        self.state.store.release(id, fence, &auth.actor, None)?;
        self.state.wake();
        Ok(json!({ "ok": true, "released": id }))
    }

    fn do_archive(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        load_visible(&self.state, auth, id)?;
        let ticket = self.state.store.archive_ticket(id, &auth.actor)?;
        self.state.wake();
        Ok(json!({ "ok": true, "ticket": ticket.to_json(now_ms()) }))
    }

    fn do_projects(&self, auth: &AuthCtx) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let projects = self.state.store.list_projects()?;
        let out: Vec<Value> = projects
            .iter()
            .filter(|p| auth.can_project(&p.id))
            .map(|p| p.to_json())
            .collect();
        Ok(json!({ "ok": true, "projects": out }))
    }

    fn do_workflow(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let wf = self.workflow_for(project)?;
        let lang = self
            .state
            .store
            .get_project(project)?
            .and_then(|p| p.question_language);
        Ok(json!({ "ok": true, "workflow": wf, "question_language": lang }))
    }

    fn do_roadmap(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let roadmap = self.state.store.roadmap(project)?;
        Ok(json!({ "ok": true, "roadmap": roadmap }))
    }

    /// A hint about the project's expected human-facing question language, for
    /// attaching to work-loop responses so an agent phrases `takomo_ask`
    /// questions correctly. Null when the project sets no language.
    fn language_hint(&self, project: &str) -> Value {
        match self.state.store.get_project(project) {
            Ok(Some(p)) => match p.question_language {
                Some(lang) if !lang.trim().is_empty() => json!({
                    "question_language": lang,
                    "note": format!("This project expects human-facing questions (takomo_ask) and their options written in {lang}. Internal ticket text may be in another language."),
                }),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    /// Load a project's workflow, or a teaching not-found error.
    fn workflow_for(&self, project: &str) -> ApiResult<Workflow> {
        self.state
            .store
            .get_project(project)?
            .map(|p| p.workflow)
            .ok_or_else(|| ApiError::not_found("project", project))
    }
}

// ---- server handshake -------------------------------------------------------

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TakomoMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("takomo", crate::server::VERSION))
            .with_instructions(
                "takomo: the central tracker for AI agent fleets. Typical work loop: \
                 `takomo_next` to claim the next ready ticket, `takomo_start` to move it \
                 in-progress, `takomo_comment`/`takomo_link` to record progress, then \
                 `takomo_done` (or `takomo_block`/`takomo_cancel`). When you need a human \
                 decision (confirmation, a choice, a clarification, approval), call \
                 `takomo_ask` — it parks the ticket and releases your lease; end your run and \
                 resume once the answer appears on the ticket (`takomo_show`). When a project \
                 sets a human-facing language (surfaced as `language_hint` on \
                 takomo_next/claim/start/show and `question_language` on takomo_workflow), \
                 phrase ask-a-human questions in it. Illegal \
                 transitions return the workflow's allowed_transitions so you can self-correct; \
                 call `takomo_workflow` to see a project's full state machine."
                    .to_string(),
            )
    }
}

// ---- helpers ----------------------------------------------------------------

/// Pull the middleware-resolved identity out of the HTTP request parts that the
/// transport injects into the tool call context. The bearer auth middleware has
/// already rejected missing/invalid/share tokens, so absence here is an internal
/// invariant failure, not a client error.
fn require_auth(ctx: &RequestContext<RoleServer>) -> Result<AuthCtx, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<AuthCtx>())
        .cloned()
        .ok_or_else(|| {
            McpError::internal_error(
                "MCP request reached a tool without an authenticated identity",
                None,
            )
        })
}

/// Convert an internal result into an MCP tool result. Success serializes the
/// value; a store error is relayed verbatim (code / message / remedy /
/// current_state / allowed_transitions) as a tool-level error so the agent can
/// self-correct — mirroring the Node wrapper's error passthrough.
fn respond(result: ApiResult<Value>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
            to_pretty(&value),
        )])),
        Err(err) => {
            let mut obj = serde_json::to_value(&err.body)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            obj.insert("ok".to_string(), json!(false));
            obj.insert("status".to_string(), json!(err.status.as_u16()));
            Ok(CallToolResult::error(vec![ContentBlock::text(to_pretty(
                &Value::Object(obj),
            ))]))
        }
    }
}

fn to_pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// The fence to send on a claimed-ticket write: an explicit override wins,
/// otherwise the ticket's current `fence_seq` when this actor holds the active
/// claim (that IS the valid fencing token), otherwise none.
fn resolve_fence(ticket: &Ticket, actor: &str, override_: Option<i64>) -> Option<i64> {
    if override_.is_some() {
        return override_;
    }
    match ticket.active_claim(now_ms()) {
        Some((holder, _)) if holder == actor => Some(ticket.fence_seq),
        _ => None,
    }
}

fn is_claimable(wf: &Workflow, state: &str) -> bool {
    wf.state(state).map(|s| s.claimable).unwrap_or(false)
}

fn category_of<'a>(wf: &'a Workflow, state: &str) -> Option<&'a str> {
    wf.state(state).map(|s| s.category.as_str())
}

/// Legal target states in `category` reachable from `from_state`.
fn targets_in_category(wf: &Workflow, from_state: &str, category: &str) -> Vec<String> {
    wf.transitions
        .iter()
        .filter(|t| t.from == from_state && category_of(wf, &t.to) == Some(category))
        .map(|t| t.to.clone())
        .collect()
}

fn allowed_transitions_from(wf: &Workflow, from: &str) -> Vec<AllowedTransition> {
    wf.transitions_from(from)
        .into_iter()
        .map(|t| AllowedTransition {
            to: t.to.clone(),
            requires: t.requires.clone(),
        })
        .collect()
}

/// Compact ticket shape for list/ready output (mirrors the Node MCP `brief`).
fn brief(t: &Ticket) -> Value {
    json!({
        "id": t.id,
        "title": t.title,
        "state": t.state,
        "category": t.state_category,
        "type": t.ty,
        "priority": t.priority,
        "labels": t.labels,
        "parent": t.parent,
        "blocked_by": if t.blocked_by.is_empty() { Value::Null } else { json!(t.blocked_by) },
        "claimed_by": t.active_claim(now_ms()).map(|(h, _)| h),
    })
}
