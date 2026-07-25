# Ticket schemas — enforce a per-type "definition of done" for ticket bodies (2026-07-25)

## The problem

Agents create tickets for other agents. A good ticket is picked up and resolved in minutes; a
ticket whose body is just a title forces the next worker to re-derive everything — where the code
is, how to reproduce it, what "done" means, and whether a human decision is still pending. In
practice this is the single biggest quality gap in agent-to-agent handoff: the store happily
accepts an empty body and moves it into the ready queue, so the cost lands on whoever claims it.

We want to keep **free-form bodies** (the tracker must not become a rigid form), but be able to
declare, **per ticket type**, a small set of **required sections** that a ticket must carry before
it becomes claimable — and have that actually hold, across every client (CLI, MCP, web board, raw
curl), not just the well-behaved ones.

## The model: config-as-code with a reconcile loop

Keep two concerns separate:

- **Template** = help writing (a skeleton with slots). Purely advisory.
- **Schema** = a checkable contract (which sections are mandatory). Enforceable.

A schema is a small declarative file, **versioned in the consuming repo** (git history,
PR-reviewable, evolves alongside the conventions it references), that is **pushed to the server**
and **enforced there** at the `draft → todo` transition. A drift check keeps the local source and
the server copy in sync. This is the same shape as committed-manifests + CI-apply + drift-detection
(Terraform-plan, k8s GitOps): the local file is the declarative source of truth, the server is the
applied, enforced state.

Why this split rather than pure-local or pure-server:

- **Pure local + client-side check** — versioned and transparent, but advisory only: a non-CLI/MCP
  client (curl, the web board from [10-dx-gaps.md](10-dx-gaps.md) #4, a third-party integration)
  bypasses it. Fine against forgetful agents, not against "every path holds it."
- **Pure server-side** — bypass-proof and harness-agnostic, but the schema then lives in server
  state with its own lifecycle, invisible to the agent unless the API serves it, and not
  versioned with the repo whose conventions it encodes.
- **Local source → server-enforced + drift-checked** wins on every axis (versioned, transparent,
  hard, single source of truth); the only cost is more moving parts.

One consequence worth stating plainly: because the schema lives in the repo, the **server can't
read it directly** — it has to be pushed. That is what the sync command and the drift check are
for. It also means enforcement is inherently **global/latest** (the server knows the last pushed
schema, i.e. `main`), not per-branch. For ticket schemas that is the right semantics — they are
project policy, not branch-scoped code — but a schema change lands only after it is pushed.

## Enforcement point: the `draft → todo` transition

Validate at `draft → todo`, nowhere else. This maps the check onto the state machine's existing
gate (see [../../spec/workflow-format.md](../../spec/workflow-format.md)) and onto the actual goal:

- `draft` = scratch, free hand, anything goes — an agent can think out loud.
- `todo` = claimable / ready for *another* worker → here the type's required sections must be
  present. Only tickets someone else will pick up need to be complete.

`todo → in_progress`, `done`, etc. stay untouched.

## Schema format (the local file, source of truth)

One file per type under `.takomo/schemas/<type>.md` in the consuming repo. Frontmatter is the
machine contract; the body is the agent-/human-facing template with hints. Required sections are
matched by fixed Markdown headings (`##`) — no NLP, a trivial and predictable check.

```markdown
---
type: bug
required: ["Wo", "Wie sehe ich's", "DoD", "Was offen"]
recommended: ["Fix-Richtung", "Scope / Nicht-Ziele"]
allow_na: true            # an empty required slot passes if its content is "n/a: <reason>"
---
## Wo
<evidence as path:line — new code + legacy source if porting>

## Wie sehe ich's
<repro: exact command, data preset, login, expected vs. actual>

## DoD
- [ ] one checkable "done" criterion

## Was offen
<reference to an ask-a-human question, or "none">
```

Validation rule at `draft → todo`: every heading in `required` must exist **and** have non-empty
content — or, when `allow_na: true`, content beginning `n/a: <reason>`. The `n/a` escape is
load-bearing: without it, agents satisfy the check with filler text, which is worse than an honest
"not applicable, because …". Everything not in `required`/`recommended` is free — agents add
whatever fits; the schema is a floor, not a cage.

An unknown or missing type schema means **no contract** → the transition is allowed (fail-open per
type), so adopting schemas for one type never blocks the others.

## Rollout switch

Per-project `schema_mode`:

- `off` — no effect (the default for existing projects, so nothing regresses).
- `warn` — transition succeeds, response carries a "missing sections" hint.
- `enforce` — `draft → todo` is refused until the contract is met.

New projects default to `warn`; flipping to `enforce` is a conscious call. Same posture as the
admin-token relaxation in [10-dx-gaps.md](10-dx-gaps.md) #2 — a deliberate, reversible tightening.

## Failure response

On an unmet contract, refuse the transition with a **teaching response** in the existing
`allowed_transitions` style — say exactly which sections are missing or empty, not a generic
"invalid":

```json
{
  "code": "schema.incomplete",
  "current_state": "draft",
  "missing_sections": ["Wie sehe ich's", "DoD"],
  "message": "Cannot move 'revamp-xxxx' to 'todo': the 'bug' schema requires sections that are absent or empty.",
  "remedy": "Add the missing sections (or 'n/a: <reason>' where allowed), then retry the transition. See `takomo schema show bug`."
}
```

## Surface

### API (sketch — adapt to the existing conventions in [../../spec/openapi.yaml](../../spec/openapi.yaml))
- `GET /v1/projects/<p>/schemas` · `GET /v1/projects/<p>/schemas/<type>` — serve the schema so any
  client can read/prefill it (transparency).
- `PUT /v1/projects/<p>/schemas/<type>` — push (admin/write; `If-Match` on a version/hash).
- `GET|PATCH /v1/projects/<p>/schema-mode` — `off | warn | enforce`.
- `POST /v1/tickets/<id>/transition` returns `409 schema.incomplete` (shape above) when it would
  cross `draft → todo` under `enforce` with an unmet contract.

### CLI + MCP (both must behave identically)
- `takomo schema show <type> [--project P]` — print the template (agents prefill from it).
- `takomo schema push [--project P]` — read `.takomo/schemas/*.md`, upload (CI step on merge to main).
- `takomo schema pull [--project P]` — write server → local.
- `takomo schema diff [--project P]` — compare local vs. server; **exit ≠ 0 on drift** (CI gate).
- `takomo new` / `takomo start` render the teaching response the same way transition errors render
  today. The MCP tools (`takomo_transition`, `takomo_start`) surface the identical error.

## Scope / non-goals

- No per-branch schemas — server holds the latest (`main`) policy.
- No content-quality judgement — only presence + non-emptiness of the required sections.
- No validation at any transition other than `draft → todo`.
- No prescribed wording — only the heading names listed in `required`.

## Acceptance criteria

- `.takomo/schemas/<type>.md` (frontmatter above) is parsed; unknown/missing type ⇒ transition
  allowed (fail-open per type).
- `enforce`: `draft → todo` fails with `409 schema.incomplete` + the missing-section list; `warn`:
  succeeds with a hint; `off`: no effect.
- `n/a: <reason>` satisfies a required slot when `allow_na: true`.
- `takomo schema push/pull/diff` work; `diff` exits non-zero on divergence (CI-usable).
- CLI and MCP show the identical teaching response.
- Existing projects (no schema, or `schema_mode=off`) are byte-for-byte unchanged — no regression
  on current tickets.
- Docs: format, `schema_mode` lifecycle, the CI sync + drift recipe.

## Open decisions

1. **Section detection** — fixed `##` headings in the body (proposed; zero friction for prose
   tickets, compatible with everything already written) vs. structured per-ticket fields.
2. **"Was offen" auto-satisfied** when the ticket has an open/answered ask-a-human question
   attached? (Recommended: yes — the decision is then provably routed instead of buried in prose.)
3. **Default mode for new projects** — `warn` (recommended) vs. `enforce`.
4. **Escape token** — exactly `n/a:` or another sentinel.
5. **Interim client-only step** — ship `schema show` + CLI/MCP validation reading the local file
   first (immediate value over the same file format), with server storage + enforcement + sync as
   an additive second stage. Recommended, so the format lands and pays off before the server work.

## Suggested build order

1. Freeze the schema file format (frontmatter + `##` headings) — everything else keys off it.
2. `takomo schema show` + client-side validation in CLI/MCP at `draft → todo` (interim; local file
   only). Immediate value, no server change.
3. Server storage (`GET/PUT schemas`, `schema-mode`) + server-side enforcement at the transition.
4. `schema push/pull/diff` + a CI drift gate; move enforcement from client to server as the
   authority (client check becomes a fast local pre-flight).
