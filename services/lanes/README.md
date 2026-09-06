# Lane handoff worker

This optional local Node 22 worker consumes explicitly dispatched lane handoffs
from Takomo and runs **Codex CLI or Claude Code** in a configured Git checkout.
It is separate from `services/agent`, whose section conversations remain
read-only and tool-free. Starting this worker does not create or dispatch work.

| Handoff | Local behavior | Saved outcome |
| --- | --- | --- |
| Preparation | Reads the selected ticket snapshot and workspace; cannot edit files | Suggested grouping, scope, dependencies, acceptance criteria and questions as Markdown lane context |
| Implementation | Edits a dedicated clean checkout, verifies changes, creates a local commit | Summary, actual commit revision and provider conversation reference |
| Review | Fresh provider session, exact clean target commit, supplied commit diff; cannot edit files | Findings tied to that revision and routed by Takomo back to the source implementation lane |

Preparation enriches **lane context**; it does not automatically rewrite ticket
bodies or rearrange project records. A person reviews that context and explicitly
dispatches implementation. Review findings likewise do not automatically start
another implementation. Lane names and purposes are project-defined.

## Set up

Install Node 22+, Git and the provider CLIs you intend to enable. Adapter flags
were checked against **Codex CLI 0.153.4** and **Claude Code 2.1.263**. Claude
implementation requires a working native sandbox (on Linux, `bubblewrap` and
`socat`); unavailable sandboxing fails the run instead of falling back to
unrestricted commands. Linux, macOS and WSL2 are supported; native Windows is not.

Create a **dedicated checkout on a local work branch** for each project. A Git
worktree is suitable. The mapping must name the checkout root, not a subdirectory.
Do not map your active personal checkout or a checkout shared with another
service. Implementation refuses `main`, `master`, detached HEAD, untracked files,
or pending changes at start. Preparation and review also require a clean checkout.

```sh
git worktree add /srv/takomo-work/takomo -b lanes/takomo
mkdir -p /srv/takomo-worker/codex /srv/takomo-worker/claude
```

Give each provider a **dedicated home outside the checkout**, then authenticate
locally using the same home settings the worker uses:

```sh
HOME=/srv/takomo-worker/codex CODEX_HOME=/srv/takomo-worker/codex codex login
HOME=/srv/takomo-worker/claude CLAUDE_CONFIG_DIR=/srv/takomo-worker/claude claude auth login
```

Provider API keys may instead be supplied in the worker environment:
`OPENAI_API_KEY`/`CODEX_API_KEY` for Codex,
`ANTHROPIC_API_KEY`/`CLAUDE_CODE_OAUTH_TOKEN` for Claude. Provider authentication
stays on this worker machine. Never put it in lane context, ticket bodies,
handoff instructions or the config JSON. The worker's Takomo credential is never
passed to either CLI, Git, or repository hooks.

Write a local config, for example `/srv/takomo-worker/lanes.json`:

```json
{
  "projects": {
    "takomo": "/srv/takomo-work/takomo"
  },
  "providers": {
    "codex": {
      "executable": "codex",
      "home": "/srv/takomo-worker/codex"
    },
    "claude": {
      "executable": "claude",
      "home": "/srv/takomo-worker/claude"
    }
  }
}
```

Omit a provider to leave its handoffs for another worker. An optional `model`
string selects a model supported by that locally installed CLI; otherwise the
provider's default applies. Executables and workspace paths are local operator
configuration, never taken from handoff text. Do not put command arguments in
`executable`.

Use a dedicated Takomo token scoped to the mapped projects with
`read,write,agent:run`. Set it through your process manager's secret environment;
do not put the value into the example config or commit it to Git.

```sh
export TAKOMO_URL=https://takomo.example.com
export TAKOMO_LANE_CONFIG=/srv/takomo-worker/lanes.json
export TAKOMO_LANE_STATE_DIR=/srv/takomo-worker/state
# Set TAKOMO_LANE_TOKEN using your local secret manager.
node services/lanes/worker.mjs
```

`--once` scans the configured queues and executes at most one claimable handoff.
Without it, the worker polls every three seconds when idle. The default provider
timeout is 30 minutes; `TAKOMO_LANE_TIMEOUT_MS` accepts 1,000–86,400,000 ms.
The worker has no dependency installation step and uses Node's built-in modules.
Configure the supervisor to stop the whole process group/cgroup on shutdown.

## Execution and review boundaries

The worker sends the draft-time ticket and lane snapshot through stdin, never
through a shell command. Codex uses `read-only` for preparation/review and
`workspace-write` for implementation, with approvals disabled and no tool
network access. Claude preparation/review expose only `Read,Glob,Grep`; they
cannot run shell commands, edits or writes. Claude implementation enables
sandboxed Bash and file editing, with the unsandboxed escape hatch disabled.
Provider hooks, MCP configuration, browser integrations and user customization
are disabled for these runs. The existing section-agent policy is unchanged.

Implementation changes are committed locally by the worker so the result's
revision identifies the actual code. The checkout must initially have no tracked
or untracked changes, and its branch and HEAD must remain unchanged until the
worker creates the sole handoff commit. Provider-created commits are rejected
without discarding them, so the reviewed commit covers the entire handoff.
An implementation with no edits gets an empty commit, ensuring its review does
not accidentally inspect an unrelated earlier change.
Git hooks **run**;
a failed hook fails the handoff and leaves the checkout for local diagnosis.
Commit signing is disabled for this one unattended commit. The worker does not
push, merge, deploy or configure remotes. Dependencies must already be available
locally because agent tool network access is disabled. Results should state
verification limits when a test needs unavailable dependencies or services.

For review, the full target commit hash must equal the clean checkout's `HEAD`.
The worker never checks out a revision or discards changes automatically. It
supplies `git show` for that exact commit (against its parent), capped at 120 KB
with a visible truncation notice, and the reviewer can inspect source files.
This reviews the implementation commit, not an unspecified branch-wide diff.
The worker verifies that the checkout remained clean and unchanged afterward.
Read-only review cannot run tests which write build files; implementation test
evidence and review coverage limitations belong in the reported findings.

Implementation may resume the last locally recorded session for the same
project, lane, checkout and provider, preserving context when a human sends back
findings. A provider-prefixed snapshot reference (`codex:<UUID>` or
`claude:<UUID>`) is usable only if this worker previously recorded it as an
implementation session for this project and checkout. A foreign, missing or
preparation-only reference starts fresh using the snapshot. Review **always**
starts fresh, including when it uses the same provider as implementation. If a
recorded provider session was deleted locally, the CLI failure is visible; there
is no hidden second model execution.

The workspace is not a hostile multi-tenant container. Run this worker under a
dedicated OS account containing only intended project/provider credentials, and
use trusted project hooks. Do not edit the checkout concurrently outside the
worker. Local CLI sandboxes and tool limits constrain model execution; the
worker's local Git process must still be able to create the authorized commit.

## Leases, cancellation and recovery

The server owns dispatch authorization. The worker reads
`GET /v1/projects/{project}/handoffs?status=ready&limit=100&offset=…`, selects an
enabled provider, and claims with `POST /v1/handoffs/{id}/claim {}`. Ready includes
expired running handoffs. Queue scans follow bounded pages; if the queue changes
while scanning, the next poll starts at the beginning.

Claims return an `attempt` fence and a 120-second lease. Heartbeats every 20
seconds and final results send that exact attempt. Any heartbeat failure stops
the provider and its child command process group. SIGINT/SIGTERM and the provider
timeout also stop execution. Server cancellation reaches an active provider on
the next heartbeat (normally within 20 seconds, plus the 10-second HTTP timeout);
cancellation does not undo edits already made. Stale attempts never submit a
result. Inspect partial work before retrying an interrupted implementation.

Successful outcomes are saved in the private state directory before HTTP
delivery. A reclaimed lease for the same handoff reports the recorded outcome
with its new fence; it does not rerun a completed model turn. Delivery retries
are bounded. An explicit retry is a **new handoff**, preserving prior history.
Keep the state directory across restarts; deleting it loses delivery receipts
and session ownership records. Provider stderr is drained without putting
credentials or diagnostic dumps into the shared handoff result.

An atomic `takomo-lanes.lock` directory inside each checkout's Git directory
prevents two local workers using that checkout simultaneously. Normal exits
release it. After a crash the lock deliberately remains: first stop any surviving
provider commands, inspect partial changes, then remove only that lock directory
before restarting. The worker never steals a lock based only on PID age and
never resets or cleans a dirty checkout.

## Checks

```sh
cd services/lanes
npm test
```

Tests use fake Codex/Claude executables and real temporary Git repositories.
They cover output protocol handling, stdin and environment isolation, read-only
tool policies, timeout/process-group cancellation, stale fences, delivery replay,
local commits and failing hooks, exact-revision review, scoped conversation reuse,
workspace locking and queue pagination. They execute no model calls.

Provider references: [Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
and [Claude Code sandboxing](https://code.claude.com/docs/en/sandboxing). Local
CLI `--help` is the source for the installed version's supported flags.
