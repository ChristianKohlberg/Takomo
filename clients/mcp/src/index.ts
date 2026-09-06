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
    tags: t.tags?.length ? t.tags : undefined,
    parent: t.parent ?? undefined,
    blocked_by: t.blocked_by?.length ? t.blocked_by : undefined,
    claimed_by: t.claim?.holder ?? undefined,
  };
}

// ---- shared ticket operations ----------------------------------------------

async function getTicket(id: string): Promise<any> {
  return client.request({ path: `/tickets/${encodeURIComponent(id)}` });
}

async function claimTicket(id: string, ttlSeconds?: number): Promise<any> {
  const body: Record<string, unknown> = {};
  if (ttlSeconds !== undefined) body.ttl_seconds = ttlSeconds;
  const lease = await client.request<any>({
    method: "POST",
    path: `/tickets/${encodeURIComponent(id)}/claim`,
    body,
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

// Bug research is explicit on every surface; use the same REST operations.
const bugId = { id: z.string().describe("Bug ticket ID (job ID for run/steer/cancel).") };
const requestId = z.string().min(1).describe("Reuse after transport failure; use a new ID for a new run/message.");
server.registerTool("takomo_bugs", {
  description: "List ticket-backed bugs; defaults to open, nonarchived tickets. Reading never starts research.",
  inputSchema: { project: z.string(), triage: z.string().optional(), severity: z.string().optional(), state: z.string().optional(), q: z.string().optional(), assignee: z.string().optional(), research_status: z.string().optional(), all: z.boolean().optional(), limit: z.number().int().min(1).max(100).optional(), offset: z.number().int().nonnegative().optional() },
}, tool(async (a) => ok(await client.request({ path: "/bugs", query: { ...a, all: a.all === undefined ? undefined : String(a.all) } }))));
server.registerTool("takomo_bug", {
  description: "Read a bug and triage metadata. Report with takomo_new type bug; ticket creation starts no research.", inputSchema: bugId,
}, tool(async (a) => ok(await client.request({ path: `/bugs/${encodeURIComponent(a.id)}` }))));
server.registerTool("takomo_bug_update", {
  description: "Record a deliberate triage/severity decision and rationale without changing ticket workflow or launching research.",
  inputSchema: { ...bugId, triage: z.string().optional(), severity: z.string().optional(), duplicate_of: z.string().optional(), note: z.string().optional() },
}, tool(async ({ id, ...body }) => ok(await client.request({ method: "PATCH", path: `/bugs/${encodeURIComponent(id)}`, body }))));
server.registerTool("takomo_bug_research", {
  description: "Explicitly start read-only Codex research for a bug; one active run per ticket. A new request_id explicitly retries. Never call automatically on intake.",
  inputSchema: { ...bugId, request_id: requestId, message: z.string().optional() },
}, tool(async ({ id, ...body }) => ok(await client.request({ method: "POST", path: `/bugs/${encodeURIComponent(id)}/research`, body }))));
server.registerTool("takomo_bug_runs", {
  description: "Read research history, retaining earlier findings and inputs.", inputSchema: bugId,
}, tool(async (a) => ok(await client.request({ path: `/bugs/${encodeURIComponent(a.id)}/research` }))));
server.registerTool("takomo_bug_run", {
  description: "Inspect a research job, messages, evidence and status without changing it.", inputSchema: bugId,
}, tool(async (a) => ok(await client.request({ path: `/agent-jobs/${encodeURIComponent(a.id)}` }))));
server.registerTool("takomo_bug_steer", {
  description: "Add steering to an active bug research run without starting another run.",
  inputSchema: { ...bugId, request_id: requestId, message: z.string().min(1) },
}, tool(async ({ id, ...body }) => ok(await client.request({ method: "POST", path: `/agent-jobs/${encodeURIComponent(id)}/steer`, body }))));
server.registerTool("takomo_bug_cancel", {
  description: "Cancel bug research, retaining its ticket and recorded evidence.", inputSchema: bugId,
}, tool(async (a) => ok(await client.request({ method: "POST", path: `/agent-jobs/${encodeURIComponent(a.id)}/cancel`, body: {} }))));
server.registerTool("takomo_bug_research_config", {
  description: "Read project research configuration, or replace with admin scope. repository is a worker allowlist key, not a filesystem path.",
  inputSchema: { project: z.string(), repository: z.string().optional(), revision: z.string().optional(), enabled: z.boolean().optional() },
}, tool(async ({ project, ...body }) => {
  const write = Object.keys(body).length > 0;
  return ok(await client.request({ method: write ? "PUT" : "GET", path: `/projects/${encodeURIComponent(project)}/bug-research-config`, ...(write ? { body: { revision: "HEAD", enabled: true, ...body } } : {}) }));
}));

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
      tags: z
        .array(z.string())
        .optional()
        .describe("Tag refs to attach, each kind:handle (e.g. person:ada, component:billing). Unknown handles are registered on the fly."),
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
    if (a.tags) body.tags = a.tags;
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
      tag: z.string().optional().describe("Filter by an exact tag ref, kind:handle (e.g. person:ada)."),
      tag_kind: z.string().optional().describe("Filter by tag kind — match any tag of this kind (e.g. person)."),
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
        tag: a.tag,
        tag_kind: a.tag_kind,
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
      "later start/transition/done/release calls include it automatically. The lease expires " +
      "(default 900s, max 3600) — keep it with takomo_heartbeat.",
    inputSchema: {
      id: z.string().describe("Ticket id to claim."),
      ttl_seconds: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Lease lifetime in seconds (1-3600, default 900)."),
    },
  },
  tool(async (a) => {
    const lease = await claimTicket(a.id, a.ttl_seconds);
    return ok({ ok: true, lease });
  })
);

server.registerTool(
  "takomo_heartbeat",
  {
    title: "Renew a claim",
    description:
      "Renew the lease you hold on a ticket, so long-running work does not lose its claim. Echoes the " +
      "remembered fencing token and refreshes what is remembered. Call it before the lease's expires_at; " +
      "an already-expired lease cannot be revived and must be re-claimed.",
    inputSchema: {
      id: z.string().describe("Ticket id."),
      fence: z.number().int().optional().describe("Override the remembered fencing token."),
      ttl_seconds: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("New lease lifetime in seconds from now (1-3600, default 900)."),
    },
  },
  tool(async (a) => {
    const fence = resolveFence(a.id, a.fence);
    if (fence === undefined) {
      return fail({
        ok: false,
        message:
          `No remembered lease for '${a.id}', so there is nothing to renew. Claim it first ` +
          `(takomo_claim / takomo_next / takomo_start), or pass an explicit fence.`,
      });
    }
    const body: Record<string, unknown> = { fence };
    if (a.ttl_seconds !== undefined) body.ttl_seconds = a.ttl_seconds;
    const lease = await client.request<any>({
      method: "POST",
      path: `/tickets/${encodeURIComponent(a.id)}/heartbeat`,
      body,
    });
    // Re-remember: the fence is unchanged by a beat, but expires_at is not, and
    // a stale expiry in memory is what makes an agent think it still has time.
    if (lease?.fence !== undefined) {
      rememberLease(a.id, { fence: lease.fence, holder: lease.holder, expiresAt: lease.expires_at });
    }
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
      ttl_seconds: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Lease lifetime in seconds for the ticket this claims (1-3600, default 900)."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = {};
    if (a.project) body.project = a.project;
    if (a.type) body.type = a.type;
    if (a.priority) body.priority = a.priority;
    if (a.ttl_seconds !== undefined) body.ttl_seconds = a.ttl_seconds;

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
      ttl_seconds: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Lease lifetime in seconds, if this call is what takes the claim (1-3600, default 900)."),
    },
  },
  tool(async (a) => {
    const ticket = await getTicket(a.id);
    const wf = await getWorkflow(client, ticket.project);

    let fence = resolveFence(a.id, a.fence);
    // Claim if we do not already hold a lease and the current state is claimable.
    // `ttl_seconds` only applies on that path — a lease we already hold is
    // extended by heartbeating it, not by starting again.
    if (fence === undefined && isClaimable(wf, ticket.state)) {
      const lease = await claimTicket(a.id, a.ttl_seconds);
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
  "takomo_tag",
  {
    title: "Tag a ticket",
    description:
      "Tag people or other entities onto a ticket (reference metadata only — never changes ticket " +
      "state, claims, or routing). `add`/`remove` take kind:handle refs like 'person:ada' or " +
      "'component:billing'; an unknown handle is registered automatically. Filter with takomo_list's " +
      "tag / tag_kind.",
    inputSchema: {
      id: z.string().describe("Ticket id to tag."),
      add: z.array(z.string()).optional().describe("Tag refs to add, each kind:handle."),
      remove: z.array(z.string()).optional().describe("Tag refs to remove, each kind:handle."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = {};
    if (a.add?.length) body.tags_add = a.add;
    if (a.remove?.length) body.tags_remove = a.remove;
    if (!body.tags_add && !body.tags_remove) {
      return fail({ ok: false, message: "Provide at least one of add / remove (kind:handle refs)." });
    }
    const fence = resolveFence(a.id);
    if (fence !== undefined) body.fence = fence;
    const res = await client.request<any>({
      method: "PATCH",
      path: `/tickets/${encodeURIComponent(a.id)}`,
      body,
    });
    return ok({ ok: true, tags: (res as any)?.tags ?? [] });
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
  "takomo_promote",
  {
    title: "Promote a ticket",
    description:
      'Record that this ticket\'s work reached a named target/stage. `target` is free-form ' +
      '("staging", "production", "published", "delivered", …), so it is not limited to software. ' +
      "Optional url/ref/note. Append-only history; the latest shows on the board.",
    inputSchema: {
      id: z.string().describe("Ticket id to promote."),
      target: z.string().describe('Stage the work reached, e.g. "staging", "production", "published".'),
      url: z.string().optional().describe("Optional link (deploy, published page, PR, …)."),
      ref: z.string().optional().describe("Optional reference (version, commit, build id, …)."),
      note: z.string().optional().describe("Optional note."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { target: a.target };
    if (a.url) body.url = a.url;
    if (a.ref) body.ref = a.ref;
    if (a.note) body.note = a.note;
    const res = await client.request<any>({
      method: "POST",
      path: `/tickets/${encodeURIComponent(a.id)}/promote`,
      body,
    });
    return ok({ ok: true, promotion: res });
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
      option_notes: z.array(z.string()).optional().describe("For kind=choose: a one-line trade-off per option, parallel to `options` (same length) — lets the inbox show what each choice means."),
      multi: z.boolean().optional().describe("For kind=choose: allow selecting several options at once (multi-select)."),
      recommended_multi: z.array(z.string()).optional().describe("For a multi choose: the recommended set of options."),
      recommended: z.string().optional().describe("Your recommended answer (for a single choose, the exact option string; applied on timeout if on_timeout=recommended)."),
      recommended_note: z.string().optional().describe("A short rationale for your recommendation (the 'why')."),
      confidence: z.number().int().min(1).max(4).optional().describe("How strong your recommendation is: 1 tentative … 4 very strong."),
      summary: z.string().optional().describe("A one-line summary for the inbox list preview (optional; else derived from the body)."),
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
    if (a.option_notes) body.option_notes = a.option_notes;
    if (a.multi) body.multi = a.multi;
    if (a.recommended_multi) body.recommended_multi = a.recommended_multi;
    if (a.recommended !== undefined) body.recommended = a.recommended;
    if (a.recommended_note !== undefined) body.recommended_note = a.recommended_note;
    if (a.confidence !== undefined) body.confidence = a.confidence;
    if (a.summary !== undefined) body.summary = a.summary;
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
  "takomo_reopen",
  {
    title: "Reopen a question",
    description:
      "Reopen an answered question — take back a decision (a conditional undo beyond the inbox's 30s window). " +
      "Requires the human scope. Refused with a teaching 409 if the ticket already relies on the answer " +
      "(claimed, moved on, or archived); re-park and re-ask instead in that case.",
    inputSchema: {
      id: z.string().describe("Question id to reopen (an answered question)."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/reopen`,
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
  "takomo_reply",
  {
    title: "Reply to a follow-up",
    description:
      "Reply to a question a human bounced back for more research (its `awaiting` is \"agent\", visible " +
      "on takomo_show / takomo_questions). Post the context they asked for; this flips the thread back " +
      "to the human so they can answer. The ticket stays parked meanwhile.",
    inputSchema: {
      id: z.string().describe('Question id a human bounced back to you (awaiting == "agent").'),
      message: z.string().describe("The research/context the human asked for."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/reply`,
      body: { message: a.message },
    });
    return ok({ ok: true, question: res });
  })
);

server.registerTool(
  "takomo_options",
  {
    title: "Revise a choose question's options",
    description:
      "Revise a still-open 'choose' question's options. Use this when research (often the follow-up a " +
      "human asked for) shows the choices you offered were wrong, incomplete, or misleading — better " +
      "than withdrawing the question, which throws the whole thread away. Send the FULL replacement " +
      "set (at least 2); it does not merge. `recommended` must be one of the revised options, so pass " +
      "a new one whenever you drop the option you had recommended, or null to clear it. Give a " +
      "`reason`: a human may already have read the old set. Options can only be revised while the " +
      "question is open — a settled question keeps the choices it was decided on.",
    inputSchema: {
      id: z.string().describe("Question id (must still be open, and of kind 'choose')."),
      options: z
        .array(z.string())
        .min(2)
        .describe("The FULL revised option set — replaces the options, does not merge into them."),
      option_notes: z
        .array(z.string())
        .optional()
        .describe("One-line trade-off per option, parallel to options (same length, or omit)."),
      recommended: z
        .string()
        .nullable()
        .optional()
        .describe("New recommendation; must be one of the revised options. null clears it. Omit to keep."),
      recommended_multi: z
        .array(z.string())
        .optional()
        .describe("For a multi choose — the new recommended subset. Empty list clears it. Omit to keep."),
      recommended_note: z.string().optional().describe("Short rationale for the recommendation. Omit to keep."),
      reason: z.string().optional().describe("Why the options changed — recorded on the ticket."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { options: a.options };
    // Only forward what the caller actually set: absent means "leave it alone",
    // and an explicit null means "clear it".
    if (a.option_notes !== undefined) body.option_notes = a.option_notes;
    if (a.recommended !== undefined) body.recommended = a.recommended;
    if (a.recommended_multi !== undefined) body.recommended_multi = a.recommended_multi;
    if (a.recommended_note !== undefined) body.recommended_note = a.recommended_note;
    if (a.reason !== undefined) body.reason = a.reason;
    const res = await client.request<any>({
      method: "POST",
      path: `/questions/${encodeURIComponent(a.id)}/options`,
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
      ttl_seconds: z.number().int().positive().optional().describe("Link lifetime in seconds; wins over the project default. Omit to use the project's answer_link_ttl_seconds, and then the built-in 7 days. Max 30 days."),
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

// ---- verification: checks, cases, verdicts, environments ---------------------
//
// How a "done" claim becomes a VERIFIED one. A check is one action with one
// entry precondition at one layer; a case is that check crossed with one
// parameter assignment, and the case is what actually gets executed.
//
// **Takomo stores; you compute.** Nothing here generates a case model, validates
// one, or judges whether a coverage claim is true. The store persists what you
// file and enforces who may assert what — the intelligence stays where the
// context is.
//
// The loop these serve: `takomo_worklist` says what needs re-verifying and who
// can clear it, `takomo_environments` says where to run it and whether writing
// there is safe, and `takomo_verdict` reports what you observed.

server.registerTool(
  "takomo_checks",
  {
    title: "List checks",
    description:
      "List a project's checks with their case counts, resolved policy and any orphaned globs. " +
      "A check is ONE action with ONE entry precondition at ONE layer. " +
      "`initiative: \"none\"` narrows to checks no initiative claims — the gap between what was " +
      "agreed and what got written down.",
    inputSchema: {
      project: z.string().describe("Project id."),
      initiative: z
        .string()
        .optional()
        .describe('Narrow to one initiative, or "none" for checks no initiative claims.'),
      epic: z.string().optional().describe('Narrow to one epic, or "none" for ungrouped checks.'),
      severity: z.string().optional().describe("blocking, advisory or low."),
      layer: z.string().optional().describe("ui, api or other."),
      limit: z.number().optional().describe("1..=200 (default 200). `total` always reports how many matched."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/checks`,
      query: {
        initiative: a.initiative,
        epic: a.epic,
        severity: a.severity,
        layer: a.layer,
        limit: a.limit,
      },
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_check",
  {
    title: "Show one check",
    description:
      "One check: its traversal body, entry precondition, claimed globs, the environments it must " +
      "pass in, its resolved policy and its case counts. Pass cases=true to include every case with " +
      "its per-environment state.",
    inputSchema: {
      id: z.string().describe("Check id."),
      cases: z.boolean().optional().describe("Include the check's cases."),
      limit: z.number().optional().describe("With cases: 1..=500 (default 500)."),
    },
  },
  tool(async (a) => {
    const check = await client.request<any>({ path: `/checks/${encodeURIComponent(a.id)}` });
    const out: any = { ok: true, check };
    if (a.cases) {
      const page = await client.request<any>({
        path: `/checks/${encodeURIComponent(a.id)}/cases`,
        query: { limit: a.limit },
      });
      out.cases = page?.items ?? [];
      out.case_total = page?.total;
      if (page?.note) out.note = page.note;
    }
    return ok(out);
  })
);

server.registerTool(
  "takomo_check_file",
  {
    title: "Declare a check",
    description:
      "Declare a check. Draw its boundary at a STATE TRANSITION, not a screen: if something needs a " +
      "persisted record, has its own permission gate, or is only reachable from another check's " +
      "terminal state, it is a SEPARATE check. A create form, a finalize step and a cancel action " +
      "are three checks, not one. " +
      "One check covers ONE layer — a rule enforced only in the interface passes at the API layer, " +
      "so those two verdicts are not interchangeable. " +
      "Takomo stores what you file and does not judge whether the model is right.",
    inputSchema: {
      project: z.string().describe("Project the check belongs to."),
      title: z.string().describe('The one action this verifies, e.g. "Create a claim".'),
      initiative: z
        .string()
        .optional()
        .describe(
          "The initiative whose conversation agreed this check should exist. File it even before " +
            "an epic exists — that is usually when the agreement is made."
        ),
      environments: z
        .array(z.string())
        .optional()
        .describe(
          "Where it must pass, by slug or id. Each case is then tracked PER ENVIRONMENT, so " +
            '"verified on staging, never run on production" is expressible instead of collapsing ' +
            "into one verdict. Omit for a check whose result does not depend on where it runs."
        ),
      epic: z.string().optional().describe("Epic ticket id to group under."),
      body: z.string().optional().describe("Free-form traversal to follow. No step model, no DAG — prose is the content."),
      precondition: z.string().optional().describe("The data state and permissions needed before this check can start."),
      layer: z.string().optional().describe("ui, api or other (default api)."),
      severity: z.string().optional().describe("blocking, advisory or low (default advisory). Only blocking blocks a gate."),
      globs: z
        .array(z.string())
        .optional()
        .describe("Paths of the app under test this check claims, e.g. src/claims/**. Hand-declared and known to rot."),
      verification: z.string().optional().describe("agent, human or agent_then_human. Omit to inherit."),
      expiry_days: z.number().optional().describe("Re-verify after this many days. Omit to inherit."),
      expiry_releases: z.number().optional().describe("Re-verify after this many releases. Omit to inherit."),
      cost_agent_minutes: z.number().optional().describe("Rough agent-minutes to run once."),
      cost_human_minutes: z.number().optional().describe("Rough human-minutes to run once."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { title: a.title };
    for (const k of [
      "initiative",
      "environments",
      "epic",
      "body",
      "precondition",
      "layer",
      "severity",
      "globs",
      "verification",
      "expiry_days",
      "expiry_releases",
      "cost_agent_minutes",
      "cost_human_minutes",
    ] as const) {
      if (a[k] !== undefined) body[k] = a[k];
    }
    const res = await client.request<any>({
      method: "POST",
      path: `/projects/${encodeURIComponent(a.project)}/checks`,
      body,
    });
    return ok({ ok: true, check: res });
  })
);

server.registerTool(
  "takomo_cases_file",
  {
    title: "File a check's case set",
    description:
      "File the generated case set for a check. Upsert is by `key`, so DERIVE EACH KEY FROM ITS " +
      "PARAMETER ASSIGNMENT: a case still present keeps its verdict history, one that vanished is " +
      "retired rather than deleted, one that returns is revived. That is what makes regenerating a " +
      "model after adding a parameter safe. " +
      "A large real form yields around 76 pairwise cases — if you have thousands, most of your " +
      "parameters are inert fields that do not belong in the model. " +
      "`prune: false` extends the set instead of replacing it.",
    inputSchema: {
      check: z.string().describe("Check id."),
      cases: z
        .array(
          z.object({
            key: z.string().describe("Stable identity derived from the assignment, e.g. entities=2."),
            label: z.string().optional().describe("Human-readable name."),
            assignment: z.record(z.any()).optional().describe("The parameter assignment this case stands for."),
            seeded: z.boolean().optional().describe("True when the fixture data already exists."),
          })
        )
        .describe("The generated case set."),
      prune: z.boolean().optional().describe("False extends rather than replaces (default true)."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { cases: a.cases };
    if (a.prune !== undefined) body.prune = a.prune;
    const res = await client.request<any>({
      method: "PUT",
      path: `/checks/${encodeURIComponent(a.check)}/cases`,
      body,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_verdict",
  {
    title: "Record a verdict",
    description:
      "Record what you OBSERVED on one case: pass, fail, blocked or unreachable. " +
      "`fail` needs a note — a failure nobody described is one nobody can act on. " +
      "`unreachable` means the declared layer gives no way to reach this configuration, and it is " +
      "the most valuable output here: it is how UI/API drift and dead code fall out of coverage " +
      "bookkeeping instead of needing a separate audit. " +
      "This tool always records an AGENT verdict. Asserting that a PERSON approved a case needs a " +
      "human-scoped token through POST /v1/cases/{id}/verdict — the same line ask-a-human draws.",
    inputSchema: {
      case: z.string().describe("Case id."),
      verdict: z.string().describe("pass, fail, blocked or unreachable."),
      note: z.string().optional().describe("What you observed. Required for `fail`."),
      environment: z
        .string()
        .optional()
        .describe(
          "Where you observed it, by slug or id. REQUIRED when the check declares more than one " +
            "environment — a bare verdict there does not say what you saw, and it is refused rather " +
            "than filed against a guess. Omit for a check declaring one (that one is meant) or none."
        ),
      release: z.string().optional().describe("Release id this verdict was taken against."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { verdict: a.verdict };
    if (a.note !== undefined) body.note = a.note;
    if (a.environment !== undefined) body.environment = a.environment;
    if (a.release !== undefined) body.release = a.release;
    const res = await client.request<any>({
      method: "POST",
      path: `/cases/${encodeURIComponent(a.case)}/verdict`,
      body,
    });
    return ok({ ok: true, case: res });
  })
);

server.registerTool(
  "takomo_worklist",
  {
    title: "What needs re-verifying",
    description:
      "What must be re-verified, split by WHO CAN CLEAR IT. Human time is the scarce resource — a " +
      "hundred cases cost an agent minutes and cost a person most of a day — so the split is the " +
      "answer, not a formatting choice. " +
      "Each item carries its reason (stale, expired, failed, never, awaiting_human) and, when the " +
      "check declares environments, WHERE to run it and that environment's base URL. " +
      "A stale case under agent_then_human stays on the AGENT list until it has a fresh agent " +
      "verdict, so it never sits in a person's queue waiting for work only an agent can do.",
    inputSchema: { project: z.string().describe("Project id.") },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/checklist/worklist`,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_coverage",
  {
    title: "Coverage rollup",
    description:
      "Coverage per epic, and per environment where checks declare them. " +
      "`percent` is verified-or-approved over VERIFIABLE cases: stale, failed and never are outside " +
      "the numerator, and unreachable is outside the denominator too, so a fully verified project " +
      "can actually reach 100% and the unreachable count stands on its own as a finding. " +
      "Coverage is of the DECLARED surface — hand-written globs, not measured execution.",
    inputSchema: { project: z.string().describe("Project id.") },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/checklist/coverage`,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_gate",
  {
    title: "Can this ship",
    description:
      "Is verification good enough to ship? Only `blocking` severity blocks; advisory and low nag. " +
      "A gate that fires on everything gets overridden out of habit and stops meaning anything.",
    inputSchema: { project: z.string().describe("Project id.") },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/checklist/gate`,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_verification",
  {
    title: "An initiative's verification standing",
    description:
      "Do the tests this initiative agreed on still pass? Returns how many of its checks' cases are " +
      "verified, stale, failed or never run, when one was last verified, and whether anything " +
      "blocking is outstanding. " +
      "This is the question a characterisation test exists to answer months after the conversation " +
      "that produced it.",
    inputSchema: { initiative: z.string().describe("Initiative id (ini-…).") },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/initiatives/${encodeURIComponent(a.initiative)}/verification`,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_releases",
  {
    title: "List releases",
    description:
      "A project's releases, newest first, with their sequence numbers. " +
      "Not to be confused with `takomo_release`, which gives back a ticket CLAIM — these are the " +
      "shipped-code markers verdicts are dated against.",
    inputSchema: {
      project: z.string().describe("Project id."),
      limit: z.number().optional().describe("How many to return."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/releases`,
      query: { limit: a.limit },
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_release_push",
  {
    title: "Record a merged release",
    description:
      "Record a release you just merged, and learn what it invalidated. " +
      "Send the tag or FULL sha as `ref`, the paths the diff touched, and any check globs that " +
      "matched NO file in the tree. Every check claiming a touched path has its verified cases " +
      "marked stale; globs that matched nothing are flagged so those checks stop counting as " +
      "covered. " +
      "YOU supply the paths and the empty globs because you have the tree checked out — the server " +
      "clones nothing, which is the cheapest possible place to learn the truth. There is no direct " +
      "integration by design: the agent that merged the work is what tells Takomo a release happened.",
    inputSchema: {
      project: z.string().describe("Project the release belongs to."),
      ref: z.string().describe("The tag or FULL commit sha. Short shas are ambiguous."),
      note: z.string().optional().describe("What shipped."),
      touched_paths: z.array(z.string()).optional().describe("Paths the diff touched."),
      orphan_globs: z.array(z.string()).optional().describe("Check globs that matched NO file in this tree."),
    },
  },
  tool(async (a) => {
    const body: Record<string, unknown> = { ref: a.ref };
    if (a.note !== undefined) body.note = a.note;
    if (a.touched_paths !== undefined) body.touched_paths = a.touched_paths;
    if (a.orphan_globs !== undefined) body.orphan_globs = a.orphan_globs;
    const res = await client.request<any>({
      method: "POST",
      path: `/projects/${encodeURIComponent(a.project)}/releases`,
      body,
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_environments",
  {
    title: "Where a check can be run",
    description:
      "The places a check can be run: base URL, how to bring each one up and give it back, what " +
      "data is in it, and whether writing to it is safe. " +
      "READ THIS BEFORE RUNNING A CHECK — it is where the URL and the credential pointer live, so " +
      "you do not have to be told them out of band. " +
      "`writable` is advisory: Takomo executes nothing and cannot enforce it. `credentials_hint` is " +
      "a POINTER to where a credential lives, never a credential — every token with `read` can see it.",
    inputSchema: {
      project: z.string().describe("Project id."),
      kind: z.string().optional().describe("local, ephemeral, shared, staging, production or other."),
      archived: z.boolean().optional().describe("Include archived environments."),
    },
  },
  tool(async (a) => {
    const res = await client.request<any>({
      path: `/projects/${encodeURIComponent(a.project)}/environments`,
      query: { kind: a.kind, archived: a.archived ? "include" : undefined },
    });
    return ok({ ok: true, ...res });
  })
);

server.registerTool(
  "takomo_environment_file",
  {
    title: "Register an environment",
    description:
      "Register an environment, or update the one already holding that slug. " +
      "Use it when you stand up an instance others will verify against — an ephemeral preview, a " +
      "seeded local box — so the next runner is not told the URL out of band. " +
      "Filing the same slug twice updates in place, so this is safe to call every run, and a field " +
      "you omit keeps whatever is already recorded. " +
      "Put a POINTER in `credentials_hint` (\"env:STAGING_TOKEN\", a vault path), NEVER a credential: " +
      "any token with `read` can see it.",
    inputSchema: {
      project: z.string().describe("Project the environment belongs to."),
      slug: z.string().describe('Stable handle you will pass everywhere else, e.g. "staging". Immutable once created.'),
      name: z.string().optional().describe("Human-readable name. Defaults to the slug."),
      kind: z.string().optional().describe("local, ephemeral, shared, staging, production or other."),
      base_url: z.string().optional().describe("Where the application answers."),
      bring_up: z.string().optional().describe("How to get it running. Takomo never runs it — it hands it to whoever needs it next."),
      teardown: z.string().optional().describe("How to give it back when the run is over."),
      data_state: z.string().optional().describe("seeded, empty, production_like or unknown."),
      writable: z
        .boolean()
        .optional()
        .describe("Whether a destructive case may run here. ADVISORY. Defaults to false for kind=production."),
      credentials_hint: z.string().optional().describe("WHERE a credential lives. Never the credential itself."),
      notes: z.string().optional().describe("Caveats that would make a verdict untrustworthy — reset cadence, shared sandboxes."),
    },
  },
  tool(async (a) => {
    const fields: Record<string, unknown> = {};
    for (const k of [
      "name",
      "kind",
      "base_url",
      "bring_up",
      "teardown",
      "data_state",
      "writable",
      "credentials_hint",
      "notes",
    ] as const) {
      if (a[k] !== undefined) fields[k] = a[k];
    }
    // Upsert by (project, slug): create, and on a slug conflict patch the one
    // already holding it. Two calls rather than a second server path, so the
    // validation, caps and events all stay in the routes that own them.
    try {
      const created = await client.request<any>({
        method: "POST",
        path: `/projects/${encodeURIComponent(a.project)}/environments`,
        body: { slug: a.slug, ...fields },
      });
      return ok({ ok: true, created: true, environment: created });
    } catch (err) {
      if (!(err instanceof StoreError) || err.body?.code !== "conflict.environment_slug") throw err;
      const list = await client.request<any>({
        path: `/projects/${encodeURIComponent(a.project)}/environments`,
        query: { archived: "include" },
      });
      const existing = (list?.items ?? []).find((e: any) => e.slug === a.slug);
      if (!existing) throw err;
      const patched = await client.request<any>({
        method: "PATCH",
        path: `/environments/${encodeURIComponent(existing.id)}`,
        body: fields,
      });
      return ok({ ok: true, created: false, environment: patched });
    }
  })
);

// Lanes: organization and drafts; dispatch remains an explicit human action.
server.registerTool("takomo_lanes", {description: 'List project lanes.', inputSchema: { project:z.string(),limit:z.number().int().positive().optional(),offset:z.number().int().nonnegative().optional() } }, tool(async (a) => ok(await client.request({method:"GET",path:`/projects/${encodeURIComponent(a.project)}/lanes`,body:undefined,query:{limit:a.limit,offset:a.offset}}))));
server.registerTool("takomo_lane_show", {description: 'Read lane context and tickets.', inputSchema: { id:z.string() } }, tool(async (a) => ok(await client.request({method:"GET",path:`/lanes/${encodeURIComponent(a.id)}`,body:undefined,query:undefined}))));
server.registerTool("takomo_lane_create", {description: 'Create a lane without executing work.', inputSchema: { project:z.string(),title:z.string(),purpose:z.string().optional(),context:z.string().optional() } }, tool(async (a) => ok(await client.request({method:"POST",path:`/projects/${encodeURIComponent(a.project)}/lanes`,body:{title:a.title,purpose:a.purpose,context:a.context},query:undefined}))));
server.registerTool("takomo_lane_update", {description: 'Read first, then preserve decisions while updating lane context.', inputSchema: { id:z.string(),title:z.string().optional(),purpose:z.string().optional(),context:z.string().optional() } }, tool(async (a) => ok(await client.request({method:"PATCH",path:`/lanes/${encodeURIComponent(a.id)}`,body:{title:a.title,purpose:a.purpose,context:a.context},query:undefined}))));
server.registerTool("takomo_lane_ticket", {description: 'Organize a same-project ticket; existing snapshots stay fixed.', inputSchema: { lane:z.string(),ticket:z.string(),remove:z.boolean().optional() } }, tool(async (a) => ok(await client.request({method:a.remove ? "DELETE" : "PUT",path:`/lanes/${encodeURIComponent(a.lane)}/tickets/${encodeURIComponent(a.ticket)}`,body:{},query:undefined}))));
server.registerTool("takomo_lane_handoff", {description: 'Draft only. A human must explicitly dispatch. Review requires an implementation parent and exact target revision.', inputSchema: { lane:z.string(),kind:z.enum(["preparation","implementation","review"]),provider:z.enum(["codex","claude"]),instructions:z.string(),ticket_ids:z.array(z.string()),target_revision:z.string().optional(),parent_handoff:z.string().optional() } }, tool(async (a) => ok(await client.request({method:"POST",path:`/lanes/${encodeURIComponent(a.lane)}/handoffs`,body:{kind:a.kind,provider:a.provider,instructions:a.instructions,ticket_ids:a.ticket_ids,target_revision:a.target_revision,parent_handoff:a.parent_handoff},query:undefined}))));
server.registerTool("takomo_lane_handoffs", {description: 'Read returned results and review findings.', inputSchema: { lane:z.string(),limit:z.number().int().positive().optional(),offset:z.number().int().nonnegative().optional() } }, tool(async (a) => ok(await client.request({method:"GET",path:`/lanes/${encodeURIComponent(a.lane)}/handoffs`,body:undefined,query:{limit:a.limit,offset:a.offset}}))));

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
