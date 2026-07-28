# Factory vision: brief → spec → remote implementation, on the Takomo spine

Date: 2026-07-19. Status: direction + incremental build plan.

> **Historical build plan, dated 2026-07-19 — steps 1 through 4 are done and step 7 was never taken as
> written.** The direction it sets out (humans interact by exception, at decision points, with evidence
> attached) is still the point of the system, which is why the file is kept; the "to build" and
> "deliberately deferred" labels are July-2026 labels, not current ones.
>
> For what exists today read the **Architecture** section of [CLAUDE.md](../../CLAUDE.md); for the built-in
> pipeline read `workflows/factory-default.yaml`, which *is* the definition (`src/workflow.rs` embeds it).
> **Shipped:** notes below mark the passages the code has overtaken; the original prose stands.

## The shape

Three existing/planned layers plus two missing ones:

- **takomo** (to build): coordination spine — tickets, hierarchy, metadata, enforced state machine, claims/leases, event log.
- **backlot** (exists, published): environment substrate — warm pooled seeded app instances that work visits; machine-verdict checks; remote substrates (Morph/SSH) designed but not yet live.
- **handrail** (exists): quality guidance wired into the agent's inner loop via harness adapters.
- **Missing: the front of the pipeline** — brief capture and spec development.
- **Missing: the runner** — a daemon on a (remote) machine that turns a claimed ticket into a live harness session with env + gates.

> **Shipped:** "takomo (to build)" is now built — all six capabilities listed (tickets, hierarchy, metadata,
> enforced state machine, claims/leases, event log) exist, plus a hosted MCP endpoint, two web surfaces,
> ask-a-human and tokens over HTTP. **The runner is still missing**, and so is the front of the pipeline as an
> automated stage: the `brief`/`spec` states exist and a spec agent can be pointed at them by hand, but
> nothing dispatches work to machines on its own.

## Pipeline as ticket lifecycle

`brief → spec → ready → implementing → review → done`

1. **Brief (inbox).** Zero quality bar; capture must be frictionless or the factory starves.
2. **Spec development.** A spec agent claims briefs: reads the target repo, drafts acceptance criteria/constraints/test plan/env needs, and asks clarifying questions *now* — ambiguity dies at the cheapest point.
3. **Dispatch.** Spec approved → ready. Runner daemons long-poll `claim` filtered by repo/harness labels. Machines are cattle (lease + fence).
4. **Environment.** Runner materializes worktree, `backlot up` leases a warm instance, handrail adapters wire into harness hooks; session starts with spec + `backlot ctx` + gates brief.
5. **Implement & prove.** Handrail nudges in-loop; `backlot run` produces verdicts; ticket accumulates evidence (branch, PR, verdicts, env URL) as events.
6. **Review & land.** PR + CI + verdicts attach to ticket; merge webhook auto-transitions; env lease lapses back to pool.

## Interaction design: MUST vs CAN

Principle: **humans interact by exception, at decision points, with evidence attached — never by watching.** The state machine encodes it: some transitions require a `human`-scoped token; per-project/risk-class policy decides which gates auto-pass (yolo generalized into schema).

MUST (enforced gates): spec approval (highest-leverage moment; auto-passable for low-risk classes) · escalated ambiguity (`needs-decision` + doorbell push) · destructive/irreversible/security-sensitive + credentials (never auto) · merge above risk threshold · terminal failure.

CAN (pull-based): watch event feed/board · steer mid-flight (ticket comment → event → channel push into the live session) · **open the running backlot instance** — review becomes "try the feature in a seeded authenticated env," not "read the diff" · amend spec mid-flight (event the worker must acknowledge).

Success metric: a good ticket needs exactly two touches — approve the spec, try the feature and say "land it."

## Tiny-step build plan

Each step is independently useful and sized for a single agent session; nothing depends on a later step.

**Step 0 — the contract (no code).** OpenAPI sketch + workflow-definition format + token scheme. Half a day; everything else implements it.

**Step 1 — takomo v0: the sync store.** Rust/axum + SQLite: ticket CRUD, `parent_id`, JSON metadata, one fixed workflow with enforced transitions + teaching 409s, bearer tokens, append-only event log + `GET /events?since=`. No claims, no SSE, no MCP, no UI. *Already useful:* the single source of truth, reachable from every machine via curl.
Need: a host — Tailscale-reachable box (home server/VPS) or a managed platform (e.g. Render) + a way to mint tokens (CLI subcommand).

**Step 2 — claims & leases.** `POST /ready/claim?wait=60s` (long-poll), lease TTL + heartbeat + fencing token. *Useful:* race-free work distribution to any machine; workers need no push infrastructure.

**Step 3 — agent client.** Thin skill/prime-prompt (+ optional MCP wrapper) so harness sessions use takomo instead of TodoWrite: claim, comment, transition, attach metadata. Pilot: route ONE real project's work through it from an orchestrator.
Need: nothing new — curl + a skill file.

**Step 4 — brief → spec stage.** Add `brief`/`spec`/`needs-decision` states to the workflow; a spec-agent brief template; run the spec agent manually at first (`claude -p` over a brief ticket). Human approves via CLI comment/transition.
Need: only prompt engineering; no infra.

> **Shipped — steps 1 through 4 are done, and steps 1 and 3 overshot their own scope.**
> Step 1's exclusions no longer hold: claims, SSE (`GET /v1/events/stream`), MCP (`/mcp`) and two web
> surfaces all exist. Step 2 landed as described — `POST /v1/ready/claim` with long-poll, lease TTL,
> `POST /v1/tickets/{id}/heartbeat`, and per-ticket fencing; lease TTL is per-project policy now, not one
> global constant. Step 3's "thin skill/prime-prompt (+ optional MCP wrapper)" became the reverse: the MCP
> surface is first-class and in-process, with `clients/claude-skill/` and `plugins/takomo/` alongside it.
> Step 4's `brief`, `spec` and `needs-decision` states are in `workflows/factory-default.yaml`, with the
> spec-approval edge gated on a `human`-scoped token; the spec agent itself is still triggered by hand, as
> this step assumed.

**Step 5 — the runner.** ~200-line daemon on one machine: long-poll claim → clone/worktree → spawn `claude -p` (or codex exec) with the spec → push branch/PR → transition ticket → heartbeat lease throughout. First fully remote implementation.
Need: a worker box with git/GitHub auth, harness CLIs authenticated, and outbound access to takomo. (This is where "ideally remote" becomes real.)

**Step 6 — environment & gates.** Runner adds `backlot up` before the session and injects `backlot ctx` + handrail brief; verdicts land in ticket metadata.
Need: `backlot.yml` in target repos (backlot), `.handrail/` gates in target repos; backlot remote-substrate work only if the worker box can't run envs locally.

**Step 7 — the doorbell.** SSE endpoint + push for `needs-decision`/`review` events: Telegram (official channel plugin, zero custom code) first; custom takomo channel post-preview.

> **Not as written.** The doorbell exists but Telegram never happened: `src/notify.rs` routes a question to a
> human by expertise over **slack | webhook | email**, pushed at the moment it is asked. The pull side is
> `takomo watch` and the `/inbox` surface, both long-polling `GET /v1/events?since=&wait=` — the SSE endpoint
> exists but nothing in the repo subscribes to it. A Telegram hop would be one more `transport` value, not the
> first one.

**Step 8 — review surface & auto-land.** GitHub webhook → auto-transition on merge; ticket carries env URL for click-review; risk-class policy for auto-merge of green low-risk work.

> **Still open**, and the reason is recorded rather than assumed: there is no webhook route in
> `src/server.rs`. What exists instead is the evidence half — `links` (branch, PR, commit) and promotions on
> a ticket, plus `guard:has_link:commit` so a `done` claim can be checked by a later reader. Nothing
> auto-transitions on merge.

Deliberately deferred: web UI (CLI + events suffice long into this), MCP server (skill + curl first), multi-tenant/orgs, spec-agent automation (manual trigger is fine for months), backlot remote substrate drivers (only when a worker box can't host envs).

> **Shipped:** the first two came forward — `/board` and `/inbox` are web UIs, and the MCP server is a
> first-class surface rather than a wrapper over curl. Multi-tenancy, spec-agent automation and remote
> substrate drivers are still deferred.

## Deployment topology (decided 2026-07-19)

Sort components by "state that must survive vs. compute that can die":

- **takomo → Render eventually, own VM meanwhile.** The ledger wants boring managed hosting (TLS, stable URL for webhooks, git-push deploys, persistent disk for SQLite; Render Postgres later via the repository trait if ever needed). Runs on the VM under systemd during the pilot; clients only see a URL change when it moves.
- **Agents, runner, backlot → own VM(s), never a PaaS.** Workers need authenticated harness CLIs, long sessions, real Docker for backlot warm pools, fat disks; backlot's work-visits-warm-env model wants agent and environment on the same box over localhost.
- **Key property:** long-poll claim means workers need *zero inbound connectivity* — true cattle: new VM + runner + CLIs = capacity; turn it off and its leases lapse back to the queue.
- **Blast-radius rule:** agents will eventually trash a worker box; with the store elsewhere that's an annoyance, colocated it decapitates the factory. Interim colocated phase is acceptable but must ship with off-box backup (Litestream or nightly `.backup`) from day one.

> **Shipped:** "Render eventually" and "Litestream from day one" are both in the tree — `render.yaml`,
> `Dockerfile` and `litestream.yml` sit at the repo root, with the setup written up in
> [docs/hosting.md](../hosting.md). The server also refuses a non-loopback bind unless
> `TAKOMO_ALLOW_PUBLIC_BIND=1`, since it speaks plain HTTP and expects TLS in front.

## What's needed, total (shopping list)

- One reachable host for takomo (Tailscale box or managed platform) — the only new infrastructure.
- One worker machine (can be the same box initially) with git + GitHub auth + authenticated harness CLIs.
- GitHub webhook (or polling fallback) for merge auto-transition (step 8).
- Target repos gain `backlot.yml` + `.handrail/` gates as they onboard (steps 6+).
- Token discipline: per-actor bearer tokens, `human` scope for gated transitions.
