# DX gaps — what's missing for a great task store (2026-07-20)

Takomo is functionally usable (store, workflow, CLI, tokens). This is the honest gap list between "works" and "a joy to adopt and live in," ranked by leverage for the goal of replacing beads/beans.

## Tier 1 — highest leverage

1. **MCP server.** The single biggest gap. Agents (Claude Code, Codex, …) natively speak MCP, and both beads and beans ship one. A thin takomo MCP (tools over the existing API: `new/ready/claim/start/done/comment/dep/show/ls`) makes adoption one line of harness config instead of wiring a skill + CLI prime per project. Without it, every project onboarding is manual.

2. **Token & identity over HTTP.** Today minting needs SSH to the server (the deliberate "shell = root of trust" posture in auth.md). Great onboarding needs one-command token creation. Add admin-scoped `POST /v1/tokens` (+ list/revoke) and `GET /v1/whoami` (removes the `TAKOMO_ACTOR` footgun). Deliberate posture shift: admin can already create projects/workflows, so letting admin mint tokens over HTTP is a reasonable relaxation — but it IS a relaxation, worth a conscious call. (Being built.)

3. **The WAF landmine.** Render's edge WAF silently 403s clients that send the default library User-Agent (e.g. python-urllib) and can block `<...>`-containing bodies as XSS. curl-based clients pass; library clients mysteriously fail. Either tune/whitelist the WAF for this service or guarantee every shipped client sets a UA — otherwise third-party integrations break confusingly.

## Tier 2 — real quality-of-life

4. **A minimal web board.** Humans need to see the work at a glance. beans has `beans serve` (kanban), beads has viewers. Even a read-only board rendered from the event log is a large legibility win and reduces "what's the state of things" friction.

5. **Export / import & portability.** No way to get data out or in. Matters for backup confidence, anti-lock-in, and enabling "start fresh now, import history later." A `GET /v1/export` (JSONL) + an importer for beads-jsonl / beans-markdown would also make future migrations trivial.

6. **One-command install for the CLI.** Today `takomo` is a repo script you symlink. A `curl | sh` installer (or a released single binary) makes "get the CLI" a one-liner.

## Tier 3 — polish (mostly from the pilot findings)

7. `similar[]` is shallow keyword overlap (cry-wolf) — score by title-token ratio + type, threshold it.
8. `fence.stale` conflates "wrong fence" (client bug) with "lease lost" — distinguish them.
9. `links` is a whole-object replace server-side — make it a merge, or document loudly (the CLI already merges client-side).
10. `/healthz` requires auth (contradicts the spec and blocks platform health checks) — make it truly open.
11. Better full-text search and saved filters/views.
12. Notifications: the SSE stream exists but nothing consumes it for humans ("watch my tickets").
13. Observability: no metrics/log surface for the store itself.

## Backups (adequate, upgradeable)

Render persistent-disk daily snapshots (~7-day retention) are the current baseline — whole-disk, 24h RPO. Litestream (SQLite → object storage, continuous, point-in-time) is the upgrade when RPO matters; small build change.

## Recommended order

MCP server (1) → finish token/whoami HTTP + `takomo init` (2) → resolve the WAF landmine (3) → web board (4) → export/import (5). Tiers 1–2 are what make it feel like a product; tier 3 is steady polish.
