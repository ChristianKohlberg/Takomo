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

**Decided: B, with `sqlx`.** Which means the async conversion reaches
`src/api/`, `src/mcp.rs` and `src/main.rs`, and the exactly-one-claimant
guarantee has to be re-proven. That proof is below, and it was done first
precisely because everything else is mechanical translation whose value is zero
if this does not hold.

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

## Not in scope for the spike

Merging. This branch exists to find out what the port costs and whether the
`Store` seam survives contact. Nothing here is intended for `main`.
