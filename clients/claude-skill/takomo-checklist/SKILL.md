---
name: takomo-checklist
description: Author a Checklist check and the test cases beneath it for an application under test — deciding check boundaries, reducing a large form to a combinatorial model, generating cases with Microsoft PICT, and filing them into Takomo with coverage claims and verification policy. Use when asked to add test coverage for a flow, to work out how many cases a flow needs, or to record verification evidence against a release.
---

# Authoring a check

Checklist is Takomo's verification surface: a durable description of what "working" means for an
application, that you execute and record verdicts into. This skill is the authoring process.

**Takomo stores; you compute.** Takomo does not generate your cases, does not validate your model,
and does not check whether your coverage claim is true. You own the recipe and its correctness. That
split is deliberate — it keeps the store simple and keeps the intelligence where the context is.

> The `takomo` verbs for filing checks and cases ship with the Checklist implementation. Until then,
> the method below is the deliverable and the filing step describes *what must be recorded*.
> See `docs/design/12-checklist.md`.

## 1. Draw the check boundary at a state transition, not a screen

A check is **one action with one entry precondition**. It is not "the complaint page".

Getting this wrong is the most common and most expensive mistake, because it silently welds
unrelated behaviour into one unmanageable model. Signals that you are looking at a *separate* check:

- it needs a **persisted record** to exist (it takes an id)
- it has its **own permission gate**
- it is reachable only from a **terminal state** of another check (e.g. only after finalize)

So a create form, a finalize step, a print action, a send-email action and a cancel action are
**five checks**, not one. Each declares its own entry precondition ("a persisted record in status
*final*, plus right N"). Checks have **no ordering and no dependency graph** — the precondition is a
statement about data state, which is what makes them independently runnable.

Watch for **cross-action coupling** and write it down when you find it: a value entered during
create can change the output of a later, separate action (an age captured at create suppressing a
document at print time). That does not merge the checks. It means the print check's model must carry
that value as a parameter of its own.

### Model the lifecycle as a state axis, not a call sequence

"Draft, then finalize" is a sequence of calls; it is not the state space. Derive the real one from
the columns the code actually tests — a status field, a separate boolean latch and a timestamp are
**three independent columns**, and their reachable combinations are the states. Expect surprises:
a record cancelled *after* finalization can be a distinct state from one cancelled before, because
the latch survives.

Then establish, for each transition, whether an **inverse exists**. If nothing clears a status or a
latch, there is no reopen path and that is a fact worth a case of its own. And look specifically for
**write-lock bypasses** — actions that mutate a supposedly locked record. Those are where the
defects live, and a lifecycle axis of `{draft, final}` will never find them.

Beware of things that look like states and are not: "close" is often just navigating away, guarded
by an unsaved-changes check. The testable behaviour there is the **guard racing autosave**, not a
state transition.

## 2. Bucket every input; most of them are not parameters

Walk the flow's inputs and put each in exactly one bucket:

| Bucket | Meaning | Becomes a parameter? |
|---|---|---|
| **A — inert** | persisted, no branching, no conditional validation | **No.** Any one case that fills the form exercises it. |
| **B — branching** | changes what is validated, which path runs, or what fires | Yes |
| **C — structural** | selects between materially different sub-flows | Yes, and pick these first |

On a real large form this ran 107 / 54 / 7 out of 168 inputs. **A form with ~100 fields is not a
100-parameter model** — treating every field as a parameter produces something that cannot be run,
and leads people to conclude the method does not work.

Include **configuration** as parameters: per-installation flags, permissions, and tenant data
presence all branch behaviour and belong in the model alongside the fields.

## 3. Collapse to equivalence classes — relative, never absolute

Each parameter takes the smallest set of values that still distinguishes behaviour. An actor field
becomes `{none, existing, new}`, not every possible record.

**When a threshold is configurable, the classes must be relative to it.** A configurable age
threshold modelled as `{0, 18}` is wrong — it hard-codes today's default and stops testing the
boundary. Model the *relationship*, and give "not configured at all" its own value when the code
paths disagree about the fallback:

```
ThresholdConfigured:  Absent, Off, Low, Default     # Off disables the rule entirely
AgeRelativeToThreshold: WellBelow, JustBelow, AtThreshold, Above
AgeAbsoluteToHardBand:  Below14, Exactly14, Above14
```

**Keep hard-coded constants as their own absolute axis** — do not fold them into the relative one.
The two axes are genuinely independent in both directions: with the threshold at 21 a 16-year-old is
*under* threshold and *over* the hard band; with the threshold at 0 a 12-year-old is *over*
threshold and *under* the hard band. Collapsing them loses both cases.

Check the boundary predicate before choosing values: if the comparison is strict (`<`), then
`AtThreshold` must *not* trigger the rule, and that is exactly the case most likely to be wrong in
the implementation.

**Collapse resolution chains into one parameter.** A setting with a global default, overridden per
group, overridden per site is **one** parameter with a resolution rule — not three independent
ones. Modelling it as three is how a space explodes for no gain. Two booleans that only ever appear
as three meaningful states (`off / optional / mandatory`) are likewise one parameter.

## 4. Lift kill switches out of the model

A parameter that makes every other parameter irrelevant — "no permission ⇒ 403 before anything
runs" — must **not** be crossed with the rest. PICT will warn (`all or no values satisfy relation`)
and the cases it produces are waste.

File each kill switch as its own **standalone negative check** with a single case. It is generated
once and never needs combining.

## 5. Write the impossible combinations as constraints

Anything the code makes unreachable becomes a constraint, each traceable to the line that enforces
it:

```
IF [SubjectMode] = "Anonymous" THEN [AddressKnown] = "No";
IF [DuplicateLine] = "Yes" THEN [LineCapture] IN {"CatalogPick", "ScannedCode"};
```

**Constraints will increase your case count, not reduce it** (measured: 53 → 76). They cost the
generator slot-reuse efficiency. They are a correctness device — do not conclude you did it wrong.

## 6. Generate with PICT

```sh
brew install pict                      # 3.7.4
pict model.pict /s                     # 2-way (pairwise) + statistics
pict model.pict /o:3 /s                # 3-way — expect ~6× the cases
pict model.pict /e:seed.txt            # seed known cases (tab-separated, header = param names)
```

Rules that came out of measuring this, not from the docs:

- **One model per check.** Splitting a model into two costs *more*, because each sub-model re-pays
  its own pairwise floor (measured: 70 + 30 = 100 vs 76 combined).
- **Seed the happy path.** Write it by hand, seed it, let pairwise fill the rest. It is guaranteed
  present instead of hopefully present, and the total went *down* (76 → 69).
- **2-way is the default answer.** 3-way is for a check whose severity justifies it; it is
  agent-runnable, not human-reviewable.
- **Read stderr.** Constraint warnings mean a degenerate constraint or a kill switch you failed to
  lift out.
- **Do not use value weights** to mean "exercise the common configuration more". Weights only bias
  slots that pairwise coverage leaves free; in a saturated model they can have literally no effect.
  Express importance through case severity instead.

Expect roughly **60 parameters → ~76 pairwise cases** for a large form, against a full enumeration
of ~10²³. If your numbers are wildly different, re-check step 2 before believing them.

## 7. File the check and its cases

Record on the **check**:

- where it sits in `project → epic → check`
- its **entry precondition** — the data state and permissions required to start
- **free-form body**: the traversal an agent or human follows. No step model, no DAG — prose.
- **glob claims**: which paths of the app under test this exercises (`src/claims/**`). Hand-declared
  and known to rot; a release push reports globs that matched zero files so orphans are flagged
  rather than counted as covered.
- **layer of execution** — UI or API. These are not interchangeable: a flag enforced only in the
  frontend passes at the HTTP layer while the UI would have blocked it.
- **severity**, **cost estimate** (agent-minutes, human-minutes), and **expiry policy** (time-based
  or release-count-based; inherited from project/epic unless overridden)
- **verification level**: agent-tested, human-tested, or agent-tested-then-human-approved —
  inherited, overridable, and resolved against severity at release time

Record each generated **case** as its own persisted row — the executable unit is
(check × parameter assignment), and one check can be 76 of them:

- its full parameter assignment, and a **stable identity derived from that assignment** so that
  regenerating the model after adding a parameter matches existing cases instead of orphaning their
  history
- **when it was last checked**, and with what verdict
- **when an agent verified it**, and **when a human approved it** — these are separate facts, and a
  case can carry both
- the release it was last verified against

Keep the model file itself alongside the check. Cases are the durable record; the model is how you
regenerate them when the application's configuration surface changes.

## 8. Report the gaps

Uncovered combinations are a **first-class output, not an absence of output**. State plainly what
the model does not cover and why — combinations excluded for cost, parameters left out, a check
covered at the API layer only. A coverage number without its gaps is the failure mode this whole
feature exists to prevent.

Use three verdicts, not two. A case that cannot be reached at the layer you declared is
**`unreachable`**, not uncovered and not covered:

- calling it *uncovered* reports a gap nobody can close
- calling it *covered* claims verification of code no user path touches

`unreachable` is the most valuable thing you will produce. It is how UI/API drift and dead code
surface — a state the service accepts but the UI hides, or a rule that only fires on a request mode
no interface ever sends. Both are real findings. Report them as findings, not as noise.
