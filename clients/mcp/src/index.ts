#!/usr/bin/env node
// takomo MCP server (stdio).
//
// A thin MCP wrapper over the takomo HTTP API so agents (Claude Code, Codex,
// ...) can drive the tracker through native tools instead of the CLI. Each tool
// wraps one or a couple of API calls and returns compact JSON. Claimed-ticket
// fences are tracked in memory for the life of the process; store errors are
// relayed verbatim so the agent can self-correct.
//
// Config via environment:
//   TAKOMO_URL    base URL incl. /v1 (default: https://your-takomo-host.onrender.com/v1)
//   TAKOMO_TOKEN  bearer token (required)

import { randomUUID } from "node:crypto";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

import { TakomoClient, StoreError, TransportError } from "./client.js";
import { rememberLease, resolveFence, forgetLease, getLease } from "./fences.js";
import { getWorkflow, isClaimable, categoryOf, targetsInCategory } from "./workflow.js";

const DEFAULT_URL = "https://your-takomo-host.onrender.com/v1";
const baseUrl = process.env.TAKOMO_URL || DEFAULT_URL;
const token = process.env.TAKOMO_TOKEN || "";

if (!token) {
  // Fail loud on stderr; stdout is reserved for the MCP JSON-RPC stream.
  process.stderr.write("takomo-mcp: TAKOMO_TOKEN is not set. Export it before launching.\n");
  process.exit(1);
}

const client = new TakomoClient({ baseUrl, token });

// ---- result helpers ---------------------------------------------------------

type ToolResult = { content: { type: "text"; text: string }[]; isError?: boolean };

function ok(data: unknown): ToolResult {
  return { content: [{ type: "text", text: JSON.stringify(data, null, 2) }] };
}
function fail(data: unknown): ToolResult {
  return { content: [{ type: "text", text: JSON.stringify(data, null, 2) }], isError: true };
}

// Turn any thrown error into an agent-actionable payload. Store errors keep the
// server's own fields (message / remedy / current_state / allowed_transitions)
// verbatim so the agent can correct course.
function toError(err: unknown): ToolResult {
  if (err instanceof StoreError) {
    return fail({ ok: false, status: err.status, ...(err.body ?? { message: err.message }) });
  }
  if (err instanceof TransportError) {
    return fail({ ok: false, transport_error: err.message, status: err.status });
  }
  return fail({ ok: false, error: (err as any)?.message ?? String(err) });
}

// Wrap a handler so every tool returns a clean result instead of throwing.
function tool(fn: (args: any) => Promise<ToolResult>) {
  return async (args: any): Promise<ToolResult> => {
    try {
      return await fn(args);
    } catch (err) {
      return toError(err);
    }
  };
}

// Compact a ticket for list-style output.
function brief(t: any) {
  if (!t || typeof t !== "object") return t;
  return {
    id: t.id,
    title: t.title,
    state: t.state,
    category: t.state_category,
    type: t.type,
    priority: t.priority,
    labels: t.labels,
    parent: t.parent ?? undefined,
    blocked_by: t.blocked_by?.length ? t.blocked_by : undefined,
    claimed_by: t.claim?.holder ?? undefined,
  };
}

// ---- shared ticket operations ----------------------------------------------

async function getTicket(id: string): Promise<any> {
  return client.request({ path: `/tickets/${encodeURIComponent(id)}` });
}

async function claimTicket(id: string): Promise<any> {
  const lease = await client.request<any>({
    method: "POST",
    path: `/tickets/${encodeURIComponent(id)}/claim`,
    body: {},
  });
  if (lease?.fence !== undefined) {
    rememberLease(id, { fence: lease.fence, holder: lease.holder, expiresAt: lease.expires_at });
  }
  return lease;
}

async function transition(id: string, to: string, fence?: number): Promise<any> {
  const body: Record<string, unknown> = { to };
  if (fence !== undefined) body.fence = fence;
  const res = await client.request({
    method: "POST",
    path: `/tickets/${encodeURIComponent(id)}/transition`,
    body,
  });
  return res;
}

// Advance a ticket to the first legal target in a category (done/blocked/
// cancelled). Resolves state names from the project's workflow so it works for
// any workflow shape.
async function advanceToCategory(id: string, category: string, fenceOverride?: number): Promise<ToolResult> {
  const ticket = await getTicket(id);
  const wf = await getWorkflow(client, ticket.project);
  const cands = targetsInCategory(wf, ticket.state, category);
  if (cands.length === 0) {
    const legal = wf.transitions.filter((t) => t.from === ticket.state);
    return fail({
      ok: false,
      message: `No legal transition to a '${category}' state from '${ticket.state}' in workflow '${wf.name}'.`,
      current_state: ticket.state,
      allowed_transitions: legal.map((t) => ({ to: t.to, ...(t.requires ? { requires: t.requires } : {}) })),
    });
  }
  const fence = resolveFence(id, fenceOverride);
  const res = await transition(id, cands[0], fence);
  // Clear the lease once we reach a terminal state.
  const cat = categoryOf(wf, cands[0]);
  if (cat === "done" || cat === "cancelled") forgetLease(id);
  return ok({ ok: true, transitioned_to: cands[0], ticket: res });
}

// ---- server + tools ---------------------------------------------------------

const server = new McpServer({ name: "takomo-mcp", version: "0.1.0" });

server.registerTool(
  "takomo_new",
  {
    title: "Create ticket",
    description:
      "Create a new ticket. Auto-attaches an Idempotency-Key so retries are safe. " +
      "Surfaces any `similar` existing tickets the store detected (possible duplicates).",
    inputSchema: {
      project: z.string().describe("Project id the ticket belongs to."),
      title: z.string().describe("Short ticket title."),
      type: z.string().optional().describe("Ticket type, e.g. task, bug, epic, chore (workflow-dependent)."),
      priority: z.string().optional().describe("Priority, e.g. low, normal, high, urgent."),
      parent: z.string().optional().describe("Parent ticket id (for subtasks)."),
      labels: z.array(z.string()).optional().describe("Labels to attach."),
      body: z.string().optional().describe("Markdown body / description."),
      idempotency_key: z.string().optional().describe("Override the auto-generated idempotency key."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { project: a.project, title: a.title };
    if (a.type) body.type = a.type;
    if (a.priority) body.priority = a.priority;
    if (a.parent) body.parent = a.parent;
    if (a.labels) body.labels = a.labels;
    if (a.body !== undefined) body.body = a.body;
    const res = await client.request<any>({
      method: "POST",
      path: "/tickets",
      body,
      idempotencyKey: a.idempotency_key || `mcp-${randomUUID()}`,
    });
    const out: any = { ok: true, ticket: res };
    if (Array.isArray(res?.similar) && res.similar.length > 0) {
      out.similar = res.similar;
      out.note = `Store detected ${res.similar.length} possibly-similar ticket(s); review before assuming this is new.`;
    }
    return ok(out);
  })
);

server.registerTool(
  "takomo_list",
  {
    title: "List tickets",
    description: "List tickets with optional filters. Returns compact items plus a cursor for pagination.",
    inputSchema: {
      project: z.string().optional().describe("Filter by project id."),
      state: z.string().optional().describe("Filter by exact state, e.g. ready, done."),
      type: z.string().optional().describe("Filter by type."),
      priority: z.string().optional().describe("Filter by priority."),
      label: z.string().optional().describe("Filter by a single label."),
      limit: z.number().int().positive().optional().describe("Max items (default server-defined)."),
      cursor: z.string().optional().describe("Pagination cursor from a previous call's next_cursor."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: "/tickets",
      query: {
        project: a.project,
        state: a.state,
        type: a.type,
        priority: a.priority,
        label: a.label,
        limit: a.limit,
        cursor: a.cursor,
      },
    });
    return ok({ ok: true, items: (res?.items ?? []).map(brief), next_cursor: res?.next_cursor ?? null });
  })
);

server.registerTool(
  "takomo_ready",
  {
    title: "Ready queue",
    description: "List tickets that are ready to be worked (unblocked, in a claimable ready state).",
    inputSchema: {
      project: z.string().optional().describe("Filter the ready queue by project id."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({ path: "/ready", query: { project: a.project } });
    const items = Array.isArray(res) ? res : res?.items ?? [];
    return ok({ ok: true, items: items.map(brief) });
  })
);

server.registerTool(
  "takomo_show",
  {
    title: "Show ticket",
    description: "Fetch one full ticket by id, including body, links, dependencies, and any claim you hold.",
    inputSchema: { id: z.string().describe("Ticket id.") },
  },
  tool(async (a) => {
    const t = await getTicket(a.id);
    const lease = getLease(a.id);
    // Surface every open human question so a resuming agent sees the full
    // barrier (the ticket resumes only once all are answered).
    let openQuestions: any[] = [];
    try {
      const q = await client.request<any>({ path: "/questions", query: { ticket: a.id, status: "open" } });
      openQuestions = q?.items ?? [];
    } catch {
      // Older stores without the questions endpoint: ignore.
    }
    return ok({ ok: true, ticket: t, held_lease: lease ?? null, open_questions: openQuestions });
  })
);

server.registerTool(
  "takomo_claim",
  {
    title: "Claim ticket",
    description:
      "Claim a specific ticket by id, taking its lease. The fencing token is remembered in memory so " +
      "later start/transition/done/release calls include it automatically.",
    inputSchema: { id: z.string().describe("Ticket id to claim.") },
  },
  tool(async (a) => {
    const lease = await claimTicket(a.id);
    return ok({ ok: true, lease });
  })
);

server.registerTool(
  "takomo_next",
  {
    title: "Claim next ready ticket",
    description:
      "Atomically pick and claim the next ready ticket (optionally filtered). Remembers the fence. " +
      "With `wait`, polls up to that many seconds for work to appear before giving up.",
    inputSchema: {
      project: z.string().optional().describe("Restrict to a project id."),
      type: z.string().optional().describe("Restrict to a ticket type."),
      priority: z.string().optional().describe("Restrict to a priority."),
      wait: z.number().int().nonnegative().optional().describe("Seconds to poll for work (client-side). Default 0 (no wait)."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = {};
    if (a.project) body.project = a.project;
    if (a.type) body.type = a.type;
    if (a.priority) body.priority = a.priority;

    const deadline = Date.now() + (a.wait ? a.wait * 1000 : 0);
    const pollMs = 2000;
    for (;;) {
      const res = await client.request<any>({ method: "POST", path: "/ready/claim", body });
      if (res) {
        const lease = res.lease;
        if (lease?.fence !== undefined) {
          rememberLease(res.id, { fence: lease.fence, holder: lease.holder, expiresAt: lease.expires_at });
        }
        return ok({ ok: true, claimed: true, ticket: res, lease: lease ?? null });
      }
      if (Date.now() >= deadline) {
        return ok({ ok: true, claimed: false, note: "No ready ticket to claim." });
      }
      await new Promise((r) => setTimeout(r, Math.min(pollMs, Math.max(0, deadline - Date.now()))));
    }
  })
);

server.registerTool(
  "takomo_start",
  {
    title: "Start work on a ticket",
    description:
      "Begin work: claim the ticket if it is claimable and not already claimed by you, then move it into an " +
      "in-progress state. Target state is resolved from the workflow (override with `to`). Fence handled automatically.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      to: z.string().optional().describe("Explicit target state (defaults to the workflow's in-progress state)."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => {
    const ticket = await getTicket(a.id);
    const wf = await getWorkflow(client, ticket.project);

    let fence = resolveFence(a.id, a.fence);
    // Claim if we do not already hold a lease and the current state is claimable.
    if (fence === undefined && isClaimable(wf, ticket.state)) {
      const lease = await claimTicket(a.id);
      fence = lease?.fence;
    }

    let target = a.to as string | undefined;
    if (!target) {
      if (categoryOf(wf, ticket.state) === "in_progress") {
        const fresh = await getTicket(a.id);
        return ok({ ok: true, note: `Already in an in-progress state ('${ticket.state}').`, ticket: fresh });
      }
      const cands = targetsInCategory(wf, ticket.state, "in_progress");
      if (cands.length === 0) {
        const legal = wf.transitions.filter((t) => t.from === ticket.state);
        return fail({
          ok: false,
          message: `No in-progress transition available from '${ticket.state}' in workflow '${wf.name}'. Pass an explicit \`to\`.`,
          current_state: ticket.state,
          allowed_transitions: legal.map((t) => ({ to: t.to, ...(t.requires ? { requires: t.requires } : {}) })),
        });
      }
      target = cands[0];
    }

    const res = await transition(a.id, target, fence);
    return ok({ ok: true, transitioned_to: target, ticket: res });
  })
);

server.registerTool(
  "takomo_transition",
  {
    title: "Transition ticket",
    description:
      "Move a ticket to an explicit state. Includes your remembered fence automatically when you hold the lease. " +
      "On an illegal move the store's message and allowed_transitions are returned so you can pick a legal target.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      to: z.string().describe("Target state id."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => {
    const fence = resolveFence(a.id, a.fence);
    const res = await transition(a.id, a.to, fence);
    return ok({ ok: true, transitioned_to: a.to, ticket: res });
  })
);

server.registerTool(
  "takomo_done",
  {
    title: "Mark ticket done",
    description: "Move a ticket to the workflow's terminal done state. Fence handled automatically.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => advanceToCategory(a.id, "done", a.fence))
);

server.registerTool(
  "takomo_block",
  {
    title: "Block ticket",
    description:
      "Move a ticket to the workflow's blocked state (e.g. blocked / needs-decision). " +
      "Optionally record a comment explaining the blocker first.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      comment: z.string().optional().describe("Optional note explaining the blocker (added as a comment first)."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => {
    if (a.comment) {
      await client.request({
        method: "POST",
        path: `/tickets/${encodeURIComponent(a.id)}/comments`,
        body: { body: a.comment },
      });
    }
    return advanceToCategory(a.id, "blocked", a.fence);
  })
);

server.registerTool(
  "takomo_cancel",
  {
    title: "Cancel ticket",
    description: "Move a ticket to the workflow's cancelled terminal state. Fence handled automatically.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => advanceToCategory(a.id, "cancelled", a.fence))
);

server.registerTool(
  "takomo_comment",
  {
    title: "Comment on ticket",
    description: "Add a comment to a ticket.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      body: z.string().describe("Comment text."),
    },
  },
  tool(async (a) => {
    const res = await client.request({
      method: "POST",
      path: `/tickets/${encodeURIComponent(a.id)}/comments`,
      body: { body: a.body },
    });
    return ok({ ok: true, comment: res });
  })
);

server.registerTool(
  "takomo_link",
  {
    title: "Attach a named link",
    description:
      "Attach or update a named link on a ticket (e.g. key='pr' value='https://.../pull/1', or key='branch'). " +
      "Existing links are preserved (merged), not replaced.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      key: z.string().describe("Link name, e.g. 'pr', 'branch', 'design'."),
      value: z.string().describe("Link value (URL or ref)."),
    },
  },
  tool(async (a) => {
    const ticket = await getTicket(a.id);
    const links = { ...(ticket.links ?? {}), [a.key]: a.value };
    const res = await client.request({
      method: "PATCH",
      path: `/tickets/${encodeURIComponent(a.id)}`,
      body: { links },
    });
    return ok({ ok: true, links: (res as any)?.links ?? links });
  })
);

server.registerTool(
  "takomo_dep",
  {
    title: "Add a dependency",
    description: "Record that a ticket is blocked by another ticket (adds to its blocked_by set).",
    inputSchema: {
      id: z.string().describe("The dependent ticket id (the one that is blocked)."),
      blocked_by: z.string().describe("The ticket id that must finish first."),
    },
  },
  tool(async (a) => {
    const res = await client.request({
      method: "POST",
      path: `/tickets/${encodeURIComponent(a.id)}/deps`,
      body: { blocked_by: a.blocked_by },
    });
    return ok({ ok: true, dependency: res });
  })
);

server.registerTool(
  "takomo_release",
  {
    title: "Release a claim",
    description: "Release your claim/lease on a ticket, echoing the fencing token. Clears the remembered fence.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
    },
  },
  tool(async (a) => {
    const fence = resolveFence(a.id, a.fence);
    if (fence === undefined) {
      return fail({ ok: false, message: `No remembered lease for '${a.id}'. Pass an explicit fence to release.` });
    }
    await client.request({
      method: "POST",
      path: `/tickets/${encodeURIComponent(a.id)}/release`,
      body: { fence },
    });
    forgetLease(a.id);
    return ok({ ok: true, released: a.id });
  })
);

server.registerTool(
  "takomo_projects",
  {
    title: "List projects",
    description: "List all projects and their workflow names.",
    inputSchema: {},
  },
  tool(async () => {
    const res = await client.request<any>({ path: "/projects" });
    return ok({ ok: true, projects: res });
  })
);

server.registerTool(
  "takomo_workflow",
  {
    title: "Show workflow",
    description: "Show a project's workflow definition (states, categories, and legal transitions). Useful for self-correcting illegal moves.",
    inputSchema: { project: z.string().describe("Project id.") },
  },
  tool(async (a) => {
    const wf = await getWorkflow(client, a.project);
    return ok({ ok: true, workflow: wf });
  })
);

server.registerTool(
  "takomo_ask",
  {
    title: "Ask a human",
    description:
      "Ask a human for a decision (confirm / choose / clarify / approve). A blocking question (default) " +
      "parks the ticket and releases your lease (block-and-resume): end your run and resume once every open " +
      "question on the ticket is answered. An advisory question records a routed decision WITHOUT changing " +
      "ticket state — use it for epic-level or strategic questions. Route to a domain expert with `expertise` tags.",
    inputSchema: {
      id: z.string().describe("Ticket id the question is about."),
      mode: z.enum(["blocking", "advisory"]).optional().describe("blocking (default: parks+resumes the ticket) or advisory (no state change; e.g. an epic-level decision)."),
      kind: z.enum(["confirm", "choose", "clarify", "approve"]).describe("Question kind."),
      title: z.string().describe("The question, phrased for a human domain expert."),
      body: z.string().optional().describe("Context: why you are asking and what you have tried."),
      options: z.array(z.string()).optional().describe("For kind=choose: the options (>= 2)."),
      recommended: z.string().optional().describe("Your recommended answer (a hint; applied on timeout if on_timeout=recommended)."),
      expertise: z.array(z.string()).optional().describe("Routing tags, e.g. [\"domain:billing\"]."),
      urgency: z.enum(["critical", "high", "normal", "low"]).optional().describe("Urgency (default normal)."),
      expires_in_seconds: z.number().int().positive().optional().describe("Auto-expire after this many seconds."),
      on_timeout: z.enum(["recommended", "escalate", "cancel"]).optional().describe("What the expiry sweep does on timeout."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { ticket: a.id, kind: a.kind, title: a.title };
    if (a.mode) body.mode = a.mode;
    if (a.body !== undefined) body.body = a.body;
    if (a.options) body.options = a.options;
    if (a.recommended !== undefined) body.recommended = a.recommended;
    if (a.expertise) body.expertise = a.expertise;
    if (a.urgency) body.urgency = a.urgency;
    if (a.expires_in_seconds) body.expires_in_seconds = a.expires_in_seconds;
    if (a.on_timeout) body.on_timeout = a.on_timeout;
    const fence = resolveFence(a.id);
    if (fence !== undefined) body.fence = fence;
    const res = await client.request<any>({ method: "POST", path: "/questions", body });
    forgetLease(a.id); // lease was released server-side by the ask
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_answer",
  {
    title: "Answer a question",
    description:
      "Answer an open question (requires the human scope on your token). Records the reply and performs the " +
      "ticket's human-gated transition to resume it.",
    inputSchema: {
      id: z.string().describe("Question id (from takomo_questions or the question_asked event)."),
      answer: z.string().describe("\"yes\"/\"no\" for confirm/approve, the chosen option for choose, or the text for clarify."),
      note: z.string().optional().describe("Optional note recorded with the answer."),
      resume_to: z.string().optional().describe("Override the workflow state the ticket resumes into."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { answer: a.note ? { value: a.answer, note: a.note } : a.answer };
    if (a.resume_to) body.resume_to = a.resume_to;
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/answer`,
      body,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_questions",
  {
    title: "List questions (inbox)",
    description:
      "List questions on the ask-a-human board. Filter by project/ticket/status, or `mine` to see only " +
      "questions routed to your expert:<tag> scopes.",
    inputSchema: {
      project: z.string().optional().describe("Filter by project id."),
      ticket: z.string().optional().describe("Filter by ticket id."),
      status: z.string().optional().describe("Statuses (comma-separated); default open."),
      mine: z.boolean().optional().describe("Only questions routed to your expert:<tag> scopes."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: "/questions",
      query: { project: a.project, ticket: a.ticket, status: a.status, mine: a.mine ? "true" : undefined },
    });
    return ok({ ok: true, items: res?.items ?? [], ...(res?.note ? { note: res.note } : {}) });
  })
);

server.registerTool(
  "takomo_withdraw",
  {
    title: "Withdraw a question",
    description:
      "Withdraw an open question you no longer need answered (e.g. you resolved the blocker yourself). " +
      "The ticket stays parked; resume it with takomo_transition.",
    inputSchema: {
      id: z.string().describe("Question id to withdraw."),
      reason: z.string().optional().describe("Optional reason recorded on the withdrawal."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = {};
    if (a.reason) body.reason = a.reason;
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/withdraw`,
      body,
    });
    return ok({ ok: true, question: res });
  })
);

server.registerTool(
  "takomo_answer_link",
  {
    title: "Mint an answer link",
    description:
      "Mint a per-question answer link for an OUTSIDE expert who shouldn't hold a token. Requires the " +
      "human scope (and, for an approve question, the matching expert:<tag>). Returns a single-use, " +
      "expiring tka_ token and a /board#a=<token> path — share it with the person.",
    inputSchema: {
      id: z.string().describe("Question id to mint an answer link for."),
      ttl_seconds: z.number().int().positive().optional().describe("Link lifetime (default 3 days, max 30 days)."),
      actor: z.string().optional().describe("Who a use of the link is attributed to (default human:link:<qid>)."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = {};
    if (a.ttl_seconds) body.ttl_seconds = a.ttl_seconds;
    if (a.actor) body.actor = a.actor;
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/answer-link`,
      body,
    });
    return ok({ ok: true, answer_link: res });
  })
);

server.registerTool(
  "takomo_whoami",
  {
    title: "Whoami",
    description: "Identify the caller behind the current token, if the store exposes /whoami. Returns a graceful note if unavailable.",
    inputSchema: {},
  },
  tool(async () => {
    try {
      const res = await client.request({ path: "/whoami" });
      return ok({ ok: true, whoami: res });
    } catch (err) {
      if (err instanceof StoreError && err.status === 404) {
        return ok({ ok: true, whoami: null, note: "This store build does not expose /whoami." });
      }
      throw err;
    }
  })
);

// ---- boot -------------------------------------------------------------------

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  process.stderr.write(`takomo-mcp connected (store: ${baseUrl})\n`);
}

main().catch((err) => {
  process.stderr.write(`takomo-mcp fatal: ${err?.stack ?? err}\n`);
  process.exit(1);
});
