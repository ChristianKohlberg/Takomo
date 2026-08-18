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

  // A check that must pass in BOTH, so the ambiguity rule has something to bite.
  const check = await call("takomo_check_file", {
    project: PROJECT,
    title: `e2e check ${Date.now()}`,
    layer: "api",
    severity: "advisory",
    environments: ["e2e-staging", "e2e-prod"],
    globs: ["src/e2e/**"],
  });
  expect(!check.isError && check.data.check?.id, "filed a check");
  expect(check.data.check?.environments?.length === 2, "the check declares both environments");
  const checkId = check.data.check.id;

  const filed = await call("takomo_cases_file", {
    check: checkId,
    cases: [
      { key: "n=1", label: "one", assignment: { n: 1 } },
      { key: "n=2", label: "two", assignment: { n: 2 } },
    ],
  });
  expect(!filed.isError && filed.data.live === 2, "filed two cases");

  const shown = await call("takomo_check", { id: checkId, cases: true });
  expect(!shown.isError && shown.data.cases?.length === 2, "check shows its cases");
  const caseId = shown.data.cases[0].id;
  expect(
    shown.data.cases[0].environments?.length === 2,
    "each case carries a reading per declared environment"
  );

  // The refusal: two environments declared, so a bare verdict does not say what
  // was observed. Filing a staging run as production would be worse than no
  // record, which is why this is an error rather than a default.
  const ambiguous = await call("takomo_verdict", { case: caseId, verdict: "pass" });
  expect(
    ambiguous.isError && ambiguous.data.code === "conflict.environment_ambiguous",
    "a verdict that does not say where is refused when that is ambiguous"
  );

  const scoped = await call("takomo_verdict", {
    case: caseId,
    verdict: "pass",
    environment: "e2e-staging",
  });
  expect(!scoped.isError, "a verdict naming its environment is recorded");
  expect(
    scoped.data.case?.state === "never",
    "one environment passing does not verify the case while the other is untouched"
  );

  // `fail` without a note is refused: a failure nobody described is one nobody
  // can act on.
  const bareFail = await call("takomo_verdict", {
    case: caseId,
    verdict: "fail",
    environment: "e2e-prod",
  });
  expect(bareFail.isError, "a fail with no note is refused");

  const worklist = await call("takomo_worklist", { project: PROJECT });
  expect(!worklist.isError && worklist.data.agent, "worklist splits by who can clear it");
  const mine = (worklist.data.agent.items ?? []).filter((i) => i.check === checkId);
  expect(mine.length > 0 && mine.every((i) => i.environment_slug), "every item says where to run it");

  await call("takomo_coverage", { project: PROJECT });
  const gate = await call("takomo_gate", { project: PROJECT });
  expect(!gate.isError && typeof gate.data.blocked === "boolean", "the gate answers can-this-ship");

  // One last read, so the run also exercises the list route it started from.
  // Nothing is torn down: a check and its verdicts are the record of what was
  // verified, and the environment slugs are reused by the next run.
  await call("takomo_checks", { project: PROJECT });

  line(`\n=== e2e complete: ${failures === 0 ? "ALL ASSERTIONS PASSED" : failures + " ASSERTION(S) FAILED"} ===`);
  await client.close();
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  line("FATAL: " + (err?.stack ?? err));
  process.exit(1);
});
