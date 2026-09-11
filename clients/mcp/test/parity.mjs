// The same observable contract through hosted HTTP and the actual stdio process.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { Client, StreamableHTTPClientTransport } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const base = process.env.TAKOMO_URL;
const admin = process.env.TAKOMO_TOKEN;
const worker = process.env.TAKOMO_WORKER_TOKEN;
const other = process.env.TAKOMO_OTHER_TOKEN;
const project = process.env.TAKOMO_TEST_PROJECT;
assert(base && admin && worker && other && project, "run through cargo test --test mcp_clients -- --ignored");
const entry = fileURLToPath(new URL("../dist/index.js", import.meta.url));

async function api(path, body, token = admin, method = body === undefined ? "GET" : "POST") {
  const response = await fetch(`${base}${path}`, {
    method, headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json();
  assert(response.ok, `${method} ${path}: ${JSON.stringify(result)}`);
  return result;
}

async function connect(kind, token) {
  const client = new Client({ name: "takomo-parity", version: "1" }, { versionNegotiation: { mode: "auto" } });
  const transport = kind === "hosted"
    ? new StreamableHTTPClientTransport(new URL(base.replace(/\/v1\/?$/, "/mcp")), {
        requestInit: { headers: { Authorization: `Bearer ${token}` } },
      })
    : new StdioClientTransport({ command: "node", args: [entry], env: { ...process.env, TAKOMO_TOKEN: token }, stderr: "inherit" });
  await client.connect(transport);
  assert.equal(client.getNegotiatedProtocolVersion(), "2026-07-28", `${kind} negotiates the current protocol`);
  return client;
}
async function tool(client, name, args, error = false) {
  const result = await client.callTool({ name, arguments: args });
  const payload = JSON.parse(result.content.find(c => c.type === "text").text);
  assert.equal(!!result.isError, error, `${name}: ${JSON.stringify(payload)}`);
  return payload;
}
async function ready(title) {
  const ticket = await api("/tickets", { project, title });
  for (const to of ["spec", "ready"]) await api(`/tickets/${ticket.id}/transition`, { to });
  return ticket.id;
}
async function show(id) { return api(`/tickets/${id}?include=comments`); }

for (const kind of ["hosted", "stdio"]) {
  const client = await connect(kind, worker);
  const rival = await connect(kind, other);
  try {
    const listed = await client.listTools();
    for (const name of ["takomo_start", "takomo_block", "takomo_heartbeat"]) {
      assert(listed.tools.some(t => t.name === name), `${kind} lists ${name}`);
    }
    const id = await ready(`${kind} rollback`);
    const refused = await tool(client, "takomo_start", { id, to: "done" }, true);
    assert(Array.isArray(refused.allowed_transitions));
    assert.equal((await api(`/tickets/${id}/claim`)).holder, null, "rejected start leaves no claim");
    assert.equal((await show(id)).state, "ready");

    // Rival ownership must not be stolen or compensated away on failure.
    await tool(rival, "takomo_claim", { id });
    await tool(client, "takomo_start", { id }, true);
    assert.equal((await api(`/tickets/${id}/claim`)).holder, "agent:w2");
    await tool(rival, "takomo_release", { id });

    // Unclaimed start must return/remember a fence that heartbeat can use.
    const started = await tool(client, "takomo_start", { id, ttl_seconds: 60 });
    assert.equal(started.ticket.state, "implementing");
    await tool(client, "takomo_heartbeat", { id });
    const before = await api(`/tickets/${id}/claim`);
    await tool(client, "takomo_start", { id });
    assert.equal((await api(`/tickets/${id}/claim`)).expires_at, before.expires_at, "no-op start does not renew");

    const bad = await tool(client, "takomo_block", { id, fence: -1, comment: "must roll back" }, true);
    assert(bad.code, "fencing error carries store code");
    assert.equal((await show(id)).comments.length, 0);
    assert.equal((await show(id)).state, "implementing");
    const blocked = await tool(client, "takomo_block", { id, comment: "needs a decision" });
    assert.equal(blocked.ticket.state, "needs-decision");
    assert.equal((await show(id)).comments.length, 1);

    // No legal target must not append the optional comment either.
    const brief = await api("/tickets", { project, title: `${kind} no block target` });
    const error = await tool(client, "takomo_block", { id: brief.id, comment: "orphan" }, true);
    assert(Array.isArray(error.allowed_transitions));
    assert.equal((await show(brief.id)).comments.length, 0);

    // A legal edge whose human scope guard fails must roll back a new claim.
    const spec = await api("/tickets", { project, title: `${kind} gated spec` });
    await api(`/tickets/${spec.id}/transition`, { to: "spec" });
    await tool(client, "takomo_start", { id: spec.id, to: "ready" }, true);
    assert.equal((await api(`/tickets/${spec.id}/claim`)).holder, null);
    assert.equal((await show(spec.id)).state, "spec");
    console.log(`${kind}: lifecycle, rollback, ownership, scope, and lease parity passed`);
  } finally {
    await Promise.all([client.close(), rival.close()]);
  }
}
