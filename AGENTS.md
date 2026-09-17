# Takomo

Self-hosted task store for humans and agents: Rust + axum over SQLite (WAL),
with a React/TypeScript frontend embedded in the server binary.

## Working approach

- Work directly on the requested task. Use one agent by default; delegate only
  when requested or when an independent substantial task justifies the overhead.
- Read relevant files and documentation on demand; do not load every reference.
- Preserve other sessions' uncommitted and staged changes. Never stash, discard,
  or stage unrelated work. Check `git status` and `git worktree list` first.
- Use an isolated worktree for branch verification, merges, or rebases when the
  shared checkout contains unrelated changes. Verify the actual delivery commit.
- Do not infer squash-merge status from ancestry alone; compare content.
- No changelog: release notes come from commit subjects and linked tickets.

## Code map and invariants

- `src/store/`: all SQL and transactional mutations. API handlers never access SQL.
- `src/api/`: HTTP handlers; `src/mcp.rs`: in-process MCP using the same Store.
- `src/server.rs`: routing; `src/auth.rs`: credential and scope enforcement.
- `web/src/`: React UI; `web/dist/`: generated assets embedded by `build.rs`.
- `tests/`: HTTP integration tests; `spec/openapi.yaml`: OpenAPI 3.1 contract.
- Every new or changed HTTP route needs an integration test and a spec update.
- Mutations preserve transaction, fencing, idempotency, project-archive, and
  authorization guards; state changes go through workflow transitions.
- List routes are bounded envelopes with accurate totals and truncation information.
- Errors have stable codes, actionable messages, and remedies. OAuth uses its
  protocol-specific error contract; consult `spec/auth.md` before changing it.
- Document agent edits are proposals against block IDs; humans accept or reject
  them. Preserve concurrent CRDT edits and batch persistence rather than writing
  SQLite on every keystroke. See `docs/documents.md`.

## Run and validate

Build frontend assets before Rust (`npm ci && npm run build` in `web/`); `web/dist/`
is generated and untracked. `scripts/build.sh` builds both for delivery.

Use Backlot for a seeded app instead of manually building, seeding, and serving:

```sh
backlot status
backlot up --ttl 900
backlot ctx
backlot token --role human
backlot release
```

Agents use TTL leases and release explicitly. `BACKLOT_HOLDER_PID=$$` is only
appropriate for a persistent interactive shell. Token output is JSON.

Choose verification by the changed behavior; do not run every suite for every edit.
PRs run static checks, Rust unit tests, and Vitest's affected tests. Full verification
runs nightly and on manual dispatch; merging/pushing to `main` triggers no CI run.

- Documentation only: check instructions, links, and `git diff --check`.
- Rust: run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  relevant tests. Cargo supports target/name filters, not automatic delta testing:
  `cargo test --test api <substring>`, `cargo test --test oauth`, or
  `cargo test --lib <substring>`. Check the executed count: zero tests is no evidence.
- Frontend (in `web/`): `npm run check`, `npm run lint`, and
  `npm run test:changed -- <base-ref>` use Vitest's native import graph. With no
  ref it examines uncommitted changes. New/untracked tests should be run explicitly.
  If selection is empty or misses the behavior, run the relevant file or `npm test`.
  Changes outside the import graph need explicit checks; layout changes need a browser.
- Shared auth, store, workflow, schema, synchronization, or dependency changes:
  broaden to the relevant integration suites, or the full suite when impact is unclear.
  Full release verification is `cargo test --release`; do not invent a Rust delta mapper.
- Frontend delivery: `npm run build` and `npm run size`; keep generated assets ignored and untracked.
- MCP client: `npm ci && npm run build` in `clients/mcp/`; for behavior changes,
  run `python3 scripts/test-mcp-local.py <built-takomo-binary>` from the repo root.
- Shell: `shellcheck -x clients/cli/takomo clients/cli/install.sh scripts/*.sh`.
  Workflow changes: run `actionlint` and exercise changed commands locally.

Do not rerun passing checks without a relevant change. Report what ran, its result,
the revision/worktree tested, and skipped or unmeasured layers. A nightly schedule
is not evidence that a pending change passed. See `docs/testing.md` for coverage
commands, layer definitions, and limitations; coverage has no enforced thresholds.

Use a private `CARGO_TARGET_DIR` for trustworthy verification when another
session may build concurrently. Do not commit machine-specific cache paths.
For UI development, `npm run dev` in `web/` proxies to Backlot. To serve embedded
UI changes, build `web/` first, then `cargo build --release`; source edits alone
cannot change the assets embedded in an existing binary.

## Frontend conventions

- Keep generated assets flat under `web/dist/assets/`; preserve the manifest contract.
- Render user text through the safe markdown utilities, never raw `innerHTML`.
- Shared code belongs in `web/src/components/` or `web/src/lib/`.
- Export shared components from `web/src/components/index.ts` for design sync.
- Preserve locale parity and mobile layouts; `md` is the phone/desktop breakpoint.
- Consult `web/README.md` for UI contracts; jsdom does not verify browser layout.

## Read only when relevant

- `docs/development.md`: development workflow.
- `CLAUDE.md`: additional build and product conventions.
- `docs/validation.md`: impact-based validation and release policy.
- `spec/auth.md`, `spec/workflow-format.md`: authorization and workflow contracts.
- `docs/documents.md`, `docs/initiatives.md`, `docs/mindmaps.md`: collaborative content.
- `docs/ask-a-human.md`, `docs/users.md`, `docs/epic-claims.md`: people and work ownership.
- `docs/checklist.md`, `docs/environments.md`: verification and environments.
- `docs/hosting.md`, `docs/hosted-mcp-clients.md`: deployment and hosted clients.

`AGENTS.md` owns verification policy; consult `CLAUDE.md` for additional conventions.
