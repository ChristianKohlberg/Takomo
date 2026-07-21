# Research: hmans/beans

Repo: https://github.com/hmans/beans — "A CLI-based, flat-file issue tracker for humans and robots."

## 1. What beans is

**Data model** (from `pkg/bean/bean.go` and `pkg/config/config.go`):
- An issue ("bean") = one Markdown file with YAML frontmatter in `.beans/` in the repo. Archived beans move to `.beans/archive/`.
- Frontmatter fields: `title`, `status`, `type`, `priority`, `tags[]`, `created_at`, `updated_at`, `order` (fractional index for manual sorting), `parent` (single parent ID), `blocking[]`, `blocked_by[]`. Body = free markdown (todo checklists `- [ ]` are the working convention).
- ID = configurable prefix + short NanoID (e.g. `beans-abc1`), encoded in the filename (`beans-abc1--slug.md`); prefix and ID length set in `.beans.yml`.
- **Hierarchy**: single-parent tree via `parent`, conventionally milestone → epic → feature → task/bug. "Epic" and "milestone" are *types*, not separate entities. Types (hardcoded): `milestone`, `epic`, `bug`, `feature`, `task`.
- **Dependencies**: explicit `blocking`/`blocked_by` edges, plus *implicit* blocking inherited down the parent chain; `ready`/`next` filters respect both. Children implicitly inherit terminal status (completed/scrapped) from ancestors.
- **Priorities** (hardcoded): `critical`, `high`, `normal` (default), `low`, `deferred`.
- **Custom fields**: none. Tags are the only open-ended metadata (lowercase, URL-safe, validated by regex). Types/statuses/priorities are explicitly hardcoded ("Statuses are not configurable - they are hardcoded like types").

**Storage**: flat files only, no sqlite. Config in `.beans.yml` (project name, path, ID prefix, default status/type, `require_if_match`, worktree settings, agent settings, server port). Designed to be committed to git alongside code; completed beans stay as queryable "project memory."

**Surfaces** (Go, single binary + companions):
- `beans` CLI: `init`, `create`, `list` (with `--ready`, filters, full-text `-S`), `show`, `update`, `archive`, `delete`, `roadmap`, `prime`, `query`, `tui`, `serve`, `check`, `graphql`.
- `beans tui`: Bubbletea terminal UI.
- `beans serve`: embedded Go HTTP server + SvelteKit SPA ("Beans UI"), GraphQL over HTTP/WebSocket (default port 8080). Includes an agent-orchestration workspace feature: spawns Claude Code sessions in git worktrees (kept outside the repo, default `~/.beans/worktrees/`), with PR-based integration (`worktree.integrate: pr`), run buttons, terminals, diff views.
- **GraphQL engine** built in: `beans query '<graphql>'` from the CLI (same schema serves the web UI); `beans query --schema` dumps the schema. Marketed as the token-efficient agent interface.

## 2. How agents interact with it

- **Priming, not MCP**: `beans prime` emits an embedded prompt instructing the agent to use beans instead of TodoWrite/todo lists, with the full CLI cheat-sheet, types/statuses/priorities, relationship semantics, body-edit and etag protocol, and GraphQL examples. Wired in via Claude Code hooks (`SessionStart` + `PreCompact` → `beans prime`) and an OpenCode plugin.
- **No official MCP server.** Third-party wrappers exist. The design bet is CLI + GraphQL in-context instead of MCP.
- All commands support `--json`. Body edits are agent-shaped: `--body-replace-old/--body-replace-new` (exact-match, must occur exactly once, errors otherwise), `--body-append`, combinable with status changes in one atomic call; multi-replacement via the `updateBean` GraphQL mutation (transactional, single etag validation).
- Prescribed agent workflow: check for existing bean → create with `-s in-progress` → keep checklist current → mark `completed` only when no unchecked items → commit bean files together with code → add `## Summary of Changes` / `## Reasons for Scrapping` sections.
- Third-party ecosystem: `internet-development/daedalus` (AI planning/orchestration CLI built on beans), a VS Code extension, Claude Code plugin marketplace listing.

## 3. Sync / concurrency story

- **Repo-local only. No hosted, server, or central mode exists or is planned.** Sync between machines/clones = git commit/push/merge of `.beans/` markdown files; conflicts are ordinary git conflicts.
- `beans serve` is a *local* single-project web UI, not a server product: **no authentication anywhere in the codebase**, localhost-oriented, runtime state in-process.
- **Same-checkout multi-agent concurrency**: optimistic locking via content-hash ETags — `beans show <id> --etag-only`, then `update --if-match "$ETAG"`; `require_if_match: true` makes ETags mandatory. `updateBean` GraphQL mutation is the single atomic entry point.
- **Known holes** — open issue #205 (no maintainer response): (a) etag skew — never-updated beans report an etag from `show` that `update --if-match` rejects, deterministically breaking read-then-CAS on fresh beans; (b) **lost updates under true concurrency** — validation is read-check-write inside each process with no cross-process/file locking, so two writers with the same etag can both "succeed" and one silently wins. (An earlier related race was fixed by moving validation under a write lock — but that lock is in-process only.)
- **Multi-worktree model**: each worktree carries its own `.beans/`; the serve process watches them and marks changes "dirty" until PR merge; known bug #181 (beans created in a workspace and merged to main don't appear in backlog view).
- The author's **agent orchestration direction was shelved** (June 2026 blog "An Update on Beans"): Anthropic's `claude -p` restrictions made the approach untenable; he muses most of beans "could be replaced with a good skill and a generic tool that can manage Markdown files."

## 4. Statuses / state machine

- Exactly five hardcoded statuses: `in-progress`, `todo` (default), `draft`, `completed` (archive-flagged), `scrapped` (archive-flagged).
- **No state machine.** Transitions are completely free-form; nothing enforces ordering. The only "workflow" is convention via the prime prompt plus the `archive` flag.
- Derived states: "ready" (not blocked directly or via ancestors, status not in-progress/completed/scrapped/draft) and implicit terminal-status inheritance from parents — computed at query time, not stored.

## 5. Maturity

- 876 stars, 60 forks, created 2025-12-06, Apache-2.0, Go 72% / TypeScript 17% / Svelte 9%.
- 40 releases; rapid pace Dec 2025, then slowing: v0.4.0 Feb 2026, v0.4.2 Mar 2026, **last push 2026-04-06** (~3.5 months stale as of 2026-07).
- 53 open issues; **maintainer explicitly deprioritized it** (June 2026 blog), focus moved elsewhere, **does not accept pull requests**. README warns of schema changes possibly requiring manual migration.
- Extensibility: low by design — statuses/types/priorities hardcoded, no custom fields, no plugin API, no PRs accepted.

## 6. Gaps vs. a hosted central SSOT with auth for many parallel agents

1. **No server mode at all.** The GraphQL server exists only as a localhost web-UI backend coupled to a single working directory. No remote-access design, no TLS story, no deployment story.
2. **Zero authentication/authorization.** Any network exposure of `beans serve` is fully open, including its agent-spawning and terminal/PTY features — actively dangerous to expose.
3. **No multi-project / multi-tenant model.** One `.beans.yml` + one directory per instance.
4. **Storage is per-clone flat files; the only cross-machine sync is git.** No event stream across machines (subscriptions are per-process).
5. **Concurrency control not trustworthy under parallel writers** (issue #205: etag skew + silently lost updates).
6. **Identity gaps for fleet use**: no assignee/actor field, no leasing/claiming primitive.
7. **Project health**: single maintainer stepped back, no PRs accepted, hardcoded schema, pre-1.0.

**Positive prior art to borrow**: prime-prompt pattern, `--json` everywhere, exact-match body edits, atomic combined mutations, ready/blocked graph semantics, archive-as-memory, small clean data model, GraphQL schema that could front a real backend.

Sources: repo + source files (`pkg/bean/bean.go`, `pkg/config/config.go`, `pkg/beancore/core.go`, `internal/commands/prime.go`, `internal/commands/prompt.tmpl`), issue #205, releases, hmans.dev blog "An Update on Beans", daedalus.
