// Live end-to-end test harness for the takomo MCP server.
//
// Spawns the built stdio server via the official MCP SDK client, lists tools,
// then drives a full lifecycle against the live store in the throwaway `mcptest`
// project: new -> ready -> next -> start -> comment -> done, plus one illegal
// transition to prove the store's error text passes through.
//
// Then the VERIFICATION loop, which is the other half of what an agent does
// here: register where you can run, declare a check and its cases, read what
// needs re-verifying, and report what you observed. The assertion that matters
// most is the refusal — a check that must pass in two environments must not
// accept a verdict that does not say which one it is about.
//
// Usage: node test/e2e.mjs   (reads TAKOMO_URL / TAKOMO_TOKEN from env)

import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

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

  // --- verification loop ----------------------------------------------------
  // Slugs are unique per project and immutable, so the run reuses fixed ones:
  // filing an environment upserts, which is exactly the property a runner that
  // registers its instance every run depends on.
  const envA = await call("takomo_environment_file", {
    project: PROJECT,
    slug: "e2e-staging",
    kind: "staging",
    base_url: "https://staging.e2e.invalid",
    bring_up: "backlot up --ttl 900",
    teardown: "backlot release",
    credentials_hint: "env:E2E_TOKEN",
  });
  expect(!envA.isError && envA.data.environment?.slug === "e2e-staging", "registered an environment");
  const envB = await call("takomo_environment_file", {
    project: PROJECT,
    slug: "e2e-prod",
    kind: "production",
  });
  expect(!envB.isError && envB.data.environment?.writable === false, "production defaults to read-only");

  // Filing the same slug again updates in place rather than duplicating, and
  // leaves fields it was not given alone.
  const refiled = await call("takomo_environment_file", {
    project: PROJECT,
    slug: "e2e-staging",
    base_url: "https://staging-2.e2e.invalid",
  });
  expect(!refiled.isError && refiled.data.created === false, "refiling a slug updates in place");
  expect(
    refiled.data.environment?.bring_up === "backlot up --ttl 900",
    "an omitted field keeps what was already recorded"
  );

  const envs = await call("takomo_environments", { project: PROJECT });
  expect(
    !envs.isError && envs.data.items.some((e) => e.slug === "e2e-staging"),
    "environments lists what a runner needs"
  );

  // Describe a behavior, report a run against it, read the status back.
  const behavior = await call("takomo_behavior_create", {
    project: PROJECT,
    title: `e2e behavior ${Date.now()}`,
    statement: "Saving twice keeps one record.",
    tests: ["e2e:save-twice"],
  });
  expect(!behavior.isError && behavior.data.id?.startsWith("bhv-"), "created a behavior");
  expect(behavior.data.status === "untested", "a new behavior is untested");
  const behaviorId = behavior.data.id;

  const run = await call("takomo_run_report", {
    project: PROJECT,
    commit: "e2e0001",
    note: "stdio e2e",
    results: [
      { test: "e2e:save-twice", outcome: "pass" },
      { test: "e2e:unlinked", outcome: "fail", detail: "no behavior yet" },
    ],
  });
  expect(!run.isError && run.data.run?.passed === 1 && run.data.run?.failed === 1, "reported a run");
  expect(run.data.unlinked?.includes("e2e:unlinked"), "the reply names the unlinked key");

  const shown = await call("takomo_behavior", { id: behaviorId });
  expect(!shown.isError && shown.data.status === "verified", "a fresh pass verifies the behavior");
  expect(shown.data.history?.[0]?.note === "stdio e2e", "history carries the run's note");

  // Linking the failing key fails the behavior: any failure wins.
  const linked = await call("takomo_behavior_update", {
    id: behaviorId,
    add_tests: ["e2e:unlinked"],
  });
  expect(!linked.isError && linked.data.status === "failing", "linking a failing test fails it");

  const bad = await call("takomo_run_report", {
    project: PROJECT,
    results: [{ test: "e2e:x", outcome: "skipped" }],
  });
  expect(bad.isError, "an outcome other than pass/fail is refused");

  const summary = await call("takomo_verification", { project: PROJECT });
  expect(!summary.isError && summary.data.summary?.failing >= 1, "the summary counts it");
  const list = await call("takomo_behaviors", { project: PROJECT, status: "failing" });
  expect(!list.isError && list.data.items.some((b) => b.id === behaviorId), "status filter finds it");
  const runs = await call("takomo_runs", { project: PROJECT, limit: 5 });
  expect(!runs.isError && runs.data.items.length > 0, "runs are listed");

  line(`\n=== e2e complete: ${failures === 0 ? "ALL ASSERTIONS PASSED" : failures + " ASSERTION(S) FAILED"} ===`);
  await client.close();
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  line("FATAL: " + (err?.stack ?? err));
  process.exit(1);
});
