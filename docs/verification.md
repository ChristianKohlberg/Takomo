# Verification — behaviors, linked tests, reported runs

The goal is confidence: if everything linked passes, the software does what the
specification asks. Tests alone cannot give that, because a green suite says
nothing about what nobody tested. So Takomo keeps the list of what must work —
**behaviors** — and treats test results as evidence against it.

```
Specification section
└── Behavior        what must work, in words a person can check
    └── test key    a concrete check that shows it, e.g. playwright:editor.spec.ts › keeps edits
        └── result  pass | fail, from one reported run (commit, when, who, why)
```

Terminology follows [Specification, verification, and evidence](specification-verification.md):
a behavior is a verification case, and results are the evidence.

## The model

| Table | Holds |
|---|---|
| `behaviors` | `title`, `statement`, optional plan `section` (node id) |
| `behavior_tests` | which test keys verify a behavior; many-to-many |
| `verification_runs` | one report: `commit`, `note`, actor, time |
| `verification_results` | per run and test key: `outcome` (`pass`/`fail`), `detail` |

**Tests are keys, not records.** Takomo never stores test code. A test exists once
a behavior links its key or a run reports it. Use the identifier your runner
prints; prefix the runner so keys stay unique (`cargo:…`, `vitest:…`,
`playwright:…`).

**Agent verification is just another key.** When no automated test exists yet,
link a key such as `agent:failed-save-retry`; the behavior's statement is the
script an agent follows, and it reports the result like any other test. Replacing
it with an automated test later is linking a new key, not a remodel.

## Status is computed, never stored

From the latest result of each linked key:

| Status | When |
|---|---|
| `failing` | any linked key's latest result is `fail` |
| `verified` | otherwise, some linked key passed within 14 days |
| `stale` | otherwise, a linked key has results, but none is fresh |
| `untested` | no linked key has ever reported |

**One fresh pass is enough for `verified`.** A behavior may have many variants and
nobody runs all of them every time; which to run is the reporter's judgement,
recorded in the run's `note`. A failure anywhere still wins, however old — it
stays failing until that test passes again.

**Freshness is by age.** Every result shows its commit and time, so a reader can
tell current evidence from old, but a new commit does not make everything stale:
that would reset verification on every merge and the view would stop meaning
anything.

## The loop

1. **Describe** what must work: `POST /v1/projects/{project}/behaviors`, tied to
   the plan section it comes from when there is one. A behavior restates the
   specification; it must not silently add requirements. If a success condition
   is unclear, clarify the specification instead.
2. **Link** the tests that show it: `PATCH /v1/behaviors/{id}` with `tests`
   (replaces the list).
3. **Report** runs from CI or an agent: `POST /v1/projects/{project}/runs` with
   `commit`, `note` and `results`. Send an `Idempotency-Key` header so a retried
   report records once. The reply lists reported keys no behavior links.
4. **Read** where things stand: `GET /v1/projects/{project}/verification` gives
   status counts overall and per section, behaviors with no section, the
   reported tests no behavior links, and the latest run.

```sh
# CI, after the suite ran — one call:
curl -X POST "$TAKOMO_URL/v1/projects/$PROJECT/runs" \
  -H "Authorization: Bearer $TAKOMO_TOKEN" -H "Idempotency-Key: ci-$RUN_ID" \
  -H 'Content-Type: application/json' \
  -d '{"commit":"'"$SHA"'","results":[{"test":"cargo:api::save_conflict","outcome":"pass"}]}'
```

A test that could not run is a `fail` with the reason in `detail`. A skipped
test is left out of the report.

## Gaps worth reading

- **Untested and stale behaviors** — work to do.
- **Reported tests no behavior links** — tested but not described; link them or
  write the behavior they verify.
- **Behaviors with no section**, and sections with no behaviors — drift between
  the specification and what is checked.

A section removed from the plan leaves its behaviors in place; the UI shows the
link as missing rather than deleting anything.

## Surfaces

- **UI:** Specification → Verification and Evidence
  (`/projects/{project}/specification?view=tests`). Section counts appear in the
  Document and Map views; `?behavior=<id>` opens one.
- **MCP:** `takomo_verification`, `takomo_behaviors`, `takomo_behavior`,
  `takomo_behavior_create`, `takomo_behavior_update`, `takomo_run_report`,
  `takomo_runs`. Reads are not charged against the write budget.
- **CLI:** `takomo verify`, `takomo report`, `takomo behavior ls|new|show|link|unlink|set|rm`.
- **REST:** `spec/openapi.yaml`, tag `verification`. Live updates publish the
  `behaviors` project topic.

## Authorization and guards

Reads need `read`; every write needs `write` and a writable project — an
archived project refuses behavior and run writes. Deleting a behavior removes
its links but keeps reported results, which are evidence about tests and may
back another behavior.

## Deliberately not built

Priorities or tiers on behaviors, a release gate, per-environment requirements,
human sign-off, parameterised case generation, path-based coverage and release
staling. The previous checklist and test-run model had all of these and was
replaced before production because the machinery outweighed the answer it gave.
Each can return as a column or a table without changing what exists.
