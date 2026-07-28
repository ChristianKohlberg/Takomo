# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Takomo is a **self-hosted task store that AI agent fleets, orchestrators, and humans all talk to
over HTTP** — one authority for work instead of a todo list trapped in a single checkout. Single
Rust + axum binary over SQLite (WAL). There is no public instance; you run your own.

## Running the app: use backlot, don't hand-roll build/seed/serve

`backlot.yml` at the repo root brokers a warm, **seeded** instance. Use it instead of composing
`cargo build` + `takomo seed` + `takomo serve` + `token create` yourself — that reimplements
`backlot up` in four steps, pays a cold release build instead of binding a pooled env (the SQLite
store is a file-copy template, so a bind is milliseconds), and leaves a throwaway instance on an
ad-hoc port nothing else can find.

```sh
backlot status                     # existing envs — check before spinning anything up
BACKLOT_HOLDER_PID=$$ backlot up   # build, seed, serve, wait for /healthz, print URL + port
backlot token --role human         # bearer token for /board and /inbox
backlot ctx                        # URLs/ports as one blob
backlot run api                    # integration suite with a work-vs-env-vs-infra verdict
backlot release                    # return the env to the pool, warm
```

`BACKLOT_HOLDER_PID=$$` frees the lease when the shell exits. Roles map onto scopes via
`scripts/backlot-token.sh`: `agent` → `read,write`; `human` → `+human`; `admin` → `+admin`;
`expert` → `+expert:domain:billing,expert:domain:product` (the scope the seeded `approve`
question gates on — a plain `human` token is refused there by design). A session lease seeds the
`dev` preset: a `demo` project with ten tickets across every workflow state plus claims, a
dependency, an epic, and questions of all four kinds — which is what makes `/board` and `/inbox`
worth looking at. A `backlot run` lease defaults to `empty`.

## Build, test, lint

```sh
cargo build --release
cargo test --release                                     # unit + tests/api.rs + tests/mcp.rs
cargo test --release --test api <substring>               # ONE integration test
cargo test --release --test api <substring> -- --nocapture
cargo test --release --lib <substring>                    # ONE unit test
cargo clippy --all-targets -- -D warnings                 # CI denies warnings
cargo fmt                                                 # CI runs --check
shellcheck -x clients/cli/takomo clients/cli/install.sh scripts/backlot-token.sh .handrail/*.sh .handrail/adapters/*.sh
(cd clients/mcp && npm ci && npm run build)               # MCP typecheck
```

The weight is in `tests/` (`api.rs`, `mcp.rs`): `TestApp::spawn()` opens a temp SQLite DB, mints
four tokens (`admin`/`human`/`worker`/`worker2`), and serves on an ephemeral port, so tests drive
the real HTTP surface over `reqwest`. Only `src/workflow.rs` and `src/seed.rs` carry `#[cfg(test)]`
units — anything touching the HTTP surface belongs in `tests/`.

`.handrail/` gates surface project norms in-session via hooks in `.claude/settings.json`; they
guide, CI is the wall. Run `handrail run --changed` before wrapping up (`handrail list` for the
menu: `fmt`, `clippy`, `route-test-pairing`, `openapi-current`, `openapi-valid`). When no changed
file maps to a gate — a clean tree, or a docs-only edit — `--changed` reports "no gates selected"
and they stay `stale`. `stale` means "not evaluated against this tree", not "failing"; a `git pull`
invalidates every earlier verdict the same way. Name them explicitly to evaluate them:
`handrail run fmt clippy openapi-valid`. The two detectors then report `skipped` (exit 2) rather
than green, because they compare a changed HTTP surface against its companion and there is none.

**Checking a branch you have already committed:** the detectors diff the working tree, so on a
committed tree they see nothing and skip. Point them at the fork point instead —
`HR_BASE=origin/main handrail run route-test-pairing openapi-current` — and they compare
`merge-base(origin/main, HEAD)` against the working tree, i.e. everything the branch changed plus
anything still uncommitted, without picking up what landed on `main` underneath you. Read their
`skipped` output rather than assuming: `SKIP[not-in-scope]` means your change did not touch that
surface, `SKIP[no-changes-visible]` means the detector saw nothing at all and checked nothing —
that one is not a pass. A bogus `HR_BASE` exits 3 (red) rather than silently reporting "no changes".

### Verify a branch in a worktree, not in a dirty tree

This repo is worked on by several sessions at once, so the working tree often carries **someone
else's uncommitted changes**. When it does, a local `cargo test` says nothing about your branch: it
compiles their code together with yours. Two ways that has already drawn blood here:

- Staging a whole file with `git add <file>` swept up another change's tests. They passed locally
  because that change's `src/` was present uncommitted; CI, building the branch alone, failed them.
- `git apply -3` writes to the **index**, not just the worktree, so it silently stages the other
  change's hunks along with yours.

So before trusting a result, check out the exact commit somewhere clean:

```sh
git worktree add --detach /tmp/verify <sha>
cd /tmp/verify && cargo test --release && cargo clippy --all-targets -- -D warnings && cargo fmt --check
git worktree remove /tmp/verify
```

A worktree is also the way to merge or rebase at all when the tree is dirty: `git merge` refuses if
any file it must touch has local changes, and stashing someone else's work in order to proceed risks
losing it. Check `git worktree list` first — another session's live workspace shows up there, and its
branch must be left alone.

Related: `git rev-list main..<branch>` reports commits "not in main" even for a fully merged branch,
because the repo **squash-merges** PRs — the original commits never become ancestors of the squash
commit. Decide whether work has landed by comparing content (`git diff main <branch>`, or the trees),
never by ancestry.

## Architecture

**One binary, four surfaces** (`src/server.rs` assembles them):

| Surface | Notes |
|---|---|
| REST `/v1/*` | The contract. Hand-parsed from `serde_json::Value` so bad input gets teaching errors. |
| MCP `/mcp` | `src/mcp.rs` — rmcp streamable-HTTP **in-process**; tools call `Store` directly, no HTTP loopback, no duplicated logic. |
| `/board`, `/inbox` | Dependency-free SPAs `include_str!`'d from `src/board.html` / `src/inbox.html` (`src/api/mod.rs`). |
| CLI subcommands | `token`, `project`, `seed` in `src/main.rs` operate on the DB file directly — the server is not the root of trust, shell access is. |

**Layering is strict:** all SQL lives under `src/store/`; handlers in `src/api/` never touch the
database. The `Store` surface is kept connection-agnostic so Postgres could slot in behind it.

**Four independent auth paths, not one middleware with branches** — a token of one kind cannot
reach another's routes (`src/auth.rs` + the router in `src/server.rs`):

- `tk_` bearer → `auth_middleware` (the whole `/v1` API) or `mcp_auth_middleware` (`/mcp`) —
  same token lookup, scopes, per-project allowlist, and per-token sliding-window write budget;
  they differ only in where a write is classified. REST classifies by method in the middleware
  (GET/HEAD free). MCP cannot — every frame is `POST /mcp` — so the middleware only
  authenticates and `src/mcp.rs` debits at tool dispatch by name (`READ_TOOLS` free, `initialize`
  and `tools/list` free, unlisted name counts as a write).
- `tks_` share → `share_auth_middleware` → **only** `/v1/shares/self*`, read-only.
- `tka_` answer grant → `answer_auth_middleware` → **only** `/v1/answer/self`: read and answer
  exactly one question, then it's spent. This is what an outside expert gets.

**Concurrency is the load-bearing design.** `Store::with_tx` runs every mutation as one SQLite
`IMMEDIATE` transaction behind a process-wide `Mutex<Connection>`; that single-writer
serialization *is* the exactly-one-claimant guarantee for the ready queue. Layered on top:

- **Fencing** (`src/store/claims.rs`): a per-ticket monotonic `fence_seq` bumped on each new
  claim; a zombie worker writing with a stale fence gets a teaching 409 rather than winning.
- **Leases** expire; a background sweeper (`spawn_sweeper`) clears them and expired questions,
  emits events, and wakes long-pollers.
- **CAS + idempotency**: replacing a ticket `body` requires `If-Match: "<version>"` (from the
  ETag); `Idempotency-Key` on ticket create replays the original instead of duplicating.

**State changes only through transitions** (`src/store/transition.rs`) against the per-project
state machine (`src/workflow.rs`, format in `spec/workflow-format.md`). A transition's `requires`
entries are `claim`, `scope:<s>`, or `guard:<id>` — guards being `no_open_children`,
`no_open_blockers`, and `has_link:<key>` (e.g. `has_link:commit`, which turns "done" from a claim
into something a later reader can verify). Workflow structs use `deny_unknown_fields`: a typo
like `require:` must 422, never silently drop an approval gate.

**Ask-a-human** (`src/store/questions.rs`, the largest file) is how an agent hands a decision to
a person. A `blocking` question parks the ticket in a blocked state and releases the lease;
answering resumes it through the workflow's human-gated edge — but only once *every* open
blocking question on the ticket is answered (a barrier). An `advisory` question records a routed
decision and never touches ticket state.

**Event log + long polling:** `emit_event` writes inside the same transaction as its mutation, so
the log cannot drift from state. `AppState::notify` is woken after every commit and long-pollers
(ready/claim, `events?wait=`, SSE) re-check. Note `/board` and `/inbox` poll
`GET /v1/events?since=<cursor>` rather than using the SSE stream, because the browser
`EventSource` API cannot set an `Authorization` header.

## Conventions that bite

- Every new/changed HTTP route ships with an integration test **and** an `spec/openapi.yaml`
  update. The spec is the contract and it drifts silently.
- `spec/openapi.yaml` must stay a *valid* OpenAPI **3.1** document, not merely parseable YAML.
  Two traps invisible to a human reader: a comma inside an unquoted description in a flow mapping
  truncates the sentence and turns its tail into a junk key; `nullable: true` is 3.0 syntax that
  3.1 tooling ignores (write `type: [string, "null"]`).
- **Errors are part of the contract** (`src/error.rs`): a stable machine `code`, a `message`
  written for an LLM reader saying what is wrong *and* what to do, plus a `remedy` and, for
  transitions, `allowed_transitions` so the caller sees the legal moves without a second request.
  Never fail silently.
- Editing `src/board.html` / `src/inbox.html` only takes effect after a rebuild, since they are
  compiled into the binary. Confirm before concluding anything: `curl -s "$URL/board" | grep -c <id>`.
- Both SPAs render via DOM construction (never `innerHTML` on user data) and carry full DE/EN
  `STR` tables — keep the two locales in key parity when adding UI strings.
- The server refuses non-loopback binds unless `TAKOMO_ALLOW_PUBLIC_BIND=1`: it terminates plain
  HTTP and expects TLS in front.

Deeper docs: `docs/development.md` (dev loop), `spec/openapi.yaml`, `spec/workflow-format.md`,
`spec/auth.md`, `docs/ask-a-human.md`, `docs/promotions.md`, `docs/hosting.md`.
