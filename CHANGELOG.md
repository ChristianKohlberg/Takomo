# Changelog

All notable changes to takomo are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/), and the project aims
to follow [Semantic Versioning](https://semver.org/). The `/v1` HTTP API evolves
additively only.

## [Unreleased]

Naming things, and proving them. Tickets can carry **tags** from a per-project
registry, so a project names the people, components and teams it cares about
instead of encoding them in free text. `done` can be made **checkable** — a
project can require the closing commit as a link. Projects can declare the
**house style** agents write in, and both web surfaces can be narrowed to a
single ticket.

### Added

- **Per-project tag registry.** A project names entities of any `kind`
  (`person`, `component`, `team`, …) and attaches them to tickets by
  `kind:handle`. A new kind is just a new string — no schema change — and
  per-kind attributes live in a free-form `meta`. Tagging is reference metadata
  only: it never touches ticket state, claims, or question routing.
  `/v1/projects/{project}/tags` CRUD, `tags`/`tags_add`/`tags_remove` on ticket
  create and patch, `?tag=` / `?tag_kind=` list filters, `takomo tag …` /
  `takomo person …`, and `takomo_tag` over MCP. Tagging a ticket with an unknown
  handle lazily registers it; deleting a tag keeps ticket references and reports
  how many still point at it. Both `/board` and `/inbox` gain a two-step
  kind-then-value tag filter.

- **`guard:has_link:<key>` — prove "done" instead of claiming it.** A
  parameterized guard family: the ticket must carry a non-empty `links.<key>`.
  `has_link:commit` is the intended use — a full SHA stays checkable long after
  everyone has forgotten the ticket, and release/deploy questions derive from it
  (`git tag --contains`, `git merge-base --is-ancestor`) with no extra
  bookkeeping — but the key is free-form, so a project can demand `pr`, `run` or
  `env`. Opt-in per project; `factory-default` is unchanged, so no existing
  workflow starts rejecting anything. Rejections name the missing key and the
  remedy. `takomo link ID --commit SHA` warns on a short SHA, and `/board` shows
  a quiet `⌗ <sha>` badge — deliberately softer than the promotion badge, since
  it means "verifiable", not "shipped".

- **Per-project style guide for agent-written text.** The house style for ticket
  titles and bodies, comments and questions, declared once on the project so it
  reaches every client instead of one checkout. `PUT /v1/projects/{project}/style`
  (admin), or `style_guide` at creation; capped at 2000 characters
  (`422 project.style_guide_too_long`). Surfaced as `style_hint`, alongside
  `language_hint`, on the work loop of **every** client — `POST /v1/tickets`,
  `GET /v1/tickets/{id}`, `POST /v1/tickets/{id}/claim` and `POST /v1/ready/claim`
  over REST, the matching MCP tools, and printed by `takomo new` / `show` /
  `claim` / `next` / `start` — so an agent driving the store through the CLI reads
  the house style at the moment it has just written something and can still fix
  it. A project that sets no conventions gets no extra keys, and the hints stay
  off list responses, which are per ticket where the conventions are per project.
  Also on `takomo_workflow` as `style_guide`. Advisory, never enforced. Editable,
  with the question language, from a project-settings sheet on `/board`.

- **Filter the queue and the board by one ticket.** Both surfaces could already
  be narrowed by epic or expertise, but neither answered "what is still open on
  this one ticket". `/board` gets a ticket picker that composes with the epic
  filter and keeps subtasks visible; `/inbox` gets one built from the tickets
  that actually carry questions, deep-linkable as `#ticket=<id>`. A filtered
  empty queue says "no questions for this ticket" rather than "all clear", which
  would lie about the whole queue.

- **Real markdown in the inbox**, clamped long replies, and agents can revise a
  question's options (`takomo options <qid>`) after research shows the original
  set was wrong — instead of withdrawing and throwing the thread away.

- **Deep-linkable inbox**, an answer-link modal, and epic grouping on the board.

- **backlot 0.7 integration** (`backlot.yml`) — a warm, seeded instance in one
  command, with role-mapped tokens via `scripts/backlot-token.sh`.

### Fixed

- **The `/inbox` answer button now says what it will do, and is live only when it
  can do it.** Three faults in the same control. It was armed before anything had
  been chosen, so it could be pressed in a state where it could only refuse; it
  is now inert until the question is actually answered — non-empty text for a
  `clarify`, at least one option for a `multi` choose, a preset or a non-empty
  "write your own" for a single choose, yes or no for `confirm`/`approve` — and
  it says why in a hint that reaches hover, keyboard and screen reader alike
  (`aria-disabled` plus an `aria-describedby` hint, rather than a native
  `disabled` that would drop it out of the tab order and explain nothing). With
  the **follow-up composer** open it kept submitting an answer, so a reader who
  had just typed a question back to the agent could resume the ticket by pressing
  the one big button under it; while the composer is open the primary now *is*
  the follow-up submit, by the `↵` shortcut as well as by click. And its label is
  simply **Submit** / **Absenden** instead of "Answer & resume" / "Answer &
  record" — advisory questions stay marked as advisory in the header, in the list
  and in the confirmation.
- **Reading the tracker over MCP no longer spends the write budget.** Every MCP
  frame is a `POST /mcp`, and the rate limiter classified writes by HTTP method,
  so `takomo_show`, `takomo_list`, `takomo_ready` — and even `tools/list` — each
  debited the token's 120 writes/minute and, on exhaustion, came back with a 429
  saying the token had "exceeded its write budget", sending an agent hunting for
  writes it never made. An agent could rate-limit itself out of the tracker with
  zero mutations. The budget is now charged per tool call: the read-only tools
  are free (as `GET /v1/...` already was), the handshake and `tools/list` are
  free, and every mutating tool debits exactly one write. The 429 also says what
  it means — it names the budget, states that reads are free and still work, and
  carries a `remedy`.
- **One `/v1/export` no longer stalls every claim and heartbeat in the process.**
  Reads and writes shared a single SQLite connection behind one mutex, so any
  long read — an unfiltered export, `/v1/metrics`, a project roadmap, a
  transitive dep graph — froze every claim, transition and heartbeat for its
  whole duration. Measured on 8k tickets: a claim that normally takes 0.2ms took
  **104ms**, about 80% of the export. Reads now run on read-only companion
  connections (WAL makes them concurrent with the writer, and each read still
  gets one consistent snapshot), and the scan-shaped endpoints run off the async
  runtime. Same claim during the same export: **under 10ms**, with roughly twice
  as many claims completing while it runs. There is still exactly one writer —
  the guarantee that a ready ticket goes to exactly one claimant is untouched.
- **An answer link is now spent in the same transaction as the answer it
  carries.** The `tka_` token you hand an outside expert is single-use, but the
  write that marked it used committed *after* the answer, in a transaction of its
  own. Single-use still held — a second attempt found the question no longer open
  — yet it held by accident of unrelated bookkeeping rather than by the
  transaction that claimed it, and the observable behaviour was wrong in a way
  the expert saw: because the question's resolution sweep revoked the link before
  the follow-up write could mark it used, someone who reloaded a link they had
  just used was told it *"has been revoked"*. The spend is now the first thing the
  answering transaction does, so it is what orders simultaneous submissions on
  one link (two tabs, a double-click, a forwarded message): exactly one is
  applied, and the rest get `410` with the new `answer_link.spent`, telling the
  reader another answer landed first and nothing of theirs was recorded. A link
  spent by its own answer now correctly reports itself used, a revoke arriving
  while an answer is in flight wins, and a rejected answer — a bad option, a
  missing expert scope — rolls the spend back with it, so a link is never burned
  by an attempt that did not land.
- **The inbox no longer goes silent while you are answering.** `/inbox` defers a
  batch of events while a human is mid-answer — but it advanced the event cursor
  *before* deciding to defer, so the skipped batch was consumed and never fetched
  again. Since focus inside the reading pane counts as busy, and every answer now
  holds a 30-second undo window, a normal pass through the queue kept it busy
  continuously: new questions never arrived, answered ones never left, and only a
  reload recovered. The cursor now advances only over events that were actually
  applied, so a deferred batch is re-delivered on the next idle poll.
- **Committing one pending answer no longer resurrects the others.** When an undo
  window closed, `/inbox` wrote that answer and reloaded the list — and the reload
  replaced every other still-pending answer with the server's view, where those
  questions are of course still open. They popped back into the Open folder and its
  count while their own snackbars were still counting down, one of them was
  auto-selected with its full answer UI, and pressing "Answer & resume" on it did
  nothing at all, because that answer was already queued. Answering two questions
  within 30 seconds of each other was enough. Every reload of the question list —
  a closing window, a project switch, a withdraw, a reopen, a follow-up, a poll —
  now re-applies the answers still inside their undo window over the fresh list, so
  the queue never asks again for work you have already done. Undo keeps working
  across such a reload, and puts back what the server currently says.
- **`DELETE /v1/projects/{project}` 500'd on any project that had ever carried a
  question, a tag, an answer link or a promotion.** The cascade cleared tickets,
  comments, deps, events and idempotency records but not `questions`,
  `question_messages`, `answer_grants`, `tags` or `promotions` — each of which
  holds a real foreign key into `questions`, `tickets` or `projects`, so SQLite
  aborted the transaction and the caller got an opaque internal error. All five
  are now cleared in foreign-key-safe order inside the same transaction, and the
  `project_deleted` audit event reports the rows removed per table.
- **Every answer gets its own undo window**, and an answer is confirmed in the
  data as it is given rather than 30 seconds later or in a floating toast.
- The reading pane keeps its scroll position when a choice is selected.
- A clear answer→next transition, with the custom answer always available.
- Security, correctness, a11y and i18n findings from two review rounds:
  advisory resume, dependency scoping, and answer-grant revocation.
- `spec/openapi.yaml` was not a valid 3.1 document in two ways invisible to a
  human reader; CI now validates it against the schema and loads it from Python
  and Ruby.
- `.mcp.json` is now gitignored. `claude mcp add --scope project` writes a
  bearer token into that file in plaintext, and nothing was stopping a `git add .`
  from committing it. The README now says which scope does this next to the
  command it documents.

### Changed

- **`workflows/` is the one place a shipped workflow is defined.**
  `factory-default` moved out of a `serde_json::json!` literal in
  `src/workflow.rs` into `workflows/factory-default.yaml`, embedded with
  `include_str!`. The copies that remain for other reasons — the block in
  `spec/workflow-format.md`, and the CLI's offline fallback for `simple` — are
  pinned to the files by unit tests. `backlot.yml` and the `Dockerfile` learned
  that `workflows/` is a build input.
- Removed `prompts/spec-agent.md`; nothing referenced it.
- **`setLive` is gone from both web surfaces.** 0.3.0 dropped the navbar live
  indicator from `/board` and `/inbox` but kept `setLive` as a null-tolerant
  no-op so its call sites did not have to change — leaving a function named like
  a status indicator that wrote to `#live-dot` and `#live-text`, neither of which
  exists. Every call was silently doing nothing, which is exactly the trap the
  removal was meant to avoid elsewhere. The function, its 20 call sites (8 in
  `/inbox`, 12 in `/board`) and the orphaned `.dot` CSS are deleted from both
  files. No user-visible change — the calls already did nothing, and the strings
  they passed were hardcoded English that never reached a `STR` table — but the
  next reader of the poll loop no longer has to discover that for themselves.
- **Dependency hygiene.** Workflow YAML is now parsed by `serde_norway` instead
  of `serde_yaml`, which upstream archived in March 2024 and publishes as
  `0.9.34+deprecated`; it is the same 0.9 API, keeps the MIT OR Apache-2.0
  licensing, and drops the archived `unsafe-libyaml` backend along with it.
  Strict workflow parsing is unchanged — a typo like `require:` is still a hard
  error naming the field and the line, not a silently dropped approval gate.
  `tower` and `tokio-stream` were declared with zero call sites and are gone
  (they remain transitively via axum/rmcp, so the build is unaffected), and
  `tokio` now names the five features this binary actually uses instead of
  `full`, dropping six crates from the lockfile. A new CI job runs
  `cargo-machete` so an unused dependency cannot creep back in.

## [0.3.0] — 2026-07-25

Human-in-the-loop, refined. The ask-a-human inbox is rebuilt to the "Aquarelle"
design with a **DE/EN** language toggle, a **trailing-undo** answer flow
(auto-advance + a 30s background undo), an **ⓘ ticket-context** popover/drawer,
and a **follow-up thread** (bounce a question back to the agent for more
research). Questions carry richer, decision-ready fields — **per-option
descriptions, `confidence`, a recommendation rationale, a `summary`** — and can
be **multi-select** or **reopened** (a conditional undo once the 30s window has
passed). Work can be **promoted** to a named stage (prod/staging/published/…),
and the takomo **octopus** is the site favicon.

### Added

- **Multi-select `choose` questions.** A `choose` question can set `multi: true`
  (with an optional `recommended_multi` set) so a human can pick several options;
  the answer is the chosen array. Exposed on `POST /v1/questions`, `takomo_ask`,
  and `takomo ask --multi --rec-multi …`. The `/inbox` renders checkbox-style
  options for it.

- **Reopen an answered question** — a conditional undo beyond the inbox's 30s
  window. `POST /v1/questions/{id}/reopen` / `takomo reopen` / `takomo_reopen`
  (human scope; matching expert for `approve`) returns the question to `open` and
  re-parks the ticket, but only while the answer isn't yet in use — refused with a
  teaching `409` once the ticket is claimed, has moved past the state it resumed
  into, or is archived (re-ask instead). The `/inbox` shows a **Reopen** button on
  answered questions. Emits `question_reopened`.

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
