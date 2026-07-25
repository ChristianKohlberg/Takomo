# Changelog

All notable changes to takomo are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/), and the project aims
to follow [Semantic Versioning](https://semver.org/). The `/v1` HTTP API evolves
additively only.

## [Unreleased]

### Changed

- **Inbox answering is now trailing.** Answering completes the item optimistically
  and jumps straight to the next open question (with a small micro-animation); the
  30-second undo runs in the background. Only **Undo** brings the item back to its
  former status and re-selects it. Committing a new answer flushes the previous
  one. The navbar drops the `live` indicator and (on the inbox) the unused `mine`
  toggle; the project selector is restyled and now **remembers the last-selected
  project**. The inbox navbar wraps instead of overflowing on narrow screens.

### Added

- **Richer question fields (make the inbox fully data-driven).** Questions gain
  optional, additive fields the redesigned inbox renders: per-option
  descriptions (send `options` as `[{value, desc}]`, or a parallel
  `option_notes`), `recommended_note` (the rationale), `confidence` (1–4, drives
  the recommendation gauge), and `summary` (list preview). Exposed on
  `POST /v1/questions`, `takomo_ask`, and `takomo ask`
  (`--option-desc`/`--rec-note`/`--confidence`/`--summary`). The ask response now
  returns a non-blocking **`hints`** array telling the agent what optional field
  would improve the card — it never fails the ask. Agent guidance (MCP tool
  description, plugin skill) updated to write decision-ready questions.

- **Inbox & board redesign (Aquarelle prototype) + DE/EN localization.** `/inbox`
  is rebuilt to the latest design: a **DE/EN language toggle** (whole UI
  localizable, remembered per device, defaults to the browser language), an
  urgency-**grouped** question list with rank-glyph headers and preview lines, a
  reading pane with an **ⓘ ticket-context popover** and a slide-over ticket
  drawer, a **"Ticket update" timeline** for the follow-up thread, kind-adaptive
  answer cards (choose/clarify/confirm/approve) with the recommendation
  highlighted, a follow-up **compose** control with a thread-count badge, a
  **withdraw confirmation** dialog, and a single-click **Answer** guarded by a
  centered 30-second **undo snackbar** (replacing the press-twice arming). The
  board gains the same DE/EN toggle, a 4-bar priority gauge on cards, and
  localized chrome. Behavior (token gate, live polling, share/answer-link modes,
  the ask-a-human drawer, the follow-up loop) is preserved.

- **Ticket promotions.** Record that a ticket's work reached a named
  target/stage — `POST /v1/tickets/{id}/promote {target, url?, ref?, note?}` /
  `takomo promote` / `takomo_promote`. `target` is free-form ("staging",
  "production", "published", "delivered", …) so takomo isn't tied to software
  deployment. Append-only history (`GET /v1/tickets/{id}/promotions`,
  `?include=promotions`); the latest per ticket (`GET /v1/promotions?project=`)
  badges the board card, and the detail drawer shows the full history. Emits a
  `ticket_promoted` event; `takomo_show` surfaces promotions to agents.

- **Ask-a-human follow-up thread.** A human can now bounce a question back to the
  asking agent for more research *before* answering, tracked as a clear
  thread. `POST /v1/questions/{id}/followup` (human) records the request and sets
  `awaiting: agent`; the agent replies with `POST /v1/questions/{id}/reply` /
  `takomo_reply` (write), flipping `awaiting` back to `human`. The question stays
  open and a blocking ticket stays parked throughout. `GET /v1/questions/{id}`
  now returns the `thread` and `awaiting`; `takomo_show` surfaces the thread on
  open questions so an agent's work-loop picks up the request. Surfaced in the
  `/inbox` reading pane (Ask-for-more action + thread) and the board drawer.
- `/board` and `/inbox` re-skinned to the "Aquarelle" design (structure +
  palette), and the takomo octopus mark now ships as the site favicon.

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
