// Live end-to-end test harness for the takomo MCP server.
//
// Spawns the built stdio server via the official MCP SDK client, lists tools,
// then drives a full lifecycle against the live store in the throwaway `mcptest`
// project: new -> ready -> next -> start -> comment -> done, plus one illegal
// transition to prove the store's error text passes through.
//
// Usage: node test/e2e.mjs   (reads TAKOMO_URL / TAKOMO_TOKEN from env)

import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(__dirname, "..");

const TAKOMO_URL = process.env.TAKOMO_URL || "https://your-takomo-host.onrender.com/v1";
const TAKOMO_TOKEN = process.env.TAKOMO_TOKEN;
const PROJECT = process.env.TAKOMO_TEST_PROJECT || "mcptest";

const transport = new StdioClientTransport({
  command: "node",
  args: [resolve(pkgDir, "dist/index.js")],
  cwd: pkgDir,
  env: { ...process.env, TAKOMO_URL, TAKOMO_TOKEN },
  stderr: "inherit",
});

const client = new Client({ name: "takomo-mcp-e2e", version: "0.1.0" });

function line(s = "") {
  process.stdout.write(s + "\n");
}

async function call(name, args = {}) {
  const res = await client.callTool({ name, arguments: args });
  const text = res.content.map((c) => c.text ?? "").join("\n");
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    data = text;
  }
  line(`\n> ${name}(${JSON.stringify(args)})  ${res.isError ? "[isError]" : ""}`);
  line(typeof data === "string" ? data : JSON.stringify(data, null, 2));
  return { isError: !!res.isError, data };
}

let failures = 0;
function expect(cond, msg) {
  if (!cond) {
    failures++;
    line(`  !! ASSERTION FAILED: ${msg}`);
  } else {
    line(`  ok: ${msg}`);
  }
}

async function main() {
  await client.connect(transport);
  line("=== connected ===");

  const tools = await client.listTools();
  line(`\n=== tools (${tools.tools.length}) ===`);
  for (const t of tools.tools) line(`  - ${t.name}: ${t.description.split(".")[0]}.`);

  // 1. create
  const created = await call("takomo_new", {
    project: PROJECT,
    title: "MCP e2e lifecycle ticket",
    type: "task",
    priority: "high",
    body: "Created by the MCP e2e harness. Safe to delete.",
    labels: ["e2e"],
  });
  expect(!created.isError && created.data.ticket?.id, "created a ticket");
  const id = created.data.ticket.id;
  line(`  -> ticket id: ${id}, state: ${created.data.ticket.state}`);

  // 2. move brief -> spec -> ready so it enters the ready queue (factory-default)
  await call("takomo_transition", { id, to: "spec" });
  await call("takomo_transition", { id, to: "ready" });

  // 3. ready queue should include it
  const ready = await call("takomo_ready", { project: PROJECT });
  expect(!ready.isError && ready.data.items.some((t) => t.id === id), "ticket appears in ready queue");

  // 4. atomically claim next ready ticket
  const next = await call("takomo_next", { project: PROJECT });
  expect(!next.isError && next.data.claimed && next.data.lease?.fence !== undefined, "next claimed a ticket with a fence");

  // 5. illegal transition -> must relay store error + allowed_transitions
  const illegal = await call("takomo_transition", { id: next.data.ticket.id, to: "done" });
  expect(
    illegal.isError && Array.isArray(illegal.data.allowed_transitions),
    "illegal transition returns store error with allowed_transitions"
  );

  // 6. start work on the ticket we just claimed (fence auto-included)
  const started = await call("takomo_start", { id: next.data.ticket.id });
  expect(!started.isError && started.data.ticket?.state, `start moved ticket to '${started.data?.ticket?.state}'`);
  const workId = next.data.ticket.id;

  // 7. comment (fence not required)
  const commented = await call("takomo_comment", { id: workId, body: "e2e: working on it" });
  expect(!commented.isError && commented.data.comment?.id, "added a comment");

  // 8. advance toward done (implementing -> review), then done (review -> done)
  await call("takomo_transition", { id: workId, to: "review" });
  const done = await call("takomo_done", { id: workId });
  expect(!done.isError && done.data.ticket?.state === "done", "ticket reached done");

  // 9. whoami graceful fallback
  await call("takomo_whoami", {});

  line(`\n=== e2e complete: ${failures === 0 ? "ALL ASSERTIONS PASSED" : failures + " ASSERTION(S) FAILED"} ===`);
  await client.close();
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  line("FATAL: " + (err?.stack ?? err));
  process.exit(1);
});
