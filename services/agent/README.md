# Takomo agent service

A standalone, single-job worker for read-only section conversations. It claims jobs from Takomo, runs a Codex App Server turn over stdio, and delivers the completed response. The same process can run beside Takomo, on a developer machine, or on another server; it only needs an outbound connection to Takomo and Codex's provider.

## Start

Requires Node.js 22+ and Codex CLI. The protocol and restrictive configuration were checked with Codex **0.153.4**. No npm dependencies are needed.

1. Choose a **dedicated persistent service state directory**. It contains the service ID, Codex authentication and conversation state, and an empty workspace. Do not point it at an existing repository or your normal Codex home.
2. Authenticate Codex into that dedicated home using its supported login flow:

   ```sh
   mkdir -p "$HOME/.takomo-agent/codex"
   CODEX_HOME="$HOME/.takomo-agent/codex" codex login
   ```

   For a headless host, use the login method supported by your Codex installation (`codex login --help`). Authentication is explicit: this service does not copy credentials or inherit provider API keys from the launching shell. Retain the Codex state directory across restarts. Protect it as credentials and conversation content.
3. On the Takomo host, mint a token restricted to the intended project (use your actual database path and project slug):

   ```sh
   takomo --db /path/to/takomo.db token create --actor agent:section-review --scopes agent:run --projects my-project
   ```

   The token has queue access only. It cannot edit specifications, tickets, or tests. Then start the service:

   ```sh
   export TAKOMO_URL=http://127.0.0.1:3000
   export TAKOMO_AGENT_TOKEN='<agent:run token>'
   node services/agent/service.mjs
   ```

Use `--once` to claim at most one immediately available job and exit after its result is delivered. No queued job is a successful no-op. Model or Codex configuration failures are delivered as failed jobs and exit successfully after delivery. Startup configuration and unrecoverable queue/result transport failures exit nonzero.

| Setting | Default |
| --- | --- |
| `TAKOMO_URL` | Required; HTTPS except on loopback |
| `TAKOMO_AGENT_TOKEN` | Required; token with `agent:run` scope |
| `TAKOMO_AGENT_STATE_DIR` | `~/.takomo-agent` |
| `TAKOMO_AGENT_SERVICE_ID` | Random stable ID saved in the state directory |
| `TAKOMO_CODEX_BIN` | `codex` on PATH |

The service's persisted ID and Codex state belong together. Run only one service process against a state directory. Changing the identity/home does not migrate existing conversations. A system supervisor can start this command, provide the environment, and restart it after a crash; no inbound worker port is needed.

## Behavior and boundaries

- Jobs are claimed sequentially with 25-second long polling. Idle/transient connection failures use bounded backoff. Invalid worker credentials stop the process.
- Session/thread and turn IDs are sent to Takomo as soon as they exist, before the answer. Follow-ups resume the saved thread. Only final user-facing assistant text is returned; commentary and reasoning are excluded.
- A heartbeat renews the job lease every 15 seconds. Losing a heartbeat stops Codex, preventing the worker from continuing after losing ownership. Takomo marks expired jobs failed. Turns are never automatically rerun; result delivery alone may be retried, using the same attempt ID and payload.
- Codex starts in an empty workspace with a dedicated HOME/CODEX_HOME and a small environment allowlist. The Takomo token and parent process secrets are not passed to Codex. Read-only sandbox policy and disabled network access apply to each turn. Shell execution, apps, plugins, hooks, browser/computer tools, images, multi-agent tools, code mode, and web search are disabled. The process is started for one job kind: a research process additionally enables Codex's dynamic-tool host (`features.code_mode_host`), which is what lets the declared repository tools execute at all, while a section process keeps it disabled and is refused a research job. Before starting a thread, effective configuration is checked against that kind's profile: inherited MCP servers/plugins, a feature that differs from the profile, relaxed permissions, and custom instruction files/notify hooks fail closed. Unsupported tool/approval requests fail the turn.
- No document API, edit tool, or test creation tool is exposed. Section text is supplied as review material. The worker has a five-minute turn timeout and a 64,000-byte response limit, and SIGINT/SIGTERM stop the active Codex process. An interrupted job is resolved by lease expiry.
- This MVP returns the response after completion; it does not stream text, expose approval dialogs, migrate sessions, retry failed turns, or generate code.

## Check

Open **Agent queues** in Takomo's navigation (`/agent-queues`) to inspect recent jobs. Filter by project and status, then open a job to see its original section snapshot, prompt, reply or failure, conversation history, and worker/session identifiers. Counts and details refresh automatically while the page is open. A queued follow-up shows the worker its conversation is bound to. The page requires project read access and does not change jobs.

```sh
node --test services/agent/test/*.test.mjs
```

The fake JSON-RPC process tests thread start/resume, early IDs, final-answer filtering/deduplication, provider failure, process death, unsupported approvals, timeouts, result retry without model reexecution, lease-loss handling, and rejection of inherited tool configurations. To check the real integration, authenticate the dedicated home, queue a section grill in Takomo, run `--once`, then submit a follow-up and run it again. Reload Takomo to verify persisted messages. Stop a running service to verify lease expiry displays a failed job.

Protocol references: [Codex App Server](https://learn.chatgpt.com/docs/app-server), [Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference). For a CLI upgrade, regenerate its schema with `codex app-server generate-json-schema --out /tmp/codex-schema` and rerun these checks.

## Explicit bug research

The same queue and local service handle `bug_research` jobs. Creating a bug does
not start a job: a human or agent explicitly requests research through Takomo.
Every bug remains a normal ticket. Research produces evidence for review; it does
not change the ticket's workflow, implement fixes, or claim runtime reproduction.

Configure the repositories this worker may inspect before starting it:

```sh
export TAKOMO_AGENT_REPOSITORIES='{"takomo":"/absolute/path/to/Takomo"}'
node services/agent/service.mjs
```

Keys correspond to the job's `repository_ref.repository` (the project's configured
repository key). Paths belong to worker configuration, never ticket or steering
input. A missing key fails the job visibly. `repository_ref.revision` defaults to
`HEAD` and resolves once, on the worker, to an exact commit before inspection.
Uncommitted files are excluded. The worker heartbeats the resolved commit before starting the Codex session and
includes partial inspection evidence in subsequent heartbeats, preserving progress
if the process dies. Evidence is capped at 48 KB with an explicit truncation flag.
Results persist that commit, the original ticket
snapshot, and inspected file/line references; the server retains every attempt.
A deliberate retry is a new run and may resolve a newer HEAD.

Research uses **one lead, zero helpers**, with a **15-minute turn deadline**, a
100-call tool budget, and the existing 64 KB answer limit. It exposes three Codex
App Server dynamic tools: `repository_files`, `repository_search`, and
`repository_read`. They perform bounded Git object reads at the pinned revision:
file lists cap at 200, literal search matches at 100, file reads at 200 lines / 24 KB,
and files at 1 MB. Totals/truncation are explicit. Symlinks and submodules cannot
be followed. Git subprocesses have a 10-second timeout and 2 MB output limit.
No checkout is created and no repository scripts, tests, shell commands, network
requests, or modifications can be requested by the model. Repository content is
research material, including any instruction files. Section conversation jobs keep
their original text-only restrictions and five-minute limit.

Steering received through job heartbeats uses `turn/steer` against the active turn;
cumulative steering IDs are delivered once per worker attempt. Cancellation uses
`turn/interrupt` and closes the owned process; lease loss also closes it. Failure,
timeout, and cancellation never automatically reexecute the model. Transport-only
result delivery retries reuse the same payload. A supervisor restart claims new
work; expired active attempts remain failed and need an explicit retry.

The integration launches the installed `codex app-server --stdio` locally; it
requires no listener or assumed port and does not attach to or interrupt another
application's session. Continue using the dedicated authenticated Codex home above.
Dynamic tools require experimental protocol capability negotiation, which this
worker enables only for research jobs. Configuration or protocol incompatibility
fails visibly. See the [official App Server protocol](https://learn.chatgpt.com/docs/app-server).

Focused fake-server tests also cover committed code evidence, dirty-file exclusion,
repository allowlists, symlink rejection, live steering, cancellation, timeouts,
the research-versus-section configuration profiles and unchanged section behavior.
They do not call a paid model or require credentials. An opt-in smoke
(`TAKOMO_AGENT_LIVE_SMOKE=1 node --test services/agent/test/live-smoke.test.mjs`)
runs the installed, authenticated Codex against a disposable committed fixture and
fails unless source was actually retrieved through the repository tools.
