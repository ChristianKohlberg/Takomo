# Research: task trackers built for AI coding agents (mid-2026 survey)

Requirement lens: one central hosted source of truth + basic auth + hierarchical epics + arbitrary metadata + real state machine, writable concurrently by parallel orchestrators on different machines.

## 1. beads (bd) — steveyegge/beads → now gastownhall/beads

- **Storage & sync:** Dolt (version-controlled SQL DB), not SQLite/JSONL. Default "embedded" mode: in-process Dolt in `.beads/`, **single writer**. `.beads/issues.jsonl` is an export for viewers/interchange, "not the source of truth." Cross-machine sync is git-shaped: `bd dolt push` / `bd dolt pull` against `refs/dolt/data` on your git remote — eventually consistent, merge-based. **Server mode exists**: `bd init --server` connects to an external `dolt sql-server` for multiple concurrent writers (MySQL wire protocol, port 3306; remotesapi on 8080). **Federation** (docs/multi-agent/federation.md) is peer-to-peer: each workspace keeps its own Dolt DB and pushes/pulls to registered peers (DoltHub, S3, GCS, HTTPS, SSH backends), explicitly "without a central server."
- **Auth:** No app-level auth. Server mode inherits MySQL user/password auth from `dolt sql-server`; federation peers store credentials locally (AES-256 encrypted). Maintainer/contributor role auto-detected from git remote credentials.
- **Hierarchy:** Yes — issue types `bug, feature, task, epic, chore`; hierarchical IDs (`bd-a3f8` epic → `bd-a3f8.1` → `bd-a3f8.1.1`). Parent-child is organizational; separate blocking dependencies drive the `bd ready` queue.
- **Metadata:** Yes — `metadata` field accepts **arbitrary JSON**, stored as-is; documented as "the preferred extension point."
- **Status model:** Fixed: `open, in_progress, closed, deferred` (priority 0–4). No configurable state machine, no enforced transitions. Workflow rigor comes from the dependency graph (`bd ready`, atomic `bd ready --claim`), not statuses.
- **Agent surface:** CLI-first with `--json` everywhere (`bd create/update/claim/ready/prime/remember`); MCP integration exists; no REST API.
- **Maturity:** ~25.4k stars, MIT, pushed same-day (very active). Migrated to the gastownhall org (Yegge's Gas Town orchestrator ecosystem).

## 2. Backlog.md — MrLesk/Backlog.md

- Plain Markdown in a project-local `backlog/` folder, git-synced; explicitly "local-first — no server, no account." Web UI local-only, no auth.
- Milestones + dependencies; sub-tasks via ID convention; no real epic layer. Frontmatter metadata, acceptance criteria, DoD checklists, labels, comments.
- Configurable status list (default To Do / In Progress / Done); no enforced transitions.
- CLI with JSON output; `backlog mcp start` MCP server.
- ~6.2k stars, MIT, active (July 2026). **Fails hosted requirement** — sync is git merge of markdown files.

## 3. claude-task-master / Task Master — eyaltoledano/claude-task-master

- **Solo:** local `tasks.json` (repo-local, no multi-machine story). **Team mode: genuinely hosted** — tasks live in "briefs" on Hamster's cloud (tryhamster.com), accessible from any machine, real-time sync (`tm list --watch`).
- **Auth:** Team mode via browser OAuth (`tm login`); no plain token/basic-auth or public REST API — access via their CLI/MCP client only.
- **Hierarchy:** Briefs → tasks → nested subtasks (one level). No epic type.
- **Metadata:** optional `metadata` field with arbitrary JSON (MCP writes gated behind env flag).
- **Status model:** Fixed set; not configurable, no enforced transitions.
- **Agent surface:** CLI + full MCP server.
- ~27.9k stars, MIT + Commons Clause. **OSS repo last pushed 2026-04-28 (~3 months stalled)**; the hosted Hamster side is closed-source SaaS with a 10-member team cap.

## 4. Vibe Kanban — BloopAI/vibe-kanban

- **SUNSET.** Bloop announced shutdown April 10, 2026; transitioned to fully local architecture, community-maintained. Local SQLite, flat cards, fixed kanban columns, no meaningful auth. ~27.4k stars, Apache-2.0. Do not build on it as a source of truth.

## 5. Shrimp Task Manager — cjo4m06/mcp-shrimp-task-manager

- Local JSON files, single local MCP server process. No server/hosted mode, no auth, no epics, fixed statuses. ~2.1k stars, **dormant since Aug 2025**. Fails hosted requirement entirely.

## 6. Newer 2025/2026 hosted/server-shaped entrants

### agent-kanban — saltbo/agent-kanban
- **Central server**: Hono API + SQLite/Cloudflare D1; web UI with SSE real-time; local daemons on any number of machines poll the API and spawn workers (Claude Code, Codex, Gemini CLI, Copilot, Hermes). True many-clients-one-server.
- **Auth:** strongest in class — per-agent **Ed25519 cryptographic identity**; agents sign their own JWTs, verified server-side.
- **Hierarchy:** subtasks + `depends_on` with cycle detection; roles/labels; **no epic layer**.
- **Metadata:** labels + roles; limited arbitrary-metadata support.
- **Status model:** fixed Todo → In Progress → In Review → Done, but with **enforced operations** (claim/review/complete/reject/release) — closest to a real transition machine in this list.
- `ak` CLI (kubectl-style), REST API, agent-to-agent chat; no MCP.
- 406 stars, created Mar 2026, pushed today — very young, very active. FSL-1.1-ALv2 (→ Apache-2.0 after 2 years).

### kandev — kdlbs/kandev
- Central server accessible from any device; SQLite at `~/.kandev`; agents run local/Docker/SSH/cloud. **No built-in auth** — punts to Tailscale/VPN. Sub-tasks resume from parent session; multi-repo tasks. External MCP over HTTP/SSE + ACP. AGPL-3.0, 454 stars, created Jan 2026, pushed today.

### sortie — sortie-ai/sortie
- Not a tracker: a bridge that turns tickets from an existing tracker (Jira etc.) into autonomous agent sessions. Relevant as a pattern: real hosted tracker + agent-side runner. 109 stars.

### Agent-Kanban — Adam-Dangerfield/Agent-Kanban
- **Exact feature-shape match**: self-hosted server (nginx + Express + PostgreSQL, Docker Compose), **bearer-token auth** (manager JWTs + per-agent bcrypt tokens, per-project read/write grants), **epics → stories → tasks hierarchy**, notes/priority/branch + merge-state tracking (none→dev→pr→merged), file attachments (local or S3), Backlog→To Do→In Progress→Done workflow with **immutable per-task audit trail**, cross-project request lifecycle, REST API + portable Claude Code skill wrapper. AGPL-3.0.
- **Maturity: 3 stars, self-described "working personal tool"** (created June 2026). Full spec fit, near-zero community.

### AgentsMesh — AgentsMesh/AgentsMesh
- Hosted and self-hostable AI-workforce platform; ticket store is a subsystem of a much larger platform (AgentPods, gRPC+mTLS runners), not a standalone tracker.

### Hermes Kanban (Nous Research)
- SQLite at `~/.hermes/kanban.db`, six states with dependency auto-promotion — but "multi-host orchestration is deliberately out of scope." Local-only; fails hosted.

# Verdict

**No mature candidate satisfies all five requirements.** The field splits into repo-embedded trackers (great metadata/hierarchy, no server) and orchestrator boards (server, weak ticket semantics):

- **Backlog.md, shrimp, Hermes, solo task-master: eliminated** — repo/machine-local.
- **Vibe Kanban: eliminated** — sunset, retreated to local-only.
- **Full-spec matches exist but are immature:** Adam-Dangerfield/Agent-Kanban ticks literally every box at 3 stars / personal-tool status. saltbo/agent-kanban is the most credible young server (real auth, enforced claim/review/complete transitions, SSE, multi-machine daemons) but lacks epics and rich metadata.
- **task-master team mode (Hamster)** is the only managed hosted option, but auth is OAuth-through-their-CLI (no token API you control), statuses fixed, hierarchy is briefs+subtasks not epics, and it couples you to a closed SaaS whose OSS repo has gone quiet.
- **beads is the best 4-of-5 with real maturity (25k stars, daily commits):** epics/hierarchical IDs, arbitrary JSON metadata, agent-native CLI/MCP, and a genuine multi-writer server story — central `dolt sql-server` (MySQL-protocol auth) with `bd init --server` on every checkout, or default git-remote push/pull sync. Gap: fixed 4-status model, no configurable/enforced state machine — enforcement comes from the dependency graph (`bd ready --claim`).

If "real state machine" can relax to "dependency-gated ready queue," **beads in Dolt server mode is the only battle-tested answer** today. If the strict spec is non-negotiable: adopt an immature server (saltbo/agent-kanban; Adam-Dangerfield/Agent-Kanban), or put an agent-facing layer over a general hosted tracker (the sortie pattern) — the ecosystem keeps converging there precisely because no mature agent-native hosted tracker exists yet.

Sources: beads repo + FAQ + federation/metadata docs, Yegge's Beads post, Backlog.md, claude-task-master + Hamster team docs, vibe-kanban + shutdown notice, shrimp-task-manager, saltbo/agent-kanban, kandev, sortie docs, Adam-Dangerfield/Agent-Kanban, AgentsMesh, Hermes Kanban docs, awesome-agent-orchestrators.
