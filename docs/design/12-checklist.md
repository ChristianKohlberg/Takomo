# Checklist — a verification surface for agent fleets

Design conversation, 2026-07-31. Not yet implemented; no tickets filed. This file is the
record so the context survives the session.

The design was driven by a real application under test, but that application is a client system
and its specifics are deliberately **not** in this repo — Takomo is public. The worked example
below is a neutral stand-in with the same shape. Sizing evidence (parameter counts, generated
case counts) is recorded privately outside this repository.

## The problem

An agent finishes work and says "done". `guard:has_link:commit` makes that claim *attributable*
— any reader can check which commit closed the ticket — but not *verified*. Nobody knows whether
the flow that work touched still functions.

Checklist is the missing half: a durable, structured description of what "working" means for a
product, that an agent can execute against a running app, record results into, and optionally
hand to a human mid-run. Humans supervise by exception, not by default.

## Shape

A tree per product, with the leaf being the thing that gets executed:

```
<product>                     project
├── Reporting                 epic     (same `type: epic` tickets already use)
├── Claims
│   ├── Create a claim        lane     (one traversal of the app)
│   └── Claim approval flow   lane
└── Timesheets
```

Grouping is load-bearing, not decoration: it is what makes a partial result meaningful
("Claims 4/5 green, Timesheets never tested") and what gives policy somewhere to be inherited
from.

## The spine: lanes claim code

Four separate asks all want the same edge to exist — **a mapping from each lane to the parts of
the codebase it exercises**:

| Requirement | What it needs |
|---|---|
| Agent merges a PR; touched paths → retest those lanes | changed paths → lanes claiming them |
| "Coverage really means coverage — confidence we test most of the code" | code as denominator, code claimed by a *live* lane as numerator |
| Avoid one lane already being covered by another | subsumption: `paths(A) ⊆ paths(B)` |
| Show what is *not* covered | code no lane claims |

One relation, four features. It also answers the traversability requirement directly: the agent's
question is a single lookup — *"here are the files in my diff, give me the lanes"* — not a tree
walk it has to reason about.

**Decided: globs, declared by hand.** `src/claims/**` written by whoever authors the lane.
This will rot, and that is accepted for v1 — simple and understandable beats accurate and
unbuilt. The consequence must be stated honestly in the UI: this measures coverage *of declared
surface*, not measured execution. The rejected alternative was instrumented runs reporting real
coverage back into Takomo; the ingestion shape should stay possible, but is not v1.

**Orphan detection, because rot must be visible rather than silent.** Definitions live on the
server while the globs point into a repo, so nothing validates them: a rename in the app under
test orphans a lane, and an orphaned lane reads as *still covered* — the worst failure mode this
feature has, since it inflates confidence exactly where confidence is unwarranted. So the agent
pushing a release **also reports which glob patterns matched zero files in that tree**, and those
lanes are flagged rather than counted toward coverage. This is a field on the release push, not a
new subsystem: the agent already has the tree checked out, so it is the cheapest possible place to
learn the truth.

## Configuration variation — the hard part

The app under test is configurable and configuration changes the flow. In the real driving case
these live in a per-installation key/value settings table: e.g. a *"claimant search mandatory"*
flag on → the flow has a mandatory step; off → it does not. Two flags give four worlds, ten give a
thousand, and some combinations are impossible.

Configuration is not only flat flags. Real settings resolve through a **chain** — a global
default, overridden per customer group, overridden per site — and a chain is *one* parameter with
a resolution rule, not three independent ones. Modelling it as three is how a combinatorial space
explodes for no gain.

So a lane **declares the configuration it assumes** as a precondition, not as prose. From those
declarations Takomo derives (a) which lanes apply to a given configuration, and (b) — the part
that matters most — an explicit statement of what is *not* covered. **Uncovered combinations are
a first-class output, not an absence of output.** A path exercised only under one setting of a
flag is genuinely less covered than one exercised under both, and the model should say so rather
than counting it once.

Full enumeration is off the table for economic reasons. The accepted answer is **pairwise**, per
Microsoft's PICT: cover every *pair* of parameter values at least once, plus declared constraints
for impossible combinations. Takomo should teach this to agents rather than leave them to invent
a strategy.

**The parameter set is much smaller than the field count, and getting that right is the whole
game.** A form with ~100 inputs is not a 100-parameter model. Most inputs are inert data entry —
persisted, no branching — and any single lane that fills the form exercises them. Only inputs that
*change behaviour* belong in the model: conditional mandatoriness, a different save path, a
sub-flow, a distinct side effect. Each of those then collapses to the smallest set of equivalence
classes that still distinguishes behaviour (an actor field becomes `{none, existing, new}`, not
every possible record). Guidance Takomo gives agents must lead with this, because an agent that
treats every field as a parameter produces a model that cannot be run and concludes the method
does not work.

## Releases

Releases become first-class in Takomo; they do not exist today. They need:

- an ordered sequence with a sha/tag, so "every 5 releases" is computable;
- a touched-path set, derived from the diff against the previous release;
- a **zero-match glob report** — which lane globs matched nothing in this tree (see orphan
  detection above);
- **an MCP tool to push one.** No direct integration, ever: the agent that merges the work is the
  thing that tells Takomo a release happened. Work is always pushed back to the agent.

**Invalidation timing:** staleness accrues *per merge* (continuous, cheap); the *release* is
where a gate evaluates. Whether invalidation is hard (blocking) or soft (a nag) is decided by the
lane's severity.

## Policy: inherited, overridable

Two policies resolve down the chain **project → epic → lane**, each level overriding the one
above:

1. **Expiry** — time-based (retest every 6 months / year) *or* release-count-based (every
   release / every 5 / every 10). Whichever trips first.
2. **Verification level** — agent-tested, human-tested, or agent-tested-then-human-approved.

Severity is a field on the lane. At release time, severity × policy resolves to the required
level: low-severity flows clear on an agent's word, a revenue-critical happy path wants a human.
Human time is the scarce resource, so the worklist separates human-required items from the rest.

This maps cleanly onto existing Takomo machinery: a `blocking` question parks a lane awaiting
human judgement and releases the lease; a `tka_` answer grant lets an outside expert judge
exactly one lane and nothing else.

## Where lane definitions live

**Decided: on the server.** Version-controlled files synced into Takomo was attractive — the
definitions would sit next to the code they claim — but the server is the authority for
everything else here (releases, results, policy, coverage), and splitting the source of truth for
v1 buys nothing.

## What a lane body is

**No ordering model, no dependencies between lanes.** A lane body is free-form prose that both an
agent and a human can follow: "open the claims list, traverse to X, trigger Y, confirm Z". The
traversal instructions are the content. Takomo does not model steps, sequencing, or a DAG.

Corollary: Takomo is **agnostic about who executes**. It stores the contract, the schedule, the
severity, the verdict and the coverage — indifferent to whether a Playwright spec, an agent
driving a browser, or a human produced the verdict.

## Takomo stores; the agent computes

**Decided, and it is the load-bearing division of labour.** Takomo does not generate cases, does not
validate a model, and does not check whether a coverage claim is true. The agent owns the recipe and
its correctness; Takomo owns persistence, scheduling, policy and history. "This is the
configuration, this is the recipe, here are the cases" — filed, not adjudicated.

This keeps the store simple and puts the intelligence where the context is. It also means a wrong
model is a wrong model: nothing server-side will catch it. Accepted, because the alternative is
Takomo growing an opinion about every application under test.

Consequence for the executable unit: **each generated case is persisted as its own row.** Per case:

- the full parameter assignment, plus a **stable identity derived from that assignment**, so
  regenerating after a parameter is added matches existing cases instead of orphaning their history
- **when it was last checked**, and the verdict
- **when an agent verified it** and **when a human approved it** — separate facts; a case can carry
  both, and the policy may require both
- the release it was last verified against

Storing rows rather than only the model is what makes the release view cheap: "what changed, what
went stale, what has never been checked" is a query over rows, not a regeneration.

## Kill switches are lanes, not parameters

A parameter that makes every other parameter irrelevant — *no permission ⇒ 403 before anything
runs* — does not belong in the combinatorial model. Crossing it with everything else generates
waste, and PICT warns about it. It is filed as its own **standalone negative lane** with a single
case, generated once. Still authored by the agent; simply not part of the cross-product.

## Lane boundaries are state transitions, not screens

A lane is **one action with one entry precondition** — not "the complaint page". Signals that
something is a separate lane: it requires a persisted record, it has its own permission gate, or it
is reachable only from another lane's terminal state. A create form, a finalize step, a print
action, a send action and a cancel action are five lanes.

Cross-action coupling does not merge them: a value captured during create that changes a document
produced later at print time means the *print* lane carries that value as a parameter of its own.
This is also why lanes need no dependency graph — the precondition is a statement about data state,
which is what keeps them independently runnable.

## Checks are not tickets

A lane has stable identity across releases, is re-run forever, and carries inherited policy. A
ticket is done once. So the leaf is its own object — but the *grouping* reuses `type: epic`
tickets, sharing vocabulary with tickets and inheriting the roadmap rollup, which is already the
right shape.

## Roadmap integration

Two numbers on the roadmap, alongside the existing per-epic done/total rollup:

- how many lanes within each epic are currently verified;
- **the delta against recent releases** — how many were invalidated and need retesting.

## Manageability — the part that makes this wanted rather than endured

Getting coverage *up* is the stated goal, so Takomo should answer two questions:

1. **"What must I retest for this release?"** — the ranked worklist: lanes whose claimed globs
   intersect the release diff, plus lanes whose expiry tripped, ordered by severity, with
   human-required ones separated out.
2. **"What is the best lane I could write next?"** — rank uncovered code by coverage gained per
   unit cost. That needs a cost estimate per lane (agent-minutes, human-minutes). The economic
   constraint then stops being a shrug and becomes a budget the selector optimises against. The
   same machinery flags redundancy: a lane whose path set is a subset of another's, under an
   implied config predicate, buys nothing.

## Settled

| | Decision |
|---|---|
| Lane → code mapping | hand-declared globs; accepted to rot |
| Coverage claim | of *declared surface*, stated honestly; measured coverage deferred |
| Config strategy | pairwise (PICT) + constraints, not full enumeration |
| Releases | first-class, ordered, pushed via **MCP** by the merging agent |
| Orphaned globs | release push reports zero-match patterns; those lanes are flagged, not counted |
| Invalidation | accrues per merge; gate evaluates per release; hard vs soft by severity |
| Policy | expiry + verification level, inherited project → epic → lane |
| Definitions live | on the server |
| Lane body | free-form prose; no ordering, no dependency graph |
| Executor | agnostic — Takomo stores contract, schedule, verdict, coverage |
| Leaf object | not a ticket; grouping reuses `type: epic` |
| Vocabulary | project → epic → lane |
| Division of labour | **Takomo stores; the agent computes.** No server-side model validation |
| Case persistence | every generated case is a row: assignment, stable id, last-checked, agent-verified, human-approved, release |
| Kill switches | lifted out of the model; filed as standalone single-case negative lanes |
| Lane boundary | one action + one entry precondition (state transition, not screen) |
| Authoring process | captured as a skill: `clients/claude-skill/takomo-checklist/` |

## Still open

- **Is the live configuration of a real installation known to Takomo?** i.e. can it say "in this
  customer's config, these 12 lanes apply and 3 never passed" — or is config purely a
  hypothetical space enumerated by hand? Different features.
- ~~Whether "lane" subdivides into individual checks, or is itself atomic.~~ **Answered by the
  sizing spike:** it subdivides, and the subdivision is generated. The executable unit is
  (lane × config assignment); one lane produced 76 of them. **The storage question is now also
  settled** — persist the rows, keep the model alongside; see "Takomo stores; the agent computes".
- **Deferred: what happens when the application's configuration surface changes over time?** Adding
  a parameter, removing a flag, or changing a threshold's meaning invalidates part of a generated
  set — some cases should be added, some updated, some removed. The design accommodates this rather
  than solving it: a case's identity derives from its parameter assignment, so regeneration can
  match survivors and preserve their history instead of orphaning it. The policy questions — does a
  removed case's history survive, does an added parameter invalidate every existing verdict or none
  — are explicitly out of scope for v1 and should be revisited before the model surface is
  considered stable.
- Language: the driving application's domain vocabulary is not English. Lane titles presumably
  stay in the domain language while Takomo's own chrome keeps DE/EN parity.
- Whether a lane declares its config precondition as a full assignment or a partial predicate
  ("requires flag X on, indifferent to the rest"). Partial is more expressive and harder to
  compute coverage from.
- **At which layer was a lane executed, and does the lane declare it?** Open only in the sense that
  it is undecided — the evidence is no longer speculative. Three concrete classes turned up:
  1. Several configuration flags are enforced **only in the frontend**, so an HTTP-level case
     *passes* where the UI would have blocked.
  2. A whole reachable state exists **only** at the API: cancelling an already-finalised record is
     accepted by the service, but the UI hides the button. Testable over HTTP, unreachable via the
     SPA.
  3. The reverse — a documented rule (suppress two documents for an under-14 subject) that fires
     only on a request mode **no UI path ever emits**, so it is live code the SPA cannot reach.

  An API verdict and a UI verdict are therefore not interchangeable in either direction. My
  recommendation is that layer becomes part of a lane's identity and coverage is counted per layer.
  Still your call, and it has real cost.

- **A third case status is needed: `unreachable`.** Follows directly from (2) and (3) above. Today
  the model has covered/uncovered, and both readings are wrong for a case that cannot be reached at
  the declared layer: calling it uncovered reports a gap nobody can close, and calling it covered
  claims verification of code no user path touches. `unreachable` is also the more valuable signal —
  it is how Checklist would surface dead code and UI/API drift as a by-product of coverage
  bookkeeping rather than as a separate audit.

## Status: implemented

The backend shipped on branch `worktree-checklist-design`: schema, store
(`src/store/checklist.rs`), REST (`src/api/checklist.rs`), ten MCP tools, 17
integration tests plus 3 MCP tests, the full `openapi.yaml` contract including 27
error codes, and `docs/checklist.md`.

Decisions taken during implementation, all adaptable:

- **Layer is a lane attribute**, not a per-verdict key. A lane spanning UI and API is
  two lanes. Cleanest reading of the evidence, and it avoids migrating verdicts later.
- **`unreachable` is a first-class verdict** alongside pass / fail / blocked.
- **Named config profiles deferred.** Cases carry a free-form assignment; a profile
  registry can layer on without migration. This is the one prototype element the
  backend does not yet support.
- **Stable case identity shipped**: `key`, derived by the agent from the assignment,
  unique per lane. Retire-not-delete on regeneration, revive on return.
- **Never ≠ stale.** A release only stales cases that were actually verified. Marking
  a never-verified case stale would shrink the never-tested gap the feature exists to
  expose — caught by a test, not by design review.

Still not built: the `/checklist` web surface, subsumption detection, and next-best-lane
ranking.

## Next steps

1. ~~**Sizing spike:**~~ done — 76 pairwise cases, numbers above. Originally: derive a PICT model from a real, large create-form flow and
   find out how many pairwise lanes it actually implies. The answer decides whether this feature
   is tractable at all. Evidence lives outside this repo; only the conclusion belongs here.
2. Prototype the two screens that decide whether this reads as obvious or as bureaucracy — the
   coverage grid and the release gate.
3. Only then: data model and implementation tickets in Takomo.

## Sizing: what the spike actually measured

Run against one large real create-form flow in the driving application. Client specifics stay out
of this repo; these are the numbers and what follows from them.

The funnel, which is the whole argument for tractability:

| Stage | Count |
|---|---|
| Rendered inputs on the form | 168 |
| — inert data entry (persisted, no branching) | 107 |
| — branching | 54 |
| — structural / sub-flow | 7 |
| Model parameters after equivalence-class collapse (incl. 29 config axes) | **60** |
| Full enumeration of those 60 | 2.65 × 10²³ |
| **Pairwise cases generated (2-way, constrained)** | **76** |

Variants, all measured with PICT 3.7.4:

| Variant | Parameters | 2-way | 3-way |
|---|---:|---:|---:|
| One combined model, 27 constraints | 60 | **76** | 503 |
| Same, constraints removed | 60 | 53 | — |
| Split: capture semantics only | 31 | 70 | 458 |
| Split: configuration + 4 anchors | 33 | 30 | 133 |
| Combined, with the happy path seeded (`/e:`) | 60 | **69** | — |

### What follows for the design

- **76 cases for one lane, not thousands.** The method is tractable. But that number is per *lane*
  — and it means **a lane is not the atomic executable unit**. The unit is (lane × config
  assignment). This settles the earlier open question about whether a lane subdivides: it does, and
  the subdivision is *generated*, not authored. Takomo must model that or the roadmap counts a
  76-case lane the same as a 1-case lane.
- **Do not split into multiple models.** The intuitive split cost *more* than the combined model
  (70 + 30 = 100 vs 76), because each sub-model re-pays its own pairwise floor. One model per lane.
- **Seed the hand-written happy path.** PICT's `/e:` honours a seed row and the total came *down*
  (76 → 69). So the right authoring flow is: a human or agent writes the happy path by hand, seeds
  it, and pairwise fills the remainder. The happy path is guaranteed present rather than hopefully
  present.
- **Constraints are a correctness device, not an optimisation.** Adding them *raised* the count
  (53 → 76): excluding combinations costs the generator slot-reuse efficiency. Anyone expecting
  constraints to shrink the suite will conclude they did it wrong.
- **Lift kill-switch parameters out of the model.** PICT warned on three ("all or no values satisfy
  relation") — parameters like *no permission ⇒ 403 before anything runs* that make every other
  parameter irrelevant. Crossing a kill switch with 59 other parameters buys nothing; it deserves
  one negative case of its own. Guidance to agents must say this, because the warning is easy to
  ignore.
- **The economics land on the human, exactly as the policy design assumed.** 76 agent-run cases is
  routine; 76 human-verified cases at a few minutes each is most of a working day for *one* lane.
  So severity × verification level is not a nicety — it is what makes the feature affordable, and
  it must default to sending humans a small high-severity subset.

## A note on tooling, verified during the spike

PICT installs on macOS via `brew install pict` (3.7.4). Default invocation is 2-way; `/o:3` gives
3-way, `/s` prints statistics. `IF … THEN` constraints and sub-models (`{A, B} @ 2`) both work.
Two traps worth writing down because they cost time:

- **Constraint literals are typed.** A numeric parameter needs `= 397` unquoted, a string
  parameter `= "on"` quoted; a mismatch is a hard error, not a warning.
- **Value weights only bias slots that pairwise coverage leaves free.** In a coverage-saturated
  model a 1:20 weighting can have literally no effect, so weights cannot be used to express
  "exercise the common configuration more often". If Takomo wants that, it has to come from lane
  severity or cost, not from the generator.
