# Test definitions and execution attempts

A check and its active cases describe **what must be true**. Their title, steps
and expected outcomes (`body`), precondition, parameters, policy, environments
and linked specification section are editable definitions. The text editor keeps
its existing CRDT replica and websocket collaboration.

A **run** describes **what was tested**. Creating one atomically compares the
selected definition and specification fingerprints with the committed state,
then captures immutable revisions, case parameters, environment details, code
reference and creator. Edits afterward never rewrite that attempt. Definition
revisions are materialized when used by a run, rather than on every keystroke.
Specification revisions include the linked section and ancestors, including prose;
canvas position and appearance do not invalidate a test.

The specification workspace has **Tests → Definitions / Runs**. Its section
counts describe definition coverage; a failure indicator comes from execution
of the current revision. The run composer restores the selected check's durable
local replica and waits for the server's ordered save acknowledgment, together
with the specification save acknowledgment. Offline edits are not silently
excluded. An intervening edit returns `conflict.definition_changed`: refresh the
selection and reconsider before submitting again.

## Execution protocol

1. Read `GET /v1/projects/{project}/test-definitions` (follow `next_offset`).
2. Create `POST /v1/projects/{project}/test-runs`, selecting check IDs and their
   `definition_revision` / `specification_revision`. Supply a `code_ref` naming
   an immutable commit or build, an environment ID/slug when required, and an
   `idempotency_key`. A run accepts 1–100 definitions and at most 5,000 cases.
3. `PATCH /v1/test-runs/{id}` with `{"action":"start"}` atomically claims execution.
   Takomo stores the attempt; the caller performs the tests. There is no automatic
   runner launch. Read the captured instructions and parameters from that run.
4. `POST /v1/test-runs/{id}/results` records `case`, `actor_kind`, `verdict`,
   optional `note`, `evidence` references and an `idempotency_key`. A non-pass
   requires a note. Evidence is a reference, not uploaded file contents.
5. Complete with `{"action":"complete"}` after each case has an execution outcome.
   Only the executor can submit agent outcomes and complete the run. Queued runs
   may be cancelled by their creator; running runs by their executor.
6. Human review uses the same result route with `actor_kind: "human"` and requires
   human scope. For `agent_then_human`, a passing agent observation in **this same
   attempt** is required. Reviews may follow completion; completion is not approval.

Results are immutable per attempt, case and actor kind. A failed test stays
failed in history. `POST /v1/test-runs/{id}/retry` creates a fresh queued attempt
with the same snapshots, code and environment and no inherited outcomes or
approvals. To test changed code or definitions, create a new run instead.
Reuse the same idempotency key and identical payload after a lost response;
reusing a key for different content is a conflict.

All reads enforce project access. Writes require write scope and a writable
project. Mutations notify the existing project websocket and append audit events;
execution records use transactional state transitions rather than editable CRDT
registers. This prevents concurrent executors from overwriting one another.

Hosted MCP exposes `takomo_test_definitions`, `takomo_test_runs`,
`takomo_test_run`, `takomo_test_run_create`, `takomo_test_run_transition`,
`takomo_test_result` and `takomo_test_run_retry`. Create/result tools take their
request in `request`; the REST contract is in `spec/openapi.yaml`.

## Reading results

Definition summaries use the latest non-cancelled attempt in each declared
environment. Missing environments remain `not_executed`. Changed definition or
specification revisions and expired evidence read `outdated`. Different code
references across environments read `mixed_versions`, never overall `verified`.
A queued or running attempt reads `in_progress`; completed execution may still
`need_approval`. Time expiry is measured from execution start; release expiry
counts releases published after that start. A run is evidence for its captured
code version, not a claim about all future code.

Run lists are bounded and contain summaries; read a run by ID for cases and
snapshots. The `definitions` object is keyed by check ID, so thousands of cases
share one captured definition and specification instead of repeating their prose. Follow `next_cursor` until null. Definition lists contain `total`,
`limit`, and `next_offset`; run lists contain `total`, `limit`, and `next_cursor`.
Concurrent definition edits can change an offset page; creation always rechecks
selected fingerprints transactionally.

## Existing integrations and history

Migration imports each existing case verdict as **legacy evidence**, retaining
its actor, timestamp, verdict, note, known environment and release. Original
revision and start time are unknown and remain null. Reopening the store does
not duplicate imports. Legacy evidence is visible in Runs, but cannot be retried
or used to verify a current definition.

The existing check/case/verdict routes and CLI remain compatible. Old verdict
writes also appear as legacy evidence. The old checklist gate, worklist and
coverage APIs retain their legacy verdict semantics; they do not evaluate the
new run ledger. New integrations should use the revision-aware definition
summaries and run records. Do not combine legacy gate results with current
revision verification. The Tests workspace uses the new summaries.
