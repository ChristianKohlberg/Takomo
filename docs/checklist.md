# Checklist — verifying that the work actually works

An agent finishes and says "done". `guard:has_link:commit` makes that claim
*attributable* — any reader can check which commit closed the ticket — but not
*verified*. Checklist is the missing half: a durable description of what "working"
means for an application, that an agent executes and records verdicts into, with a
human supervising by exception rather than by default.

The design record, including what was deliberately deferred, is
[`docs/design/12-checklist.md`](design/12-checklist.md). The authoring method an
agent should follow is the `takomo-checklist` skill
(`clients/claude-skill/takomo-checklist/`).

## The shape

```
project
└── epic           a ticket of type `epic` — same vocabulary tickets use
    └── check      ONE action, ONE entry precondition, ONE layer
        └── case   the check crossed with one parameter assignment
```

**A check used to be called a lane.** The word was already taken: on the roadmap and
in `/initiatives`, a lane is the *initiative* a feature is worked in — it spans
versions and never closes. One product cannot carry two lanes, so the verification
one became a check. Tables are `checks` and `check_globs`, the case's column is
`check_id` (bare `CHECK` is a SQL keyword), routes live under `/v1/checks`, and the
MCP tools are `takomo_check*`. Checks created before the rename keep their `lane-…`
ids: an id is opaque, and rewriting primary keys is the one part of a rename that can
lose data.

The **case** is what gets executed. One check routinely produces dozens: a pairwise
model over a large real form measured 76. So the roadmap cannot treat a 76-case
check like a 1-case check, and a person cannot be handed "case 41 of 76" without the
assignment that makes it reproducible.

## Takomo stores; the agent computes

This is the load-bearing division of labour. Takomo does **not** generate cases,
validate a model, or check whether a coverage claim is true. The agent owns the
recipe and its correctness; Takomo owns persistence, policy, expiry arithmetic and
history.

A wrong model is therefore stored faithfully. That is accepted: the alternative is
Takomo growing an opinion about every application under test.

## Check boundaries are state transitions, not screens

If something needs a persisted record, has its own permission gate, or is only
reachable from another check's terminal state, it is a **separate check**. A create
form, a finalize step, a print action, a send action and a cancel action are five
checks, not one.

Cross-action coupling does not merge them. A value captured during create that
changes a document produced later at print time means the *print* check carries that
value as a parameter of its own. This is also why checks need no dependency graph —
the precondition is a statement about data state, which keeps them independently
runnable.

**A check covers one layer.** A rule enforced only in a frontend passes at the HTTP
layer while the UI would have blocked it, so a UI verdict and an API verdict are
not interchangeable in either direction. A check spanning both is two checks.

## Coverage is of the *declared* surface

A check claims paths of the application under test as hand-written globs
(`src/claims/**`). This is simple, understandable, and **will rot** — that trade
was made deliberately. Two consequences are built in rather than hoped away:

- **Orphan detection.** The agent pushing a release also reports which globs
  matched **zero files** in that tree. Those checks are flagged and stop counting
  toward coverage. An orphaned glob reading as "still covered" is the worst failure
  mode this feature has, because it inflates confidence exactly where confidence is
  unwarranted.
- **Honest wording.** `percent` is verified-or-approved over *verifiable* cases.
  Stale, failed and never are outside the numerator; `unreachable` is outside the
  denominator too, so a fully-verified project can actually reach 100% and the
  unreachable count stands on its own as a finding.

### The three verdict outcomes that are not pass/fail

- **`stale`** — was verified, then the claimed code moved. A case that was *never*
  verified stays `never`: calling it stale would report re-testing for work nobody
  has done once, and shrink the gap this feature exists to show.
- **`unreachable`** — the declared layer gives no way to reach this configuration.
  Counted apart from both covered and uncovered, because calling it a gap reports
  work nobody can do and calling it covered claims verification of code no path
  reaches. It is the most valuable output: UI/API drift and dead code fall out of
  coverage bookkeeping instead of needing a separate audit.
- **`blocked`** — the runner could not get far enough to judge.

## The initiative that agreed the check

A characterisation test is usually settled while a feature is being *discussed* —
in the initiative, months before anyone asks whether it still passes. So a check
names its initiative directly:

```sh
POST /v1/projects/{project}/checks
{ "title": "Split an invoice across two entities", "initiative": "ini-9f3ka2xz" }

GET /v1/initiatives/{id}/verification
# → 3 stale, 1 never run, 6 verified, last verified 2026-08-14, blocked: true
```

**Direct, not derived through the epic.** An epic already joins an initiative by
its `initiative:<id>` tag, so the link could have been two hops — but the
agreement usually predates any epic, and deriving it would make the link
unstateable at exactly the moment it is being made.

It is also validated on the way in, unlike that tag. A tag naming a missing
initiative is left dangling on purpose (the roadmap reports its ticket under
`uninitiated` rather than losing it), because a tag is typed by hand. This is a
column chosen from a list, so a wrong id is refused where the mistake happens.

`?initiative=<id>` narrows the check list, and `?initiative=none` finds the
checks no initiative claims — the gap between what was agreed and what got
written down.

`GET /v1/initiatives/{id}/verification` is a **sub-resource** rather than a field
on the initiative, for cost: the rollup scans the checks and cases beneath it, and
the initiative's JSON shape is shared by the list read, so inlining it would make
listing 200 initiatives pay that scan 200 times.

## Releases

Releases are first-class and **pushed by the agent that merged the work**. There is
no direct integration by design:

```sh
# what the merging agent sends
POST /v1/projects/{project}/releases
{ "ref": "v2.0.0",
  "touched_paths": ["src/claims/create.rs", "..."],
  "orphan_globs": ["src/claims/legacy/**"] }
```

The pusher supplies the diff and the empty globs because it has the tree checked
out — the server clones nothing, and that is the cheapest possible place to learn
the truth. The response reports what the push invalidated, so the agent learns the
consequence of its own merge without a second call.

`seq` is monotonic per project, which is what makes "retest every 5 releases"
arithmetic. A `ref` is immutable: pushing one twice is a 409.

## Policy: inherited, overridable

Two settings resolve **project → epic → check**, each level overriding the one above,
and every resolved value reports which level supplied it (`verification_from`,
`expiry_from`) — an inherited setting nobody can trace is worse than no setting.

| Setting | Values |
|---|---|
| `verification` | `agent` · `human` · `agent_then_human` |
| expiry | `expiry_days` and/or `expiry_releases` — whichever trips first |

Send an override as `null` to clear it and resume inheriting. Absent and null mean
different things on the wire for exactly this reason.

`agent_then_human` needs **both** verdicts, which is why the agent's and the human's
are stored as separate facts rather than one column.

## Who may record what

An agent may record what it observed. Only a token carrying the `human` scope may
assert that a **person** approved a case — the same line `ask-a-human` draws. Over
MCP the option does not even exist: `takomo_verdict` has no `actor_kind`, so a human
approval must come through `POST /v1/cases/{id}/verdict`.

## The worklist is the product

Human time is the scarce resource: a hundred cases cost an agent minutes and cost a
person most of a day. So `GET /v1/projects/{project}/checklist/worklist` splits by
*who can clear it*, not by check.

A stale case under `agent_then_human` appears on the **agent** list until it has a
fresh agent verdict, so it never sits in a person's queue waiting for work only an
agent can do.

## The gate

`GET /v1/projects/{project}/checklist/gate` answers "is verification good enough to
ship". Only `blocking` severity blocks; advisory and low checks nag. A gate that
fires on everything gets overridden out of habit and stops meaning anything.

## The agent loop over MCP

```
takomo_check_file      declare a check
takomo_cases_file     file the generated case set (upsert by key)
takomo_worklist       what needs re-verifying, and who can clear it
takomo_verdict        record what you observed
takomo_release_push   report the release you merged, and learn what it invalidated
takomo_coverage       the rollup, per epic
takomo_gate           can this ship
```

`takomo_coverage`, `takomo_gate`, `takomo_check`, `takomo_checks`, `takomo_releases`
and `takomo_worklist` are reads and are not charged against the write budget.

## Case identity, and regeneration over time

A case's `key` is derived by the agent from its parameter assignment. Filing a set
upserts by that key:

- still present → keeps its id **and its verdict history**
- gone → **retired**, not deleted; its verdicts stay auditable and it refuses new
  ones
- back again → **revived**, not duplicated

That is what makes regenerating a model after adding a parameter safe. What is
*deferred*: the policy questions around drift — whether a removed case's history
should eventually be reaped, and whether adding a parameter should invalidate every
existing verdict or none. Stable identity ships now so those remain answerable.

## What is not implemented

- **Named configuration profiles.** Cases carry a free-form assignment; there is no
  registry of "this customer runs config X", so coverage is over the space you
  enumerate rather than over real installations. Layerable later without migration.
- **Measured coverage.** Globs are declared, never instrumented. The ingestion shape
  is deliberately left possible.
- **A `/checklist` web surface.** The REST and MCP surfaces are complete; the human
  page is not built.
- **Subsumption and next-best-check ranking.** The data is there (globs, costs,
  counts); the analysis is not.
