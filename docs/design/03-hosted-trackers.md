# Research: general hosted / self-hostable issue trackers as an agent task store

Evaluated July 2026 against: HTTP-reachable central store, simple token auth, epic→task→subtask hierarchy, rich metadata for agent context, a real configurable/enforced status state machine, agent-ergonomic API (MCP a plus), lightweight footprint.

## 1. GitHub Issues

- **Hosting**: SaaS (all tiers incl. Free). Self-host = GHES only (Enterprise license) — effectively SaaS-only for a solo operator.
- **Auth**: Classic PATs, fine-grained PATs, GitHub Apps (best for bot identities). Fine-grained PATs still patchy with Projects v2.
- **Hierarchy**: First-class since 2025 — **sub-issues GA (Apr 2025), up to 8 levels deep, 100 children per parent**, cross-repo, full REST + GraphQL, progress rollup. **Issue types** (GA 4/2025) let you model "Epic" as a type.
- **Metadata**: Labels; **issue fields GA July 2, 2026** (typed org-level fields on issues, Free tier, exposed via MCP); Projects v2 custom fields. No arbitrary JSON store — agent context goes in a fenced JSON block / text field / comments (convention, not schema).
- **State machine**: **Weak.** open/closed + `state_reason`. Projects "Status" is an unenforced single-select; no transition rules anywhere. Enforcement must live in the orchestrator.
- **API**: Excellent docs; REST covers projects items + field values (Sept 2025). **Official MCP server is the most complete first-party one anywhere** (sub-issue tools, projects, issue fields). Rate ceiling for swarms: **~80 content-writes/min, 500/hr per identity** — plan one App identity + write coalescing.
- **Footprint/cost**: Zero ops; all relevant features on Free plan.
- **Verdict**: Strongest SaaS default. Flags: no enforced transitions; 500 writes/hr/identity; no self-host.

## 2. Linear

- SaaS only, closed source. API keys / OAuth with bot-actor identity; official hosted MCP.
- Hierarchy: Initiatives (5 levels) → Projects → Issues → sub-issues. Solid.
- **No custom fields by design**; escape hatch: **Attachments API carries arbitrary key-value metadata, idempotent by URL, queryable by URL** — near-perfect for branch/run-id context. 
- State machine: per-team custom states in fixed categories; **no enforced transitions**.
- Best-in-class GraphQL + SDK + webhooks; rate limits (2,500 req/hr/key) push parallel agents into one pooled OAuth app.
- Free tier capped at **250 issues** — non-starter for agent churn; $10/user/mo.
- **Verdict**: Best DX; dealbreakers: SaaS-only, no custom fields, no enforced transitions.

## 3. Plane (makeplane/plane)

- Self-hosted CE (AGPL, Free-tier features only), Commercial self-host, or Cloud.
- API tokens (`X-API-Key`); decent REST + webhooks in CE; official MIT MCP server.
- **Epics and Initiatives are Pro-tier ($6/seat)** — CE has no epic layer. Custom work-item types & properties are Pro. **Enforced transitions ("Workflows") are Business-tier ($13/seat)**; CE unenforced.
- 60 req/min per token — tight. ~12 containers, 2 vCPU/4 GB min; documented history of rough upgrades; thin funding.
- **Verdict**: Only modern tracker genuinely self-hostable with API free, but everything an orchestrator wants is paywalled out of CE.

## 4. Tegon — DEAD

Repo archived June 13, 2025; org pivoted (RedPlanetHQ "CORE"); domains dead; never left alpha. Do not use.

## 5. Huly

- SaaS (generous free plan) + official self-host. Alive, well-funded (~27k stars). EPL-2.0.
- Multi-level sub-issues, milestones, components; no first-class epic object. States not enforced.
- **Dealbreaker: no documented public REST/GraphQL API** — only a TS `api-client` speaking a proprietary document/transaction protocol. No OpenAPI, no rate-limit contract, no non-JS story. Auth = bot-user login, not scoped tokens.
- Heavy: CockroachDB + Elasticsearch + MinIO + Redpanda + ~15 services, 8–16 GB RAM.
- **Verdict**: Great product, wrong tool.

## 6. OpenProject / Redmine

**OpenProject (Community, GPLv3)**
- Docker; Rails + Postgres; ~4–8 GB RAM. API key as basic auth.
- Unlimited-depth parent/child work packages; fully customizable types (Epic/Feature/Task/Bug) free; custom fields free and API-writable.
- **State machine: best-in-class among survivors** — transition matrix per (type × role); APIv3 form/schema flow exposes `allowedValues` for legal next statuses and **rejects illegal PATCHes with 422**. UI and API share enforcement.
- HAL+JSON API: self-describing but verbose and roundtrip-heavy for LLMs. Official MCP is Enterprise; good community MCP servers exist.

**Redmine (GPLv2, zero open-core gating)**
- **Smallest footprint surveyed**: one Rails container + DB; runs in 1–2 GB.
- `X-Redmine-API-Key` header or basic auth; admin user-impersonation header.
- Unlimited `parent_issue_id` nesting; arbitrary trackers model Epic/Task/Subtask; rich free custom fields (incl. key/value list, link), API read/write/filterable.
- **State machine: real and granular** — per (tracker × role) transitions plus per-status field permissions. **Wart**: REST API **silently ignores an illegal `status_id` and returns success** (redmine #8626/#10233). Mitigation: `?include=allowed_statuses` pre-check + read-after-write.
- Flat, compact JSON REST — dated but the most token-efficient per call of the classics. Community MCP servers exist.

**Verdict**: The only free, self-hostable options with genuinely enforced configurable workflows.

## 7. GitLab Issues / Work Items

- Free tier = **issue → task only (2 levels)**; epic→issue requires Premium ($29/user/mo); nested epics Ultimate. Free = labels only; custom fields Premium+. Work-item Status Premium+ and still not transition-enforced.
- API split-brain: legacy REST + GraphQL-only widgets model for anything new; epics REST deprecated → brittle clients. Platform self-host needs ~8 vCPU/16 GB.
- **Verdict**: Weaker than GitHub Free on every task-store axis unless paying Premium.

## 8. Lightweight / API-first field survey

- **Vikunja** — dream footprint (single Go binary, SQLite, scoped API tokens, Swagger REST) but **no custom fields and no status model at all**. Disqualified.
- **Leantime** — JSON-RPC, no enforced workflow. Disqualified.
- **Taiga/Tenzu** — governance churn (Tenzu rebrand dropping epics/scrum). Too risky.
- **WeKan** — subtasks are a linked-board hack. Disqualified.
- **Shortcut** — clean token REST, epics→stories, but SaaS-only, unenforced states.
- **Jira** — the enforcement gold standard (per-transition conditions/validators) but heavy, verbose, SaaS-or-Data-Center. Not "lightweight."
- **Dead pool**: Height (9/2025), Kitemaker (9/2025), Focalboard, Tegon. Watch-item: Hiveship (2026 "agent-native tracker", SaaS, pre-launch).

## Shortlist, ranked

**Key structural finding**: only **Jira, OpenProject, and Redmine** have API-enforced, role-based, configurable state machines. Every modern tool either lacks transitions entirely (GitHub, Linear, Huly, GitLab) or paywalls them (Plane Business). The ranking hinges on whether "proper state machine" means *enforced by the store* or *well-modeled states with enforcement in the orchestrator*.

1. **GitHub Issues** — best overall if orchestrator-side transition enforcement is acceptable. Native 8-level hierarchy, issue types, typed issue fields, full REST, best first-party MCP, zero cost/ops. Flags: 500 content-writes/hr/identity; agent context is convention not schema; no self-host.
2. **Redmine** — best if the store itself must enforce transitions. Tiny, all-free, trivial token auth, unlimited hierarchy, rich custom fields, tracker×role workflows. Must-handle: silent-success wart → pre-check `allowed_statuses` + read-after-write.
3. **OpenProject Community** — enforcement done loudly: 422 on illegal transitions, machine-readable `allowedValues` — arguably the most agent-legible state machine anywhere. Cost: ~8 GB footprint, verbose HAL API.
4. **Linear** — best pure DX and best arbitrary-metadata mechanism if SaaS + ~$10/seat is fine.
5. **Plane** — only worthwhile on paid self-hosted tiers; CE alone too stripped.

**Hybrid note**: no tracker surveyed does atomic claiming; a "GitHub Issues or Redmine as system of record + beads-style claiming semantics in the orchestrator" hybrid may beat any single tracker.
