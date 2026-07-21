# Build architecture: a central task store for agent fleets

Premise: we build our own hosted task manager (Option C from [00-synthesis.md](00-synthesis.md)). What shape, and what breaks in reality with 8–64 concurrent agents.

## Guiding insight

64 agents is a **trivial load** (single-digit requests/second). Throughput and horizontal scale are non-goals; every real problem is *semantic*: atomic claiming, lease lifecycle, optimistic concurrency, status drift, and LLM compliance. Optimize for correctness under contention and for agent-legible errors, not for QPS.

## Architecture

**One single-binary HTTP service + one database. Nothing else.**

```
agents/orchestrators ──REST/JSON (bearer token)──▶ ┌──────────────┐
agents (MCP adapter) ──────────────────────────────▶ │  takomo   │──▶ SQLite (WAL) + Litestream
humans (thin CLI/web) ─────────────────────────────▶ │  single bin  │      (or Postgres backend)
orchestrators ◀──SSE event stream (?since=seq) ──── └──────────────┘
```

- **Server**: Go (or TS) single static binary. Compact REST/JSON — no HAL, no GraphQL. Systemd + Tailscale/reverse-proxy TLS. MCP server and CLI are thin adapters over the same REST API; one write path only.
- **Storage**: SQLite in WAL mode default (single file, single writer = trivially correct serialization; Litestream for continuous backup). Postgres optional backend (JSONB queries, LISTEN/NOTIFY, `FOR UPDATE SKIP LOCKED`) — same API, config choice.
- **Auth**: bearer tokens per actor (agent, orchestrator, human), hashed at rest. Token = identity for attribution and per-token rate limiting. Scopes: read-only, project-scoped, admin.

## Data model

- `ticket`: prefixed short id, `project`, `type` (epic|task|subtask|bug|…), `parent_id` (single-parent tree, beans-style), `title`, `body` (markdown), `priority`, `labels[]`, `metadata` (freeform JSON, namespaced keys — beads-style extension point), `state`, `claim` (holder, lease_expiry, fence), timestamps, `created_by`.
- `dependency`: `blocked_by` edges, separate from hierarchy. Cycle-checked. Drives the ready queue (unblocked = no open blockers, no open blocking ancestors).
- `event`: append-only log — `(global_seq, ticket_id, actor, kind, payload, at)`. Serves as audit trail, SSE feed, and poll cursor (`GET /events?since=<seq>`). Server-assigned sequence, never client timestamps.
- `comment`: append-only, separate from body (commutative writes).

## The state machine (the differentiator)

- Per-project workflow definition: named states mapped to fixed **categories** (`todo`, `in_progress`, `blocked`, `review`, `done`, `cancelled`) so tooling can reason generically while projects customize.
- Explicit transition table + guards: cannot enter `done` with open children; cannot enter `in_progress` without holding the claim; `review → done` may be reserved to tokens with a `merge` scope.
- **Server-enforced, and rejections teach**: illegal transition → 409 with machine-readable body: current state, attempted transition, `allowed_transitions[]`, and remedy (e.g. "claim first: POST /tickets/{id}/claim"). Prior art: OpenProject's 422 + allowedValues. Error responses are written for LLM readers — this is the feature no incumbent has.

## Claiming and leases

- `POST /ready/claim?project=&labels=&type=` — **atomically** pop the next ready ticket matching the filter and lease it to the caller. One operation; never read-then-claim. (SQLite: single-writer transaction; Postgres: `FOR UPDATE SKIP LOCKED`.)
- Lease = holder + TTL + **fencing token** (monotonic per-ticket counter). Mutations while claimed must present the current fence; a zombie returning after lease expiry bounces with 409 + explanation.
- `POST /tickets/{id}/heartbeat` renews. **Renewal is the harness/orchestrator's job, not the model's** — ship a sidecar/wrapper recipe, don't hope the LLM remembers.
- Expired lease → ticket returns to ready, event emitted, previous holder recorded (duplicate-work forensics).

## API ergonomics for agents

- Most writes **commutative and CAS-free**: add comment, set single field, append checklist item, attach metadata key. Whole-body edits use ETag If-Match CAS — enforceable atomically because there is exactly one authority (fixes the beans #205 class of race by construction).
- **Idempotency keys** on all creates (agents retry on timeout; no twin tickets).
- Create-time dedup hint: response includes `similar[]` (title/keyword match) so an agent can notice an existing ticket.
- `GET /ready` (peek) vs `POST /ready/claim` (take). Filters: project, type, labels, priority.
- Compact JSON everywhere; pagination by cursor; `?fields=` sparse responses (token economy).
- Versioned API (`/v1/`), additive evolution only; metadata JSON absorbs experiments before they become schema.

## Reality check: what actually goes wrong at 8 → 64 agents

1. **Ready-queue stampede.** All idle agents race for the same top item. Mitigation: claim-as-atomic-pop (above); optionally jittered polling; SSE nudges instead of tight polls.
2. **Zombie claimants (the hard one).** Agents die constantly — context exhaustion, killed panes, crashes. Short TTLs steal tickets from slow-but-alive agents (→ duplicate PRs); long TTLs leave tickets stuck. Mitigations: harness-owned heartbeats, fencing tokens so stale writers bounce, `reclaimed_from` audit trail, and orchestrator policy for "previous holder produced partial work."
3. **Lost updates.** Two writers, one ticket. Mitigation: commutative write surface + CAS only for rare whole-body edits; conflict errors that include the current ETag and a diff hint.
4. **Retry duplicates.** Timeouts + client retries → twin tickets without idempotency keys; semantic duplicates because agents don't search first → search endpoint + `similar[]` hints (mitigable, not solvable).
5. **Status drift vs. reality.** Store says done, PR unmerged; store says in_progress, worktree deleted. The DB is a claim about the world, not the world. Mitigations: first-class `links` (branch, PR URL, run id) on tickets; GitHub webhook receiver that auto-transitions on PR merge/close; documented orchestrator reconciliation loop. Drift management is a permanent operational duty, not a bug to fix once.
6. **LLM non-compliance.** Agents attempt illegal transitions constantly (skip review, close epics with open children). The error path is the *common* path — invest in teaching errors, expect and measure thrash (agents retrying rejected transitions in a loop → per-token rate limits double as circuit breakers).
7. **Garbage accumulation.** Thousands of stale tickets, comment spam, bloated metadata within weeks. Performance survives; human legibility dies. Mitigations: archive category, retention policy for cancelled/done, size caps (body, metadata value, comment count), `similar[]` dedup pressure.
8. **Runaway clients.** A looping agent hammering the API burns tokens and floods events. Per-token rate limits + anomaly flag ("token X made 500 writes in 10 min").
9. **SPOF discipline.** Single source of truth = single point of failure. Keep the server boring; Litestream/pg streaming backup; documented client behavior when store is down (degrade to read-only cache; never invent local truth; queue nothing client-side).
10. **Fleet-safe evolution.** Never coordinate 64 clients for a breaking change: `/v1` frozen, additive fields only, experiments in metadata namespaces.

**8 vs 64:** identical architecture. At 8 you can be sloppy about all ten and rarely notice; at 64 stampedes, zombies, duplicates, and spam are weekly events. Build the semantics correctly at 8.

## What to steal from the field

- **beans**: prime-prompt pattern, exact-match body edits, `--json` everywhere, archive-as-memory, small single-parent data model.
- **beads**: arbitrary JSON metadata as the extension point, dependency-driven `ready`, atomic `claim`, hierarchical ids.
- **saltbo/agent-kanban**: enforced operation verbs (claim/review/complete/reject/release), per-agent identity.
- **OpenProject**: machine-readable allowed-transitions in rejections.
- **Redmine lesson (inverted)**: never silently ignore an illegal write.

## Stack decision (2026-07-19)

**Language: Rust.** axum + tokio single static binary; `rmcp` (official Rust MCP SDK) for the MCP adapter; tower middleware for auth/rate limiting; utoipa for OpenAPI generation.

**Database: SQLite (or libsql) behind a repository trait; Postgres as optional later backend.**

Considered and rejected:
- **Dolt** — its superpower is divergence (offline clones, branches, cell-level merge); our premise is one always-on authority that never diverges, so we'd pay its costs and use none of its powers. History/audit comes from our event log already. From Rust it's a second Go process over MySQL wire protocol — kills single-binary deployment and inherits the latency/TLS warts documented in [04-beads-server-mode.md](04-beads-server-mode.md). If Dolt were the answer, the honest answer would be "use beads."
- **Neo4j** — right instinct (tickets are a graph), wrong scale. Graph DBs earn their keep on deep traversals over millions of edges; we do 2–4-hop traversals over thousands of nodes, which a recursive CTE answers in microseconds. Costs: JVM server process, CE licensing (GPLv3, no hot backup/clustering), young Rust driver, weaker fit for short strictly-serialized claim/fence transactions. Fanciest component in a system whose requirement is "not fancy."
- **Postgres now** — nothing in v1 needs it; keep as the someday-backend behind the trait (LISTEN/NOTIFY, SKIP LOCKED, multi-node) if ever justified.

Why SQLite is not a compromise: at 1–5 writes/sec the DB is not where the interestingness lives (state machine, leases, fencing, events are app logic). WAL-mode SQLite's single-writer model *is* the claim-serialization guarantee; zero ops; Litestream streaming backup; first-class `rusqlite`/`sqlx`. Boring is load-bearing in the fleet's single point of truth. **libsql** (Rust-native SQLite fork) is the tasteful non-boring variant: same semantics, native crate, replica/sync upgrade path later — treat as "SQLite with an upgrade path," don't depend on its exotic features in v1.

## Event delivery into agents (verified 2026-07-19)

The store stays harness-agnostic: SSE stream + durable `?since=<seq>` cursor is the only push surface. Harness-specific delivery is an adapter concern:

- **Claude Code**: hooks are outbound-only (cannot receive HTTP). Inbound options today: **Channels** (research preview; `claude --channels plugin:<name>` — a push-capable MCP server that delivers external events into a *running* session; custom channels documented, sender allowlist built in) — the sanctioned replacement for tmux keystroke injection, which remains unsupported/fragile. **Routines** `/fire` API endpoint — webhook that *starts a new cloud session* per event (stateless handlers). **Agent SDK** service holding a session and feeding it events. **`claude -p`** one-shot per event.
- Adapter shape: a small bridge (e.g. a custom Claude Code channel server) subscribes to takomo SSE and pushes "ticket ready/changed" into sessions. ~100 lines per harness; a "takomo channel for Claude Code" is itself a publishable artifact.
- **Verified end-to-end 2026-07-19** with the official `fakechat` demo channel (Claude Code 2.1.215, `--channels` present though hidden from help; requires Bun): channel = stdio MCP server *spawned by Claude Code as a child process* declaring `capabilities.experimental['claude/channel']`; events are pushed via `notifications/claude/channel` MCP notifications (content + meta). No inbound networking to the agent machine required — the channel process dials out (or binds localhost). Round trip observed: HTTP POST → `<channel source="fakechat">` event in session → model replied via the channel's `reply` tool. Caveats seen live: research preview (custom channels need `--dangerously-load-development-channels`); tool replies hit permission prompts unless pre-allowed → unattended setups need allow rules or the channel's permission-relay capability.

## Event-delivery design space (alternatives to channel push)

The store only guarantees: durable event cursor (`/events?since=`) + long-poll claim (`POST /ready/claim?wait=`). Clients pick their own wake mechanism:

1. **Hook-based pull** (stable, supported): SessionStart hook injects "N tickets changed" context; a **Stop hook can block stop and hand new instructions** — push-like behavior from outbound primitives. Latency = next lifecycle event.
2. **Long-poll claim** (the queue-worker model, SQS-style): request hangs until work arrives; the blocking call *is* the sleep. **Workers need no push at all** — claim delivery is race-free by construction. Push only matters for supervisors.
3. **Watcher + injection**: daemon on SSE injects into a running session (tmux keystrokes / resume). Unsupported, version-fragile, battle-tested in practice; the incumbent channels will replace.
4. **Spawn-per-event**: webhook receiver runs `claude -p` per event, or Routines `/fire` (managed cloud session per POST). Stateless, robust, no session continuity; right for discrete reactions.
5. **Agent SDK resident service**: own process holds a session (streaming input) and feeds it events. Max control; you own session lifecycle.
6. **Scheduled polling** (cron//loop): simple, minutes-latency, token cost per empty poll; dominated by (2) for workers.
7. **Piggyback existing channels**: takomo → Telegram bot → official telegram channel plugin. Zero custom code; preview-era stopgap.

Fleet recipe: workers = (2); supervisors = (1) baseline, custom channel as low-latency upgrade post-preview; (4) for stateless side-tasks. The store stays ignorant of client choice.

## Deliberate non-goals (v1)

Multi-node HA, horizontal scaling, users/teams/permissions beyond token scopes, human web UI beyond a read-only board, plugins, non-HTTP protocols, real-time collaborative editing, general PM features (sprints, estimates, burndowns).
