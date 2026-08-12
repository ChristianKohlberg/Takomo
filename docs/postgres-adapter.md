# A Postgres adapter behind `Store`

Status: **spike**, branch `spike/postgres-adapter`, not for merge.

`src/store/mod.rs` has claimed since v0 that the surface is "kept narrow and
connection-agnostic so a Postgres implementation could be added behind the same
methods later". This document is the audit of how true that turned out to be,
written before any code, so the cost is visible up front.

## The good news: the layering claim holds

`rusqlite` appears outside `src/store/` in exactly one file — `src/error.rs`,
for the error conversion. No handler in `src/api/`, no MCP tool in `src/mcp.rs`,
and no CLI subcommand touches a `Connection`. The public `Store` methods really
are the boundary.

## The bad news: the boundary is public, the internals are not

Inside `src/store/` the coupling is total, and it is not a matter of swapping a
driver:

| Coupling | Count | Notes |
|---|---:|---|
| `with_tx` call sites | 69 | each closure receives `&rusqlite::Transaction` |
| module fns taking `&Transaction` / `&Connection` | 48 | rusqlite types in private signatures |
| `?N` positional placeholders | 714 | Postgres wants `$N` |

So "add an adapter" means introducing a trait at the `with_tx` / `with_conn`
seam and rewriting every private signature behind it. There is no smaller
version of this that is still honest.

## Portability hazards, worst first

### 1. `rowid` — 29 uses, no Postgres equivalent

SQLite's implicit `rowid` is used here as a **monotonic insertion counter**, not
as an identifier. Two distinct jobs depend on it:

- **Insertion-order tiebreaking.** `ORDER BY t.created_at ASC, t.rowid ASC` in
  the ready queue (`claims.rs:192`), the roadmap, ticket lists, and the
  checklist's verdict history. `created_at` is a second-resolution integer, so
  without the tiebreaker, rows created in the same second come back in an
  arbitrary order.
- **Keyset pagination.** `AND t.rowid < ?` plus `ORDER BY t.rowid DESC LIMIT ?`
  in `tickets.rs:658-752`, and `MAX(rowid)` to pick the newest promotion per
  ticket (`tickets.rs:1248-1255`).

This is a known-sharp area: commit `1c58e1c` ("order verdict history by
insertion, not by a random id") was a real bug of exactly this kind. A port must
add an explicit `BIGSERIAL`/`GENERATED ALWAYS AS IDENTITY` sequence column and
migrate every one of these sites onto it. Getting it wrong reintroduces
`1c58e1c` in 29 places, and the failure is silent — rows in a plausible but
wrong order.

### 2. `BEGIN IMMEDIATE` — 6 uses, and it is the concurrency design

`Store::with_tx` runs every mutation as one SQLite `IMMEDIATE` transaction
behind a process-wide `Mutex<Connection>`, and the module doc-comment is explicit
that *that serialization is the exactly-one-claimant guarantee for the ready
queue*: "no call site in this codebase handles `SQLITE_BUSY` on a write, because
with one writer it cannot happen."

Postgres has no `IMMEDIATE`. It is MVCC — concurrent writers are the normal case
and serialization failures are returned at commit, not prevented. Every claim
path, the fencing in `claims.rs`, and the sweeper would need their concurrency
argument re-made from scratch rather than inherited. **This is the actual work of
the port.** See "The open decision" below.

### 3. `json_each` — 16 uses

Label, tag and expertise filters all use
`EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = ?)`.
Postgres equivalent is `jsonb_array_elements_text` or the `?`/`@>` containment
operators, which also want a `jsonb` column type and a GIN index rather than the
`TEXT`-holding-JSON these columns are today.

### 4. `GLOB` — 7 uses

Checklist coverage globs (`checklist.rs`). Postgres has no `GLOB`; it needs
`LIKE`, `SIMILAR TO`, or a regex translation — none of which share GLOB's exact
semantics. Given the house rule that *Takomo stores, the agent computes*, doing
the match in Rust rather than in SQL is arguably the more consistent fix, and
would remove the divergence entirely.

### 5. Schema-level

`AUTOINCREMENT`, `WITHOUT ROWID` (×3), `INTEGER PRIMARY KEY`,
`last_insert_rowid()`, `INSERT OR IGNORE` (×6), and 5 `PRAGMA` statements. All
have Postgres analogues; all need writing. The schema currently lives as Rust
string constants in `src/store/mod.rs` rather than as `.sql` files, so a second
backend also forces a decision about where schema lives and whether a real
migration tool arrives with it.

### 6. Sync vs async — the one that reaches outside `src/store/`

`rusqlite` is synchronous. The mainstream Postgres crates (`sqlx`,
`tokio-postgres`) are async. Adopting an async driver turns every `Store` method
into an `async fn`, and that propagates through `src/api/`, `src/mcp.rs`, and the
CLI — i.e. it breaks the very boundary that makes this port look cheap.

The `postgres` crate (the sync wrapper over `tokio-postgres`) keeps every
signature shape intact and confines the change to `src/store/`. For a spike whose
goal is to learn whether the seam holds, that is almost certainly the right
trade, even if a production port later wants async.

## The open decision

Two coherent targets, and they are not the same project:

**A. Postgres as SQLite-shaped storage.** Keep the process-wide writer mutex,
keep one writer, translate SQL. Preserves every concurrency property by
construction — the exactly-one-claimant guarantee, fencing, the no-`BUSY`
invariant — and the tests should pass unchanged. But it throws away the main
reason to want Postgres: a process-wide mutex does not serialize across
processes, so this still cannot run two app instances against one database.

**B. Postgres-native.** Replace the mutex with `SELECT ... FOR UPDATE SKIP
LOCKED` on the ready queue — the canonical Postgres job-queue pattern — and let
Postgres arbitrate. This is what buys multi-instance deploys and managed
failover. It also means the exactly-one-claimant property must be **re-proven**
rather than inherited, under real concurrency, with the fencing sequence and the
lease sweeper both in scope.

B is the version worth building. A is the version that can be finished quickly
and is a useful stepping stone, since the SQL translation is common to both.

**Decided, then re-decided.** The first answer was B with `sqlx`, on the
assumption that multi-instance was the goal. It is not: the target deployment is
**one takomo process against one Postgres**. That withdraws the question both
earlier answers were answering, so:

| | first answer | after "one instance is fine" |
|---|---|---|
| concurrency | B — drop the mutex, `SKIP LOCKED` | **A — keep the writer mutex** |
| driver | `sqlx`, async | **`postgres`, sync** |
| shape | adapter behind a trait | **one-way port, no trait** |

Why each flipped:

- **Mutex retained.** With one process it still provides exactly-one-claimant by
  construction. Nothing to re-prove, and `tests/api.rs` / `tests/mcp.rs` — which
  drive the real HTTP surface and know nothing about the backend — validate the
  port *unchanged*. That is the single most valuable property available here.
- **Sync driver.** Async was forced by Postgres-native concurrency. Without it,
  the sync `postgres` crate keeps every `Store` signature intact and confines
  the change to `src/store/`, which removes the 69-`with_tx`-async-closure
  problem entirely (a lifetime-generic closure returning a future borrowing the
  transaction — awkward Rust, hit on line one of the conversion).
- **No trait.** The brief is Postgres *instead of* SQLite, not alongside it. Two
  backends would mean every future query written twice and every concurrency
  invariant holding under two models, forever. A one-way port needs no
  abstraction at all.

The `SKIP LOCKED` result below is not wasted: it is the standing proof that IF a
second instance is ever wanted, the claim path already works. It just is not
needed today.

### What retaining the mutex costs, measured

Under SQLite the mutex is nearly free — a write is a local page-cache write. Over
a socket it is held across a full BEGIN/UPDATE/COMMIT round trip. Measured
against Postgres 16 in Docker on localhost (`spikes/pg-claim`, `bench`):

| | 300 txns | per txn | throughput |
|---|---:|---:|---:|
| serial (mutex retained) | 431 ms | 1.44 ms | **~696 writes/s** |
| pooled (mutex dropped) | 65 ms | 0.22 ms | ~4629 writes/s |

6.6× — and that is the *best* case. The serial arm is latency-bound, so it
degrades linearly with round-trip time: on a managed Postgres at ~2 ms RTT it
lands nearer 150 writes/s while the pooled arm barely moves. Acceptable because
the workload is nowhere near it (100 agents writing every 10 s is 10 writes/s),
and because dropping the mutex later is a contained change against a store that
is already on Postgres. Worth re-measuring against the real database before
production, not assuming.

The existing structure maps over almost literally: today's `Mutex<Connection>`
writer plus `READ_CONNECTIONS = 4` read companions becomes one writer connection
plus a 4-connection read pool.

## The claim guarantee, re-proven (`spikes/pg-claim/`)

Two arms against a real Postgres 16, 60 claimable tickets, 16 concurrent
workers, no writer mutex, a 2 ms window opened between the SELECT and the UPDATE
so the race is observable rather than left to the scheduler:

| | claims | claimed twice+ | `fence_seq <> 1` | never claimed | verdict |
|---|---:|---:|---:|---:|---|
| **NAIVE** — SQL ported literally, mutex deleted | 477 | 60 | 60 | 0 | **VIOLATED** |
| **LOCKED** — `FOR UPDATE OF t SKIP LOCKED` | 60 | 0 | 0 | 0 | **HOLDS** |

The naive arm handed out **477 claims for 60 tickets** — individual tickets went
to nine different workers. That is what a literal port costs: the SQL is
unchanged and correct-looking, and every concurrency property quietly evaporates
with the mutex, because the mutex was the only thing providing them.

The locked arm is exact: 60 claims, 60 tickets, every `fence_seq` at exactly 1,
nothing passed over. Two details that are load-bearing and easy to get wrong:

- **`OF t` is not optional.** The query joins `workflow_states`; without `OF t`
  Postgres also locks the matched `workflow_states` row, which every ticket in
  the project shares — that would serialize the whole queue back down to one
  worker and quietly undo the reason for the port.
- **A worker retiring early is correct.** With `SKIP LOCKED` an empty result
  means "nothing claimable *that a peer isn't already holding*", not "queue
  empty". Any retry logic that treats the two as the same will spin.

Consistent with this, the existing note in CLAUDE.md that *the ready queue gets
neither offset nor cursor, because other workers claim from it as you read*
survives the port unchanged — and `ready_total` stays as approximate under
Postgres as it is today.

### What the spike does NOT yet prove

- **Lease expiry under concurrency.** The sweeper clears expired claims and
  bumps nothing; the interaction between a sweeper-cleared lease and an
  in-flight `SKIP LOCKED` claim is untested.
- **Fencing against a zombie writer.** `fence_seq` is bumped here, but no arm
  writes with a *stale* fence to confirm it is still refused.
- **Multi-process.** Both arms are 16 tasks in one process against one pool. The
  whole point of B is two app instances; that wants two processes.
- **`SERIALIZABLE` vs `READ COMMITTED`.** Everything above ran at Postgres's
  default `READ COMMITTED`. `SKIP LOCKED` is why that is sufficient for the
  queue, but other multi-statement invariants in the store have not been audited
  for it.

## Result: parity

The same suite, unchanged, against both engines:

| suite | SQLite | Postgres |
|---|---:|---:|
| unit (`--lib`) | 73 | 73 |
| `src/main.rs` | 4 | 4 |
| `tests/api.rs` | 198 | 198 |
| `tests/mcp.rs` | 37 | 37 |
| `tests/oauth.rs` | 36 | 36 |
| **total** | **348** | **348** |

`TAKOMO_TEST_PG=postgres://... cargo test` runs the Postgres arm; without it the
suite is SQLite as before. `clippy --all-targets -D warnings` and `fmt --check`
clean.

That number is the whole claim. The tests were not rewritten to accommodate
Postgres — they drive the real HTTP surface and know nothing about the backend,
so a green run is evidence about behaviour rather than about the port's own
opinion of itself.

## What the port found that a port is supposed to find

Five things that would each have been a silent production defect:

1. **The sync driver cannot be called from inside a tokio runtime.** The
   `postgres` crate drives its own runtime, and `block_on` panics on a thread
   already inside one — which every axum handler is. Connecting, querying AND
   dropping the client all run on a scoped thread. The `Drop` case was the worst:
   unwinding from a failed assertion drops the `Store`, the destructor panics
   during that unwind, and the double panic aborted the test binary with SIGABRT,
   so one ordinary failure destroyed the other 197 results.

2. **`patch_ticket` was only safe because SQLite locks the whole database.** It
   reads a ticket, merges `links` in Rust, writes it back. On SQLite nothing can
   commit in between. On Postgres at READ COMMITTED another writer can, and the
   merge then resurrects a key that writer just deleted — a lost update, in the
   path whose comment says it exists to prevent exactly that.
   `get_ticket_for_update` takes a row lock; `FOR UPDATE` is stripped on the
   SQLite arm, which neither supports nor needs it.

3. **A guessed default changed the security posture.** `schedule_approval` is
   added by `migrate()` rather than by `SCHEMA`, so it was missing from the first
   translation; added back with `DEFAULT 0`, it silently turned agent-proposed
   schedules from "wait for a human" into "activate yourself".

4. **Postgres's stricter grouping exposed ambiguous SQL of our own.** Three
   rollups grouped by `t.state` while a correlated subquery read `t.project`, so
   SQLite was free to take the category from an arbitrary row. Rewritten as LEFT
   JOINs grouped on the joined column — better SQL on both engines.

5. **Two dynamic queries mixed `?1` and bare `?`.** SQLite numbered them
   correctly by luck. The shim refuses the mix rather than guessing, which is how
   they surfaced.

## What is NOT done

- **The SQLite arm is still there.** The port is one-way by intent; the second
  backend exists so the two can be compared and is deleted when this lands.
- **`Store::snapshot_into` is SQLite-only.** It copies the database file, WAL
  included. The Postgres equivalent is `pg_dump`/basebackup — a different
  mechanism producing a different artifact — so `/v1/export`'s snapshot form has
  no Postgres implementation yet, and its test stands down there.
- **The read-modify-write audit is one deep, not exhaustive.** Finding 2 was
  fixed where a test caught it. Every other read-then-write inside `with_tx` has
  the same shape and has NOT been reviewed. Within one takomo process the writer
  mutex still serializes them; the exposure is an external writer, which this
  product has by design (`takomo token|project|seed` operate on the database
  directly).
- **Performance is unmeasured beyond the earlier micro-benchmark.** The suite
  runs ~7x slower on Postgres, but that is dominated by creating a 29-table
  schema per test, not by the store.

## Not in scope for the spike

Merging. This branch exists to find out what the port costs and whether the
`Store` seam survives contact. Nothing here is intended for `main`.
