# DX gaps — what's missing for a great task store (2026-07-20)

> **Historical gap list, dated 2026-07-20 — not a description of what Takomo is missing today.** Twelve of
> the thirteen gaps below are closed. The file is kept because the ranking and the reasoning are the
> record of *why* things were built in that order, and a gap list edited to match reality stops being one.
> Read every present-tense sentence here as a claim about July 2026, not about the current tree.
>
> **The one item still open is #11** (better full-text search, saved filters/views), now tracked as
> **`takomo-7qhf`** — that ticket, not this list, is where it lives.
>
> For what exists today read the **Architecture** section of [CLAUDE.md](../../CLAUDE.md) and
> [spec/openapi.yaml](../../spec/openapi.yaml). The **Shipped:** notes below record what happened at each
> contradicted passage; the original prose is left standing.

Takomo is functionally usable (store, workflow, CLI, tokens). This is the honest gap list between "works" and "a joy to adopt and live in," ranked by leverage for the goal of replacing beads/beans.

## Tier 1 — highest leverage

1. **MCP server.** The single biggest gap. Agents (Claude Code, Codex, …) natively speak MCP, and both beads and beans ship one. A thin takomo MCP (tools over the existing API: `new/ready/claim/start/done/comment/dep/show/ls`) makes adoption one line of harness config instead of wiring a skill + CLI prime per project. Without it, every project onboarding is manual.

> **Shipped, and not as a thin wrapper over REST.** `src/mcp.rs` hosts rmcp's streamable-HTTP transport at
> `/mcp` **in-process** (merged into the router in `src/server.rs`), and its tools call `Store` directly — no
> HTTP loopback, no second copy of the logic. `clients/mcp/` is a stdio client for harnesses that want one
> locally, and `plugins/takomo/` packages skill + remote MCP as a Claude Code plugin.

2. **Token & identity over HTTP.** Today minting needs SSH to the server (the deliberate "shell = root of trust" posture in auth.md). Great onboarding needs one-command token creation. Add admin-scoped `POST /v1/tokens` (+ list/revoke) and `GET /v1/whoami` (removes the `TAKOMO_ACTOR` footgun). Deliberate posture shift: admin can already create projects/workflows, so letting admin mint tokens over HTTP is a reasonable relaxation — but it IS a relaxation, worth a conscious call. (Being built.)

> **Shipped:** `GET /v1/whoami`, `GET|POST /v1/tokens` and `DELETE /v1/tokens/{id}` are routed in
> `src/server.rs`. The conscious call this paragraph asks for was made and written down — see
> "Deliberate posture shift (bounded relaxation)" in [spec/auth.md](../../spec/auth.md). Shell access is
> still the root of trust for the *first* admin token, which is minted by the `takomo token` CLI subcommand
> against the DB file.

3. **The WAF landmine.** Render's edge WAF silently 403s clients that send the default library User-Agent (e.g. python-urllib) and can block `<...>`-containing bodies as XSS. curl-based clients pass; library clients mysteriously fail. Either tune/whitelist the WAF for this service or guarantee every shipped client sets a UA — otherwise third-party integrations break confusingly.

> **Resolved** by the second option. `clients/mcp/src/client.ts` sets an explicit `User-Agent` on every
> request and, when a response body looks like a block page rather than JSON, says so by name instead of
> failing mysteriously.

## Tier 2 — real quality-of-life

4. **A minimal web board.** Humans need to see the work at a glance. beans has `beans serve` (kanban), beads has viewers. Even a read-only board rendered from the event log is a large legibility win and reduces "what's the state of things" friction.

> **Shipped, and it outgrew "minimal read-only".** There are two dependency-free SPAs compiled into the
> binary: `/board` (`src/board.html`) and the ask-a-human triage surface `/inbox` (`src/inbox.html`), together
> around 6000 lines, both with DE/EN string tables. Neither is read-only — humans answer questions from both,
> and the board PUTs project settings. They poll `GET /v1/events?since=<cursor>` rather than the SSE stream,
> because the browser `EventSource` API cannot set an `Authorization` header.

5. **Export / import & portability.** No way to get data out or in. Matters for backup confidence, anti-lock-in, and enabling "start fresh now, import history later." A `GET /v1/export` (JSONL) + an importer for beads-jsonl / beans-markdown would also make future migrations trivial.

> **Shipped as specified:** `GET /v1/export` emits JSONL, one ticket per line (`src/api/export.rs`), and
> `takomo import --from takomo|beads|beans` (`clients/cli/takomo`) reads it back, mapping foreign state names
> per source.

6. **One-command install for the CLI.** Today `takomo` is a repo script you symlink. A `curl | sh` installer (or a released single binary) makes "get the CLI" a one-liner.

> **Shipped:** `clients/cli/install.sh`.

## Tier 3 — polish (mostly from the pilot findings)

7. `similar[]` is shallow keyword overlap (cry-wolf) — score by title-token ratio + type, threshold it.
8. `fence.stale` conflates "wrong fence" (client bug) with "lease lost" — distinguish them.
9. `links` is a whole-object replace server-side — make it a merge, or document loudly (the CLI already merges client-side).
10. `/healthz` requires auth (contradicts the spec and blocks platform health checks) — make it truly open.
11. Better full-text search and saved filters/views.
12. Notifications: the SSE stream exists but nothing consumes it for humans ("watch my tickets").
13. Observability: no metrics/log surface for the store itself.

> **Shipped, except #11.** Item by item:
>
> - **7** — implemented as prescribed: `similar[]` is a blended Jaccard score over title terms plus a
>   same-type bonus, thresholded at 0.30 (`src/store/tickets.rs`); the code comment names "the old cry-wolf
>   behaviour" it replaced.
> - **8** — split: a fence the store never issued is `fence.invalid`, a fence it issued but superseded is
>   `fence.stale` (`src/store/helpers.rs`). One residue, still true: `lease_expired_error` reports
>   `fence.stale` for a lease that expired without being superseded.
> - **9** — `links` merges per key server-side now, with `null` deleting a key (`src/store/tickets.rs`).
> - **10** — `/healthz` is registered on the bare router in `src/server.rs`, outside every auth layer, so it is
>   genuinely open.
> - **11** — **STILL OPEN. This is the only live item on this page**, tracked as `takomo-7qhf`. `q=` has since
>   become per-term (each whitespace-separated term must match), but it is still `LOWER(title|body) LIKE
>   '%term%'` with no FTS index and no ranking, and nothing anywhere saves a named view.
> - **12** — closed, though **not by consuming the SSE stream**, which is worth stating precisely. "Watch my
>   tickets" is `takomo watch`, which long-polls `GET /v1/events?since=&wait=25`; `/board` and `/inbox` poll the
>   same endpoint (the browser cannot use `EventSource`, which sends no `Authorization` header); and a question
>   reaches a human by push, `src/notify.rs` firing at the ask itself over slack | webhook | email. So
>   `GET /v1/events/stream` exists and is tested, but nothing in this repo actually subscribes to it.
> - **13** — `GET /v1/metrics` (`src/api/metrics.rs`).

## Backups (adequate, upgradeable)

Render persistent-disk daily snapshots (~7-day retention) are the current baseline — whole-disk, 24h RPO. Litestream (SQLite → object storage, continuous, point-in-time) is the upgrade when RPO matters; small build change.

> **Shipped:** the upgrade was taken — `litestream.yml` sits at the repo root and
> [docs/hosting.md](../hosting.md) documents the off-box backup setup.

## Recommended order

MCP server (1) → finish token/whoami HTTP + `takomo init` (2) → resolve the WAF landmine (3) → web board (4) → export/import (5). Tiers 1–2 are what make it feel like a product; tier 3 is steady polish.

> **Shipped:** all five, in roughly this order.
