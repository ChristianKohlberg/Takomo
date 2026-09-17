# Testing and coverage

## When checks run

`pr.yml` runs Rust formatting, clippy and unit tests; frontend types, lint, native
Vitest affected tests, build and size (Rust consumes the generated assets); shell lint, skill-copy parity, OpenAPI lint and MCP types.
Documentation-only PRs skip this workflow. There is no push or merge trigger.

`ci.yml` runs full verification at **02:23 UTC daily** and on `workflow_dispatch`.
Scheduled runs use the default branch; manual runs use the selected ref. It retains
release tests, Docker smoke tests, generated asset checks/size, spec validation and dependency
checks, and adds coverage artifacts. The existing hosted/stdio MCP integration job remains. Failures appear as
failed Actions runs with ordinary GitHub notifications; configure your watching/email
preferences accordingly. Artifacts are retained for 14 days. No new external coverage
service, coverage threshold, or branch protection is imposed.

## Focused tests

Frontend, from `web/`:

```sh
npm run test:changed -- origin/main
npm run test:changed                  # uncommitted changes
npm test -- src/lib/documents.test.ts # explicit test file
```

Vitest follows static imports. Package/lock/config/setup/global-style changes invalidate
all tests. PR checkout includes history and passes the PR base SHA. Vitest natively exits successfully for an empty
changed selection: a backend-only PR can affect no frontend tests. The PR log explicitly
says that this provides no test evidence. Always inspect the selected/executed count. Run new/untracked tests explicitly. Runtime relationships, server behavior and
browser layout are outside this selector's guarantee. Use full tests when unsure.

Backend: Cargo has native target/name filters, but no built-in affected-test selector.
We deliberately do not maintain a guessed file-to-test map:

```sh
cargo test --lib schedule
cargo test --test api fence
cargo test --test oauth
cargo test --release                 # full release suite
```

Inspect the number executed. A mistyped filter can succeed with zero tests. Rust PR
unit tests are a smoke check, not a substitute for relevant integration tests locally.
Compilation caching saves build work; it does not mean unchanged tests are skipped.

## Measured layers

| Layer | Backend | Frontend |
|---|---|---|
| Unit | Library and binary unit-test targets | Tests under `src/lib/` |
| Integration | All top-level `tests/*.rs`, real HTTP/SQLite/WebSocket behavior | Component, hook and page tests in jsdom, often with mocks |
| E2E | Stdio MCP client → disposable running server: functional results only | Real-browser coverage unmeasured |
| Combined | Unit + integration | All Vitest tests |

These are operational groupings, not claims that every test in a folder is purely
unit or integration. Layer coverage overlaps; **do not add percentages**. Each frontend
report includes all production TS/TSX files, including untouched files. Build frontend assets before running Rust coverage in a fresh checkout (`npm run build`
in `web/`). Backend reports
exclude test harnesses using cargo-llvm-cov defaults. Rust line/region/function coverage
works on stable; branch coverage is not reported because it requires nightly features.
Frontend V8 reports lines, statements, functions and branches. Neither measures CSS layout.
E2E coverage is **unmeasured**, not zero and not equivalent to HTTP integration coverage.

```sh
# Install once (CI pins the tool too):
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
bash scripts/coverage-backend.sh
# JSON + HTML: ${CARGO_TARGET_DIR:-target}/coverage/{unit,integration,combined}*

cd web
npm ci
npm run coverage:unit
npm run coverage:integration
npm run coverage
# JSON summary, LCOV and HTML: coverage/{unit,integration,combined}/
```

Coverage runs use debug instrumentation, separate from nightly release verification.
Use a private `CARGO_TARGET_DIR` when other sessions may build concurrently. The backend
script resets profiles per layer and runs the combined suite again to measure the union
without summing overlapping counters. Do not run multiple coverage commands concurrently
in the same target directory. All reports are generated artifacts, not committed assets.

## Stdio MCP E2E

Build the binary and run `npm ci && npm run build` in `clients/mcp/`, then:

```sh
python3 scripts/test-mcp-local.py target/release/takomo
```

This automated fixture creates a temporary database, project and token, starts a loopback
server, runs the existing harness with a timeout, then stops it and deletes its database.
It never uses the configured/shared Takomo store. Interactive inspection still uses Backlot.

Tool references: [Vitest changed selection](https://vitest.dev/config/changed),
[Vitest coverage](https://vitest.dev/guide/coverage),
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov).

## Initial baseline — 2026-09-16

Measured in an isolated worktree based on `7b38d03`, with only the verification
configuration changes. Other sessions' uncommitted product changes are excluded.

| Layer | Backend lines | Frontend lines | Frontend branches |
|---|---:|---:|---:|
| Unit | 21.48% (6,003 / 27,941) | 21.94% (1,286 / 5,859) | 20.97% |
| Integration | 81.74% (22,299 / 27,282) | 22.25% (1,304 / 5,859) | 18.89% |
| Combined | 82.84% (23,146 / 27,941) | 38.38% (2,249 / 5,859) | 36.04% |
| E2E | Unmeasured; stdio MCP functional harness passed | Unmeasured |

All 72 Rust unit tests and 406 integration tests passed. Frontend: 386 logic tests
and 165 component/hook/page tests passed (551 total across 58 files).
Rust denominators differ because test targets compile different `cfg(test)` code;
these are LLVM's instrumented-source totals, not an identical production-only line
inventory. Colocated Rust test code may contribute to the unit/combined totals.
Do not compare the layer percentages as disjoint shares or sum them. Frontend reports
use the same explicit source inventory in every layer.

This baseline is evidence of exercised code, not a quality threshold. The largest
remaining measurement gap is real-browser behavior; focus new tests on critical
uncovered behavior rather than increasing the percentage with shallow assertions.

Native frontend delta selection was verified with reversible probes: a `format.ts`
edit selected 4 files / 43 tests instead of all 58 files / 551 tests; a lockfile edit
selected the full suite, including when an explicit base ref was supplied. An empty
selection exited 0 as Vitest specifies. Trigger paths are resolved from the config
so worktrees inside hidden directories also invalidate correctly. Backend automatic
delta selection was deliberately not added; native Cargo filters remain available.
