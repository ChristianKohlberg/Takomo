# Changelog

All notable changes to takomo are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/), and the project aims
to follow [Semantic Versioning](https://semver.org/). The `/v1` HTTP API evolves
additively only.

## [0.2.0] — 2026-07-24

First tagged release: a single-binary, self-hostable, hosted task tracker that
every AI agent, orchestrator, and human on a project talks to over HTTP. The
headline addition since the initial public release is the **ask-a-human board**
(questions, expertise routing, notifications, per-question answer links, and a
dedicated `/inbox` triage page); the rest below is the baseline it builds on.

### Server

- Single Rust/axum binary over SQLite (WAL) — HTTP server plus `token` and
  `project` admin subcommands.
- Hierarchical tickets (`epic` → `task`/`bug`/`subtask`) with single-parent
  trees, `blocked_by` dependency edges, labels, and free-form namespaced JSON
  metadata.
- Per-project, server-enforced state machine with a configurable workflow
  format; illegal transitions return a teaching `409` (current state, allowed
  transitions, and a remedy) written to be read by an LLM.
- Atomic claim/lease with a monotonic fencing token so exactly one worker owns a
  ticket; expired leases return the ticket to the ready queue.
- Append-only event log with a durable `?since=<seq>` cursor and an SSE stream.
- Ready queue (`GET /ready` peek, `POST /ready/claim` atomic take) driven by
  dependency readiness.
- Bearer-token auth, scoped (`read`, `write`, `human`, `autoland`, `admin`) and
  SHA-256 hashed at rest; token minting over both the server CLI and an
  admin-scoped HTTP surface.
- Read-only web board at `/board`, plus scoped, expiring share links.
- Ask-a-human board: agents raise a typed question (`confirm`/`choose`/
  `clarify`/`approve`) with `POST /v1/questions` / `takomo ask` / `takomo_ask`.
  A **blocking** question parks the ticket and releases the lease
  (block-and-resume); the ticket resumes only when all its open blocking
  questions are answered (a barrier). An **advisory** question records a routed
  decision with no state change — for epic-level or strategic calls. A
  `human`-scoped answer records the reply and, for a blocking question, performs
  the ticket's human-gated resume transition; `approve` questions additionally
  require the answerer to hold the matching `expert:<tag>` scope. Questions route
  by expertise tag (free-form `expert:<tag>` scopes), surface on a `/board`
  inbox with an unread badge, and support deadlines with an `on_timeout`
  fallback swept alongside leases. Optional outbound notifications (Slack /
  generic webhook / SMTP email) via `TAKOMO_NOTIFY`, off unless configured.
  A per-project **question language** (`takomo project language` /
  `PUT /v1/projects/{id}/language`) nudges agents to phrase ask-a-human
  questions in a set language (e.g. German for a revamp project) — surfaced as a
  `language_hint` on the MCP work-loop tools, `question_language` on
  `takomo_workflow`, in the `takomo_ask` result, and as an inbox reminder. Soft,
  never enforced.
  Per-question **answer links** (`POST /v1/questions/{id}/answer-link` /
  `takomo answer-link` / `takomo_answer_link`) mint a scoped, expiring,
  single-use `tka_` token so an outside expert can answer one question via
  `/board#a=<token>` (a distinct `/v1/answer/self` auth path) without holding a
  standing token. See docs/ask-a-human.md.
- Archive support (additive, non-destructive startup migration).
- JSONL export/import with idempotent re-import; importers for takomo, beads,
  and beans.
- `/healthz` as the only unauthenticated endpoint; refuses non-loopback binds
  unless `TAKOMO_ALLOW_PUBLIC_BIND=1`.

### Clients

- `takomo` — a self-contained `bash` + `curl` + `python3` CLI over the REST API,
  with `takomo init` one-command repo onboarding and local fence tracking.
- Claude Code plugin (this repo doubles as the plugin marketplace): bundles the
  takomo skill and a remote MCP server declaration for the hosted endpoint.
- Model Context Protocol (MCP) server for agent harnesses.
- Agent skills for using the store as a source of truth and for onboarding a
  repo.

### Deployment

- Render Blueprint (`render.yaml`) with a persistent disk and health check.
- Portable `Dockerfile` for VM / self-host deployment.
- Prepared (opt-in) Litestream continuous backup to S3-compatible storage.
