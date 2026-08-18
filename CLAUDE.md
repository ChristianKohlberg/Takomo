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
shellcheck -x clients/cli/takomo clients/cli/install.sh scripts/*.sh
(cd clients/mcp && npm ci && npm run build)               # MCP typecheck
(cd web && npm ci && npm run check && npm test)           # the six surfaces
(cd web && npm run build)                                 # then cargo build — see below
```

**A page change needs TWO builds.** `npm run build` in `web/` regenerates
`web/dist/`; `cargo build --release` embeds it. Editing `web/src/` alone changes nothing the
server serves. In the dev loop use `npm run dev` instead — it proxies `/v1` at a `backlot up`
instance, so there is no Rust rebuild at all.

`web/` carries its own gates: `npm run check` (tsc), `npm run lint` (eslint, defect rules only),
`npm test` (vitest), and `npm run size` (a gzip budget on FIRST LOAD, plus one on the vendor chunk —
every later route costs nothing, because there is one bundle and a router). They replace `scripts/lint-spa.sh`, which existed only to
read the JavaScript inside the hand-written pages.

The weight is in `tests/` (`api.rs`, `mcp.rs`): `TestApp::spawn()` opens a temp SQLite DB, mints
four tokens (`admin`/`human`/`worker`/`worker2`), and serves on an ephemeral port, so tests drive
the real HTTP surface over `reqwest`. Only `src/workflow.rs` and `src/seed.rs` carry `#[cfg(test)]`
units — anything touching the HTTP surface belongs in `tests/`.

**There is no in-session gate runner. CI is the only wall**, so run the commands above yourself
before wrapping up — `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test --release`,
and the `web/` gates. Two conventions that used to have their own detectors are now yours to keep:
a new or changed HTTP route ships with an integration test, and with an `spec/openapi.yaml` update.
Nothing will remind you.

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

**One binary, five surface kinds** (`src/server.rs` assembles them):

| Surface | Notes |
|---|---|
| REST `/v1/*` | The contract. Hand-parsed from `serde_json::Value` so bad input gets teaching errors. |
| MCP `/mcp` | `src/mcp.rs` — rmcp streamable-HTTP **in-process**; tools call `Store` directly, no HTTP loopback, no duplicated logic. |
| OAuth `/oauth/*`, `/.well-known/oauth-*` | `src/api/oauth.rs` + `src/store/oauth.rs` — an OAuth 2.1 authorization server in front of `/mcp`, so **hosted** clients (claude.ai, ChatGPT, the Gemini app), which can only be handed a URL, can connect at all. Off unless `TAKOMO_PUBLIC_URL` is set to a usable issuer origin — that variable predates OAuth and has an older, tolerant reader (notification links), so an unusable value turns OAuth off on a startup line rather than stopping the server (`resolve_oauth` in `src/server.rs`). |
| `/board`, `/inbox`, `/initiatives`, `/schedules`, `/verification`, `/environments` | **ONE app built from `web/`** (React 19 + React Router + Tailwind + shadcn, TypeScript, vitest). Every route serves the same `index.html`; the router picks the surface from the path. The binary embeds the shell plus four fixed assets (`assets/{app,vendor,runtime}.js`, `assets/app.css`) by name — not a static-file handler, so nothing to traverse. `web/dist/` is **committed** so `cargo build --release` stays node-free on Render and in the Dockerfile. `/initiatives`, `/schedules`, `/verification` and `/environments` are the ones that WRITE. See `web/README.md`. |
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

**A user is a person, and still not a credential** (`src/store/users.rs`, `docs/users.md`). The
directory is global with per-project membership, and it authenticates nothing: **a user says who
work is waiting on, a scope says what a credential may do.** So the four paths above stay four.
The one deliberate exception is that a question's named `assignee` may answer an `approve` — which
makes `tokens.user` an *authorization* fact, admin-set at mint, and puts four guards around it.
Identity is carried on `AuthCtx.user` and passed explicitly as `Answerer`, **never as a scope
string**: scopes are free-form (`expert:<tag>` proves it), so a `user:usr-…` scope would be a
forgeable identity — `a_user_scope_string_cannot_forge_assignee_identity` in `tests/api.rs` is the
guard. Relaying an approval stays refused, an answer link for a person-gated approval is only
mintable by that person, and disabling them closes the route. `may_approve` in
`src/store/questions.rs` is the only place that decides, because it is asked from four.
A user handle is validated by the **tag** handle rule, so `person:<handle>` stays a legal reference
to the same person and the convention that predates the directory converges on it — and the
registry reads join the two (`TAG_COLS_JOINED` in `src/store/tags.rs`), so a person tag carries the
directory's name rather than the stub label lazy-creation wrote. Resolved, never copied: a rename
in the directory is right everywhere at once. `takomo person` is retired for the same reason —
two people-shaped commands where one did not add a person is a trap, so it is `takomo user` for the
directory and `takomo tag` for the reference.

**Concurrency is the load-bearing design.** `Store::with_tx` runs every mutation as one SQLite
`IMMEDIATE` transaction behind a process-wide `Mutex<Connection>`; that single-writer
serialization *is* the exactly-one-claimant guarantee for the ready queue. Layered on top:

- **Fencing** (`src/store/claims.rs`): a per-ticket monotonic `fence_seq` bumped on each new
  claim; a zombie worker writing with a stale fence gets a teaching 409 rather than winning.
- **Leases** expire; a background sweeper (`spawn_sweeper`) clears them and expired questions,
  emits events, and wakes long-pollers.
- **CAS + idempotency**: replacing a ticket `body` requires `If-Match: "<version>"` (from the
  ETag); `Idempotency-Key` on ticket create replays the original instead of duplicating.

**A project can be archived, which is a gate rather than a state.** `archived_at`
on `projects` freezes every write beneath it: `ensure_project_writable`
(`src/store/helpers.rs`) is called at the top of every project-scoped mutation and
returns a teaching 409 `project.archived`. It sits in the **store**, not the
middleware, because the middleware cannot know which project a request touches
(`POST /v1/tickets` carries it in the body, `/mcp` is one POST for every tool) —
which is also why REST, MCP and the CLI all inherit it for free. Reads are
untouched; the ready query and the two sweepers (schedules, question timeouts)
filter archived projects out instead of erroring on them. Reversible by design:
`POST /v1/projects/{p}/unarchive` restores the project because archiving changed
nothing about it. If you add a mutating store call, add the guard —
`project_archive_refuses_every_write_and_allows_every_read` in `tests/api.rs` is
what notices when you don't.

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

**Mindmaps** (`src/store/mindmaps.rs`) are what comes *before* an initiative: a tree grown at
conversation speed, six words a node, whose branches graduate into epics and initiatives. Two rules
shape it and both are load-bearing. **Deleting one is ordinary** — an initiative is nurtured, a
mindmap is scratch, and that is what makes it safe to start one early. **A node caps at 280 chars**,
which is the method rather than a limitation: past that it wants to be an initiative, and the refusal
says so. Promotion never moves a node — it keeps a `promoted_kind`/`promoted_id` link, so a map that
produced work becomes a picture of that work. The whole map comes back in one read because a canvas
cannot draw half a tree, which is affordable precisely because of the 500-node cap; `POST .../nodes`
takes a **batch** because that is what an agent adding a branch sends. See `docs/mindmaps.md`.

**Initiatives** (`src/store/initiatives.rs`) are the one thing here that is *not* work: an idea
being nurtured — a product direction, the residue of a good conversation — fed by appending
entries over time, each recording where it came from. No workflow, no claim, no lease, no ready
queue; `status` is a label, not a state machine. Written over MCP (`takomo_initiative_*`) because
an agent in a conversation is what produces one, with POST/PATCH added for `/initiatives`, the one
SPA that writes (a browser cannot call an MCP tool). `takomo initiative new|append|ls|show|set` is a
third caller of those routes, minus pane writing — prose through shell flags is what the pane editor
exists to avoid, so the CLI carries what a shell is better at instead (`--text-file`, `--attach`).
Entries stay append-only on every surface.
Entries are the only place in the store that holds binary blobs, which is why they are the only
thing with byte caps — an unbounded upload would hold the write mutex every claim waits on.

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
- **A list route is bounded, and says what it left out.** The reader is usually an agent, and a
  full page it cannot distinguish from a complete one is how it comes to treat a fraction of the
  work as all of it. So a list returns an envelope, never a bare array — `items` plus `total`
  (counted with the *same* predicate that selected the page) plus the `limit` applied, and a prose
  `note` when the two differ. `api::paged` builds it. Which continuation to offer depends on
  whether the order is stable: `cases` are keyed and get an `offset`, `tickets` get a rowid
  `cursor`, and the **ready queue gets neither** — other workers claim from it as you read, so a
  positional page 2 would promise a sequence that does not exist. Raise `limit` there instead.
  A walk that cannot be paged at all (`dep_graph`) carries `truncated` and stops at a ceiling.
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
- **A page change needs TWO builds:** `npm run build` in `web/`, then `cargo build --release`. The
  Rust build embeds `web/dist/`, so editing `web/src/` alone changes nothing the server serves.
- **The asset names are load-bearing.** Rust `include_str!`s `assets/app.js`, `assets/vendor.js`,
  `assets/runtime.js` and `assets/app.css` by name, so content hashing is off and a fifth chunk would
  404 at runtime. `web/vite.config.ts` fails the build if the output is not exactly that set — it
  caught the bundler's own runtime chunk the first time it ran. Cache correctness comes from an ETag,
  not the filename.
  In the dev loop use `npm run dev` instead — it proxies `/v1` to a `backlot up` instance, so there
  is no Rust rebuild at all.
- **No page renders user text through `innerHTML`.** `dangerouslySetInnerHTML` and `innerHTML =`
  are eslint ERRORS in `web/`; agent- and human-written text goes through `web/src/lib/markdown.ts`,
  which builds DOM nodes and allows only http(s)/mailto links. Locale parity is a compile error
  (`defineStrings` makes EN the reference shape), not a test that has to remember each page —
  `scripts/lint-spa.sh` and `spa_string_tables_agree_on_every_key` are both gone, along with the
  hand-enumeration hazard that made them necessary.
- Code more than one page needs goes in `web/src/components/` or `web/src/lib/` and is imported —
  the marker-substitution trick the hand-written pages needed is gone, because a bundler does this
  properly. Two things that were per-page out of necessity stay per-page by decision: the `STR`
  tables (takomo-2hk4 — four keys collide across pages with *different* values), and nothing else.
  The typeahead, which the old pages could not share, is now ONE `Typeahead` with five mounts.
- **One breakpoint, `md`, meaning "phone or not".** Four eslint rules in `web/eslint.config.js` reject a
  desktop-shaped value with no mobile fallback (`w-72`, `grid-cols-[180px_320px_1fr]`, `h-screen`), because
  every one of those shipped and broke a phone while looking correct in source. `max-w-*` is exempt — a cap
  cannot overflow. jsdom has no layout engine, so nothing in vitest can catch this class; the contract is in
  `web/README.md`.
- **A component in `web/src/components/` is invisible to the design system unless it is exported
  from `web/src/components/index.ts`.** That barrel is the contract `.design-sync/` converts; the
  page still works without it, so nothing fails — the component just silently never appears.
- The server refuses non-loopback binds unless `TAKOMO_ALLOW_PUBLIC_BIND=1`: it terminates plain
  HTTP and expects TLS in front.

**Checklist** (`src/store/checklist.rs`, `src/api/checklist.rs`) is how a "done" claim becomes a
*verified* one: releases, checks, the cases generated beneath them, and the verdicts recorded
against those cases. The rule that shapes every part of it is **Takomo stores, the agent
computes** — nothing server-side generates a combinatorial model, validates one, or judges
whether a coverage claim is true. A check is ONE action with ONE entry precondition at ONE layer
(a rule enforced only in a frontend passes at the API layer, so those verdicts are not
interchangeable), and coverage is of the *declared* surface: hand-written globs, known to rot,
with orphan detection so the rot stays visible. See `docs/checklist.md`.

**A check used to be called a lane, and `lane` still means something else.** On the roadmap and
in `/initiatives` a lane is the *initiative* a feature is worked in — it spans versions and never
closes. One product cannot carry two lanes, so the verification one is a **check**: tables
`checks` / `check_globs`, `cases.check_id` (the column is `check_id` because `CHECK` is a SQL
keyword), routes under `/v1/checks`, and MCP `takomo_check*`. Existing rows keep their `lane-…`
ids — an id is opaque, and rewriting primary keys is the one part of a rename that can lose data.
`rename_lanes_to_checks` in `src/store/mod.rs` migrates an older database, and runs **before** the
schema batch: `CREATE TABLE IF NOT EXISTS checks` would otherwise create an empty table beside the
populated one.

**A check may declare which environments it must pass in**, and then each of its cases is tracked
per `(case, environment)` — so "verified on staging, never run on production" is expressible and the
case's own state is the WORST of its environments. Declaring none is a legitimate steady state, not
a gap: that check keeps the original environment-agnostic reading, stored in the verdict columns on
`cases` rather than in `case_environments`. The two are mutually exclusive by construction, which is
what stops them disagreeing. An omitted environment resolves when the check declares exactly one and
is refused with `conflict.environment_ambiguous` when it declares more — filing a staging run as
production is worse than no record.

**Environments** (`src/store/environments.rs`) are where a check can be run: a base URL, prose for
bringing the thing up and giving it back, what data is in it, and whether writing to it is safe.
It exists because a verdict with no environment behind it is a claim nobody can reproduce, and all
of that used to travel out of band. Takomo runs none of it — `bring_up`/`teardown` are prose handed
to whoever needs them next, `writable` is advisory, and `credentials_hint` is a POINTER to where a
credential lives and **never** a credential, because every `read` token can see it. A slug is
immutable (checks and tool calls address environments by it) and archiving is reversible. Writes
take `write`, not `human`: an agent registering the ephemeral instance it just leased is the caller
this serves. See `docs/environments.md`.

Deeper docs: `docs/development.md` (dev loop), `spec/openapi.yaml`, `spec/workflow-format.md`,
`spec/auth.md`, `docs/ask-a-human.md`, `docs/users.md`, `docs/checklist.md`,
`docs/environments.md`, `docs/epic-claims.md`
(claiming an epic reserves its subtree; no-TTL claims judged by movement),
`docs/initiatives.md`, `docs/mindmaps.md`, `docs/promotions.md`,
`docs/hosting.md`, `docs/hosted-mcp-clients.md` (wiring claude.ai / ChatGPT / Gemini).
