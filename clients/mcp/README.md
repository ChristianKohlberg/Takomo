# Takomo MCP server

A thin [Model Context Protocol](https://modelcontextprotocol.io) server that exposes the
Takomo HTTP API as native MCP tools, so agents (Claude Code, Codex, ...) can drive the
tracker directly instead of shelling out to a CLI. It is a **drop-in replacement for a
beads/beans-style MCP** for Takomo.

It is a pure client: it makes ordinary HTTP calls to an existing takomo server and does
not change or embed the server in any way.

## What it does

- Speaks MCP over **stdio** using the official TypeScript SDK.
- Covers **both loops**: the work loop (claim, transition, comment, done) and the
  verification loop (behaviors, linked tests, reported runs, environments).
- Wraps each tracker verb as one MCP tool returning compact JSON.
- `start` and `block` use the server's atomic REST operations, backed by the
  same Store transactions as hosted MCP. A refused transition leaves no new
  claim or blocker comment. This requires a Takomo server with
  `/v1/tickets/{id}/start` and `/v1/tickets/{id}/block`; upgrade the server before
  the client. There is no fallback to partially applying the operation.
- **Tracks claim fences in memory** for the life of the process, keyed by ticket id, so
  `start` / `transition` / `done` / `release` include the fencing token automatically. Pass
  an explicit `fence` argument to override.
- **Relays store errors verbatim.** On a 409/403 the tool result carries the store's own
  `message`, `remedy`, `current_state`, and `allowed_transitions`, so the agent can
  self-correct. It works for any workflow shape (`simple`, `factory-default`, ...) because
  convenience verbs resolve target states from the project's workflow definition by
  category, not by hard-coded names.
- Sends an explicit `User-Agent: takomo-mcp/0.1` on every request, which is required to
  get past the store's edge WAF (a default library User-Agent is served an HTML 403 block
  page). Any non-JSON response is surfaced as a transport error rather than parsed.

## Tools

| Tool | Purpose |
| --- | --- |
| `takomo_new` | Create a ticket (`project`, `title`, `type`, `priority`, `parent`, `labels`, `tags`, `body`). Auto Idempotency-Key; surfaces `similar` duplicates. |
| `takomo_list` | List tickets with filters (`project`, `state`, `type`, `priority`, `label`, `tag`, `tag_kind`, `limit`, `cursor`). |
| `takomo_ready` | List the ready queue (optionally by `project`). |
| `takomo_show` | Fetch one full ticket by `id` (plus any lease you hold). |
| `takomo_claim` | Claim a specific ticket by `id`; remembers the fence. |
| `takomo_next` | Atomically claim the next ready ticket (`project`/`type`/`priority` filters, optional `wait` seconds to poll). |
| `takomo_start` | Claim if needed, then move into the workflow's in-progress state (override with `to`). |
| `takomo_transition` | Move a ticket to an explicit `to` state (fence auto-included). |
| `takomo_done` | Move to the workflow's terminal `done` state. |
| `takomo_block` | Move to the workflow's blocked state; optional `comment` recorded first. |
| `takomo_cancel` | Move to the workflow's cancelled terminal state. |
| `takomo_comment` | Add a comment (`id`, `body`). |
| `takomo_link` | Attach/update a named link (`key`, `value`), merging with existing links. |
| `takomo_tag` | Tag entities onto a ticket — `add`/`remove` `kind:handle` refs (e.g. `person:ada`); reference metadata only. |
| `takomo_dep` | Record that a ticket is `blocked_by` another. |
| `takomo_release` | Release your claim, echoing the fence; clears the remembered fence. |
| `takomo_projects` | List projects and their workflows. |
| `takomo_workflow` | Show a project's workflow (states/categories/transitions). |
| `takomo_whoami` | Identify the token holder if `/whoami` exists; graceful note if not. |

### Verification — does the software still do what it should?

A **behavior** is what the software must do, in words a person can check. Tests
are external: Takomo stores only the key a runner reports for each, and a **run**
is one report of pass/fail per key against a commit. A behavior is `failing` if
any linked test's latest result failed, `verified` if one passed within
`fresh_days` (14), `stale` if its passes are older, and `untested` otherwise. See
[docs/verification.md](../../docs/verification.md).

| Tool | Purpose |
| --- | --- |
| `takomo_verification` | Status counts overall and per plan section, plus reported tests no behavior links. Start here. |
| `takomo_behaviors` / `takomo_behavior` | List behaviors (filter by `section`, `status`, `q`), or show one with each linked test's latest result and recent history. |
| `takomo_behavior_create` | Describe a behavior, optionally tied to a plan section and the test keys that verify it. Restate the specification; do not invent requirements. |
| `takomo_behavior_update` | Edit it, move it to a section, or replace its linked test keys. |
| `takomo_run_report` | Report one run: commit, a note on why these tests, and pass/fail per key. Retry-safe via an idempotency key. |
| `takomo_runs` | Reported runs, newest first, with pass/fail counts. |
| `takomo_environments` / `takomo_environment_file` | Where the software runs, and registering an instance you just stood up. `credentials_hint` is a **pointer**, never a credential. |

To verify a behavior by hand when no automated test exists, link a key such as
`agent:failed-save-retry` and report results against it like any other test.

The bug tools (`takomo_bugs`, `takomo_bug`, `takomo_bug_update`, `takomo_bug_research`,
`takomo_bug_runs`, `takomo_bug_run`, `takomo_bug_steer`, `takomo_bug_cancel`,
`takomo_bug_research_config`) are described in [docs/bugs.md](../../docs/bugs.md).

## Install & build

Requires Node.js >= 20 (TypeScript MCP SDK v2).

```bash
cd clients/mcp
npm install
npm run build   # compiles TypeScript to dist/
```

This produces `dist/index.js`, the stdio entrypoint.

## Environment

| Variable | Required | Default |
| --- | --- | --- |
| `TAKOMO_TOKEN` | yes | — (bearer token, `tk_...`) |
| `TAKOMO_URL` | no | `https://your-takomo-host.onrender.com/v1` |

## Wiring: Claude Code

From a machine where you built `dist/`, add the server (replace the absolute path and
token):

```bash
claude mcp add takomo \
  --env TAKOMO_URL=https://your-takomo-host.onrender.com/v1 \
  --env TAKOMO_TOKEN=tk_your_token_here \
  -- node /absolute/path/to/clients/mcp/dist/index.js
```

Or add it to a project's `.mcp.json`:

```json
{
  "mcpServers": {
    "takomo": {
      "command": "node",
      "args": ["/absolute/path/to/clients/mcp/dist/index.js"],
      "env": {
        "TAKOMO_URL": "https://your-takomo-host.onrender.com/v1",
        "TAKOMO_TOKEN": "tk_your_token_here"
      }
    }
  }
}
```

Verify with `claude mcp list` (should show `takomo: ... - ✓ Connected`), then in a session
the tools appear as `takomo_new`, `takomo_next`, etc.

## Wiring: Codex

Codex reads MCP servers from `~/.codex/config.toml`:

```toml
[mcp_servers.takomo]
command = "node"
args = ["/absolute/path/to/clients/mcp/dist/index.js"]

[mcp_servers.takomo.env]
TAKOMO_URL = "https://your-takomo-host.onrender.com/v1"
TAKOMO_TOKEN = "tk_your_token_here"
```

## Test (live)

`test/e2e.mjs` spawns the built server through the MCP SDK client and drives a full
lifecycle against the live store in a throwaway project (`mcptest` by default):

```bash
npm run build
TAKOMO_TOKEN=tk_your_token_here npm test
```

It initializes, lists tools, then runs new -> ready -> next -> start -> comment -> done, plus
one deliberately illegal transition to confirm the store's error text passes through.
Override the project with `TAKOMO_TEST_PROJECT`.

## Notes

- The `wait` option on `takomo_next` is implemented client-side (polls the atomic
  ready/claim every ~2s up to `wait` seconds).
- Fences live only in this process. A fresh server process starts with no remembered leases;
  pass an explicit `fence`, or re-`claim`, after a restart.

Lane tools let organizing agents create lanes, retain preparation context, associate
existing tickets, and draft immutable handoffs. `takomo_lane_handoff` never executes
work; an authorized person dispatches the draft separately. Read returned review
findings using `takomo_lane_handoffs`. See [lanes and review handoffs](../../docs/lanes.md).

## Isolated client tests (also required in CI)

From the repository root:

```sh
(cd clients/mcp && npm ci && npm run build)
cargo test --release --test mcp_clients -- --ignored
```

The Rust harness starts temporary stores and real HTTP servers, then runs the
existing stdio lifecycle/verification suite and shared hosted/stdio regression
scenarios. It supplies scoped test credentials without an external deployment.
The tests cover rejected transitions, rollback, competing claim holders, fence
reuse, and SDK protocol compatibility. They are explicitly ignored by ordinary
`cargo test` so Rust-only builds do not require Node; the MCP CI job runs them.
