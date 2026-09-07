# Takomo agent service

A standalone, single-job worker for read-only section conversations, [document workspaces](#document-workspace-retrieval), [lane organization](#lane-organization), and [explicit bug research](#explicit-bug-research). It claims jobs from Takomo, runs a Codex App Server turn over stdio, and delivers the completed response. The same process can run beside Takomo, on a developer machine, or on another server; it only needs an outbound connection to Takomo and Codex's provider.

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
- Responses arrive after completion; the worker does not stream text, expose approval dialogs, migrate sessions or retry failed turns. Document requests can draft illustrative test code as reply text; the worker never writes or runs it.

## Check

Open **Agent queues** in Takomo's navigation (`/agent-queues`) to inspect recent jobs. Filter by project and status, then open a job to see its original section snapshot, prompt, reply or failure, conversation history, and worker/session identifiers. Counts and details refresh automatically while the page is open. A queued follow-up shows the worker its conversation is bound to. The page requires project read access and does not change jobs.

```sh
node --test services/agent/test/*.test.mjs
```

The fake JSON-RPC process tests thread start/resume, early IDs, final-answer filtering/deduplication, provider failure, process death, unsupported approvals, timeouts, result retry without model reexecution, lease-loss handling, and rejection of inherited tool configurations. To check the real integration, authenticate the dedicated home, queue a section grill in Takomo, run `--once`, then submit a follow-up and run it again. Reload Takomo to verify persisted messages. Stop a running service to verify lease expiry displays a failed job.

Protocol references: [Codex App Server](https://learn.chatgpt.com/docs/app-server), [Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference). For a CLI upgrade, regenerate its schema with `codex app-server generate-json-schema --out /tmp/codex-schema` and rerun these checks.

## Document conversations and section actions

`document_chat` jobs discuss a whole document or one or more selected sections.
These remain supported for compatibility; new workspace requests use the scoped
retrieval protocol below.
Actions are `discuss`, `grill`, `draft_tests` and `draft_questions`. Custom requests
use `discuss` with the user's prompt. Test cases, illustrative test code and
questions arrive as Markdown drafts in the conversation. No document, checklist,
question record or repository is changed, and tests are never run.

Each turn receives a fresh JSON snapshot with the document identity, action,
explicit scope and section ids/titles/notes. This replaces earlier snapshots as
current reference material while the resumed Codex thread retains conversation
history. The worker validates action and scope, keeps the no-tools/read-only
profile, and uses a dedicated document drafting policy rather than the older
section review policy that prohibits drafting tests. The same five-minute timeout,
64,000-byte reply limit, lease checks and result-only retries apply.

The worker advertises `supported_kinds` when claiming jobs. Upgrade this service
alongside Takomo to enable document requests: workers that omit this capability
continue receiving only the older section, research and organizer jobs. Pending
document requests remain queued until a compatible worker is available. Preserve
the service id and Codex state directory to resume attached document sessions;
Takomo stores messages in its database, while Codex's resumable thread state lives
in that worker's persistent directory. An attached session uses this service's
managed Codex thread, not an arbitrary terminal or desktop session.

An optional provider check runs two tiny document turns in a temporary Codex home:

```sh
TAKOMO_AGENT_DOCUMENT_LIVE_SMOKE=1 node --test services/agent/test/document-live-smoke.test.mjs
```

It privately copies the configured worker home's authentication, leaves that home
unchanged, and removes its temporary state afterward. It verifies a draft response,
conversation recall after process restart and a refreshed document deadline. It
uses the authenticated provider, but creates no Takomo jobs or records. Ordinary
tests skip it.

## Document workspace retrieval

`document_workspace` jobs use schema version 2 and three read-only dynamic tools:
`document_outline`, `document_search` and `document_read`. The tools operate only
on the immutable job snapshot already supplied by Takomo. They cannot access the
live database, files, repositories or networks. This profile enables only the
dynamic-tool host in addition to the existing restrictions; ordinary document
chat and lane organizer jobs retain their no-tools profiles. Unknown tools and
unsupported arguments fail closed.

Automatic mode searches titles, content and parent headings across the document,
prioritizing pinned sections. Selected mode restricts every tool to the selected
section ids plus explicit pins, including a pins-only selection. Whole-document
mode instructs Codex to page through the outline and read all authorized content.
Broad requests in automatic mode also require systematic review or explicit
partial-coverage disclosure. The initial prompt contains at most eight section
excerpts and fifty headings; subsequent tool calls retrieve further context.

Search is deterministic lexical retrieval, with case/accent folding, German `ß`
normalization, common German/English stopwords, heading/ancestor boosts and inverse
document frequency weights. There are no embeddings or external search services.
Results include ranked snippets and explicit match counts/truncation. Section
reads retain rich document XML for tables, code blocks and formatting. Offsets
count UTF-16 characters, and `next_offset` supports reading large sections without
silently dropping their tail.

Snapshots allow at most 500 sections and 8,000,000 UTF-8 bytes. Each turn has a
200-tool-call limit and a 750,000-byte cumulative retrieval budget, alongside the
existing five-minute deadline and 64,000-byte final response limit. Exhausting a
budget returns an explicit error and leaves coverage incomplete. Source evidence
records ids and versions actually delivered in initial context, search snippets
or reads; a separate coverage list records only sections read completely with no
gaps. Replies cite sources as `[Title](takomo-section:SECTION_ID)`. Takomo validates
and stores evidence against the job's immutable source revision before exposing
source links and coverage in the conversation.

`grill` asks one consequential question per turn and follows up on the answer.
`draft_tests` and `draft_questions` produce cited drafts, and `discuss` handles
ordinary or custom requests. No artifact is automatically applied or executed.

Existing text-only Codex threads need a one-time migration because the installed
App Server accepts dynamic tools only when starting a thread. For a server-marked
migration job, the worker reads the old thread, imports only visible user and
assistant text from recent completed turns (up to 200 KB), and starts a new tool
thread. Prior source wrappers, tool output, reasoning and commentary are excluded.
The worker reports old/new thread ids and retained/omitted turn counts before the
new turn; Takomo preserves all original conversation messages and records the
migration. Subsequent workspace turns resume the new persisted thread while tools
are rebound to each turn's fresh scoped snapshot.

**Deploy the new Takomo server first, then upgrade this worker.** Claims advertise
the distinct `document_workspace` kind so older workers cannot consume these
jobs. An older server rejects the new claim capability with HTTP 422; do not leave
a new worker polling it. Preserve the service identity, Codex home and existing
launcher/token when upgrading. The runtime module set now also includes
`document-workspace.mjs`.

The opt-in provider smoke proves legacy migration, actual use of all three tools,
source citations and a resumed turn reading refreshed content:

```sh
TAKOMO_AGENT_WORKSPACE_LIVE_SMOKE=1 node --test services/agent/test/workspace-live-smoke.test.mjs
```

It uses three small provider turns in a temporary Codex home with a private auth
copy, creates no Takomo jobs, and removes its state afterward. Default tests skip
it. Protocol fields were checked against the installed Codex App Server's
experimental JSON schema and the [official App Server documentation](https://learn.chatgpt.com/docs/app-server).

## Lane organization

Explicit organizer requests use the same queue and authenticated service with
`kind: "lane_organize"`. No repository mapping or checkout is needed. The service
starts Codex in its existing empty workspace with the section conversation's
read-only, **no-tools** profile. It does not enable the research tool host.

The server supplies a bounded JSON snapshot containing pending tickets, existing
lanes and their context, source references, and any included persisted
specification material. Pending tickets and existing lane membership are distinct:
the proposal assigns only the current snapshot's pending ticket ids. Persisted
specification content excludes unsaved editor changes; the organizer must not
describe this as a live document read. Projects can concern any subject. The
instructions prescribe no lane names, development phases or project outline.

The Codex `turn/start.outputSchema` field constrains the answer to:

```json
{
  "groups": [{
    "lane_id": null,
    "title": "A project-specific lane title",
    "purpose": "What this group is for",
    "context": "Prepared context and relevant references",
    "readiness": "needs_clarification",
    "reason": "The decision that remains unresolved",
    "ticket_ids": ["project-1"]
  }],
  "unassigned": [{
    "ticket_id": "project-2",
    "reason": "Possible duplicate of project-1; confirm before grouping."
  }]
}
```

An existing group uses its snapshot lane id and preserves its title and purpose
exactly. Every pending ticket appears once across groups and unassigned. Each
group includes a readiness reason; readiness is `ready` or
`needs_clarification`. Unclear placement and possible duplicate tickets remain
unassigned with specific reasons. Prepared context may enrich an existing lane,
but the service only proposes that change.

The worker independently validates JSON shape, UTF-8 byte limits, current
snapshot ids, complete/nonduplicated ticket coverage, and existing lane identity.
The server validates again before storing the proposal. Invalid JSON, prose
around JSON, invented ids, renamed existing lanes, unsupported fields, missing
tickets, or oversized output fail the job; the worker never silently repairs or
applies model output. Snapshots allow 512,000 bytes, 200 pending tickets and 100
existing lanes; proposals allow 256,000 bytes. The ordinary human-facing message
remains a short summary under the shared 64,000-byte limit.

Completed results send `proposal` alongside the normal `message`, session ids and
lease identity. No lane, ticket, context or handoff mutation is performed by this
service. A person reviews and applies the server-stored proposal separately;
`ready` does not dispatch implementation. The server supplies an isolated
project-organizer thread id for explicit follow-ups, preserving earlier grouping
discussion without mixing section conversation or bug research history. The
current snapshot always governs which ids and lane identities are legal.

The existing heartbeat, timeout and fenced result-delivery behavior applies.
Only delivery is retried, never a failed model turn. This first version has no
live organizer steering or cancellation controls; an explicit later request can
continue the organizer conversation after completion/failure. Section
conversation behavior and the separate bug research policy remain unchanged.

Focused checks (fake App Server only; no model calls):

```sh
node --test services/agent/test/organizer.test.mjs services/agent/test/service.test.mjs services/agent/test/research.test.mjs
```

`outputSchema` was verified against the locally generated Codex App Server
`v2/TurnStartParams.json` schema (`codex app-server generate-json-schema`).

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
