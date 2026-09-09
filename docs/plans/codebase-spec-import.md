# Codebase-to-specification import: implementation plan

Status: implementation started; the scope/preflight foundation is available (see `docs/codebase-spec-import.md`). The rest of this document is the implementation contract, not a claim of completed features. Original planning baseline: agent-service branch commit `a5f8aae`, in `/tmp/takomo-codebase-spec-approach`. The original checkout was `eb3a9dc` and did not contain this service. Before implementation, reconcile this plan against the actual integration base; do not merge another session’s branch implicitly.

## Development scope amendment (9 September 2026)

The user approved starting implementation and requested partial-codebase runs to
keep the development loop fast and inexpensive. Every new import must explicitly
select files/directories, allow exclusions, pin the commit, and retain that scope
through retries and tasks. Scope is enforced on listing, search and direct reads;
dependencies outside it are reported, never followed automatically. Preflight
requires no inference and refuses selections above file/byte ceilings rather than
sampling. Develop on committed fixtures and one package/file before larger model
evaluations. Current implementation base is `73c9293` from `origin/main`, in the
separate `feat/codebase-spec-import` worktree.

## 1. Product contract

A user selects a configured repository and revision from an empty Takomo specification, starts an import, and watches a staged tree of sections fill with evidence-backed descriptions. They can inspect sources, edit the draft, pause generation, retry failed work, reject branches, and accept ready branches into the document. Accepted nodes appear in both document and map views because a node is already a section.

Generate an as-built specification: observed capabilities, actors, inputs, outputs, business rules, state transitions, permissions, failure behavior and known limitations. Label inferences and contradictions explicitly. Code does not establish product intent; tests inspected as source do not establish passing runtime behavior. Follow the project’s writing instructions, captured at run start; changing them creates a new policy revision for subsequent draft work.

First release: one configured Git repository, one fixed commit, one initially empty target, sequential analysis tasks, staged generation, online branch acceptance, and basic rich prose. No executing repository code. No automatic acceptance, arbitrary repository cloning, cross-repository import, runtime verification, diagrams/tables/code-block generation, or incremental refresh in this release. Those are separately scoped extensions. No embeddings dependency.

Generation requires one start action; outline creation is not a mandatory approval checkpoint. Users may intervene while the run continues. Publication is a separate explicit action. Retain partial results when work stops. “Complete” means the planned analysis and reconciliation finished, not that every behavior has been proven.

## 2. Architecture and reuse

Takomo owns the durable import run, task scheduling, staged sections, evidence, review state and publication. The agent service owns repository access and Codex execution. Codex receives bounded contexts and tools and returns schema-constrained analysis. Model output never directly mutates the canonical document.

Reuse the queue, worker identity, leases, fenced result delivery and inspector in `src/store/agent_chat.rs`, `src/api/agent_chat.rs`, and `services/agent/service.mjs`. Add `spec_import` to supported kinds; do not disguise it as bug research or document chat. Update kind inference, supported-kind validation (currently capped at four), claim filtering, deadlines, result validation, inspector labels and feature reporting together.

Use one import conversation anchor that cannot collide with document-chat’s empty node anchor or real node IDs. Define and enforce a reserved anchor format. Reuse the existing one-active-job-per-conversation restriction. Schedule the next task only when the previous result is committed. Each analysis task starts a fresh Codex thread, even though jobs share an import conversation for queue bookkeeping; explicitly avoid inheriting that conversation’s previous thread. Task retries use a new attempt/thread with durable context. Existing chat resume behavior stays unchanged.

The import profile reuses restricted repository access, experimental dynamic-tool negotiation and effective configuration checks. Only the host receives the Takomo credential and configured repository paths. Repository text, including instruction files, is evidence, not executable policy. Existing no-tools document chat remains no-tools.

## 3. Durable records and state machines

Add `src/store/spec_import.sql`, `src/store/spec_import.rs`, and `src/api/spec_import.rs`. Register additive migrations in `src/store/mod.rs` and routes in the established router. Concrete schema names below are proposed.

- `spec_import_runs`: project, target mindmap, requester, request id, requested revision, resolved commit, repository key, include/exclude scope, policy/model/schema versions, outline version, status, stop reason, budget settings, consumed usage, timestamps, and target baseline fingerprint. Uniqueness on requester/project/request id prevents duplicate starts. Permit only one active import for a target initially.
- `spec_import_tasks`: run, stable task key, stage, section key, dependency, input digest, output revision, status, attempt count and last error. A unique run/task/input key prevents duplicate scheduling. Snapshot the task inputs rather than reading mutable inputs halfway through a turn.
- `spec_import_jobs`: existing agent-job FK, run/task FK, attempt input digest, resolved commit and structured result. Keep queue execution status separate from import status.
- `spec_import_sections`: run, stable draft key, parent draft key, sibling order, title, purpose, structured prose, draft revision, analysis state and review state. Store immutable draft revisions or a revision history so review references the exact content seen. Canonical node mapping belongs in publication records.
- `spec_import_evidence`: run, evidence id, commit, path, blob id, range, bounded excerpt/hash and source category; claim-to-evidence links identify the statement being supported. Store contradiction/gap records explicitly.
- `spec_import_publications`: acceptance request id, run, selected draft revision set, target precondition, operation key, generated node/block mappings and committed result. This record must commit atomically with canonical content.

Worker-side cache: manifest, search metadata and blob reads keyed by repository/commit or blob identity. It is disposable; losing the cache must not lose import progress. Persist bounded coverage records centrally (path/category/disposition/section mapping), paginate them, and enforce a storage budget. Large excerpts and generated artifacts are not copied into every job snapshot.

Run states: `queued → discovering → outlining → drafting → reconciling → ready_for_review`. From generation states, transition through `pausing → paused`, or to `cancelled` / `failed`. `partial` records budget exhaustion with completed drafts available. Review completion is separate from generation completion: report accepted/rejected/pending counts instead of forcing publication into the generation state machine.

Task states: pending, queued, running, completed, failed, cancelled, superseded. Review states: pending, accepted, rejected. Pause stops new dispatch and lets the current bounded task finish; cancel requests interruption and fences further results. An explicit resume schedules unfinished work against the same pinned commit. Failed model tasks are never automatically rerun. An explicit retry creates a new attempt; transport retries reuse the same result identity. A new repository revision requires a new run.

## 4. HTTP and permission contract

Proposed routes, finalized with OpenAPI and integration tests before UI implementation:

- `POST /v1/mindmaps/{id}/imports`: repository key, revision, scope, writing options, budget preset, request id. Return 202 with run id and queued status. Validate project access, writable/unarchived target and empty-target eligibility. Repository paths come only from worker configuration.
- `GET /v1/mindmaps/{id}/imports` and `GET /v1/spec-imports/{run}`: list/detail with stage, progress, limitations and review counts.
- `GET /v1/spec-imports/{run}/sections`, `/evidence`, `/coverage`, `/events`: bounded, cursor-paginated reads. Evidence access remains project-scoped; never expose arbitrary local paths or credentials. A source viewer resolves only the run’s configured repository and pinned commit.
- `POST /v1/spec-imports/{run}/pause`, `/resume`, `/cancel`: idempotent controls. Resume/retry explicitly define which unfinished tasks will consume more inference.
- `POST /v1/spec-imports/{run}/tasks/{task}/retry`: explicit new attempt after failure, retaining earlier evidence/history.
- `PATCH /v1/spec-imports/{run}/sections/{key}`: human draft changes with expected revision. If analysis was based on an older draft/outline, preserve its result as superseded instead of overwriting the human change.
- `POST /v1/spec-imports/{run}/accept`: selected branch keys, exact draft revisions, request id and target precondition. Return the canonical node mapping or a structured conflict.
- `POST /v1/spec-imports/{run}/reject`: selected branches and expected draft revisions; record the decision without deleting history.

Reads require project read access. Starting/editing/controlling runs requires project write access. First-release acceptance/rejection additionally requires human scope. Worker `agent:run` remains queue-only and cannot publish or edit target documents. Revalidate archive status, project restrictions and current authorization on every mutation, including final acceptance. Authorship records identify both the generating agent/run and the accepting user; acceptance must not relabel generated content as human-authored.

Extend existing heartbeat/result routes for `spec_import`, with attempt identity, task input digest, pinned revision, bounded progress and validated structured results. Add an attempt-fenced checkpoint endpoint for evidence batches if heartbeats cannot carry them efficiently. Reject unknown fields, cross-run IDs and over-budget payloads. Reuse existing project events with stage/task metadata; UI polling is sufficient initially, without forwarding model reasoning or raw token streams.

## 5. Repository access and scalable discovery

First make the current Git tools exhaustive within their declared scope. `repository_files` needs stable cursor pagination and path prefix filters; `repository_search` needs path filters, bounded result pagination and continuation metadata; `repository_read` needs explicit byte-safe truncation and reliable line ranges. Use argument arrays, literal matching by default, normalized repository-relative paths and the pinned commit. Current global Git output caps can reject a large manifest before pagination; stream and bound subprocess output rather than collecting the entire listing/search in memory.

Inventory tracked regular files with size/type/category metadata. Exclude binary, generated, vendored and configured sensitive content before returning it to the model. Report excluded, inaccessible, oversized, symlink and submodule entries with reasons. Apply the same policy to lists, search, reads and source viewers. Ignore rules are configurable/versioned; useful committed docs and fixtures must not disappear merely because they resemble generated content.

Resolve the commit once and retain its objects for the run’s retention period using a host-owned Git reference or isolated object cache; otherwise garbage collection can make a resumable run unreadable. Record inaccessible/LFS-pointer/submodule content rather than pretending it was inspected. If the worker is replaced, it needs the same repository mapping and commit; it must never silently substitute HEAD.

Create deterministic candidate inventories of manifests, entry points, API definitions, schemas and tests. Report actual counts and sampling decisions. Split discovery into bounded tasks by package or meaningful area; the planner sees compact summaries and the complete area inventory, not an unbounded tree. Large scopes exceeding configured inventory limits become a visible partial run or scope error, never silent success.

## 6. Agent workflow and schemas

Implement `services/agent/spec-import.mjs` for task inputs, instructions, schemas and validation. Refactor kind-specific Codex policies only enough to register the new profile; avoid an unrelated universal agent framework.

Stages:

1. Inventory: host generates repository metadata without model execution.
2. Discover: bounded model tasks identify capabilities and evidence leads per area, explicitly including routes, persistence, authorization, configuration and failure handling.
3. Outline: one synthesis task proposes the hierarchy and glossary. Validate a connected acyclic forest, stable keys, parent links, ordering, title limits, depth policy and the existing 500-section total cap. Allocate section capacity globally rather than allowing each worker to invent an unlimited subtree.
4. Draft: one task per section with ancestor context, compact overall outline, shared glossary, relevant evidence and writing policy. Output behavior, rules, failure cases, limitations, claims, evidence IDs and proposed child topics. New children enter a versioned outline update and capacity check before being scheduled.
5. Verify: a separate bounded pass checks claims against source and records unsupported/contradictory statements. Citation validity is mechanically checked; source support still requires semantic assessment and review.
6. Reconcile: first per branch, then across branch summaries. Normalize terminology, identify duplicates and cross-cutting gaps. Return proposed revisions, not silent changes to accepted sections.

Use existing repository dynamic tools plus read-only `import_outline_read` and `import_evidence_read` tools. Prefer one structured task result via `turn/start.outputSchema` over a model-controlled document-write tool. Host-owned checkpoints capture retrieved evidence/progress; validated results persist draft changes. If task-level partial submissions are later necessary, give them separate idempotent keys and a schema, not general document mutation powers.

A draft result carries `task_key`, `input_digest`, `section_key`, `title`, `blocks`, `claims`, `evidence_ids`, `proposed_children`, `gaps` and `summary`. Blocks are a strict subset: paragraphs and bullet/ordered lists with text items. The section title supplies the heading. No arbitrary HTML or raw Yjs updates. Evidence references must have been issued by the host for this run; an unobserved reference is rejected. Content unsupported by evidence must be marked inference or unknown.

Provisional configurable defaults for initial evaluation: one active task per run, 15-minute task wall deadline, 100 repository calls per task, 64 KB structured result limit, 200 planned sections and a 500-section hard document ceiling. Cap task count, wall time and tool/output usage for the entire run too. Model-token budgeting depends on verified usage notifications from the installed protocol; reserve capacity for active tasks and label unavailable token/cost telemetry unknown. A soft token target is not a hard spending guarantee. Derive final presets from evaluations rather than presenting these initial limits as proven sizing.

## 7. Publication: the critical persistence milestone

Do not implement acceptance as browser calls to create each node followed by a proposal per paragraph. A lost response or two reviewers could create duplicate or half-written trees. Existing browser proposal acceptance explicitly documents a concurrent-accept race in `web/src/lib/plan-proposals.ts`.

Implement one server-owned acceptance operation for a bounded branch. Resolve draft keys into canonical node IDs in parent-before-child order inside the operation, with deterministic/stored mappings. Validate draft revisions, complete dependencies, document capacity and target preconditions before any live mutation. Accepting a child includes pending ancestors explicitly in the preview; accepted ancestors are reused. A partially accepted branch cannot be silently regenerated into a different hierarchy.

Persist publication record, canonical CRDT update, relational projections, evidence mappings and trace in one SQLite transaction before broadcast. Prepare changes on an isolated replica under the established room serialization boundary; on failure, discard the candidate and leave live state unchanged. Replay committed state after a crash. Publication retry returns the stored mapping. Two reviewers accepting the same revision converge on one publication; conflicting decisions receive a structured conflict. Prove compatibility with normal websocket mutations and lock ordering before building on this path.

For first-release rich prose, extend the existing narrow `src/store/prose.rs` creation path with a strictly validated paragraph/list writer for newly created sections only. Test its Yjs/Tiptap roundtrip against the editor schema. Do not send Markdown through the notes API: that would lose formatting, and replacing notes can reset existing prose/block IDs. Existing accepted prose edits continue through the existing review path. Arbitrary rich content would require a shared schema-aware renderer and is separate work.

The target must be empty at start. During publication, tolerate earlier publications from the same import through stored mappings, while rejecting unrelated target-tree changes against the captured/rebased baseline. Relevant preconditions cover tree structure and affected sections, not ephemeral awareness. If a human changes an accepted parent, pauses or resumes generation, or edits a staged section, report the applicable conflict/revision change; never replace their content. Provide explicit rebase/review to proceed. Import acceptance is online only.

## 8. User experience

Add an empty-document “Generate from codebase” action in the existing document surface. Setup chooses a configured repository, revision, scope and depth/budget preset; show what will be excluded and that output describes current implementation. Keep absolute host paths, tool flags and process details out of this flow.

Show staged tree on the left, section preview in the center, and source/gap details on demand. This is an import workspace associated with the document, not a second canonical specification. Progress labels are observable facts: areas inventoried, sections drafted/verified and tasks remaining. Do not present a fabricated percentage of total product understanding.

Each section shows analysis and review status separately, claim sources and unresolved questions. Users may edit draft title/prose and reject or accept a branch. Show exactly which ancestors and descendants an acceptance includes. Persist selections and draft changes through navigation/reload. Warn about a newer draft revision before acceptance instead of accepting unseen content. Accepted sections link to the real document/map location.

Pause is “finish current section, then pause”; cancel interrupts current work and retains completed drafts. Failure shows a readable reason and scoped retry. Budget exhaustion offers continuation with an explicit additional budget. Completion shows coverage, exclusions and unresolved gaps alongside the generated draft. Add the import kind to Agent queues with links back to run/task details. Extend localized strings and export new shared components through the component barrel.

Suggested files: `web/src/components/documents/SpecImport*.tsx`, `web/src/lib/spec-imports.ts`, document `Plan.tsx`/actions, and agent-queue inspector. Use existing appearance tokens, responsive layout and access checks. No dedicated top-level navigation entry is needed initially.

## 9. Evaluation and verification

Build versioned fixture repositories: a small service with known rules; a medium application with UI/API disagreements and conflicting docs; and a generated monorepo with more files/search hits than current tool caps. Include excluded paths, Unicode/long lines, symlinks, submodules, large files, indirect authorization checks and misleading instructions in repository text.

Mechanical gates: complete paginated enumeration within declared scope, valid hierarchy and references, no duplicate publication, no cross-project leakage, no writes from stale/cancelled attempts, and no lost concurrent human edits. Test restart after each persistence boundary: before result commit, after commit before acknowledgment, and after acceptance commit before response/broadcast. Cover worker replacement, object retention, budget exhaustion, draft edits in flight and simultaneous accept/reject.

Quality evaluations use human-authored expected capabilities and claim checks. Measure claim support precision, weighted capability recall against the fixture’s declared truth, duplication, contradiction visibility, section usefulness, human correction time, token usage and latency per accepted section. Do not equate files read with feature coverage or rely solely on the model grading itself.

Proposed initial quality gate: zero unsupported critical permission/data-loss claims in the reviewed sample, at least 95% supported factual claims, and at least 90% recall of annotated critical capabilities on fixtures. Treat these as target thresholds to calibrate and report with sample sizes, not measured performance or a guarantee for arbitrary repositories. Record model, policy, schema, commit, budgets and evaluation version for every run. Test model/reasoning alternatives on identical tasks; improve the failing stage before adding more compute.

Focused tests during each milestone: Rust API/store tests, Node fake-app-server tests, frontend behavior tests and relevant browser checks in backlot. An opt-in authenticated live smoke must prove repository tools executed and structured drafts arrived; it must not write a live user specification. At integration, use no-mistakes. Persistence/permissions/recovery changes require the repository’s full high-risk validation, full CI on the release SHA, migration/restore checks and documented rollback. This planning task does not run those pipelines or live model jobs.

## 10. Delivery sequence and acceptance gates

M0 — Baseline and contracts. Confirm integration base and supported Codex version. Capture schemas, state transitions, permission matrix, task budgets, failure semantics and fixture truth. Deliver OpenAPI draft and a short publication transaction spike. Exit: prove the canonical acceptance path is viable without partial CRDT/SQL commits.

M1 — Import persistence and queue. Add run/task tables, start/status/control endpoints, job-kind support and fake worker results. Exit: a synthetic multi-task run survives restart and cancellation without duplicate scheduling; legacy workers do not claim imports and chat/research behavior remains unchanged.

M2 — Exhaustive repository access. Implement inventory, pagination, path filters, exclusions, object retention, bounded streaming and evidence IDs. Exit: large fixture enumeration is complete or explicitly partial, with reproducible source references and bounded memory/output.

M3 — Outline and drafting engine. Add import profile, stage schemas, fresh task threads, durable context, glossary, sequential dispatch and result validation. Exit: real provider smoke produces a staged hierarchy and sourced prose at a fixed commit without canonical document writes.

M4 — Atomic branch publication. Add schema-limited prose creation, parent mapping, acceptance transaction, provenance and conflict behavior. Exit: two clients, lost responses, crashes and human edits never produce duplicate/partial accepted branches or lost content. This is the highest-risk milestone.

M5 — Product UI. Add setup, staged tree, source inspection, draft editing, branch decisions, progress and controls. Exit: the complete empty-document-to-reviewed-specification journey works in the browser, including reload, narrow viewport, read-only access and a failed task.

M6 — Quality and scale. Add verification/reconciliation passes, coverage reporting and calibrated presets. Run benchmark fixtures and a representative real repository, report cost/runtime/quality with limitations. Exit: mechanical gates pass and the documented quality targets are met or the scope is explicitly revised before exposure.

M7 — Controlled rollout. Document worker setup/CLI compatibility, migrations, retention and operating limits. Enable behind server feature flag plus compatible-worker capability checks for a small project cohort. Exit: full release validation and observed successful recovery/publication before widening access.

Dependencies: M0 → M1; M2 can be developed after M0 alongside M1 if separate implementation capacity is authorized; M3 needs M1+M2; M4 needs M1 and the M0 persistence proof; M5 needs stable M1/M3/M4 contracts; M6 completes after the full journey exists; M7 follows all gates. Organize PRs around these milestones, splitting the publication/persistence work when needed for review. Do not estimate calendar time until M0 has resolved the transaction work and M2/M3 have produced a real benchmark.

## 11. Operations, rollback and later extensions

Feature flag off prevents new starts, dispatch and acceptance; explicitly cancel/fence in-flight runs during an emergency disable, preserving drafts and accepted content. Roll back the service and UI only to a version verified compatible with the additive schema. Do not drop import tables or erase accepted sections. Before production migration, test backup/restore and old-version startup against the migrated database. Delete expired caches under a documented retention policy; retain source references/review history with the document and report when original source is unavailable.

Log run/task/attempt IDs, stage durations, tool counts, truncation, usage and reason codes. Never log credentials or unrestricted source excerpts. Track queue wait separately from model execution. Alert on repeated lease failures, stuck runs and publication conflicts. A project-level concurrency cap and round-robin task scheduling prevent long imports starving conversational jobs; start with one import task per project and preserve the existing one-job-per-worker constraint.

Later extensions, each with its own quality gate: bounded parallel branch research and a shared glossary/outline owner; symbol/import indexes; semantic retrieval; richer prose/diagrams; multi-repository scope; optional user-authorized automatic draft publication; incremental refresh using evidence-to-section links plus dependency impact checks. None should silently reinterpret accepted as-built descriptions as desired requirements.

## References

Local evidence at `a5f8aae`: `services/agent/{README.md,codex.mjs,repository.mjs,service.mjs}`, `src/store/{agent_chat.rs,agent_chat.sql,document_chat.sql,mindmapdoc.rs,prose.rs}`, `src/api/mindmaps.rs`, `web/src/lib/plan-proposals.ts`, `docs/{mindmaps.md,validation.md}`.

Official protocol: https://learn.chatgpt.com/docs/app-server — threads, turn schemas/events and experimental dynamic tools. Verify against the installed CLI’s generated schema at implementation time. No new current-model/pricing claims are made in this plan.
