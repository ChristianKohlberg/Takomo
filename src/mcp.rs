//! Hosted MCP server: the streamable-HTTP MCP transport, in the same binary.
//!
//! This exposes the tracker's work loop as native MCP tools over a remote HTTP
//! transport (rmcp's `StreamableHttpService`), so an agent can attach with
//! `claude mcp add --transport http https://<host>/mcp` instead of spawning the
//! Node stdio wrapper in `clients/mcp`. Tools call the internal `Store` directly
//! — no HTTP round-trip back to this process, no duplicated API logic.
//!
//! Auth: the `/mcp` endpoint is wrapped in the SAME bearer-token path as the
//! REST API (`crate::auth::mcp_auth_middleware`), so a missing/invalid token or
//! a share (`tks_`) token is rejected before any MCP frame is processed, and the
//! resolved [`AuthCtx`] rides in the request extensions. Every tool re-checks
//! scope and project access exactly like the matching REST handler.
//!
//! Rate limiting: the per-token write budget is debited per tool call here (see
//! [`READ_TOOLS`] and `TakomoMcp::call_tool`), not in the middleware — every MCP
//! frame is `POST /mcp`, so the HTTP method cannot tell a read from a write.
//!
//! Fences: the Node wrapper remembers a claimed ticket's fencing token in
//! process memory; a hosted server cannot rely on session affinity, so instead
//! the fence is resolved from the store — when the caller holds the active claim
//! its valid fence IS the ticket's current `fence_seq`. An explicit `fence`
//! argument always overrides. Target states for the convenience verbs
//! (start/done/block/cancel) are resolved from the project workflow by category,
//! mirroring the CLI and the Node MCP.

use crate::api::tickets::load_visible;
use crate::auth::{debit_write_budget, mcp_auth_middleware, AuthCtx};
use crate::error::{AllowedTransition, ApiError, ApiResult};
use crate::ids::now_ms;
use crate::server::AppState;
use crate::store::{ReadyFilter, Ticket, TicketCreate, TicketListFilter, TicketPatch};
use crate::workflow::Workflow;
use yrs::Transact as _;

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
        .layer(axum::middleware::from_fn_with_state(
            state,
            mcp_auth_middleware,
        ))
}

/// The MCP tools that only read. Everything else mutates and debits the
/// caller's per-minute write budget (see `TakomoMcp::call_tool`).
///
/// Reads are free here because they are free on the REST surface too — `GET
/// /v1/tickets/{id}` costs nothing, and `takomo_show` is the same query through
/// a different door. The budget exists to contain runaway *mutation* loops, not
/// to meter reads, and metering them here would let an agent rate-limit itself
/// out of the tracker without having changed anything.
///
/// A name that is not listed counts as a write, so a tool added later is
/// charged until it is deliberately declared a read — the safe direction.
pub const READ_TOOLS: &[&str] = &[
    "takomo_lanes",
    "takomo_lane_show",
    "takomo_lane_handoffs",
    "takomo_specification_history",
    "takomo_specification_version",
    "takomo_test_definitions",
    "takomo_test_runs",
    "takomo_test_run",
    "takomo_check",
    "takomo_checks",
    "takomo_claim_status",
    "takomo_coverage",
    "takomo_deps",
    "takomo_document_proposals",
    "takomo_document_read",
    "takomo_plan_read",
    "takomo_plan_proposals",
    "takomo_documents",
    "takomo_environments",
    "takomo_gate",
    "takomo_impact",
    "takomo_initiative_list",
    "takomo_initiative_show",
    "takomo_list",
    "takomo_mindmap_list",
    "takomo_mindmap_show",
    "takomo_projects",
    "takomo_questions",
    "takomo_ready",
    "takomo_releases",
    "takomo_roadmap",
    "takomo_schedules",
    "takomo_show",
    "takomo_users",
    "takomo_whoami",
    "takomo_workflow",
    "takomo_worklist",
];

/// The MCP tool surface. Cloned per session by the transport's service factory;
/// all real state lives behind the shared `Arc<AppState>`.
#[derive(Clone)]
pub struct TakomoMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<TakomoMcp>,
    /// The `tools/list` reply, built once and shared by every session (see
    /// [`slim_tools`]). `Arc` because this type is cloned per session and the
    /// list is identical for all of them.
    tools: Arc<Vec<rmcp::model::Tool>>,
}

/// The tool list with per-tool JSON Schema boilerplate removed.
///
/// Every session pays for `tools/list` before it can call anything, and on this
/// surface that reply is ~46 KB across 49 tools — roughly 12.5k tokens of an
/// agent's context spent on discovery. Two thirds of it is generated schema
/// rather than anything written here, and one line of that is pure repetition:
/// `schemars` stamps the same `$schema` dialect URI into all 49, where it says
/// nothing a client acts on. Dropping it removes about 2.8 KB.
///
/// Deliberately only the dialect line. The other obvious candidate is the
/// `["string", "null"]` type that every optional argument carries, which would
/// save more — but `Option<T>` genuinely accepts an explicit `null`, and a
/// client that sends one would then be sending something the advertised schema
/// forbids. Saving tokens by publishing a schema that is not true about the
/// arguments is the wrong trade.
fn slim_tools(mut tools: Vec<rmcp::model::Tool>) -> Vec<rmcp::model::Tool> {
    for tool in &mut tools {
        if tool.input_schema.contains_key("$schema") {
            let mut schema = (*tool.input_schema).clone();
            schema.remove("$schema");
            tool.input_schema = Arc::new(schema);
        }
    }
    tools
}

// ---- tool argument schemas --------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkLanesArgs {
    pub project: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkLaneIdArgs {
    pub id: String,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkLaneCreateArgs {
    pub project: String,
    pub title: String,
    pub purpose: Option<String>,
    pub context: Option<String>,
}
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct WorkLaneUpdateArgs {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkLaneTicketArgs {
    pub lane: String,
    pub ticket: String,
    pub remove: Option<bool>,
}
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct WorkLaneHandoffArgs {
    pub lane: String,
    pub kind: String,
    pub provider: String,
    pub instructions: String,
    pub ticket_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_handoff: Option<String>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkLaneHandoffsArgs {
    pub lane: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

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
    /// Tag references to attach, each `kind:handle` (e.g. `person:ada`,
    /// `component:billing`). Unknown handles are registered on the fly.
    pub tags: Option<Vec<String>>,
    /// Markdown body / description.
    pub body: Option<String>,
    /// Optional metadata object (arbitrary JSON).
    pub metadata: Option<Value>,
    /// Ticket ids this one is blocked by at creation.
    pub blocked_by: Option<Vec<String>>,
    /// Idempotency key for safe retries. Omit to create a new ticket on every
    /// call; pass the same key on a retried MCP frame to replay the original.
    pub idempotency_key: Option<String>,
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
    /// Filter by an exact tag reference, `kind:handle` (e.g. `person:ada`).
    pub tag: Option<String>,
    /// Filter by tag kind — match tickets carrying any tag of this kind (e.g.
    /// `person`).
    pub tag_kind: Option<String>,
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
    /// How many to return, 1..=200 (default 20). The response always reports the
    /// queue's `total`, so a page that left work out says so.
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdArgs {
    /// Ticket id.
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClaimArgs {
    /// Ticket id.
    pub id: String,
    /// Lease lifetime in seconds (1-3600, default 900). Ask for what the work
    /// will plausibly take; extend with `takomo_heartbeat` rather than guessing
    /// high, since a lease you stop renewing is how a dead worker is detected.
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct HeartbeatArgs {
    /// Ticket id.
    pub id: String,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
    /// New lease lifetime in seconds from now (1-3600, default 900).
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NextArgs {
    /// Restrict to a project id.
    pub project: Option<String>,
    /// Restrict to a ticket type.
    pub r#type: Option<String>,
    /// Restrict to a single label.
    pub label: Option<String>,
    /// Seconds to long-poll for work before giving up (0-120, default 0).
    pub wait: Option<i64>,
    /// Lease lifetime in seconds for the ticket this claims (1-3600, default 900).
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StartArgs {
    /// Ticket id.
    pub id: String,
    /// Explicit target state (defaults to the workflow's in-progress state).
    pub to: Option<String>,
    /// Override the fencing token (normally resolved automatically).
    pub fence: Option<i64>,
    /// Lease lifetime in seconds, if this call is what takes the claim
    /// (1-3600, default 900). Ignored when you already hold the lease — use
    /// `takomo_heartbeat` to extend one you hold.
    pub ttl_seconds: Option<i64>,
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
    /// Link value (URL or ref). `null` deletes the key.
    pub value: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TagArgs {
    /// Ticket id to tag.
    pub id: String,
    /// Tag references to add, each `kind:handle` (e.g. `person:ada`,
    /// `component:billing`). Unknown handles are registered automatically.
    pub add: Option<Vec<String>>,
    /// Tag references to remove.
    pub remove: Option<Vec<String>>,
    /// Fencing token, if you hold the ticket's lease.
    pub fence: Option<i64>,
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
    /// Optional epic id — narrow the report to that epic's descendant subtree.
    /// Only `takomo_roadmap` and `takomo_impact` read it; the other tools taking
    /// these args ignore it.
    #[serde(default)]
    pub epic: Option<String>,
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
    /// For kind=choose: a one-line trade-off description per option, parallel to
    /// `options` (same length). Lets the inbox show what each choice means.
    pub option_notes: Option<Vec<String>>,
    /// For kind=choose: allow selecting several options at once (multi-select).
    pub multi: Option<bool>,
    /// For a multi choose: the recommended set of options. Every entry must be one
    /// of `options`.
    pub recommended_multi: Option<Vec<String>>,
    /// Your recommended answer (a hint for the human; also applied on timeout if
    /// on_timeout=recommended). When the question has `options` it must be one of
    /// them exactly — since a timeout stores it as the answer, a recommendation that
    /// is not an option could never be one, and is refused. For multi-select,
    /// pass a JSON array of option strings.
    pub recommended: Option<Value>,
    /// A short rationale for your recommendation ("why"), shown by the recommendation.
    pub recommended_note: Option<String>,
    /// How strong your recommendation is, 1-4 (1 tentative … 4 very strong).
    pub confidence: Option<i64>,
    /// A one-line summary for the inbox list preview (optional; else derived).
    pub summary: Option<String>,
    /// Routing tags for the human queue, e.g. ["domain:billing"].
    pub expertise: Option<Vec<String>>,
    /// Address this question to one person, by their handle (see takomo_users).
    /// Use it when you know who owns the decision; leave it out and the question
    /// goes to the queue, where anyone with the expertise can pick it up. They must
    /// be a member of the ticket's project.
    pub assignee: Option<String>,
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
    /// Who actually decided, when you are RELAYING a decision a human already
    /// made elsewhere (e.g. they told you in chat) rather than answering
    /// yourself. Requires the `answer:relay` scope, and records this name as
    /// `answered_by` with you as `relayed_by`.
    ///
    /// Name the real person. Inventing one falsifies an audit trail someone will
    /// later rely on to see who approved what. If no human has actually decided,
    /// leave the question open — that is what it is for. You cannot relay a
    /// question you asked yourself, and `approve` questions are never relayable.
    pub on_behalf_of: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QuestionsArgs {
    /// Filter by project id.
    pub project: Option<String>,
    /// Filter by ticket id.
    pub ticket: Option<String>,
    /// Statuses to include (comma-separated); default open.
    pub status: Option<String>,
    /// Only questions waiting on you: addressed to the person this token belongs
    /// to, or covered by its expert:<tag> scopes.
    pub mine: Option<bool>,
    /// Only questions addressed to this person (a user handle), or the string
    /// "none" for the ones nobody has been asked yet.
    pub assignee: Option<String>,
    /// How many to return, 1..=500 (default 500). `total` always reports how
    /// many matched, so a capped read is visible as one.
    pub limit: Option<i64>,
    /// Skip this many — pass the previous reply's `next_cursor` to continue.
    pub cursor: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapNewArgs {
    /// Project id the map belongs to.
    pub project: String,
    /// The root: what this brainstorm is about, e.g. "Payments rebuild".
    pub title: String,
    /// One line on why it exists, if it needs one.
    pub summary: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapNodeArg {
    /// The node this hangs off. Omit for a branch straight off the root.
    pub parent: Option<String>,
    /// The thought — a sentence or two, 280 characters at most. Brevity is the
    /// method: if it needs more, it wants to be an initiative.
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapGrowArgs {
    /// Mindmap id, e.g. "mm-9f3ka2xz".
    pub id: String,
    /// The thoughts to add — up to 50 in one call, which is how a whole branch
    /// arrives while somebody is still talking. They land together or not at all.
    pub nodes: Vec<MindmapNodeArg>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapShowArgs {
    /// Mindmap id.
    pub id: String,
    /// Narrow to one branch instead of the whole map.
    pub node: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapListArgs {
    /// Filter by project id.
    pub project: Option<String>,
    /// open | parked | distilled.
    pub status: Option<String>,
    /// Substring over title and summary.
    pub q: Option<String>,
    /// How many to return, 1..=200 (default 50).
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MindmapPromoteArgs {
    /// Mindmap id.
    pub id: String,
    /// The node whose branch graduates.
    pub node: String,
    /// `epic` — an epic with this node's direct children as tickets under it.
    /// `initiative` — an initiative seeded with the whole subtree.
    pub target: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UsersArgs {
    /// Only people who are members of this project — the ones work here can be
    /// addressed to.
    pub project: Option<String>,
    /// Case-insensitive substring match on handle, name or email.
    pub q: Option<String>,
    /// Include people who have been disabled (they cannot be assigned new work).
    pub include_disabled: Option<bool>,
    /// How many to return, 1..=200 (default 100). `total` always reports how many
    /// matched, so a capped read is visible as one.
    pub limit: Option<i64>,
    /// Skip this many — the listing is ordered by handle, so offsets are stable.
    pub offset: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignArgs {
    /// Question id (from takomo_questions or the question_asked event).
    pub id: String,
    /// The person's handle (see takomo_users), or null to return the question to
    /// the open queue.
    pub assignee: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WithdrawArgs {
    /// Question id to withdraw.
    pub id: String,
    /// Optional reason recorded on the withdrawal.
    pub reason: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MoveArgs {
    /// Ticket ids to move. An epic id moves the whole epic; whether its children
    /// come along is `descendants`.
    pub tickets: Vec<String>,
    /// The project they should live in afterwards.
    pub to_project: String,
    /// Move each named ticket's full descendant subtree with it. Defaults to
    /// true; false moves only the named tickets and orphans their children,
    /// because a parent and child must share a project.
    pub descendants: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PromoteArgs {
    /// Ticket id to promote.
    pub id: String,
    /// The stage/target the work reached — free-form, e.g. "staging",
    /// "production", "published", "delivered".
    pub target: String,
    /// Optional link (deploy, published page, PR, …).
    pub url: Option<String>,
    /// Optional reference (version, commit, build id, …).
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
    /// Optional note.
    pub note: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReplyArgs {
    /// Question id a human bounced back to you (awaiting == "agent").
    pub id: String,
    /// The research/context the human asked for. Flips the thread back to the
    /// human so they can answer.
    pub message: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReviseOptionsArgs {
    /// Question id (must still be open, and of kind 'choose').
    pub id: String,
    /// The FULL revised option set — this replaces the options, it does not
    /// merge into them. At least 2.
    pub options: Vec<String>,
    /// Optional one-line trade-off per option, parallel to `options` (same
    /// length, or omit entirely).
    pub option_notes: Option<Vec<String>>,
    /// New recommendation. Omit to keep the current one; it must be one of the
    /// revised options, so pass this whenever you drop the option you had
    /// recommended.
    pub recommended: Option<String>,
    /// For a multi choose: the new recommended set. Omit to keep, or send an
    /// empty list to clear.
    pub recommended_multi: Option<Vec<String>>,
    /// Short rationale for the (new) recommendation. Omit to keep.
    pub recommended_note: Option<String>,
    /// Why the options changed — shown to the human who may already have read
    /// the old set, and recorded on the ticket.
    pub reason: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnswerLinkArgs {
    /// Question id to mint an answer link for.
    pub id: String,
    /// Link lifetime in seconds, max 30 days. Omit to take the project's
    /// answer_link_ttl_seconds, and failing that the built-in 7 days.
    pub ttl_seconds: Option<i64>,
    /// Who a use of the link is attributed to (default human:link:<qid>).
    pub actor: Option<String>,
}

// ---- initiative tool argument schemas ---------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SchedulesArgs {
    /// Project id to list schedules for. Omit for every project the token can see.
    pub project: Option<String>,
    /// Narrow by status: pending | active | paused | rejected | retired.
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScheduleNewArgs {
    /// Project id the schedule belongs to.
    pub project: String,
    /// What the cadence is called, e.g. "Weekly review".
    pub name: String,
    /// day | week | month. The finest cadence is daily; there is no sub-daily
    /// unit, because a ticket a fleet churns through every few minutes is a
    /// worker loop rather than tracked work.
    pub every: String,
    /// Repeat every N units (default 1), counted from `starts_at` so "every 2
    /// weeks" keeps landing on the same weeks.
    pub interval: Option<u32>,
    /// Weekday tokens (mon..sun) — required for `every: week`, refused otherwise.
    /// Setting it on a daily cadence is an error, not an ignored extra: it almost
    /// certainly means a weekly cadence was intended.
    pub on: Option<Vec<String>>,
    /// Day of month 1-31 for `every: month`; a day past a short month's end is
    /// clamped to that month's last day.
    pub day: Option<u32>,
    /// Local wall-clock time, `HH:MM`, 24-hour and zero-padded.
    pub at: String,
    /// IANA zone name (e.g. "Europe/Berlin"). Defaults to UTC. Slots are computed
    /// in LOCAL time, so 09:00 stays 09:00 across a daylight-saving boundary.
    pub tz: Option<String>,
    /// Title of the ticket each occurrence creates. Four placeholders are
    /// substituted: {date} {week} {month} {slot}.
    pub title: String,
    /// Body of that ticket (markdown). Same placeholders apply.
    pub body: Option<String>,
    /// Labels for the created tickets.
    pub labels: Option<Vec<String>>,
    /// Why you think this work recurs. Shown to whoever reviews the proposal, so
    /// it is worth writing: name the tickets that already repeated.
    pub rationale: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InitiativeNewArgs {
    /// Project id the initiative belongs to.
    pub project: String,
    /// Quick title — the name the idea goes by.
    pub title: String,
    /// A very short description: what the idea is, in a sentence or two. The long
    /// form belongs in an entry, where it carries provenance.
    pub summary: Option<String>,
    /// Lifecycle label: open (being fed) | parked (set aside) | distilled (its
    /// substance became tickets). Defaults to open.
    pub status: Option<String>,
    /// Free-form labels.
    pub labels: Option<Vec<String>>,
    /// Tag references, each `kind:handle` (e.g. `person:ada`,
    /// `component:billing`) — the same project registry tickets use. Unknown
    /// handles are registered on the fly.
    pub tags: Option<Vec<String>>,
    /// Free-form JSON object for anything structured this initiative carries.
    ///
    /// One key is read by /initiatives: `path` is the FOLDER the document is
    /// filed in, slash-separated (`"product/billing"`). Folders exist only
    /// because a document names one, so there is nothing to create first and
    /// nothing left behind when the last document moves out. Omit it and the
    /// document sits at the root.
    pub metadata: Option<Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InitiativeUpdateArgs {
    /// Initiative id (`ini-…`).
    pub id: String,
    /// New title. Omit to keep.
    pub title: Option<String>,
    /// New short description. Omit to keep.
    pub summary: Option<String>,
    /// New lifecycle label: open | parked | distilled. Omit to keep.
    pub status: Option<String>,
    /// Replace the labels outright. Omit to keep.
    pub labels: Option<Vec<String>>,
    /// Replace the tag references outright. Omit to keep.
    pub tags: Option<Vec<String>>,
    /// Merge into `metadata` (RFC 7386: a null value removes that key). Moving a
    /// document between folders is `{"path": "product/billing"}`; `{"path": null}`
    /// returns it to the root.
    pub metadata_merge: Option<Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InitiativeAppendArgs {
    /// Initiative id (`ini-…`) to append to.
    pub id: String,
    /// What sort of input this is — a free-form slug matching
    /// `^[a-z][a-z0-9-]*$`. Conventional: `note`, `research`, `feedback`,
    /// `transcript`, `document`, `decision`. The vocabulary is open.
    pub kind: String,
    /// Where this input came from, and REQUIRED — an agent id, a person, a
    /// conversation (`agent:w1`, `person:ada`, `claude:chat`). Without it the
    /// collection is text nobody can attribute.
    pub source: String,
    /// Optional short heading for this entry.
    pub title: Option<String>,
    /// The markdown content. Give this, an attachment, or both.
    pub text: Option<String>,
    /// A link to where the input lives (the conversation, the doc, the PR).
    pub source_uri: Option<String>,
    /// When the content ORIGINATED, RFC 3339 (e.g. `2026-07-01T09:00:00Z`) —
    /// as opposed to when it landed here, which is recorded automatically. Set
    /// it when appending something written earlier.
    pub origin_at: Option<String>,
    /// An attached document, base64-encoded (standard alphabet, padded). Capped
    /// at 5 MiB decoded; host anything larger elsewhere and put its URL in
    /// `source_uri`.
    pub content_base64: Option<String>,
    /// The attachment's media type as bare `type/subtype`, e.g.
    /// `application/pdf`. Required unless `filename` is given.
    pub mime: Option<String>,
    /// The attachment's filename. Required unless `mime` is given.
    pub filename: Option<String>,
    /// Free-form JSON object for anything structured about this entry. This is
    /// where the document lives: `pane`, `cites`, `proposed`, `origin`, and the
    /// `quote`/`prefix`/`suffix` anchor. See this tool's description.
    pub meta: Option<Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InitiativeListArgs {
    /// Filter by project id.
    pub project: Option<String>,
    /// Filter by lifecycle label: open | parked | distilled.
    pub status: Option<String>,
    /// Text search over title and summary (every whitespace-separated term must
    /// match).
    pub q: Option<String>,
    /// Filter by a single label.
    pub label: Option<String>,
    /// Filter by an exact tag reference, `kind:handle`.
    pub tag: Option<String>,
    /// Page size, 1-200 (default 50).
    pub limit: Option<i64>,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DocumentsArgs {
    /// Project id.
    pub project: String,
    /// Substring match over title and folder.
    pub q: Option<String>,
    /// 1-200 (default 200).
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DocumentReadArgs {
    /// Document id (`doc-…`).
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DocumentProposeArgs {
    /// Document id (`doc-…`).
    pub id: String,
    /// The operations, as an array. Each is
    /// `{"op":"replace"|"insert_after"|"delete","id":"blk_…","markdown":"…"}`.
    /// `markdown` is omitted for `delete`.
    pub ops: serde_json::Value,
    /// What you were asked to do, in one line. Shown to the person deciding.
    pub instruction: Option<String>,
    /// What you changed and why, in one or two sentences. This is what a reviewer
    /// reads before the diff, so it should say the REASON, not restate the edit.
    pub summary: Option<String>,
    /// Restrict the run to these block ids. Enforced server-side: an op outside
    /// the list is dropped and reported back to you, not silently applied.
    pub scope: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlanReadArgs {
    /// Mindmap id (`mm-…`) — a project has one, and it IS the plan.
    pub id: String,
    /// One section (`mn-…`). Omit for the whole plan.
    pub node: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlanProposeArgs {
    /// Mindmap id (`mm-…`).
    pub id: String,
    /// The section (`mn-…`) this is about. Read it first: an operation names a
    /// block id, and the ids come from `takomo_plan_read`.
    pub node: String,
    /// The operations, as an array. Each is
    /// `{"op":"replace"|"insert_after"|"delete","id":"blk_…","markdown":"…"}`.
    /// `markdown` is omitted for `delete`.
    pub ops: serde_json::Value,
    /// What you were asked to do, in one line. Shown to the person deciding.
    pub instruction: Option<String>,
    /// What you changed and why, in one or two sentences. This is what a
    /// reviewer reads before the diff, so it should say the REASON.
    pub summary: Option<String>,
    /// Restrict to these block ids. Enforced server-side: an op outside the list
    /// is dropped and reported back, not silently applied.
    pub scope: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlanProposalsArgs {
    /// Mindmap id (`mm-…`).
    pub id: String,
    /// Only this section's.
    pub node: Option<String>,
    /// Only proposals in this state: `pending`, `accepted` or `rejected`.
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DocumentProposalsArgs {
    /// Document id (`doc-…`).
    pub id: String,
    /// Only proposals in this state: `pending`, `accepted` or `rejected`.
    /// Omit for all of them.
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InitiativeShowArgs {
    /// Initiative id (`ini-…`).
    pub id: String,
    /// How many entries to return, newest first: 1-200 (default 50). The rollup
    /// always describes the whole collection, not just this page.
    pub limit: Option<i64>,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Include this initiative's verification standing: how many of its checks'
    /// cases are verified, stale, failed or never run, and when one was last
    /// verified. Off by default because it costs a scan over the checks and
    /// cases beneath the initiative.
    pub verification: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReleasePushArgs {
    /// Project the release belongs to.
    pub project: String,
    /// The tag or FULL commit sha this release stands for. Short shas are
    /// ambiguous.
    pub r#ref: String,
    /// Optional note about the release.
    pub note: Option<String>,
    /// Paths the release's diff touched. You have the tree checked out; Takomo
    /// clones nothing. Any check claiming a glob that matches one of these has its
    /// cases marked stale.
    pub touched_paths: Option<Vec<String>>,
    /// Check globs that matched NO file in this tree. An orphaned glob reads as
    /// "still covered" while covering nothing, so report them and those checks stop
    /// counting toward coverage.
    pub orphan_globs: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChecklistProjectArgs {
    /// Project id.
    pub project: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EnvironmentsArgs {
    /// Project id.
    pub project: String,
    /// Narrow by kind: local, ephemeral, shared, staging, production, other.
    pub kind: Option<String>,
    /// Include archived environments, which are kept because past verdicts
    /// still reference them.
    pub archived: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EnvironmentFileArgs {
    /// Project the environment belongs to.
    pub project: String,
    /// Stable handle you will pass everywhere else, e.g. "staging". Filing the
    /// same slug twice updates that environment rather than creating a second.
    pub slug: String,
    /// Human-readable name. Defaults to the slug.
    pub name: Option<String>,
    /// local, ephemeral, shared, staging, production, other.
    pub kind: Option<String>,
    /// Where the application answers, e.g. "https://staging.example.com".
    pub base_url: Option<String>,
    /// How to get it running, in prose or as a command — Takomo never runs it,
    /// it hands it to whoever needs it next.
    pub bring_up: Option<String>,
    /// How to give it back when the run is over. The half nobody writes down.
    pub teardown: Option<String>,
    /// seeded, empty, production_like or unknown — what is in it, which decides
    /// whether a case's preconditions can be met at all.
    pub data_state: Option<String>,
    /// Whether a destructive case may run here. ADVISORY: Takomo executes
    /// nothing and cannot enforce it. Defaults to false for kind=production.
    pub writable: Option<bool>,
    /// WHERE a credential lives — "env:STAGING_TOKEN", a vault path, a runbook
    /// URL. Never the credential itself: every token with `read` can see this.
    pub credentials_hint: Option<String>,
    /// Caveats worth knowing before running: reset cadence, shared sandboxes,
    /// anything that would make a verdict untrustworthy.
    pub notes: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChecksArgs {
    /// Project id.
    pub project: String,
    /// Narrow to one epic's checks, or "none" for checks nobody grouped.
    pub epic: Option<String>,
    /// Narrow to one initiative's checks, or "none" for checks no initiative
    /// claims — the gap between what was agreed and what got written down.
    pub initiative: Option<String>,
    /// Narrow by severity: blocking, advisory, low.
    pub severity: Option<String>,
    /// Narrow by layer: ui, api, other.
    pub layer: Option<String>,
    /// How many to return, 1..=200 (default 200). `total` always reports how
    /// many matched.
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckShowArgs {
    /// Check id.
    pub id: String,
    /// Include the check's cases in the response.
    pub cases: Option<bool>,
    /// With `cases`: how many to return, 1..=500 (default 500). `case_total`
    /// always reports how many the check holds.
    pub limit: Option<i64>,
    /// With `cases`: skip this many, for reading past the first page.
    pub offset: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckFileArgs {
    /// Project the check belongs to.
    pub project: String,
    /// The one action this check verifies, e.g. "Create a claim".
    pub title: String,
    /// Epic ticket id to group under. Omit to leave it ungrouped.
    pub epic: Option<String>,
    /// The initiative whose conversation agreed this check should exist. This is
    /// how a characterisation test you settled on while discussing a feature
    /// stays attached to that discussion — file it even before an epic exists.
    pub initiative: Option<String>,
    /// Environments this check must be verified in, by slug or id. Each case is
    /// then tracked per environment, so "passes on staging, never run on
    /// production" is expressible instead of collapsing into one verdict. Omit
    /// for a check whose result does not depend on where it runs.
    pub environments: Option<Vec<String>>,
    /// Free-form traversal an agent or a human follows. No step model, no DAG —
    /// prose is the content.
    pub body: Option<String>,
    /// The data state and permissions needed before this check can start.
    pub precondition: Option<String>,
    /// Which layer this check exercises: ui, api, other. A rule enforced only in
    /// the interface passes at the API layer, so the two are NOT interchangeable —
    /// one check covers one layer.
    pub layer: Option<String>,
    /// blocking, advisory or low. Only blocking severity blocks a release gate.
    pub severity: Option<String>,
    /// Override the inherited verification level: agent, human, agent_then_human.
    pub verification: Option<String>,
    /// Override the inherited time-based expiry, in days.
    pub expiry_days: Option<i64>,
    /// Override the inherited release-count expiry.
    pub expiry_releases: Option<i64>,
    /// Rough agent cost for one case, in minutes.
    pub cost_agent_minutes: Option<i64>,
    /// Rough human cost for one case, in minutes.
    pub cost_human_minutes: Option<i64>,
    /// Paths of the application under test this check claims to exercise, e.g.
    /// ["src/claims/**"].
    pub globs: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CaseFileArgs {
    /// Check id the cases belong to.
    pub check: String,
    /// The generated case set. Each entry needs a `key` derived from its parameter
    /// assignment so regeneration matches existing cases and keeps their history.
    pub cases: Vec<CaseArg>,
    /// Retire live cases the set no longer contains (default true). False extends
    /// the set instead of replacing it.
    pub prune: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CaseArg {
    /// Stable identity derived from the parameter assignment. Same assignment ⇒
    /// same key ⇒ history survives regeneration.
    pub key: String,
    /// One line naming what makes this case different from its siblings.
    pub label: Option<String>,
    /// The parameter assignment: the setup a person or agent must reproduce.
    pub assignment: Option<serde_json::Value>,
    /// True for a hand-written happy path you seeded rather than generated.
    pub seeded: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VerdictArgs {
    /// Which environment you observed this in, by slug or id. Required when the
    /// check declares more than one — a bare verdict there does not say what you
    /// saw, and it is refused rather than guessed at. Omit for a check that
    /// declares one (that one is meant) or none.
    pub environment: Option<String>,
    /// Case id.
    pub case: String,
    /// pass, fail, blocked or unreachable. `unreachable` is NOT a failure — use it
    /// when the declared layer gives no way to reach this configuration. That is a
    /// finding worth reporting, and it is counted apart from covered and uncovered.
    pub verdict: String,
    /// What you observed. Required on a fail.
    pub note: Option<String>,
    /// Release id this verdict was taken against.
    pub release: Option<String>,
}

// ---- tools ------------------------------------------------------------------

#[tool_router]
impl TakomoMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        // Two routers, merged. The initiative tools sit in their own
        // `#[tool_router]` block because they are a genuinely separate
        // surface — no claims, no fences, no workflow — and keeping them
        // out of the work-loop block keeps each block readable.
        let tool_router = Self::tool_router()
            + Self::initiative_router()
            + Self::schedule_router()
            + Self::test_run_router();
        let tools = Arc::new(slim_tools(tool_router.list_all()));
        Self {
            state,
            tool_router,
            tools,
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
        start/transition/done/release calls resolve the fencing token automatically. \
        The lease expires (default 900s, max 3600) — keep it with `takomo_heartbeat`. \
        Claiming an EPIC reserves its whole subtree: nobody else can claim tickets under \
        it and the ready queue stops offering them. An epic claimed without ttl_seconds \
        never expires — it is held until you release it (expires_at comes back null); \
        check on one with `takomo_claim_status`."
    )]
    async fn takomo_claim(
        &self,
        Parameters(a): Parameters<ClaimArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_claim(&require_auth(&ctx)?, &a.id, a.ttl_seconds))
    }

    #[tool(
        description = "Inspect the claim on a ticket: holder, held-for, expiry (null = an \
        epic claim held until released) and — while held — what moved in its subtree since \
        the claim: tickets created, closed, in progress, blocked, and how long since the \
        last movement. How you judge whether an epic claim is live work or abandoned; an \
        abandoned one is an admin force-release away."
    )]
    async fn takomo_claim_status(
        &self,
        Parameters(a): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_claim_status(&require_auth(&ctx)?, &a.id))
    }

    #[tool(
        description = "Renew the lease you hold on a ticket, so long-running work does not \
        lose its claim. Call it before `expires_at` (every lease response carries one); each \
        beat sets a fresh lifetime from now. An already-expired lease cannot be revived — \
        that is the point of leases, so you get a teaching 409 and must re-claim. Renewing \
        emits no event: lease lifecycle stays observable through claimed/released/lease_expired."
    )]
    async fn takomo_heartbeat(
        &self,
        Parameters(a): Parameters<HeartbeatArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_heartbeat(&require_auth(&ctx)?, &a.id, a.fence, a.ttl_seconds))
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
        automatically. Attach the closing commit first (`takomo_link` with key='commit') so \
        the finished state carries proof; some projects enforce it via \
        `guard:has_link:commit` and will reject this call until it is set."
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
        description = "Attach, update, or delete a named link on a ticket (e.g. key='pr'). \
        Only the key you name is written: the ticket's other links are left alone, so two \
        agents attaching different keys never clobber each other. Pass value=null to delete \
        the key (deleting one that is not set is a no-op, not an error). Use key='commit' \
        with the FULL commit SHA (or its commit URL) for the work that closes the ticket — \
        that is the proof a later reader checks instead of trusting the status, and what \
        release/deploy answers are derived from. Short SHAs are ambiguous; don't use them."
    )]
    async fn takomo_link(
        &self,
        Parameters(a): Parameters<LinkArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_link(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Tag people or other entities onto a ticket (reference metadata only — \
        never changes ticket state, claims, or routing). `add`/`remove` take kind:handle refs \
        like 'person:ada' or 'component:billing'; an unknown handle is registered on the fly. \
        Filter with takomo_list's `tag`/`tag_kind`."
    )]
    async fn takomo_tag(
        &self,
        Parameters(a): Parameters<TagArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_tag(&require_auth(&ctx)?, a))
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

    #[tool(
        description = "Move tickets to another project, in bulk. Ticket ids never change \
        (nothing reads a project out of an id), so links and commit messages keep resolving. \
        `descendants` defaults to true: naming an epic moves its whole subtree; false moves only \
        the named tickets and leaves their children behind as orphans, since a parent and child \
        must share a project. A state the target workflow does not define lands on that \
        workflow's initial state — the response reports every such reset per ticket. Refused \
        while any ticket in the set is claimed."
    )]
    async fn takomo_move(
        &self,
        Parameters(a): Parameters<MoveArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_move(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Record that this ticket's work reached a named target/stage — \
        `target` is free-form (\"staging\", \"production\", \"published\", \"delivered\", …), so it \
        is not limited to software. Optional url/ref/note. Append-only history; the latest shows on \
        the board."
    )]
    async fn takomo_promote(
        &self,
        Parameters(a): Parameters<PromoteArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_promote(&require_auth(&ctx)?, a))
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
        transitions), plus its conventions and its lease policy — `claim_ttl_seconds` (what a \
        claim gets by default) and `max_claim_ttl_seconds` (the most you may ask for). Useful for \
        self-correcting illegal moves, and for deciding how often to heartbeat."
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
        `unparented` rollup over the non-epic tickets no epic owns. Every rollup also \
        splits its claimable work into `ready` (what the ready queue would hand out) and \
        `backlog` (claimable but blocked or already claimed), and carries \
        `awaiting_answer` — tickets holding an open question, which is an OVERLAY on the \
        state counts, not a separate bucket. `initiatives` is the same rollup per \
        INITIATIVE — the long-lived lane a feature is worked in, which never closes — over \
        every ticket tagged `initiative:<id>` and everything beneath it, with `epics` \
        naming the versions filed under that check; `uninitiated` covers work no check owns. \
        Check rollups MAY OVERLAP and must not be summed. Pass `epic` to report on ONE \
        epic's subtree; the project-wide `unparented`, `initiatives` and `uninitiated` \
        sections are then omitted rather than returned empty."
    )]
    async fn takomo_roadmap(
        &self,
        Parameters(a): Parameters<ProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_roadmap(&require_auth(&ctx)?, &a.project, a.epic.as_deref()))
    }

    #[tool(
        description = "Rank a project's open blockers by how much work each one releases. \
        For every non-terminal ticket blocking something in the project, `unblocks` is how \
        many tickets would leave the blocked set if that ONE ticket closed, split into \
        `direct` (they hold the blocked_by edge) and `downstream` (they inherit it from an \
        ancestor). Counterfactual, not reachability: a ticket held by two blockers counts \
        towards neither alone, and closing a blocker does not release what its own \
        dependents block. Use it to pick the single close that buys the most. Pass `epic` \
        to count only work inside that epic's subtree — blockers OUTSIDE it are still \
        reported, since an external ticket holding the epic up is the thing worth naming."
    )]
    async fn takomo_impact(
        &self,
        Parameters(a): Parameters<ProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_impact(&require_auth(&ctx)?, &a.project, a.epic.as_deref()))
    }

    #[tool(
        description = "Ask a human for a decision when you are blocked (confirmation, a choice, \
        a clarification, or approval). Parks the ticket in a blocked state and releases your \
        lease: end your run and resume the ticket after a human answers. Route to a domain \
        expert with `expertise` tags like [\"domain:billing\"]. Write a decision-ready question: \
        for kind=choose give each option a one-line trade-off via `option_notes` (parallel to \
        `options`); set `recommended` to your suggested answer with a short `recommended_note` \
        (why) and `confidence` 1-4; add a one-line `summary` for the list preview. The ask \
        response returns `hints` naming anything that would make the inbox render richer. Phrase \
        the question (and options) in the project's expected human-facing language when one is \
        set, and follow its style guide — see the `language_hint` and `style_hint` on \
        takomo_show/next/start or takomo_workflow's `question_language` / `style_guide`."
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
        performs the ticket's human-gated transition to resume it. To write down a decision a human \
        already made elsewhere, pass `on_behalf_of` with their name instead — that needs the \
        `answer:relay` scope, records them as the decider and you as the relayer, and is refused for \
        a question you asked yourself or for any `approve`."
    )]
    async fn takomo_answer(
        &self,
        Parameters(a): Parameters<AnswerArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_answer(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Reopen an answered question — take back a decision (a conditional undo \
        beyond the inbox's 30s window). Requires the human scope. Refused with a teaching 409 if \
        the ticket already relies on the answer (claimed, moved on, or archived); re-park the \
        ticket and re-ask instead in that case."
    )]
    async fn takomo_reopen(
        &self,
        Parameters(a): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_reopen(&require_auth(&ctx)?, &a.id))
    }

    #[tool(
        description = "List open questions on the ask-a-human board (the inbox). Filter by \
        project/ticket/status, by `assignee` (a person's handle, or \"none\" for the ones nobody has \
        been asked yet), or `mine` for everything waiting on you — addressed to you by name, or \
        covered by your expert:<tag> scopes."
    )]
    async fn takomo_questions(
        &self,
        Parameters(a): Parameters<QuestionsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_questions(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Who the people are: the directory of humans a question can be addressed to. \
        Filter by `project` for the ones who are members of it (only they can be assigned work \
        there), or `q` to search handle, name and email. Read this before setting `assignee` on \
        takomo_ask or takomo_assign — a handle that is not in the directory is refused, because a \
        person is never created implicitly."
    )]
    async fn takomo_users(
        &self,
        Parameters(a): Parameters<UsersArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_users(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Address an open question to one person, or pass `assignee: null` to return it \
        to the open queue. Use it when you learn who owns a decision after asking — the usual case, \
        since the agent raising a question rarely knows who should take it. Assignment is routing: \
        any human can still answer the ordinary kinds, so nothing waits on someone who is away. It \
        needs the human scope, and the person must be a member of the ticket's project."
    )]
    async fn takomo_assign(
        &self,
        Parameters(a): Parameters<AssignArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_assign(&require_auth(&ctx)?, a))
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
        description = "Reply to a question a human bounced back for more research (its `awaiting` is \
        \"agent\", visible on takomo_show / takomo_questions). Post the context they asked for; this \
        flips the thread back to the human so they can answer. The ticket stays parked meanwhile."
    )]
    async fn takomo_reply(
        &self,
        Parameters(a): Parameters<ReplyArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_reply(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Revise a still-open 'choose' question's options. Use this when research (often \
        the follow-up a human asked for) shows the choices you offered were wrong, incomplete, or \
        misleading — better than withdrawing the question, which throws the whole thread away. Send \
        the FULL replacement set (at least 2); it does not merge. `recommended` must be one of the \
        revised options, so pass a new one whenever you drop the option you had recommended, or null \
        to clear it. Give a `reason`: a human may already have read the old set. Options can only be \
        revised while the question is open — a settled question keeps the choices it was decided on."
    )]
    async fn takomo_options(
        &self,
        Parameters(a): Parameters<ReviseOptionsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_revise_options(&require_auth(&ctx)?, a))
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
        description = "List the environments a check can be run against: base URL, how to bring \
        each one up and give it back, what data is in it, and whether writing to it is safe. Read \
        this BEFORE running a check — it is where the URL and the credential pointer live, so you \
        do not have to be told them out of band. `writable` and `credentials_hint` are advisory: \
        Takomo runs nothing and stores no secrets, only a pointer to where one lives."
    )]
    async fn takomo_environments(
        &self,
        Parameters(a): Parameters<EnvironmentsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_environments(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Register an environment, or update the one already holding that slug. Use \
        it when you stand up an instance others will verify against — an ephemeral preview, a \
        seeded local box — so the next runner is not told the URL out of band. Filing the same \
        slug twice updates in place, so this is safe to call every run. Put a POINTER in \
        `credentials_hint` (\"env:STAGING_TOKEN\", a vault path), never a credential: any token \
        with `read` can see it."
    )]
    async fn takomo_environment_file(
        &self,
        Parameters(a): Parameters<EnvironmentFileArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_environment_file(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Record a release you just merged, and learn what it invalidated. Send the \
        tag or FULL sha as `ref`, the paths the diff touched, and any check globs that matched NO \
        file in the tree. Every check claiming a touched path has its cases marked stale; globs that \
        matched nothing are flagged so those checks stop counting as covered. There is no direct \
        integration by design — the agent that merged the work is what tells Takomo a release \
        happened."
    )]
    async fn takomo_release_push(
        &self,
        Parameters(a): Parameters<ReleasePushArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_release_push(&require_auth(&ctx)?, a))
    }

    #[tool(description = "List a project's releases, newest first, with their sequence numbers.")]
    async fn takomo_releases(
        &self,
        Parameters(a): Parameters<ChecklistProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_releases(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "List a project's checklist checks with their case counts, resolved policy \
        and any orphaned globs. A check is one action with one entry precondition at one layer."
    )]
    async fn takomo_checks(
        &self,
        Parameters(a): Parameters<ChecksArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_checks(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Show one check: its traversal body, precondition, claimed globs, resolved \
        policy and case counts. Pass cases=true to include every case with its verdicts."
    )]
    async fn takomo_check(
        &self,
        Parameters(a): Parameters<CheckShowArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_check(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Declare a checklist check. Draw its boundary at a state transition, not a \
        screen: if something needs a persisted record, has its own permission gate, or is only \
        reachable from another check's terminal state, it is a SEPARATE check. Takomo stores what you \
        file and does not judge whether the model is right."
    )]
    async fn takomo_check_file(
        &self,
        Parameters(a): Parameters<CheckFileArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_check_file(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "File the generated case set for a check. Upsert is by `key`, so derive each \
        key from its parameter assignment: a case still present keeps its verdict history, one that \
        vanished is retired rather than deleted, one that returns is revived. A large real form \
        yields around 76 pairwise cases — if you have thousands, most of your parameters are \
        probably inert fields that do not belong in the model."
    )]
    async fn takomo_cases_file(
        &self,
        Parameters(a): Parameters<CaseFileArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_cases_file(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Legacy verdict API; prefer takomo_test_run_create and takomo_test_result for revision-pinned evidence. Record your verdict on a case: pass, fail, blocked or unreachable. A fail \
        needs a note. This records the AGENT verdict; only a human-scoped token can assert that a \
        person approved a case, so a policy of agent_then_human needs both facts and you cannot \
        supply the second one."
    )]
    async fn takomo_verdict(
        &self,
        Parameters(a): Parameters<VerdictArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_verdict(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "What must be re-verified in this project, split into what you can clear and \
        what needs a human. Human time is the scarce resource, so the split is the point. Reasons \
        are stale (the release diff touched the claimed code), expired (a policy clock ran out), \
        never, failed or awaiting_human."
    )]
    async fn takomo_worklist(
        &self,
        Parameters(a): Parameters<ChecklistProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_worklist(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "Checklist coverage for a project, rolled up per epic. Counts unreachable \
        apart from both covered and uncovered on purpose: calling it a gap reports work nobody can \
        do, calling it covered claims verification of code no path reaches. This measures coverage \
        of the DECLARED surface — hand-written globs — not measured execution."
    )]
    async fn takomo_coverage(
        &self,
        Parameters(a): Parameters<ChecklistProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_coverage(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "Is this project's verification good enough to ship? Only blocking-severity \
        checks block; advisory and low ones nag, because a gate that fires on everything gets \
        overridden out of habit and stops meaning anything."
    )]
    async fn takomo_gate(
        &self,
        Parameters(a): Parameters<ChecklistProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_gate(&require_auth(&ctx)?, &a.project))
    }

    #[tool(
        description = "Start a mindmap: a tree you grow at conversation speed, BEFORE any of it is \
        an idea or work. Use it when somebody is thinking out loud — a project idea fanning out into \
        API, integrations, workflows — and the shape is not settled yet. The title is the root; \
        everything hangs off it. Grow it with takomo_mindmap_grow, then promote the branches worth \
        keeping. A mindmap is scratch by design and deleting one is ordinary, which is what makes it \
        safe to start one early."
    )]
    async fn takomo_mindmap_new(
        &self,
        Parameters(a): Parameters<MindmapNewArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_mindmap_new(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Add thoughts to a mindmap — a WHOLE BRANCH in one call, which is the point: \
        while somebody talks you capture ten nodes at once rather than one per turn. Each node is a \
        sentence or two (280 chars); if a thought needs more it wants to be an initiative. Give \
        `parent` to hang a node under another, or leave it out for a branch off the root. The batch \
        lands whole or not at all, so a reader never sees half a thought."
    )]
    async fn takomo_mindmap_grow(
        &self,
        Parameters(a): Parameters<MindmapGrowArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_mindmap_grow(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "Read a mindmap as indented text — the cheapest shape to reason about and the \
        one to read before adding to a map you did not build. Pass `node` to read a single branch. \
        Returns the node ids alongside, so you can hang new thoughts in the right place."
    )]
    async fn takomo_mindmap_show(
        &self,
        Parameters(a): Parameters<MindmapShowArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_mindmap_show(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "List mindmaps in a project, newest-touched first, with their node counts."
    )]
    async fn takomo_mindmap_list(
        &self,
        Parameters(a): Parameters<MindmapListArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_mindmap_list(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Graduate a branch that turned out to matter. `target: \"epic\"` makes an epic \
        with this node's direct children as tickets under it — the fastest path from talking to work \
        in the queue. `target: \"initiative\"` makes an initiative seeded with the whole subtree, for \
        a direction that needs nurturing before it is work. The node STAYS on the map either way and \
        keeps a link to what it became, so the map goes on being a picture of how the thinking got \
        there. Promoting the same branch twice is refused."
    )]
    async fn takomo_mindmap_promote(
        &self,
        Parameters(a): Parameters<MindmapPromoteArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_mindmap_promote(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "Identify the caller behind the current token: actor, scopes, project \
        access, and the person this credential belongs to (`user`, null for a machine token). That \
        person is who `mine` means on takomo_questions."
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
        // A bound person who no longer resolves reports null rather than failing:
        // whoami is what a client boots on.
        let user = match &auth.user {
            None => Value::Null,
            Some(id) => match self.state.store.get_user(id) {
                Ok(Some(person)) => person.to_ref_json(),
                _ => Value::Null,
            },
        };
        respond(Ok(json!({
            "ok": true,
            "whoami": {
                "token_id": auth.token_id,
                "actor": auth.actor,
                "scopes": scopes,
                "projects": projects,
                "user": user,
            }
        })))
    }
}

// ---- schedule tools ---------------------------------------------------------
//
// Read is free; creating is a `write`, and what it creates is INERT. Unless the
// project turned the flag off, a schedule proposed here lands `pending` with no
// next slot, so the sweep cannot see it and nothing fires until a human
// activates it. That is deliberate: a schedule outlives the token that made it,
// so letting an agent start one would be a write credential that keeps writing
// after it is revoked.

#[tool_router(router = schedule_router)]
impl TakomoMcp {
    #[tool(
        description = "List a project's schedules: recurrence rules that materialize ordinary \
        tickets. Each carries its cadence, status, next slot and recent occurrence history with \
        every outcome (done | open | not_fulfilled) derived from the ticket. Use it to check \
        whether recurring work is actually getting done before proposing more of it."
    )]
    async fn takomo_schedules(
        &self,
        Parameters(a): Parameters<SchedulesArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_schedules(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Propose a schedule — a cadence that creates one ordinary ticket per slot. \
        Use it when you notice the same work coming back on a rhythm (a weekly review, a monthly \
        key rotation) instead of filing the same ticket by hand again. IMPORTANT: unless the \
        project has turned approval off, this lands as `pending` and fires NOTHING until a human \
        activates it — do not wait on it, finish your ticket. Give a `rationale` naming the \
        tickets that already repeated; that is what a reviewer judges."
    )]
    async fn takomo_schedule_new(
        &self,
        Parameters(a): Parameters<ScheduleNewArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_schedule_new(&require_auth(&ctx)?, a))
    }
}

impl TakomoMcp {
    fn do_schedules(&self, auth: &AuthCtx, a: SchedulesArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        if let Some(st) = &a.status {
            if !crate::store::SCHEDULE_STATUSES.contains(&st.as_str()) {
                return Err(ApiError::validation(
                    "validation.schedule.status",
                    format!(
                        "status must be one of {}, got '{st}'.",
                        crate::store::SCHEDULE_STATUSES.join(", ")
                    ),
                ));
            }
        }
        let filter = crate::store::ScheduleListFilter {
            project: a.project,
            status: a.status,
            allowed_projects: auth.allowed_projects_vec(),
        };
        let rows = self
            .state
            .store
            .list_schedules(&filter, crate::store::MAX_SCHEDULES_PAGE)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|sched| {
                let occ = self
                    .state
                    .store
                    .schedule_occurrences(&sched.id, 8)
                    .unwrap_or_default();
                let mut out = sched.to_json(&self.state.store.upcoming_slots(sched, 1));
                out["occurrences"] = json!(occ.iter().map(|o| o.to_json()).collect::<Vec<_>>());
                out
            })
            .collect();
        Ok(json!({ "ok": true, "schedules": items }))
    }

    fn do_schedule_new(&self, auth: &AuthCtx, a: ScheduleNewArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;

        // Build the cadence as JSON and hand it to the ONE parser, so an MCP
        // caller gets exactly the teaching errors a REST caller does — including
        // the refusal of a weekday list on a daily cadence.
        let mut cadence = serde_json::Map::new();
        cadence.insert("every".into(), json!(a.every));
        cadence.insert("at".into(), json!(a.at));
        if let Some(i) = a.interval {
            cadence.insert("interval".into(), json!(i));
        }
        if let Some(on) = a.on {
            cadence.insert("on".into(), json!(on));
        }
        if let Some(d) = a.day {
            cadence.insert("day".into(), json!(d));
        }
        if let Some(tz) = a.tz {
            cadence.insert("tz".into(), json!(tz));
        }
        let cadence = crate::schedule::Cadence::parse(&Value::Object(cadence)).map_err(|m| {
            ApiError::validation("validation.schedule.cadence", m)
                .remedy("See spec/schedule-format.md for the cadence grammar.")
        })?;

        let mut template = serde_json::Map::new();
        template.insert("title".into(), json!(a.title));
        if let Some(b) = a.body {
            template.insert("body".into(), json!(b));
        }
        if let Some(l) = a.labels {
            template.insert("labels".into(), json!(l));
        }
        let template = crate::store::ScheduleTemplate::parse(&Value::Object(template))
            .map_err(|m| ApiError::validation("validation.schedule.template", m))?;

        let req = crate::store::ScheduleCreate {
            project: a.project.clone(),
            name: a.name,
            cadence,
            template,
            starts_at: None,
            ends_at: None,
            rationale: a.rationale,
        };
        // A `human` caller's own schedule is born active; the flag governs what an
        // agent proposes.
        let needs_approval = !auth.scopes.contains("human")
            && self.state.store.schedule_approval_required(&a.project)?;
        let sched = self
            .state
            .store
            .create_schedule(&req, &auth.actor, needs_approval)?;
        self.state.wake();

        let note = if sched.status == "pending" {
            "Recorded, but NOT active: this project requires a human to activate a schedule an \
             agent proposed, so nothing will fire yet. Do NOT wait on it or poll for it — finish \
             your ticket. `upcoming` shows the slots it would use once activated."
                .to_string()
        } else {
            format!(
                "Active. The next occurrence lands at {}, and each one is an ordinary ticket you \
                 can claim from the ready queue like any other.",
                sched.next_slot.map(crate::ids::iso).unwrap_or_default()
            )
        };
        Ok(json!({
            "ok": true,
            "schedule": sched.to_json(&self.state.store.upcoming_slots(&sched, 3)),
            "note": note,
        }))
    }
}

// ---- initiative tools -------------------------------------------------------

#[tool_router(router = initiative_router)]
impl TakomoMcp {
    #[tool(
        description = "Create an initiative: a durable home for an idea that is not yet work — a \
        product idea, a direction, what came out of a good conversation. Unlike a ticket it has \
        no workflow and is never claimed; it is nurtured by appending research inputs over time \
        with takomo_initiative_append."
    )]
    async fn takomo_initiative_new(
        &self,
        Parameters(a): Parameters<InitiativeNewArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_initiative_new(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Append one contribution to an initiative: a note, a research finding, a \
        colleague's feedback, a transcript, or an attached document (base64, up to 5 MiB). \
        Append-only — the accumulated record IS the initiative. `source` is required: it records \
        where the input came from, so a later reader can weigh it.\n\n\
        WHEN YOU FINISH RESEARCH, DO NOT STOP AT THE FINDING. A pile of findings is what nobody \
        reads. Append the finding as evidence, then say what it MEANS by writing the initiative's \
        document — /initiatives renders three panes from reserved kinds, and every one of them is \
        just an entry with `meta`:\n\
        • kind 'view', meta { pane: 'business'|'technical'|'verification', cites: [entryId, …] } — \
        one pane's prose. A `[n]` mark in the text cites the n-th id in YOUR cites array (1-based), \
        so you only need local numbering. Cite every assertion you can: an uncited paragraph is \
        rendered as flagged opinion, which is what it is.\n\
        • the same, plus meta.proposed = true — an AMENDMENT of the WHOLE pane. Use it when a \
        finding changes the shape of the argument. It is shown to a human as a diff to accept or \
        reject; it never silently replaces the live pane. Only the newest undecided one per pane \
        is offered, so this is a take-it-or-leave-it rewrite.\n\
        • kind 'view', meta { pane, cites: [], proposed: true, quote, prefix, suffix, para } with \
        the `text` being just the replacement words — a SUGGESTION scoped to one passage, the same \
        anchor shape as a thread. Prefer this when you want to change a sentence rather than the \
        argument: several can be pending at once without colliding, and accepting one splices it \
        into the live prose and renumbers the citations.\n\
        • kind 'thread', meta { pane, quote, prefix, suffix, para } — a note anchored to the WORDS \
        it is about. `quote` is the exact passage from the live pane; `prefix` and `suffix` are up \
        to 32 characters either side of it, which is what disambiguates a sentence that appears \
        twice. Use it for a doubt, a question, or something only a human can answer, next to the \
        words that provoked it. Anchor to words rather than to a paragraph number: the pane WILL be \
        revised, and a note carrying only `para` slides onto a paragraph it was never about. When \
        the quoted words later disappear the note is shown as orphaned rather than silently \
        re-pointed, which is the honest outcome. Keep `para` too, as a fallback hint.\n\
        • meta.origin = true on any entry — the words the idea ARRIVED in (a customer quote, the \
        original request). Quoted above every pane.\n\n\
        Revise by appending, never by rewriting: a new 'view' supersedes the old one for its pane \
        and the earlier wording stays readable. Call takomo_initiative_show first to read the \
        current panes and the entry ids you intend to cite."
    )]
    async fn takomo_initiative_append(
        &self,
        Parameters(a): Parameters<InitiativeAppendArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_initiative_append(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Update an initiative's own description: title, summary, status \
        (open|parked|distilled), labels, tags, metadata. Entries are append-only and are not \
        editable from here."
    )]
    async fn takomo_initiative_update(
        &self,
        Parameters(a): Parameters<InitiativeUpdateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_initiative_update(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "List project-defined lanes for related tickets and retained agent context."
    )]
    async fn takomo_lanes(
        &self,
        Parameters(a): Parameters<WorkLanesArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            auth.require_project(&a.project)?;
            let (items, total) = self.state.store.work_lane_list(
                &a.project,
                a.limit.unwrap_or(50).clamp(1, 200),
                a.offset.unwrap_or(0).max(0),
            )?;
            Ok(crate::api::paged(
                items,
                total,
                a.limit.unwrap_or(50).clamp(1, 200),
                "Continue with offset=N and limit=N (maximum 200).",
            ))
        })())
    }
    #[tool(
        description = "Read a lane's durable context and ticket membership before preparing work."
    )]
    async fn takomo_lane_show(
        &self,
        Parameters(a): Parameters<WorkLaneIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            let lane = self.state.store.work_lane_get(&a.id)?;
            auth.require_project(lane["project"].as_str().unwrap_or_default())?;
            Ok(lane)
        })())
    }
    #[tool(
        description = "Create a lane to collect related work. Does not dispatch or execute an agent."
    )]
    async fn takomo_lane_create(
        &self,
        Parameters(a): Parameters<WorkLaneCreateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            auth.require_project(&a.project)?;
            let lane = self.state.store.work_lane_create(
                &a.project,
                &a.title,
                a.purpose.as_deref().unwrap_or(""),
                a.context.as_deref().unwrap_or(""),
                &auth.actor,
            )?;
            self.state.wake();
            Ok(lane)
        })())
    }
    #[tool(
        description = "Update lane title, purpose or context. Read first and preserve prior decisions. Does not execute work."
    )]
    async fn takomo_lane_update(
        &self,
        Parameters(a): Parameters<WorkLaneUpdateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let lane = self.state.store.work_lane_get(&a.id)?;
            auth.require_project(lane["project"].as_str().unwrap_or_default())?;
            let mut body = serde_json::to_value(&a).expect("lane fields serialize");
            body.as_object_mut().unwrap().remove("id");
            let out = self
                .state
                .store
                .work_lane_patch(&a.id, &body, &auth.actor)?;
            self.state.wake();
            Ok(out)
        })())
    }
    #[tool(
        description = "Add or remove a same-project ticket from a lane. Existing handoff snapshots stay fixed."
    )]
    async fn takomo_lane_ticket(
        &self,
        Parameters(a): Parameters<WorkLaneTicketArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let lane = self.state.store.work_lane_get(&a.lane)?;
            auth.require_project(lane["project"].as_str().unwrap_or_default())?;
            let out = self.state.store.work_lane_ticket(
                &a.lane,
                &a.ticket,
                a.remove.unwrap_or(false),
                &auth.actor,
            )?;
            self.state.wake();
            Ok(out)
        })())
    }
    #[tool(
        description = "Draft an immutable preparation, implementation or review handoff for codex or claude. Does NOT dispatch work: a human must explicitly send it. Reviews need parent_handoff and exact target_revision."
    )]
    async fn takomo_lane_handoff(
        &self,
        Parameters(a): Parameters<WorkLaneHandoffArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let lane = self.state.store.work_lane_get(&a.lane)?;
            auth.require_project(lane["project"].as_str().unwrap_or_default())?;
            let mut body = serde_json::to_value(&a).expect("handoff fields serialize");
            body.as_object_mut().unwrap().remove("lane");
            let out = self
                .state
                .store
                .work_handoff_create(&a.lane, &body, &auth.actor)?;
            self.state.wake();
            Ok(out)
        })())
    }
    #[tool(
        description = "Read handoff results and revision-specific review findings returned to a lane."
    )]
    async fn takomo_lane_handoffs(
        &self,
        Parameters(a): Parameters<WorkLaneHandoffsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            let lane = self.state.store.work_lane_get(&a.lane)?;
            let project = lane["project"].as_str().unwrap_or_default();
            auth.require_project(project)?;
            let (items, total) = self.state.store.work_handoff_list(
                project,
                Some(&a.lane),
                None,
                a.limit.unwrap_or(50).clamp(1, 200),
                a.offset.unwrap_or(0).max(0),
            )?;
            Ok(crate::api::paged(
                items,
                total,
                a.limit.unwrap_or(50).clamp(1, 200),
                "Continue with offset=N and limit=N (maximum 200).",
            ))
        })())
    }

    #[tool(
        description = "List initiatives with optional filters. Each carries a rollup of what has \
        accumulated on it — entries, attachments, characters, bytes/megabytes — plus what is \
        WAITING on a person: `open_notes` is unanswered notes in the document, \
        `pending_amendments` is proposed wording nobody has accepted or rejected. Read those two \
        before adding more: an initiative with an undecided amendment wants a decision, not another \
        finding appended underneath it."
    )]
    async fn takomo_initiative_list(
        &self,
        Parameters(a): Parameters<InitiativeListArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_initiative_list(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Fetch one initiative with its rollup and a page of entries, newest first. \
        Entry text is included; attachment bytes are not — fetch those from \
        GET /v1/initiatives/{id}/entries/{entry}/content. Pass verification=true to also get the \
        standing of the checks filed under this initiative: how many of their cases are verified, \
        stale, failed or never run, and when one was last verified."
    )]
    async fn takomo_initiative_show(
        &self,
        Parameters(a): Parameters<InitiativeShowArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_initiative_show(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "List collaborative documents in a project. A document is prose people and \
        agents write AT THE SAME TIME — unlike an initiative, which is an append-only log of \
        entries. Returns each document's id, title, folder and how much edit history it holds. \
        The text itself is not here: read it with takomo_document_read."
    )]
    async fn takomo_documents(
        &self,
        Parameters(a): Parameters<DocumentsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_documents(&require_auth(&ctx)?, a))
    }

    #[tool(
        description = "Read a document as markdown annotated with BLOCK IDS:\n\n\
        <!-- blk_7f3a -->\n## Pricing\nOur current tiers are…\n\n\
        Those ids are how you change it. Read this before proposing anything — the ids move as \
        people edit, and an op naming a block that is gone is dropped. Also returns how many \
        proposals are already waiting on a person: if something is pending, prefer improving that \
        over stacking another one underneath it."
    )]
    async fn takomo_document_read(
        &self,
        Parameters(a): Parameters<DocumentReadArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_document_read(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "PROPOSE a change to a document. Nothing you send here becomes live text — \
        it is offered as a suggestion a person accepts or rejects, which is the rule this whole \
        surface exists to keep.\n\n\
        Send OPERATIONS AGAINST BLOCK IDS, never a rewritten document:\n\
        [{\"op\":\"replace\",\"id\":\"blk_7f3a\",\"markdown\":\"## Pricing\\n…\"},\n\
         {\"op\":\"insert_after\",\"id\":\"blk_7f3a\",\"markdown\":\"…\"},\n\
         {\"op\":\"delete\",\"id\":\"blk_9c1e\"}]\n\n\
        Blocks you do not name are never touched, which is what lets somebody keep typing three \
        paragraphs away while you work. Returning a whole document instead would throw their \
        words away.\n\n\
        Write a `summary` saying WHY, not what — the diff already shows what. Ops targeting a \
        block that has since disappeared, or one outside `scope`, are dropped and reported back \
        in `skipped` rather than failing the call."
    )]
    async fn takomo_document_propose(
        &self,
        Parameters(a): Parameters<DocumentProposeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_document_propose(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "List a document's proposals and what became of them: `pending` is waiting \
        on a person, `accepted` was applied, `rejected` was turned down. Check this after \
        proposing — a rejected proposal is a signal about the document you were wrong about, not \
        a reason to send the same thing again."
    )]
    async fn takomo_document_proposals(
        &self,
        Parameters(a): Parameters<DocumentProposalsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_document_proposals(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "Read the PLAN — a project's one living document, which is the same thing \
        the mindmap draws. Sections come back as markdown annotated with block ids, because that \
        is what makes a reply addressable: you answer with operations against ids, never with a \
        document. Pass `node` for one section, or omit it to read the whole plan with its \
        headings. Read before you propose."
    )]
    async fn takomo_plan_read(
        &self,
        Parameters(a): Parameters<PlanReadArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_plan_read(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "Propose a change to one section of the plan. NOTHING GOES LIVE: your \
        operations are checked against the section as it stands and left for a person to accept \
        or reject. Address block ids — replace, insert_after, delete — and never send a whole \
        document, which is what keeps somebody's concurrent typing. An op naming a block that is \
        gone comes back in `skipped` rather than being silently applied, so read that."
    )]
    async fn takomo_plan_propose(
        &self,
        Parameters(a): Parameters<PlanProposeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_plan_propose(&require_auth(&ctx)?, a).await)
    }

    #[tool(
        description = "List the plan's proposals and what became of them: `pending` waits on a \
        person, `accepted` was applied, `rejected` was turned down. A rejected proposal is a \
        signal about the plan you were wrong about, not a reason to send the same thing again."
    )]
    async fn takomo_plan_proposals(
        &self,
        Parameters(a): Parameters<PlanProposalsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        respond(self.do_plan_proposals(&require_auth(&ctx)?, a).await)
    }
}

// ---- tool implementations (call the internal store directly) ----------------

impl TakomoMcp {
    fn do_documents(&self, auth: &AuthCtx, a: DocumentsArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(&a.project)?;
        let filter = crate::store::DocumentFilter {
            project: a.project.clone(),
            q: a.q.clone(),
            limit: a.limit,
            ..Default::default()
        };
        let (docs, total) = self.state.store.list_documents(&filter)?;
        Ok(json!({
            "items": docs.iter().map(|d| d.to_json()).collect::<Vec<_>>(),
            "total": total,
        }))
    }

    /// Read the live replica, not the persisted log.
    ///
    /// It matters which: the log is up to one flush interval behind, so a read
    /// from it would hand an agent block ids that a person had already moved on
    /// from — and then every op it wrote would be dropped as stale. Opening the
    /// room puts the agent on the same replica the browsers are on.
    async fn do_document_read(&self, auth: &AuthCtx, a: DocumentReadArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let doc = self.state.store.get_document(&a.id)?;
        auth.require_project(&doc.project)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        let (markdown, blocks, pending) = room.read(|d| {
            let frag = d.get_or_insert_xml_fragment(crate::api::docprops::PROSE_FIELD);
            let txn = d.transact();
            let blocks = crate::api::docprops::read_blocks(&txn, &frag);
            drop(txn);
            let markdown = crate::api::docprops::annotate(&blocks);
            let n = blocks.len();
            let pending = crate::api::docprops::read_proposals(d)
                .iter()
                .filter(|p| p.get("status").and_then(Value::as_str) == Some("pending"))
                .count();
            (markdown, n, pending)
        });

        Ok(json!({
            "document": doc.to_json(),
            "default_writing_instruction": self.state.store.default_writing_instruction(&doc.project)?,
            "markdown": markdown,
            "blocks": blocks,
            "pending_proposals": pending,
            "note": "Address blocks by the id in the `<!-- blk_… -->` comment above them. \
                     Never send a whole document back.",
        }))
    }

    async fn do_document_propose(
        &self,
        auth: &AuthCtx,
        a: DocumentProposeArgs,
    ) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let doc = self.state.store.get_document(&a.id)?;
        auth.require_project(&doc.project)?;
        // A proposal is a write against the document, so an archived project
        // refuses it like every other write beneath one.
        self.state.store.ensure_collab_writable(&a.id)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        let instruction = a.instruction.clone().unwrap_or_default();
        let summary = a.summary.clone().unwrap_or_default();
        let scope = a.scope.clone();
        let actor = auth.actor.clone();
        let ops_raw = a.ops.clone();
        let now = now_ms();

        let (proposal, applied, skipped) = room.mutate(|d| {
            let frag = d.get_or_insert_xml_fragment(crate::api::docprops::PROSE_FIELD);
            let txn = d.transact();
            let blocks = crate::api::docprops::read_blocks(&txn, &frag);
            drop(txn);

            let validated = crate::api::docprops::validate_ops(
                &ops_raw,
                &blocks,
                scope.as_deref(),
                "takomo_document_read",
            )?;
            let id = crate::api::docprops::write_proposal(
                d,
                None,
                &actor,
                &instruction,
                &summary,
                &validated.ops,
                &validated.skipped,
                now,
            )?;
            Ok((id, validated.ops.len(), validated.skipped))
        })?;

        Ok(json!({
            "proposal": proposal,
            "document": a.id,
            "status": "pending",
            "operations": applied,
            "skipped": skipped,
            "note": "Offered, not applied. A person accepts or rejects it on /documents; \
                     poll takomo_document_proposals to see which.",
        }))
    }

    async fn do_document_proposals(
        &self,
        auth: &AuthCtx,
        a: DocumentProposalsArgs,
    ) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let doc = self.state.store.get_document(&a.id)?;
        auth.require_project(&doc.project)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        let mut items = room.read(crate::api::docprops::read_proposals);
        if let Some(want) = a.status.as_deref() {
            items.retain(|p| p.get("status").and_then(Value::as_str) == Some(want));
        }
        Ok(json!({ "items": items, "total": items.len() }))
    }

    fn do_mindmap_new(&self, auth: &AuthCtx, a: MindmapNewArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let req = crate::store::MindmapCreate {
            title: a.title,
            summary: a.summary,
            metadata: None,
        };
        let map = self
            .state
            .store
            .create_mindmap(&a.project, &req, &auth.actor)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "mindmap": map.to_json(),
            "default_writing_instruction": self.state.store.default_writing_instruction(&map.project)?,
            "note": format!(
                "Grow it with takomo_mindmap_grow {{ id: \"{}\", nodes: [{{ text: \"…\" }}, …] }} — a whole branch per call while the conversation is still going. Keep each node to a sentence or two; when a branch turns out to matter, takomo_mindmap_promote it to an epic or an initiative.",
                map.id
            ),
        }))
    }

    async fn do_plan_read(&self, auth: &AuthCtx, a: PlanReadArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        // Only for a caller that may WRITE — the same gate the REST twin
        // carries. `ensure_prose` creates a fragment per node and drops the
        // legacy field, so running it here made a `read` tool alter the shared
        // replica, broadcast that to every open canvas, and persist it. Worse
        // here than on REST: both these tools are in `READ_TOOLS`, so the write
        // was not even debited against the token's budget.
        if auth.require_scope("write").is_ok() {
            room.mutate(|doc| Ok(crate::store::mindmapdoc::ensure_prose(doc)))?;
        }

        let node = a.node.clone();
        let markdown = room.read(|doc| plan_markdown(doc, &a.id, node.as_deref()))?;
        Ok(json!({
            "ok": true,
            "mindmap": a.id,
            "node": a.node,
            "markdown": markdown,
            "default_writing_instruction": self.state.store.default_writing_instruction(&map.project)?,
            "note": "Answer with takomo_plan_propose against the block ids above. Nothing you \
                     send goes live until a person accepts it.",
        }))
    }

    async fn do_plan_propose(&self, auth: &AuthCtx, a: PlanProposeArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;
        self.state.store.ensure_collab_writable(&a.id)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        room.mutate(|doc| Ok(crate::store::mindmapdoc::ensure_prose(doc)))?;

        let instruction = a.instruction.clone().unwrap_or_default();
        let summary = a.summary.clone().unwrap_or_default();
        let why = summary.clone();
        let scope = a.scope.clone();
        let actor = auth.actor.clone();
        let ops_raw = a.ops.clone();
        let node = a.node.clone();
        let now = now_ms();

        let (proposal, applied, skipped) = room.mutate(move |doc| {
            let frag = crate::store::mindmapdoc::section_prose(doc, &node)?;
            let txn = yrs::Transact::transact(doc);
            let blocks = crate::api::docprops::read_blocks(&txn, &frag);
            drop(txn);
            let validated = crate::api::docprops::validate_ops(
                &ops_raw,
                &blocks,
                scope.as_deref(),
                "takomo_plan_read",
            )?;
            let id = crate::api::docprops::write_proposal(
                doc,
                Some(&node),
                &actor,
                &instruction,
                &why,
                &validated.ops,
                &validated.skipped,
                now,
            )?;
            Ok((id, validated.ops.len(), validated.skipped))
        })?;

        crate::api::docsync::flush(&self.state, &room, &auth.actor).await;
        self.state
            .store
            .record_trace(&crate::store::trace::Record {
                project: &map.project,
                mindmap: &a.id,
                node: Some(&a.node),
                kind: "proposed",
                actor: &auth.actor,
                user: auth.user.as_deref(),
                note: (!summary.is_empty()).then_some(summary.as_str()),
                // A proposal changes nothing yet.
                text: None,
            })?;
        self.state.wake();

        Ok(json!({
            "ok": true,
            "proposal": proposal,
            "mindmap": a.id,
            "node": a.node,
            "status": "pending",
            "operations": applied,
            "skipped": skipped,
            "note": "Offered, not applied. Poll takomo_plan_proposals to see what a person \
                     decided.",
        }))
    }

    async fn do_plan_proposals(&self, auth: &AuthCtx, a: PlanProposalsArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        let mut items = room.read(crate::api::docprops::read_proposals);
        if let Some(node) = &a.node {
            items.retain(|p| p.get("node").and_then(Value::as_str) == Some(node.as_str()));
        }
        if let Some(status) = &a.status {
            items.retain(|p| p.get("status").and_then(Value::as_str) == Some(status.as_str()));
        }
        Ok(json!({ "ok": true, "items": items, "total": items.len() }))
    }

    async fn do_mindmap_grow(&self, auth: &AuthCtx, a: MindmapGrowArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        // Scope is checked against the map's own project: an id alone never grants
        // access, the same rule initiative_append follows.
        let map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;
        self.state.store.ensure_collab_writable(&a.id)?;

        let adds: Vec<crate::store::mindmapdoc::NodeAdd> = a
            .nodes
            .into_iter()
            .map(|n| crate::store::mindmapdoc::NodeAdd {
                parent: n.parent,
                by_user: auth.user.clone(),
                title: n.text,
                // An agent's branch is marked as an agent's. Nothing renders it
                // yet, but a map that cannot say which thoughts a person had is
                // a map that can never grow a trust view.
                origin: Some("agent".to_string()),
                ..Default::default()
            })
            .collect();

        // The SAME replica the browsers are on, so a branch added while somebody
        // is looking at the map appears as it is written rather than on reload.
        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        room.mutate(|doc| Ok(crate::store::mindmapdoc::ensure_prose(doc)))?;
        let actor = auth.actor.clone();
        let created = room.mutate(|doc| crate::store::mindmapdoc::add_nodes(doc, &adds, &actor))?;
        // The same rule the REST writes follow: a tool call that answers "done"
        // has to have persisted, rather than trusting a debounce meant for
        // somebody typing.
        crate::api::docsync::flush(&self.state, &room, &auth.actor).await;

        let (all, _, _) = room.read(|doc| crate::store::mindmapdoc::snapshot(doc, &a.id));
        let nodes: Vec<Value> = created
            .iter()
            .filter_map(|(id, _)| {
                all.iter()
                    .find(|n| n["id"].as_str() == Some(id.as_str()))
                    .cloned()
            })
            .collect();

        self.state
            .store
            .note_mindmap_size(&a.id, all.len() as i64)?;
        self.state.store.note_mindmap_event(
            &a.id,
            crate::store::MindmapChange::Grown,
            json!({ "mindmap": a.id, "nodes": nodes.len() }),
            &auth.actor,
        )?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "nodes": nodes,
            "default_writing_instruction": self.state.store.default_writing_instruction(&map.project)?,
            "note": "Hang the next round under these by passing their ids as `parent`.",
        }))
    }

    async fn do_mindmap_show(&self, auth: &AuthCtx, a: MindmapShowArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let mut map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        // Only for a caller that may WRITE — the same gate the REST twin
        // carries. `ensure_prose` creates a fragment per node and drops the
        // legacy field, so running it here made a `read` tool alter the shared
        // replica, broadcast that to every open canvas, and persist it. Worse
        // here than on REST: both these tools are in `READ_TOOLS`, so the write
        // was not even debited against the token's budget.
        if auth.require_scope("write").is_ok() {
            room.mutate(|doc| Ok(crate::store::mindmapdoc::ensure_prose(doc)))?;
        }
        let (nodes, relationships, outline) = room.read(|doc| {
            let (nodes, relationships, raw) = crate::store::mindmapdoc::snapshot(doc, &a.id);
            let text = match a.node.as_deref() {
                Some(node) => crate::store::mindmapdoc::outline(&raw, node),
                None => crate::store::mindmapdoc::full_outline(&raw, &map.title),
            };
            (nodes, relationships, text)
        });
        map.nodes = nodes.len() as i64;

        Ok(json!({
            "ok": true,
            "mindmap": map.to_json(),
            "outline": outline,
            "default_writing_instruction": self.state.store.default_writing_instruction(&map.project)?,
            // The ids alongside the text, because reading a map is usually the step
            // before adding to it and every add needs a parent id.
            "nodes": nodes,
            "relationships": relationships,
        }))
    }

    fn do_mindmap_list(&self, auth: &AuthCtx, a: MindmapListArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(project) = &a.project {
            auth.require_project(project)?;
        }
        let limit = a
            .limit
            .unwrap_or(50)
            .clamp(1, crate::store::MAX_MINDMAPS_PAGE);
        let filter = crate::store::MindmapListFilter {
            project: a.project,
            allowed_projects: auth.allowed_projects_vec(),
            status: a.status,
            q: a.q,
            limit,
            offset: 0,
        };
        let (maps, total) = self.state.store.list_mindmaps(&filter)?;
        Ok(json!({
            "ok": true,
            "items": maps.iter().map(|m| m.to_json()).collect::<Vec<_>>(),
            "total": total,
            "limit": limit,
        }))
    }

    async fn do_mindmap_promote(&self, auth: &AuthCtx, a: MindmapPromoteArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let map = self
            .state
            .store
            .get_mindmap(&a.id)?
            .ok_or_else(|| ApiError::not_found("mindmap", &a.id))?;
        auth.require_project(&map.project)?;
        crate::store::validate_promotion_target(&a.target)?;
        self.state.store.ensure_collab_writable(&a.id)?;

        let room = crate::api::docsync::open_room(&self.state, &a.id).await?;
        let state = self.state.clone();
        let actor = auth.actor.clone();
        let map_id = a.id.clone();
        let node_id = a.node.clone();
        let target = a.target.clone();

        let created = room.mutate(move |doc| {
            let (_, _, nodes) = crate::store::mindmapdoc::snapshot(doc, &map_id);
            let ordered = crate::store::mindmapdoc::tree_order(&nodes);
            let branch = ordered
                .iter()
                .find(|n| n.id == node_id)
                .ok_or_else(|| ApiError::not_found("mindmap_node", &node_id))?;
            if let (Some(kind), Some(existing)) = (&branch.promoted_kind, &branch.promoted_id) {
                return Err(ApiError::conflict(
                    "mindmap.already_promoted",
                    format!(
                        "That branch already became {kind} '{existing}'. Promoting it again would make a second one from the same thought, indistinguishable from the first."
                    ),
                ));
            }
            let title = branch.title.clone();
            let branch_outline = crate::store::mindmapdoc::outline(&nodes, &node_id);
            let children: Vec<(String, String)> = ordered
                .iter()
                .filter(|n| n.parent.as_deref() == Some(node_id.as_str()))
                .map(|child| {
                    (
                        child.title.clone(),
                        crate::store::mindmapdoc::outline(&nodes, &child.id),
                    )
                })
                .collect();

            let created = state.store.promote_branch(
                &crate::store::BranchPromotion {
                    map_id: &map_id,
                    node_id: &node_id,
                    target: &target,
                    title: &title,
                    branch_outline: &branch_outline,
                    children: &children,
                },
                &actor,
            )?;
            let kind = created["kind"].as_str().unwrap_or_default();
            let created_id = created["id"].as_str().unwrap_or_default();
            crate::store::mindmapdoc::set_promoted(doc, &node_id, kind, created_id)?;
            Ok(created)
        })?;

        // The work is committed; without this the link back into the map is not
        // durable, and a promoted branch could come back looking unpromoted and
        // graduate a second time.
        crate::api::docsync::flush(&self.state, &room, &auth.actor).await;

        let (all, _, _) = room.read(|doc| crate::store::mindmapdoc::snapshot(doc, &a.id));
        let node = all
            .into_iter()
            .find(|n| n["id"].as_str() == Some(a.node.as_str()))
            .ok_or_else(|| ApiError::not_found("mindmap_node", &a.node))?;

        self.state.wake();
        Ok(json!({
            "ok": true,
            "node": node,
            "created": created,
            "note": "The node stays on the map, carrying what it became — the map is the record of how the thinking got there.",
        }))
    }

    fn do_initiative_new(&self, auth: &AuthCtx, a: InitiativeNewArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let req = crate::store::InitiativeCreate {
            title: a.title,
            summary: a.summary,
            status: a.status,
            labels: a.labels.unwrap_or_default(),
            tags: a.tags.unwrap_or_default(),
            metadata: a.metadata,
        };
        let ini = self
            .state
            .store
            .create_initiative(&a.project, &req, &auth.actor)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "initiative": ini.to_json(),
            "note": format!(
                "Feed it with takomo_initiative_append {{ id: \"{}\", kind: \"research\", source: \"…\", text: \"…\" }}. Every entry records where it came from, so the collection stays weighable later. Then write what it MEANS: append kind \"view\" with meta {{ pane: \"business\"|\"technical\"|\"verification\", cites: [entryId, …] }} so the idea reads as a document rather than a pile — and mark the words it arrived in with meta.origin = true.",
                ini.id
            ),
        }))
    }

    fn do_initiative_append(&self, auth: &AuthCtx, a: InitiativeAppendArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        // Project scope is checked against the initiative's own project, so a
        // token restricted to other projects cannot append here — the id alone
        // never grants access.
        let ini = self
            .state
            .store
            .get_initiative(&a.id)?
            .ok_or_else(|| ApiError::not_found("initiative", &a.id))?;
        auth.require_project(&ini.project)?;
        let content = match &a.content_base64 {
            None => None,
            Some(encoded) => Some(crate::api::initiatives::decode_attachment(encoded)?),
        };
        let origin_at = match &a.origin_at {
            None => None,
            Some(raw) => Some(crate::api::initiatives::parse_rfc3339_ms(raw)?),
        };
        let req = crate::store::EntryCreate {
            kind: a.kind,
            title: a.title,
            text: a.text.unwrap_or_default(),
            content,
            mime: a.mime,
            filename: a.filename,
            source: a.source,
            source_uri: a.source_uri,
            origin_at,
            meta: a.meta,
        };
        let (entry, updated) =
            self.state
                .store
                .append_initiative_entry(&a.id, &req, &auth.actor)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "entry": entry.to_json(),
            "initiative": updated.to_json(),
        }))
    }

    fn do_initiative_update(&self, auth: &AuthCtx, a: InitiativeUpdateArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ini = self
            .state
            .store
            .get_initiative(&a.id)?
            .ok_or_else(|| ApiError::not_found("initiative", &a.id))?;
        auth.require_project(&ini.project)?;
        let patch = crate::store::InitiativePatch {
            title: a.title,
            summary: a.summary,
            status: a.status,
            labels: a.labels,
            tags: a.tags,
            metadata_merge: a.metadata_merge,
        };
        if patch.is_empty() {
            return Err(ApiError::bad_request(
                "validation.no_changes",
                "The update contains no changes. Provide at least one of title, summary, status, labels, tags, metadata_merge.",
            ));
        }
        let updated = self
            .state
            .store
            .patch_initiative(&a.id, &patch, &auth.actor)?;
        self.state.wake();
        Ok(json!({ "ok": true, "initiative": updated.to_json() }))
    }

    fn do_initiative_list(&self, auth: &AuthCtx, a: InitiativeListArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let filter = crate::store::InitiativeListFilter {
            project: a.project,
            allowed_projects: auth.allowed_projects_vec(),
            status: a.status,
            q: a.q,
            tag: a.tag,
            label: a.label,
        };
        let limit = a
            .limit
            .unwrap_or(50)
            .clamp(1, crate::store::MAX_INITIATIVES_PAGE);
        let cursor = parse_cursor(a.cursor.as_deref())?;
        let (items, next_cursor) = self.state.store.list_initiatives(&filter, cursor, limit)?;
        Ok(json!({
            "ok": true,
            "items": items.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
            "next_cursor": next_cursor,
        }))
    }

    fn do_initiative_show(&self, auth: &AuthCtx, a: InitiativeShowArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let ini = self
            .state
            .store
            .get_initiative(&a.id)?
            .ok_or_else(|| ApiError::not_found("initiative", &a.id))?;
        auth.require_project(&ini.project)?;
        let limit = a
            .limit
            .unwrap_or(50)
            .clamp(1, crate::store::MAX_ENTRIES_PAGE);
        let cursor = parse_cursor(a.cursor.as_deref())?;
        let (entries, next_cursor) = self
            .state
            .store
            .list_initiative_entries(&a.id, cursor, limit)?;
        let mut out = ini.to_json();
        if let Value::Object(m) = &mut out {
            m.insert(
                "entries".to_string(),
                json!(entries.iter().map(|e| e.to_json()).collect::<Vec<_>>()),
            );
            m.insert("next_cursor".to_string(), json!(next_cursor));
            if a.verification.unwrap_or(false) {
                m.insert(
                    "verification".to_string(),
                    self.state.store.initiative_verification(&a.id)?,
                );
            }
        }
        Ok(json!({ "ok": true, "initiative": out }))
    }
}

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
            tags: a.tags.unwrap_or_default(),
            metadata: a.metadata,
            blocked_by: a.blocked_by.unwrap_or_default(),
            state: None,
        };
        let (ticket, similar, _replayed) =
            self.state
                .store
                .create_ticket(&req, &auth.actor, a.idempotency_key.as_deref())?;
        self.state.wake();
        let mut out = json!({ "ok": true, "ticket": ticket.to_json(now_ms()) });
        if !similar.is_empty() {
            out["similar"] = Value::Array(similar.clone());
            out["note"] = json!(format!(
                "Store detected {} possibly-similar ticket(s); review before assuming this is new.",
                similar.len()
            ));
        }
        // Echo the project's conventions back on create: the ticket text was just
        // written, so this is the moment an agent can still fix it.
        self.attach_conventions(&mut out, &ticket.project);
        Ok(out)
    }

    fn do_list(&self, auth: &AuthCtx, a: ListArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(p) = &a.project {
            auth.require_project(p)?;
        }
        let tag_kinds: Vec<String> = a.tag_kind.into_iter().collect();
        for kind in &tag_kinds {
            crate::store::validate_tag_kind(kind)?;
        }
        let filter = TicketListFilter {
            project: a.project,
            state: a.state,
            ty: a.r#type,
            labels: a.label.into_iter().collect(),
            tags: a.tag.into_iter().collect(),
            tag_kinds,
            // Scheduled-occurrence filters are not on the MCP list tool: an agent
            // wants work to do, and `takomo_next` already refuses to hand it an
            // expired occurrence. Tidying them up is a human-directed maintenance
            // task that goes through the REST filter.
            expired: None,
            schedule: None,
            parent: None,
            epic: None,
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
        let limit = a.limit.unwrap_or(20).clamp(1, 200);
        let (tickets, total) = self.state.store.ready_peek(&filter, limit)?;
        let items: Vec<Value> = tickets.iter().map(brief).collect();
        let mut out = json!({ "ok": true, "items": items, "total": total, "limit": limit });
        if total > items.len() as i64 {
            out["note"] = json!(format!(
                "Showing {} of {total} ready ticket(s). Raise `limit` (max 200) or narrow with project/type/label. The queue is live — other workers claim from it as you read.",
                items.len()
            ));
        }
        Ok(out)
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
            // Attach each question's follow-up thread so an agent sees when a
            // human bounced one back for more research (awaiting == "agent") and
            // can reply with takomo_reply.
            let mut enriched: Vec<Value> = Vec::with_capacity(open.len());
            for q in &open {
                let mut qj = q.to_json();
                let thread = self.state.store.question_thread(&q.id)?;
                if !thread.is_empty() {
                    if let Value::Object(m) = &mut qj {
                        m.insert(
                            "thread".to_string(),
                            json!(thread.iter().map(|t| t.to_json()).collect::<Vec<_>>()),
                        );
                    }
                }
                enriched.push(qj);
            }
            out["open_questions"] = json!(enriched);
        }
        let promos = self.state.store.promotions_for(id)?;
        if !promos.is_empty() {
            out["promotions"] = json!(promos.iter().map(|p| p.to_json()).collect::<Vec<_>>());
        }
        self.attach_conventions(&mut out, &ticket.project);
        Ok(out)
    }

    fn do_ask(&self, auth: &AuthCtx, a: AskArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let expires_at = crate::api::questions::ask_expires_at(a.expires_in_seconds);
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
            option_notes: a.option_notes.unwrap_or_default(),
            multi: a.multi.unwrap_or(false),
            recommended_multi: a.recommended_multi.unwrap_or_default(),
            recommended: a.recommended.unwrap_or(Value::Null),
            recommended_note: a.recommended_note,
            confidence: a.confidence,
            summary: a.summary,
            expertise: a.expertise.unwrap_or_default(),
            assignee: a.assignee,
            urgency: a.urgency,
            expires_at,
            on_timeout,
            fence: resolve_fence(&ticket, &auth.actor, a.fence),
        };
        let (question, updated) = self.state.store.ask_question(&req, &auth.actor)?;
        self.state.wake();
        crate::notify::question_asked(&self.state, &question);
        // Shared with `POST /v1/questions` — including the project's language and
        // style nudge, which the two surfaces used to word separately.
        let note = crate::api::questions::ask_note(
            &self.state,
            &question,
            &updated,
            crate::api::questions::AskSurface::Mcp,
        );
        let hints = crate::store::question_quality_hints(&question);
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
            "ticket": updated.to_json(now_ms()),
            "note": note,
            "hints": hints,
        }))
    }

    fn do_answer(&self, auth: &AuthCtx, a: AnswerArgs) -> ApiResult<Value> {
        // Same fork as the REST handler: answering as yourself is the human
        // gate; naming `on_behalf_of` is a relay and needs its own scope. The
        // refusals that make relaying safe live in the store.
        match &a.on_behalf_of {
            None => auth.require_scope("human")?,
            Some(_) => {
                // Same order as REST: redundancy first, so a `human` caller is
                // told to answer as itself rather than to fetch a scope it does
                // not need.
                if auth.scopes.contains("human") {
                    return Err(ApiError::bad_request(
                        "answer.relay_redundant",
                        "This token holds the 'human' scope, so answer as yourself rather than relay: drop 'on_behalf_of'. Relaying exists for a caller that cannot answer, and a human relaying would make 'answered_by' a claim instead of a fact.",
                    ));
                }
                auth.require_scope("answer:relay")?
            }
        }
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
        if let Some(decider) = a.on_behalf_of.as_deref().map(str::trim) {
            if decider.is_empty() {
                return Err(ApiError::validation(
                    "answer.relay_actor",
                    "'on_behalf_of' must name the human who made this decision — it is what 'answered_by' records. An empty value would file the decision under nobody.",
                ));
            }
            let outcome = self.state.store.answer_question_relayed(
                &a.id,
                &auth.actor,
                crate::store::Answerer::new(decider, &auth.scopes, auth.user.as_deref()),
                &answer,
                a.resume_to.as_deref(),
            )?;
            self.state.wake();
            return Ok(json!({
                "ok": true,
                "question": outcome.question.to_json(),
                "ticket": outcome.ticket.to_json(now_ms()),
                "resume": outcome.resume_json(),
                "relayed_by": auth.actor,
            }));
        }
        let outcome = self.state.store.answer_question(
            &a.id,
            crate::store::Answerer::new(&auth.actor, &auth.scopes, auth.user.as_deref()),
            &answer,
            a.resume_to.as_deref(),
        )?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "question": outcome.question.to_json(),
            "ticket": outcome.ticket.to_json(now_ms()),
            "resume": outcome.resume_json(),
        }))
    }

    fn do_users(&self, auth: &AuthCtx, a: UsersArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        if let Some(project) = &a.project {
            auth.require_project(project)?;
        }
        let limit = a
            .limit
            .unwrap_or(100)
            .clamp(1, crate::store::MAX_USERS_PAGE);
        let offset = a.offset.unwrap_or(0).max(0);
        let filter = crate::store::UserListFilter {
            q: a.q,
            project: a.project,
            include_disabled: a.include_disabled.unwrap_or(false),
            limit,
            offset,
        };
        let (users, total) = self.state.store.list_users(&filter)?;
        let shown = users.len() as i64;
        Ok(json!({
            "ok": true,
            "items": users.iter().map(|u| u.to_json()).collect::<Vec<_>>(),
            "total": total,
            "limit": limit,
            "next_offset": (offset + shown < total).then_some(offset + limit),
        }))
    }

    fn do_assign(&self, auth: &AuthCtx, a: AssignArgs) -> ApiResult<Value> {
        auth.require_scope("human")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        let question =
            self.state
                .store
                .assign_question(&a.id, a.assignee.as_deref(), &auth.actor)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
        }))
    }

    fn do_reopen(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("human")?;
        let q = self
            .state
            .store
            .get_question(id)?
            .ok_or_else(|| ApiError::not_found("question", id))?;
        auth.require_project(&q.project)?;
        let (question, ticket) = self.state.store.reopen_question(
            id,
            crate::store::Answerer::new(&auth.actor, &auth.scopes, auth.user.as_deref()),
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
        let limit = a
            .limit
            .unwrap_or(crate::store::MAX_QUESTIONS_PAGE)
            .clamp(1, crate::store::MAX_QUESTIONS_PAGE);
        let offset = a.cursor.unwrap_or(0).max(0);
        // `mine` is both senses of waiting-on-me, ORed: addressed to this
        // credential's person, or covered by its expertise. Same rule as REST.
        let mine = a.mine.unwrap_or(false);
        let mut assignee: Vec<String> = Vec::new();
        let expertise = if mine {
            let tags: Vec<String> = auth
                .scopes
                .iter()
                .filter_map(|s| s.strip_prefix("expert:").map(str::to_string))
                .collect();
            if let Some(user) = &auth.user {
                assignee.push(user.clone());
            }
            if tags.is_empty() && assignee.is_empty() {
                return Ok(json!({
                    "ok": true,
                    "items": [],
                    "total": 0,
                    "limit": limit,
                    "next_cursor": Value::Null,
                    "note": "Your token carries no expert:<tag> scopes and is not bound to a person, so no questions route to you. Drop `mine` to see the whole queue.",
                }));
            }
            tags
        } else {
            if let Some(raw) = a.assignee.as_deref() {
                if raw != "none" {
                    let person = self
                        .state
                        .store
                        .get_user(raw)?
                        .ok_or_else(|| ApiError::not_found("user", raw))?;
                    assignee.push(person.id);
                }
            }
            Vec::new()
        };
        let unassigned = !mine && a.assignee.as_deref() == Some("none");
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
            assignee,
            unassigned,
            assignee_or_expertise: mine,
            allowed_projects: auth.allowed_projects_vec(),
            limit: Some(limit),
            offset: Some(offset),
        };
        let (items, total) = self.state.store.list_questions(&filter)?;
        let shown = items.len() as i64;
        let next_cursor = (offset + shown < total).then_some(offset + limit);
        let mut out = json!({
            "ok": true,
            "items": items.iter().map(|q| q.to_json()).collect::<Vec<_>>(),
            "total": total,
            "limit": limit,
            "next_cursor": next_cursor,
        });
        if let Some(next) = next_cursor {
            out["note"] = json!(format!(
                "Showing {shown} of {total} question(s). Read the next page with cursor={next}, and repeat while next_cursor is set."
            ));
        }
        Ok(out)
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

    fn do_reply(&self, auth: &AuthCtx, a: ReplyArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        let question = self
            .state
            .store
            .reply_followup(&a.id, &auth.actor, &a.message)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
            "note": "Replied — the thread is back with the human to answer. The ticket stays parked; re-check later.",
        }))
    }

    fn do_revise_options(&self, auth: &AuthCtx, a: ReviseOptionsArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let q = self
            .state
            .store
            .get_question(&a.id)?
            .ok_or_else(|| ApiError::not_found("question", &a.id))?;
        auth.require_project(&q.project)?;
        let req = crate::store::ReviseOptionsRequest {
            options: a.options,
            option_notes: a.option_notes.unwrap_or_default(),
            // `None` means "leave it alone" all the way down, so an agent that
            // only rewords an option need not restate the recommendation.
            recommended: a.recommended.map(Value::from),
            recommended_multi: a.recommended_multi,
            recommended_note: a.recommended_note.map(Some),
            reason: a.reason,
        };
        let question = self
            .state
            .store
            .revise_question_options(&a.id, &auth.actor, &req)?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "question": question.to_json(),
            "note": "Options revised. The thread and whose-turn state are untouched; a human who already read the old set sees the change on the ticket.",
        }))
    }

    /// Mint an answer link. The policy — the `human` scope, the `open` check, the
    /// `approve` -> `expert:<tag>` delegation gate, the TTL precedence and bounds,
    /// and the response body down to the shown-once warning — is
    /// [`crate::api::questions::mint_answer_link`], shared with
    /// `POST /v1/questions/{id}/answer-link`. A credential handed to someone
    /// outside the org must not carry different guarantees per transport.
    fn do_answer_link(&self, auth: &AuthCtx, a: AnswerLinkArgs) -> ApiResult<Value> {
        let link = crate::api::questions::mint_answer_link(
            &self.state,
            auth,
            &a.id,
            a.ttl_seconds,
            a.actor,
        )?;
        Ok(json!({ "ok": true, "answer_link": link }))
    }

    fn do_claim(&self, auth: &AuthCtx, id: &str, ttl: Option<i64>) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        let (_ticket, lease) = self.state.store.claim_ticket(id, &auth.actor, ttl)?;
        self.state.wake();
        let mut out = json!({ "ok": true, "lease": lease.to_json() });
        self.attach_conventions(&mut out, &ticket.project);
        Ok(out)
    }

    fn do_claim_status(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        load_visible(&self.state, auth, id)?;
        Ok(self.state.store.claim_status(id)?.to_json())
    }

    /// Renew a lease this caller holds. The fence is resolved the same way
    /// `do_release` resolves it — holding the active claim *is* holding the
    /// ticket's current `fence_seq` — so an MCP agent never has to track the
    /// token by hand to keep its own claim alive.
    fn do_heartbeat(
        &self,
        auth: &AuthCtx,
        id: &str,
        fence_override: Option<i64>,
        ttl: Option<i64>,
    ) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, id)?;
        // Distinguish "never had it" from "had it, lost it": the store's own
        // fence.stale covers the second, but it only sees a fence, so without
        // this the first would arrive as a confusing mismatch on a lease that
        // was never this caller's.
        let fence = resolve_fence(&ticket, &auth.actor, fence_override).ok_or_else(|| {
            ApiError::conflict(
                "heartbeat.no_lease",
                format!(
                    "You do not hold an active lease on '{id}', so there is nothing to renew. \
                     Claim it first with takomo_claim (or takomo_start), then heartbeat before \
                     the `expires_at` that comes back. Pass an explicit fence to override."
                ),
            )
        })?;
        let lease = self.state.store.heartbeat(id, fence, &auth.actor, ttl)?;
        self.state.wake();
        Ok(json!({ "ok": true, "lease": lease.to_json() }))
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
        let wait = crate::api::clamp_wait(a.wait);
        let actor = auth.actor.clone();
        let ttl = a.ttl_seconds;
        let claimed = crate::api::long_poll(&self.state, wait, || {
            self.state.store.ready_claim(&filter, &actor, ttl)
        })
        .await?;
        match claimed {
            None => {
                Ok(json!({ "ok": true, "claimed": false, "note": "No ready ticket to claim." }))
            }
            Some((ticket, lease)) => {
                self.state.wake();
                let project = ticket.project.clone();
                let mut out = ticket.to_json(now_ms());
                out["lease"] = lease.to_json();
                let mut res = json!({ "ok": true, "claimed": true, "ticket": out });
                self.attach_conventions(&mut res, &project);
                Ok(res)
            }
        }
    }

    fn do_start(&self, auth: &AuthCtx, a: StartArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let wf = self.workflow_for(&ticket.project)?;

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

        let resolved_fence = resolve_fence(&ticket, &auth.actor, a.fence);
        let try_claim = resolved_fence.is_none() && is_claimable(&wf, &ticket.state);

        let updated = self.state.store.start_ticket(
            &a.id,
            &target,
            None,
            a.fence,
            &auth.actor,
            &auth.scopes,
            a.ttl_seconds,
            try_claim,
        )?;
        self.state.wake();
        let mut out =
            json!({ "ok": true, "transitioned_to": target, "ticket": updated.to_json(now_ms()) });
        self.attach_conventions(&mut out, &updated.project);
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
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let wf = self.workflow_for(&ticket.project)?;
        let target = match targets_in_category(&wf, &ticket.state, "blocked")
            .into_iter()
            .next()
        {
            Some(t) => t,
            None => {
                return Err(ApiError::conflict(
                    "transition.no_target",
                    format!(
                        "No legal transition to a 'blocked' state from '{}' in workflow '{}'.",
                        ticket.state, wf.name
                    ),
                )
                .current_state(ticket.state.clone())
                .allowed_transitions(allowed_transitions_from(&wf, &ticket.state)));
            }
        };
        let updated = self.state.store.block_ticket(
            &a.id,
            &target,
            a.comment.as_deref(),
            a.fence,
            &auth.actor,
            &auth.scopes,
        )?;
        self.state.wake();
        Ok(json!({
            "ok": true,
            "transitioned_to": target,
            "ticket": updated.to_json(now_ms()),
        }))
    }

    fn do_comment(&self, auth: &AuthCtx, id: &str, body: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        load_visible(&self.state, auth, id)?;
        let (comment, _) = self.state.store.add_comment(id, &auth.actor, body, None)?;
        self.state.wake();
        Ok(json!({ "ok": true, "comment": comment.to_json() }))
    }

    fn do_link(&self, auth: &AuthCtx, a: LinkArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        // Send only the one key being set (or `null` to delete it). The store
        // merges links per key inside the transaction, so pre-merging the
        // ticket's other links here — read outside that transaction — would
        // resurrect any key a concurrent writer deleted in between.
        let mut one = serde_json::Map::new();
        one.insert(a.key, json!(a.value));
        let patch = TicketPatch {
            links: Some(Value::Object(one)),
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

    fn do_tag(&self, auth: &AuthCtx, a: TagArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let add = a.add.unwrap_or_default();
        let remove = a.remove.unwrap_or_default();
        if add.is_empty() && remove.is_empty() {
            return Err(ApiError::bad_request(
                "validation.no_changes",
                "Provide at least one of 'add' or 'remove' (each a list of kind:handle refs).",
            ));
        }
        let patch = TicketPatch {
            tags_add: add,
            tags_remove: remove,
            fence: resolve_fence(&ticket, &auth.actor, a.fence),
            ..Default::default()
        };
        let updated = self
            .state
            .store
            .patch_ticket(&a.id, &patch, &auth.actor, None)?;
        self.state.wake();
        Ok(json!({ "ok": true, "tags": updated.tags }))
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
        let allowed = auth.allowed_projects_vec();
        let graph = self.state.store.dep_graph(
            &a.id,
            direction,
            a.transitive.unwrap_or(false),
            allowed.as_deref(),
        )?;
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

    fn do_promote(&self, auth: &AuthCtx, a: PromoteArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let ticket = load_visible(&self.state, auth, &a.id)?;
        let promo = self.state.store.promote_ticket(
            &ticket.id,
            &auth.actor,
            &a.target,
            a.url.as_deref(),
            a.ref_.as_deref(),
            a.note.as_deref(),
        )?;
        self.state.wake();
        Ok(json!({ "ok": true, "promotion": promo.to_json() }))
    }

    fn do_move(&self, auth: &AuthCtx, a: MoveArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.to_project)?;
        // Both ends checked before anything is written; descendants ride along
        // without their own check, because a subtree cannot cross projects.
        for id in &a.tickets {
            load_visible(&self.state, auth, id)?;
        }
        let req = crate::store::MoveRequest {
            tickets: a.tickets,
            to_project: a.to_project,
            descendants: a.descendants.unwrap_or(true),
        };
        let outcome = self.state.store.move_tickets(&req, &auth.actor)?;
        self.state.wake();
        let mut out = outcome.to_json();
        out["ok"] = json!(true);
        Ok(out)
    }

    fn do_archive(&self, auth: &AuthCtx, id: &str) -> ApiResult<Value> {
        auth.require_scope("write")?;
        load_visible(&self.state, auth, id)?;
        let ticket = self.state.store.archive_ticket(id, &auth.actor)?;
        self.state.wake();
        Ok(json!({ "ok": true, "ticket": ticket.to_json(now_ms()) }))
    }

    // ---- environments ------------------------------------------------------

    fn do_environments(&self, auth: &AuthCtx, a: EnvironmentsArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(&a.project)?;
        let filter = crate::store::EnvironmentFilter {
            project: a.project.clone(),
            kind: a.kind,
            include_archived: a.archived.unwrap_or(false),
            limit: None,
        };
        let (envs, total) = self.state.store.list_environments(&filter)?;
        Ok(json!({
            "ok": true,
            "environments": envs.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
            "total": total,
        }))
    }

    /// Upsert by `(project, slug)`.
    ///
    /// Implemented as create-then-patch-on-conflict rather than as a second
    /// SQL path: the validation, the caps and the events all live in
    /// `create_environment` and `patch_environment`, and a third code path
    /// writing the same table is how those three drift apart. Losing the race
    /// costs one retry and lands in the same state either way.
    fn do_environment_file(&self, auth: &AuthCtx, a: EnvironmentFileArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let create = crate::store::EnvironmentCreate {
            project: a.project.clone(),
            slug: a.slug.clone(),
            name: a.name.clone(),
            kind: a.kind.clone(),
            base_url: a.base_url.clone(),
            bring_up: a.bring_up.clone(),
            teardown: a.teardown.clone(),
            data_state: a.data_state.clone(),
            writable: a.writable,
            credentials_hint: a.credentials_hint.clone(),
            notes: a.notes.clone(),
            metadata: None,
        };
        match self.state.store.create_environment(&create, &auth.actor) {
            Ok(env) => Ok(json!({ "ok": true, "created": true, "environment": env.to_json() })),
            Err(e) if e.body.code == "conflict.environment_slug" => {
                let existing = self
                    .state
                    .store
                    .list_environments(&crate::store::EnvironmentFilter {
                        project: a.project.clone(),
                        kind: None,
                        include_archived: true,
                        limit: None,
                    })?
                    .0
                    .into_iter()
                    .find(|e| e.slug == a.slug)
                    .ok_or(e)?;
                // Only the fields the caller actually sent are applied; an
                // omitted field keeps whatever is already recorded, so a runner
                // re-registering a URL cannot silently erase someone's notes.
                let patch = crate::store::EnvironmentPatch {
                    name: a.name,
                    kind: a.kind,
                    base_url: a.base_url.map(Some),
                    bring_up: a.bring_up,
                    teardown: a.teardown,
                    data_state: a.data_state,
                    writable: a.writable,
                    credentials_hint: a.credentials_hint.map(Some),
                    notes: a.notes,
                    metadata_merge: None,
                };
                let env = self
                    .state
                    .store
                    .patch_environment(&existing.id, &patch, &auth.actor)?;
                Ok(json!({ "ok": true, "created": false, "environment": env.to_json() }))
            }
            Err(e) => Err(e),
        }
    }

    // ---- checklist ---------------------------------------------------------

    fn do_release_push(&self, auth: &AuthCtx, a: ReleasePushArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let req = crate::store::ReleasePush {
            project: a.project.clone(),
            reference: a.r#ref,
            note: a.note,
            touched_paths: a.touched_paths.unwrap_or_default(),
            orphan_globs: a.orphan_globs.unwrap_or_default(),
        };
        let (release, impact) = self.state.store.push_release(&req, &auth.actor)?;
        self.state.wake();
        let mut out = release.to_json();
        out["impact"] = impact.to_json();
        out["ok"] = json!(true);
        Ok(out)
    }

    fn do_releases(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let items = self.state.store.list_releases(project, 50)?;
        Ok(json!({
            "ok": true,
            "releases": items.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
        }))
    }

    fn do_checks(&self, auth: &AuthCtx, a: ChecksArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(&a.project)?;
        let epic = match a.epic.as_deref() {
            Some("none") => Some(String::new()),
            other => other.map(str::to_string),
        };
        let initiative = match a.initiative.as_deref() {
            Some("none") => Some(String::new()),
            other => other.map(str::to_string),
        };
        let filter = crate::store::CheckFilter {
            node: None,
            project: a.project.clone(),
            epic,
            initiative,
            severity: a.severity,
            layer: a.layer,
            include_archived: false,
            with_policy: true,
            limit: a.limit,
        };
        let limit = a
            .limit
            .unwrap_or(crate::store::MAX_CHECKS_PAGE)
            .clamp(1, crate::store::MAX_CHECKS_PAGE);
        let (checks, total) = self.state.store.list_checks(&filter)?;
        let mut out = json!({
            "ok": true,
            "checks": checks.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
            "total": total,
            "limit": limit,
        });
        if total > checks.len() as i64 {
            out["note"] = json!(format!(
                "Showing {} of {total} check(s). Raise `limit` (max 200) or narrow with epic/severity/layer.",
                checks.len()
            ));
        }
        Ok(out)
    }

    fn do_check(&self, auth: &AuthCtx, a: CheckShowArgs) -> ApiResult<Value> {
        auth.require_scope("read")?;
        let check = self.state.store.get_check(&a.id)?;
        auth.require_project(&check.project)?;
        let mut out = check.to_json();
        out["ok"] = json!(true);
        if a.cases.unwrap_or(false) {
            let (cases, total) = self
                .state
                .store
                .list_cases(&a.id, false, a.limit, a.offset)?;
            let shown = cases.len() as i64;
            out["case_list"] = json!(cases.iter().map(|c| c.to_json()).collect::<Vec<_>>());
            out["case_total"] = json!(total);
            if total > shown {
                let next = a.offset.unwrap_or(0).max(0) + shown;
                out["case_note"] = json!(format!(
                    "Showing {shown} of {total} case(s). Read the next page with offset={next}, and repeat while offset is below case_total. Cases are ordered by key, which is stable."
                ));
            }
        }
        Ok(out)
    }

    fn do_check_file(&self, auth: &AuthCtx, a: CheckFileArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        auth.require_project(&a.project)?;
        let req = crate::store::CheckCreate {
            node: None,
            project: a.project.clone(),
            epic: a.epic,
            initiative: a.initiative,
            environments: a.environments.unwrap_or_default(),
            title: a.title,
            body: a.body.unwrap_or_default(),
            precondition: a.precondition.unwrap_or_default(),
            layer: a.layer,
            severity: a.severity,
            verification: a.verification,
            expiry_days: a.expiry_days,
            expiry_releases: a.expiry_releases,
            cost_agent_minutes: a.cost_agent_minutes,
            cost_human_minutes: a.cost_human_minutes,
            globs: a.globs.unwrap_or_default(),
            metadata: None,
        };
        let check = self.state.store.create_check(&req, &auth.actor)?;
        self.state.wake();
        let mut out = check.to_json();
        out["ok"] = json!(true);
        Ok(out)
    }

    fn do_cases_file(&self, auth: &AuthCtx, a: CaseFileArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let check = self.state.store.get_check(&a.check)?;
        auth.require_project(&check.project)?;
        let cases: Vec<crate::store::CaseInput> = a
            .cases
            .into_iter()
            .map(|c| crate::store::CaseInput {
                key: c.key,
                label: c.label.unwrap_or_default(),
                assignment: c.assignment.unwrap_or(Value::Null),
                seeded: c.seeded.unwrap_or(false),
            })
            .collect();
        let outcome =
            self.state
                .store
                .file_cases(&a.check, &cases, a.prune.unwrap_or(true), &auth.actor)?;
        self.state.wake();
        let mut out = outcome.to_json();
        out["ok"] = json!(true);
        out["check"] = json!(a.check);
        Ok(out)
    }

    fn do_verdict(&self, auth: &AuthCtx, a: VerdictArgs) -> ApiResult<Value> {
        auth.require_scope("write")?;
        let (case, _, _) = self.state.store.get_case(&a.case)?;
        let check = self.state.store.get_check(&case.check)?;
        auth.require_project(&check.project)?;
        // Deliberately no actor_kind parameter: over MCP an agent records an agent
        // verdict. "A person approved this" is a claim only a human-scoped token
        // may make, and it is made through /v1/cases/{id}/verdict.
        let out = self
            .state
            .store
            .record_verdict(&crate::store::VerdictInput {
                case: &a.case,
                // Over MCP a verdict is ALWAYS the agent's. There is no
                // `actor_kind` here on purpose: asserting that a person approved
                // something is the one claim an agent must not be able to make
                // on their behalf, and it goes through POST /v1/cases/{id}/verdict
                // with a `human`-scoped token.
                actor_kind: "agent",
                actor: &auth.actor,
                // Kept even though this is always an agent verdict: an agent token
                // can belong to somebody's own automation, and the history row is
                // where "whose agent" belongs.
                user: auth.user.as_deref(),
                verdict: &a.verdict,
                note: a.note.as_deref(),
                release: a.release.as_deref(),
                environment: a.environment.as_deref(),
            })?;
        self.state.wake();
        let mut body = out.to_json();
        body["ok"] = json!(true);
        Ok(body)
    }

    fn do_worklist(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let mut out = self.state.store.checklist_worklist(project)?;
        out["ok"] = json!(true);
        Ok(out)
    }

    fn do_coverage(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let mut out = self.state.store.checklist_coverage(project)?;
        out["ok"] = json!(true);
        Ok(out)
    }

    fn do_gate(&self, auth: &AuthCtx, project: &str) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let mut out = self.state.store.checklist_gate(project)?;
        out["ok"] = json!(true);
        Ok(out)
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
        let p = self.state.store.get_project(project)?;
        let lang = p.as_ref().and_then(|p| p.question_language.clone());
        let style = p.as_ref().and_then(|p| p.style_guide.clone());
        // null = the project sets no default, so an answer link minted for one
        // of its questions lives for the built-in DEFAULT_ANSWER_TTL_SECONDS.
        let link_ttl = p.as_ref().and_then(|p| p.answer_link_ttl_seconds);
        // The project's lease policy, resolved rather than raw: an agent needs to
        // know how long it may hold a ticket and how long it may ask for, and
        // "null, go read the built-in constant" is not something it can act on.
        // A different setting from answer_link_ttl_seconds above — that bounds a
        // credential handed outside the org; these bound holding work.
        let claim_ttl = p
            .as_ref()
            .and_then(|p| p.claim_ttl_seconds)
            .unwrap_or(crate::store::DEFAULT_TTL_SECONDS);
        let max_claim_ttl = p
            .as_ref()
            .and_then(|p| p.max_claim_ttl_seconds)
            .unwrap_or(crate::store::MAX_TTL_SECONDS);
        Ok(json!({
            "ok": true,
            "workflow": wf,
            "question_language": lang,
            "style_guide": style,
            "default_writing_instruction": self.state.store.default_writing_instruction(project)?,
            "answer_link_ttl_seconds": link_ttl,
            "claim_ttl_seconds": claim_ttl,
            "max_claim_ttl_seconds": max_claim_ttl,
        }))
    }

    fn do_roadmap(&self, auth: &AuthCtx, project: &str, epic: Option<&str>) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let roadmap = self.state.store.roadmap(project, epic)?;
        Ok(json!({ "ok": true, "roadmap": roadmap }))
    }

    fn do_impact(&self, auth: &AuthCtx, project: &str, epic: Option<&str>) -> ApiResult<Value> {
        auth.require_scope("read")?;
        auth.require_project(project)?;
        let impact = self.state.store.impact(project, epic)?;
        Ok(json!({ "ok": true, "impact": impact }))
    }

    /// Attach the project's writing conventions (`language_hint` / `style_hint`)
    /// to a work-loop response.
    ///
    /// The wording lives in [`crate::api::attach_conventions`] and is shared with
    /// the REST work loop, so the two surfaces cannot drift into telling agents
    /// different things about the same project.
    fn attach_conventions(&self, out: &mut Value, project: &str) {
        crate::api::attach_conventions(&self.state, out, project)
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
    /// Dispatch a tool call, charging the write budget first when the tool
    /// writes.
    ///
    /// The debit lives here rather than in the auth middleware because this is
    /// the first point at which the operation has a *name*. The middleware sees
    /// `POST /mcp` with an opaque body; classifying there would mean buffering
    /// and parsing the JSON-RPC frame in the auth path — duplicating the
    /// transport's own parsing to reach a conclusion the transport is about to
    /// hand us for free. The cost of that placement is that this returns a
    /// tool-level error rather than an HTTP 429; the body still carries the
    /// status, code, message and remedy verbatim (`respond`), which is exactly
    /// how every other rejection reaches an MCP caller.
    ///
    /// `initialize`, `tools/list` and the rest of the handshake never reach
    /// here and therefore cost nothing: an agent must be able to discover the
    /// tools before it can spend anything, and discovery mutates nothing. An
    /// unknown tool name is not charged either — the router is about to reject
    /// it, and billing a write for a call that never ran would make the 429
    /// message a lie.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        if self.tool_router.has_route(name) && !READ_TOOLS.contains(&name) {
            let auth = require_auth(&context)?;
            if let Err(err) = debit_write_budget(&self.state, &auth) {
                return respond(Err(err));
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    /// Hand back the pre-slimmed list rather than the router's own
    /// (`#[tool_handler]` generates this method only when it is absent, so
    /// defining it here replaces the generated one).
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, McpError> {
        Ok(rmcp::model::ListToolsResult {
            tools: (*self.tools).clone(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("takomo", crate::server::VERSION))
            .with_instructions(
                "takomo: the central tracker for AI agent fleets. Typical work loop: \
                 `takomo_next` to claim the next ready ticket, `takomo_start` to move it \
                 in-progress, `takomo_comment`/`takomo_link` to record progress, then \
                 `takomo_done` (or `takomo_block`/`takomo_cancel`). A claim is a LEASE and it \
                 expires — 900s by default, and every claim/start/heartbeat response tells you \
                 its `expires_at`. Work that will outlast it must either ask for a longer one \
                 (`ttl_seconds`, max 3600) or renew with `takomo_heartbeat` before it lapses; \
                 nothing beats for you. Let it lapse and the sweeper clears it, after which \
                 fencing correctly refuses your writes and the only way forward is back through \
                 a claimable state to re-claim — losing the work's place in the queue. Before \
                 finishing a ticket, \
                 attach the commit that closes it: `takomo_link { key: \"commit\", value: \
                 \"<full sha or commit URL>\" }`. Without it, `done` is a claim nobody can \
                 check later; with it, any reader can verify the work — and derive which \
                 release and environment carry it (`git tag --contains <sha>`, \
                 `git merge-base --is-ancestor <sha> <deployed sha>`). Use the full SHA; short \
                 ones are ambiguous. Some projects enforce this with a \
                 `guard:has_link:commit` on the done transition. When you need a human \
                 decision (confirmation, a choice, a clarification, approval), call \
                 `takomo_ask` — it parks the ticket and releases your lease; end your run and \
                 resume once the answer appears on the ticket (`takomo_show`). When a project \
                 sets a human-facing language (surfaced as `language_hint` on \
                 takomo_next/claim/start/show and `question_language` on takomo_workflow), \
                 phrase ask-a-human questions in it. When it sets a style guide (`style_hint` \
                 on the same tools, `style_guide` on takomo_workflow), write ticket titles, \
                 bodies, comments, and questions the way it says. Illegal \
                 transitions return the workflow's allowed_transitions so you can self-correct; \
                 call `takomo_workflow` to see a project's full state machine. Not everything \
                 worth keeping is work: when a conversation produces an IDEA rather than a task \
                 — a product direction, an initiative to nurture — open it with \
                 `takomo_initiative_new` and feed it over time with \
                 `takomo_initiative_append`, one entry per input (a note, a research finding, a \
                 colleague's feedback, an attached document). Every entry records its `source`, \
                 so a later reader can weigh where each piece came from. An initiative has no \
                 workflow and is never claimed; when its substance becomes tickets, set its \
                 status to `distilled`."
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

/// Parse an opaque list cursor, matching the REST handlers' contract exactly: it
/// is the previous page's `next_cursor`, verbatim, or nothing.
fn parse_cursor(raw: Option<&str>) -> ApiResult<Option<i64>> {
    match raw {
        None => Ok(None),
        Some(c) => Ok(Some(c.trim().parse::<i64>().map_err(|_| {
            ApiError::bad_request(
                "validation.cursor",
                "Invalid cursor; pass the exact next_cursor value from the previous page.",
            )
        })?)),
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

/// The plan as an agent reads it: headings from the tree, each section's blocks
/// annotated with their ids beneath its own.
fn plan_markdown(doc: &yrs::Doc, map_id: &str, node: Option<&str>) -> ApiResult<String> {
    use yrs::Transact;
    let (_, _, nodes) = crate::store::mindmapdoc::snapshot(doc, map_id);
    if let Some(node) = node {
        // The NON-creating read: this is called inside `room.read`, where a
        // mutation is never queued for the flush nor broadcast, so creating a
        // fragment here would leave the server holding one no peer knows about.
        // A section with no fragment yet reads as its legacy notes, which is what
        // `read_nodes` does — otherwise a read-only agent sees blank sections on
        // a map that simply has not been converted yet.
        return Ok(
            match crate::store::mindmapdoc::read_section_prose(doc, node) {
                Some(frag) => {
                    let txn = doc.transact();
                    let blocks = crate::api::docprops::read_blocks(&txn, &frag);
                    crate::api::docprops::annotate(&blocks)
                }
                None => nodes
                    .iter()
                    .find(|n| n.id == node)
                    .map(|n| n.notes.clone())
                    .unwrap_or_default(),
            },
        );
    }
    let mut out = String::new();
    for section in crate::store::mindmapdoc::tree_order(&nodes) {
        let level = crate::store::mindmapdoc::depth_of(&nodes, &section.id).min(6);
        out.push_str(&format!("{} {}\n\n", "#".repeat(level), section.title));
        let body = match crate::store::mindmapdoc::read_section_prose(doc, &section.id) {
            Some(frag) => {
                let txn = doc.transact();
                let blocks = crate::api::docprops::read_blocks(&txn, &frag);
                crate::api::docprops::annotate(&blocks)
            }
            None => section.notes.clone(),
        };
        {
            if !body.trim().is_empty() {
                out.push_str(&body);
                out.push_str("\n\n");
            }
        }
    }
    Ok(out.trim_end().to_string())
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestDefinitionsArgs {
    project: String,
    offset: Option<i64>,
    limit: Option<i64>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunsArgs {
    project: String,
    cursor: Option<String>,
    limit: Option<i64>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunIdArgs {
    id: String,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunCreateArgs {
    project: String,
    request: crate::store::testruns::RunCreate,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunTransitionArgs {
    id: String,
    action: String,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunResultArgs {
    id: String,
    request: crate::store::testruns::ResultCreate,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TestRunRetryArgs {
    id: String,
    idempotency_key: String,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SpecHistoryArgs {
    mindmap: String,
    before: Option<i64>,
    limit: Option<i64>,
    checkpoints: Option<bool>,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SpecVersionArgs {
    mindmap: String,
    version: i64,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SpecCheckpointArgs {
    mindmap: String,
    expected_version: i64,
    name: String,
}
#[tool_router(router = test_run_router)]
impl TakomoMcp {
    #[tool(
        description = "List saved specification versions, newest first. Follow next_cursor as before. History starts at the available baseline; recorded_by is the flusher, not every author. Document and Map share these versions."
    )]
    async fn takomo_specification_history(
        &self,
        Parameters(a): Parameters<SpecHistoryArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            let map = self
                .state
                .store
                .get_mindmap(&a.mindmap)?
                .ok_or_else(|| ApiError::not_found("mindmap", &a.mindmap))?;
            auth.require_project(&map.project)?;
            self.state.store.specification_history(
                &a.mindmap,
                a.before,
                a.limit.unwrap_or(30).clamp(1, 100),
                a.checkpoints.unwrap_or(false),
            )
        })())
    }
    #[tool(
        description = "Read an immutable saved specification version, including full section text, rich prose XML and relationships. Does not modify the live CRDT. Compare stable node IDs across versions."
    )]
    async fn takomo_specification_version(
        &self,
        Parameters(a): Parameters<SpecVersionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        let state = self.state.clone();
        respond(
            tokio::task::spawn_blocking(move || {
                auth.require_scope("read")?;
                let map = state
                    .store
                    .get_mindmap(&a.mindmap)?
                    .ok_or_else(|| ApiError::not_found("mindmap", &a.mindmap))?;
                auth.require_project(&map.project)?;
                state.store.specification_version(&a.mindmap, a.version)
            })
            .await
            .unwrap_or_else(|e| Err(ApiError::internal(e.to_string()))),
        )
    }
    #[tool(
        description = "Name the exact current saved specification version. Await CRDT durability, read history head and pass expected_version. A changed head returns conflict; review before retrying. Names cannot be moved to another version. Does not include unsaved work."
    )]
    async fn takomo_specification_checkpoint(
        &self,
        Parameters(a): Parameters<SpecCheckpointArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        let state = self.state.clone();
        respond(
            tokio::task::spawn_blocking(move || {
                auth.require_scope("write")?;
                let map = state
                    .store
                    .get_mindmap(&a.mindmap)?
                    .ok_or_else(|| ApiError::not_found("mindmap", &a.mindmap))?;
                auth.require_project(&map.project)?;
                let result = state.store.checkpoint_specification(
                    &a.mindmap,
                    a.expected_version,
                    &a.name,
                    &auth.actor,
                    auth.user.as_deref(),
                )?;
                state.wake();
                Ok(result)
            })
            .await
            .unwrap_or_else(|e| Err(ApiError::internal(e.to_string()))),
        )
    }

    #[tool(
        description = "Read editable test definitions with current revision fingerprints and revision-aware execution summaries. Page using next_offset. Await your CRDT durability acknowledgment before selecting revisions. Legacy verdicts do not prove the current revision."
    )]
    async fn takomo_test_definitions(
        &self,
        Parameters(a): Parameters<TestDefinitionsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            auth.require_project(&a.project)?;
            self.state.store.list_test_definitions(
                &a.project,
                a.offset.unwrap_or(0),
                a.limit.unwrap_or(50),
            )
        })())
    }
    #[tool(
        description = "List execution attempts and legacy evidence, newest first. Follow next_cursor. Results belong to a run; definition edits never rewrite history."
    )]
    async fn takomo_test_runs(
        &self,
        Parameters(a): Parameters<TestRunsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            auth.require_project(&a.project)?;
            self.state
                .store
                .list_test_runs(&a.project, a.cursor.as_deref(), a.limit.unwrap_or(30))
        })())
    }
    #[tool(
        description = "Read one execution attempt, its pinned definition and specification snapshots, case parameters, outcomes and human reviews."
    )]
    async fn takomo_test_run(
        &self,
        Parameters(a): Parameters<TestRunIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("read")?;
            let run = self.state.store.get_test_run(&a.id)?;
            auth.require_project(run["project"].as_str().unwrap())?;
            Ok(run)
        })())
    }
    #[tool(
        description = "Create a queued run over selected revision fingerprints from takomo_test_definitions, an environment and an immutable code reference. A stale selection returns conflict.definition_changed; reread and reconsider. Reuse the same idempotency key when retrying a lost response. Takomo stores the run; you execute it."
    )]
    async fn takomo_test_run_create(
        &self,
        Parameters(a): Parameters<TestRunCreateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            auth.require_project(&a.project)?;
            let run = self
                .state
                .store
                .create_test_run(&a.project, &a.request, &auth.actor)?;
            self.state.wake();
            Ok(run)
        })())
    }
    #[tool(
        description = "Start, complete or cancel a run. Start atomically claims execution for your actor. Only that executor can record agent results and complete; completion requires an outcome for every case. Human approval is separate."
    )]
    async fn takomo_test_run_transition(
        &self,
        Parameters(a): Parameters<TestRunTransitionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let run = self.state.store.get_test_run(&a.id)?;
            auth.require_project(run["project"].as_str().unwrap())?;
            let run = self
                .state
                .store
                .transition_test_run(&a.id, &a.action, &auth.actor)?;
            self.state.wake();
            Ok(run)
        })())
    }
    #[tool(
        description = "Append an immutable result to a run case with evidence references. Agent results require the active executor; actor_kind human requires human scope. For agent_then_human, review requires a passing agent result in this same attempt. Non-pass outcomes need a note. Retry the run for a new observation."
    )]
    async fn takomo_test_result(
        &self,
        Parameters(a): Parameters<TestRunResultArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let run = self.state.store.get_test_run(&a.id)?;
            auth.require_project(run["project"].as_str().unwrap())?;
            if a.request.actor_kind == "human" {
                auth.require_scope("human")?;
            }
            let run = self.state.store.record_test_result(
                &a.id,
                &a.request,
                &auth.actor,
                auth.user.as_deref(),
            )?;
            self.state.wake();
            Ok(run)
        })())
    }
    #[tool(
        description = "Retry a completed or cancelled execution with exactly its original revisions, environment and code reference. Creates a fresh queued attempt with no inherited outcomes or approvals. To test changed code or definitions, create a new run instead."
    )]
    async fn takomo_test_run_retry(
        &self,
        Parameters(a): Parameters<TestRunRetryArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let auth = require_auth(&ctx)?;
        respond((|| {
            auth.require_scope("write")?;
            let run = self.state.store.get_test_run(&a.id)?;
            auth.require_project(run["project"].as_str().unwrap())?;
            let run = self
                .state
                .store
                .retry_test_run(&a.id, &a.idempotency_key, &auth.actor)?;
            self.state.wake();
            Ok(run)
        })())
    }
}
