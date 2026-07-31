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
backlot up --ttl 900               # …but from an agent, use this instead — see below
backlot token --role human         # bearer token for /board and /inbox  (prints JSON)
backlot ctx                        # URLs/ports as one blob
backlot run api                    # integration suite with a work-vs-env-vs-infra verdict
backlot release                    # return the env to the pool, warm
```

**`BACKLOT_HOLDER_PID=$$` only works from an interactive shell.** It frees the lease when *that*
shell exits, which is right for a human at a terminal and useless for an agent: an agent harness
runs each command in a fresh shell, so the holder is dead the instant `backlot up` returns and the
lease is released before the next command. Agents should use `backlot up --ttl <secs>` and call
`backlot release` explicitly. `backlot token` prints JSON (`{"token":…,"role":…}`), not a bare
string — parse it.

Roles map onto scopes via
`scripts/backlot-token.sh`: `agent` → `read,write`; `human` → `+human`; `admin` → `+admin`;
`expert` → `+expert:domain:billing,expert:domain:product` (the scope the seeded `approve`
question gates on — a plain `human` token is refused there by design). A session lease seeds the
`dev` preset: a `demo` project with ten tickets across every workflow state plus claims, a
dependency, an epic, and questions of all four kinds — which is what makes `/board` and `/inbox`
worth looking at. A `backlot run` lease defaults to `empty`.

If a session lease comes up with **no `demo` project**, the preset is not broken — check the lease
first. A lease whose holder process is gone (`backlot status` → `holderAlive: false`, which is what
`BACKLOT_HOLDER_PID=$$` produces from an agent) is reclaimable while you are still using it, and the
next bind can take the env out from under you; a `backlot run` bind then seeds it `empty`. Hold the
lease with `--ttl` and the store stays yours.

## Build, test, lint

```sh
cargo build --release
cargo test --release                                     # unit + tests/api.rs + tests/mcp.rs
cargo test --release --test api <substring>               # ONE integration test
cargo test --release --test api <substring> -- --nocapture
cargo test --release --lib <substring>                    # ONE unit test
cargo clippy --all-targets -- -D warnings                 # CI denies warnings
cargo fmt                                                 # CI runs --check
shellcheck -x clients/cli/takomo clients/cli/install.sh scripts/*.sh .handrail/*.sh .handrail/adapters/*.sh
./scripts/lint-spa.sh                                     # eslint over the SPAs' inline <script>
(cd clients/mcp && npm ci && npm run build)               # MCP typecheck
```

`scripts/lint-spa.sh` is the only thing in the repo that reads the ~5500 lines of JavaScript inside
`src/board.html` and `src/inbox.html`. It extracts the single inline `<script>` from each, splices
`src/spa-common.js` in at that page's `// <<SPA_COMMON>>` marker exactly as the server does, and runs
a pinned eslint over the assembly with a small, defect-only ruleset (`scripts/spa-eslint.config.mjs`) — duplicate
keys, undefined names, dead bindings, parse errors; no style rules, so a red is always real.
Findings are reported at the HTML file's own path and line. Nothing is added to the pages
themselves: the SPAs stay dependency-free in the sense that matters, which is what the browser
downloads. Offline the script exits 2 (skipped, not failed) because it cannot fetch eslint; CI has
the network and is the wall.

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
# Private target dir, deliberately — a shared one makes this result untrustworthy
# whenever another session is building. See "Share one target/" below.
cd /tmp/verify && export CARGO_TARGET_DIR=/tmp/verify-target
cargo test --release && cargo clippy --all-targets -- -D warnings && cargo fmt --check
git worktree remove /tmp/verify && rm -rf /tmp/verify-target
```

A worktree is also the way to merge or rebase at all when the tree is dirty: `git merge` refuses if
any file it must touch has local changes, and stashing someone else's work in order to proceed risks
losing it. Check `git worktree list` first — another session's live workspace shows up there, and its
branch must be left alone.

Related: `git rev-list main..<branch>` reports commits "not in main" even for a fully merged branch,
because the repo **squash-merges** PRs — the original commits never become ancestors of the squash
commit. Decide whether work has landed by comparing content (`git diff main <branch>`, or the trees),
never by ancestry.

### Share one `target/` across worktrees

A worktree gets its own empty `target/`, so a fresh one recompiles all 183 crates from scratch.
Point every checkout at one directory instead:

```sh
export CARGO_TARGET_DIR="$HOME/.cache/takomo-target"   # any path outside the worktrees
```

Measured on this repo (`cargo build --release`, two worktrees off the same commit), so you can judge
whether it is worth it rather than taking the claim on faith:

| | wall | crates compiled |
|---|---|---|
| fresh worktree, own `target/` (the default) | 168 s | 183 |
| fresh worktree, shared dir, source differs | 103 s | **1** |
| fresh worktree, shared dir, identical content | 1 s | 0 |
| rebuilding the *other* worktree afterwards | 0 s | 0 |
| two worktrees building at once, shared dir | 95 s for both | — |

So it removes 182 of 183 crate compiles and about **40%** of the wall clock — worthwhile, but not
transformative: what remains is `takomo` itself plus the LTO link, roughly 100 s, and that is paid on
every build no matter what. The larger saving is disk, at ~1.2 GB of `target/` per worktree.

Two worries that turn out not to apply. Worktrees do **not** evict each other's artifacts — cargo
keys them on a metadata hash that includes the workspace, so both sets coexist and rebuilding the
first worktree after the second stays at 0 s. And concurrent builds do **not** meaningfully
serialize: two builds of ~103 s each finished together in 95 s, the only contention being a brief
`Blocking waiting for file lock on package cache`.

**But the numbers above are all `cargo build`, and `cargo test` does not inherit them.** The
integration-test binaries are *not* keyed on the workspace the way the lib units are — two worktrees
off different commits, with different sources, produce the **same** path:

```sh
cd /tmp/wt-a && cargo test --release --test api --no-run   # deps/api-9832f7f87114d361
cd /tmp/wt-b && cargo test --release --test api --no-run   # deps/api-9832f7f87114d361  ← same file
```

Run one at a time and this is harmless: cargo notices the source changed and rebuilds. Run them **at
the same time** and whichever finishes last owns the file, so the other session executes a binary
built from *someone else's* `tests/`. It does not error — it reports a plausible-looking result. What
that looks like in practice: an integration-test count that changes between runs with no edit
(111/112/115), or a failure naming a test that does not exist on your branch.

So `CARGO_TARGET_DIR` is for **build** throughput. Any `cargo test` result you intend to trust —
especially the clean-worktree verification above — must not share a target dir with a concurrently
building session. Give that one run a private dir:

```sh
cd /tmp/verify && CARGO_TARGET_DIR=/tmp/verify-target cargo test --release
```

It costs a full cold build, which is the price of an answer that is about your branch. CI is the
other way to settle it: it builds the branch alone in a clean environment, so a green CI run is
trustworthy where a shared-dir local run is not.

Do not commit this as `.cargo/config.toml` — the path is machine-specific and would break every
other checkout. `RUSTC_WRAPPER=sccache` composes with it, but it caches codegen rather than the LTO
link, which is exactly the part that dominates here; measure before adopting it.

## Architecture

**One binary, five surfaces** (`src/server.rs` assembles them):

| Surface | Notes |
|---|---|
| REST `/v1/*` | The contract. Hand-parsed from `serde_json::Value` so bad input gets teaching errors. |
| MCP `/mcp` | `src/mcp.rs` — rmcp streamable-HTTP **in-process**; tools call `Store` directly, no HTTP loopback, no duplicated logic. |
| OAuth `/oauth/*`, `/.well-known/oauth-*` | `src/api/oauth.rs` + `src/store/oauth.rs` — an OAuth 2.1 authorization server in front of `/mcp`, so **hosted** clients (claude.ai, ChatGPT, the Gemini app), which can only be handed a URL, can connect at all. Off unless `TAKOMO_PUBLIC_URL` is set to a usable issuer origin — that variable predates OAuth and has an older, tolerant reader (notification links), so an unusable value turns OAuth off on a startup line rather than stopping the server (`resolve_oauth` in `src/server.rs`). |
| `/board`, `/inbox` | Dependency-free SPAs `include_str!`'d from `src/board.html` / `src/inbox.html` (`src/api/mod.rs`). `src/spa-common.js` — the shared markdown renderer — is inlined into both at the `// <<SPA_COMMON>>` marker, so each page stays ONE self-contained document: no second request, no new route. |
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

**OAuth adds a fifth *route group*, deliberately not a fifth credential type.** `/oauth/*` and the
two `.well-known` documents sit OUTSIDE every middleware — they are what a client reads *in order to*
obtain a credential, so requiring one makes the flow unstartable (before they existed these paths
fell through to the `/v1` middleware and answered 401, which is exactly the dead end a hosted client
cannot get past). What the token endpoint issues is an ordinary `tk_` row with an expiry, derived from
the token a human pasted into the consent screen: same actor, a **subset** of its scopes, its project
allowlist, its write budget. So `src/auth.rs` needs no new branch, and revocation, listing and rate
limiting all work by the machinery that already existed. Two rules worth knowing before touching it:
`admin` is never grantable through consent (`GRANTABLE_SCOPES`), and the `client_id`/`redirect_uri`
checks come first so an error is never redirected to an unvalidated URI. `spec/auth.md` has the
design, `docs/hosted-mcp-clients.md` the per-product wiring.

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

- **There is no changelog. Do not add one.** The release notes are the squashed commit subjects and
  the tickets they name; `git log --oneline` and `takomo show <id>` are the record. A `CHANGELOG.md`
  was removed deliberately: every change appended to the top of the same `[Unreleased]` section, so
  it conflicted on essentially every rebase while duplicating what the commit already said. If you
  want a release note, write a good commit subject.
- Every new/changed HTTP route ships with an integration test **and** an `spec/openapi.yaml`
  update. The spec is the contract and it drifts silently.
- `spec/openapi.yaml` must stay a *valid* OpenAPI **3.1** document, not merely parseable YAML.
  Two traps invisible to a human reader: a comma inside an unquoted description in a flow mapping
  truncates the sentence and turns its tail into a junk key; `nullable: true` is 3.0 syntax that
  3.1 tooling ignores (write `type: [string, "null"]`).
- **Errors are part of the contract** (`src/error.rs`): a stable machine `code`, a `message`
  written for an LLM reader saying what is wrong *and* what to do, plus a `remedy` and, for
  transitions, `allowed_transitions` so the caller sees the legal moves without a second request.
  Never fail silently. **The one exception is `/oauth/*`**, which emits RFC 6749's
  `{error, error_description}` because an OAuth client parses `error` and would never see a `code` —
  the vocabulary lives under `x-oauth-errors` in `spec/openapi.yaml`, kept separate from
  `x-error-codes` so the two namespaces are not mistaken for one. Registration is likewise the one
  mutating handler that does **not** `reject_unknown`: RFC 7591 requires unrecognized client metadata
  to be ignored, and refusing it would refuse every real client.
- Editing `src/board.html` / `src/inbox.html` only takes effect after a rebuild, since they are
  compiled into the binary. Confirm before concluding anything: `curl -s "$URL/board" | grep -c <id>`.
- Both SPAs render via DOM construction (never `innerHTML` on user data) and carry full DE/EN
  `STR` tables — keep the two locales in key parity when adding UI strings.
- Code both SPAs need goes in `src/spa-common.js`, not copy-pasted. It is inlined at the
  `// <<SPA_COMMON>>` marker, which is the **last statement inside each page's IIFE** — appending
  there keeps every page line number as it is in its own file, and putting it outside the IIFE would
  take `el()` out of scope and break the renderer at runtime. The module may depend on `el()` and
  nothing else: no `state`, no `L()`/`t()`, no `api()`. The `STR` tables stay per-page (takomo-2hk4:
  four keys collide across the two files with *different* values), and so does the typeahead, which
  genuinely differs between them.
- The server refuses non-loopback binds unless `TAKOMO_ALLOW_PUBLIC_BIND=1`: it terminates plain
  HTTP and expects TLS in front.

**Checklist** (`src/store/checklist.rs`, `src/api/checklist.rs`) is how a "done" claim becomes a
*verified* one: releases, lanes, the cases generated beneath them, and the verdicts recorded
against those cases. The rule that shapes every part of it is **Takomo stores, the agent
computes** — nothing server-side generates a combinatorial model, validates one, or judges
whether a coverage claim is true. A lane is ONE action with ONE entry precondition at ONE layer
(a rule enforced only in a frontend passes at the API layer, so those verdicts are not
interchangeable), and coverage is of the *declared* surface: hand-written globs, known to rot,
with orphan detection so the rot stays visible. See `docs/checklist.md`.

Deeper docs: `docs/development.md` (dev loop), `spec/openapi.yaml`, `spec/workflow-format.md`,
`spec/auth.md`, `docs/ask-a-human.md`, `docs/checklist.md`, `docs/promotions.md`,
`docs/hosting.md`, `docs/hosted-mcp-clients.md` (wiring claude.ai / ChatGPT / Gemini).
