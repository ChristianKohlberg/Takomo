---
name: takomo-verification
description: Keep Takomo's verification current for an application — describing behaviors from the specification, linking the tests that show them, choosing which variants to run, and reporting runs from CI or by hand. Use when asked to add test coverage for a flow, to verify a behavior, to report test results, or to find what is untested.
---

# Verifying behavior with Takomo

A **behavior** is what the software must do, in words a person can check. Tests are
external; Takomo stores only the key a runner reports for each, linked to the
behaviors it shows. A **run** reports pass/fail per key against a commit, and each
behavior's status follows from the latest results: `failing`, `verified` (a pass
within 14 days), `stale`, or `untested`. Details: `docs/verification.md`.

**Takomo stores; you compute.** It does not generate variants, judge whether a test
really covers a behavior, or run anything. You own those judgements — and you record
them, in the behavior text and in the run's note.

## 0. Start from where things stand

`takomo_verification` (or `takomo verify`): counts per status and per plan section,
and the reported test keys no behavior links. Work in this order: failing, then
untested behaviors in sections that matter, then unlinked tests, then stale.

## 1. Describe behaviors from the specification, not from the code

Read the plan section (`takomo_plan_read`) and restate one obligation per behavior:
conditions, action, expected result.

- **Do not invent requirements.** A behavior restates the specification (or a
  regression someone observed). If the success condition is ambiguous, ask — do not
  pick a pass/fail rule yourself.
- **Draw the boundary at a state transition, not a screen.** Create, finalize, print
  and cancel are separate behaviors: each has its own precondition and its own way
  to fail.
- **Name the layer when it matters.** A rule enforced only in the UI passes over
  HTTP. If both must hold, link a UI test *and* an API test — one behavior, two keys.
- Link it to its section (`section` = node id) so the specification shows what is
  covered. Leave `section` out only for behavior the specification does not state yet.

## 2. Choose variants deliberately

One behavior often has many variants (roles, configuration flags, data states). You
will not run all of them every time, and you do not need to: one fresh pass verifies
the behavior, and any failure fails it.

- **Bucket inputs.** Inert fields (stored, no branching) are not parameters — any one
  test exercises them. Only branching and structural inputs are. A 100-field form is
  rarely more than a handful of parameters.
- **Collapse to equivalence classes**, relative to configurable thresholds
  (`{below, at, above}`), not today's literal defaults.
- **Generate combinations** when there are several parameters: pairwise (e.g.
  Microsoft PICT) keeps the count tractable. Each generated combination worth
  keeping becomes a test with its own key, linked to the same behavior.
- **When you pick a subset to run**, say why in the run's `note` ("split logic
  changed in a1b2c3; ran currency and multi-entity variants"). The next reader sees
  what was skipped and why.

## 3. Link the tests

Use the key the runner prints, prefixed by runner: `cargo:api::save_conflict`,
`vitest:editor undo`, `playwright:invoices.spec.ts › split`. Link with
`takomo_behavior_update` with `add_tests` (or `remove_tests` to unlink) or
`takomo behavior link ID KEY...`. Avoid `tests`: it replaces the whole list and
drops a link another agent made meanwhile.

No automated test yet? Link `agent:<slug>`: the behavior's statement is your script.
Verify it against a registered environment (`takomo_environments`) and report the
result like any other test. Replacing it with an automated test later is linking a
new key.

## 4. Report runs

One run per suite execution, against the commit it ran on:

- MCP: `takomo_run_report { project, commit, note, results: [{test, outcome, detail?}] }`
- CLI: `takomo report --commit SHA --pass KEY --fail KEY --detail "…"` or `--file results.json`
- CI: one `POST /v1/projects/{project}/runs` with an `Idempotency-Key`.

A test that could not run is a `fail` with the reason in `detail` (an environment
problem is still a finding). A skipped test is left out. Report what you observed,
not what you expect.

## 5. Close the loop

The run's reply lists keys no behavior links. For each: link it to the behavior it
shows, or describe the behavior it verifies — or say that it verifies nothing the
specification asks for. Then re-read `takomo_verification`.
