//! Repository layer. All SQL lives under this module; handlers never touch the
//! database directly. SQLite (WAL) is the only backend in v0; the surface is
//! kept narrow and connection-agnostic so a Postgres implementation could be
//! added behind the same methods later.

pub mod agent_chat;
mod answer_grants;
mod checkcollab;
mod checklist;
mod claims;
pub mod crdt;
mod docs;
mod document_appearance;
pub use document_appearance::{DocumentAppearance, DocumentAppearanceOverrides, DocumentTemplate};
mod environments;
mod events;
mod helpers;
mod impact;
mod initiatives;
mod metrics;
pub mod mindmapdoc;
mod mindmaps;
mod model;
mod moves;
mod oauth;
mod projects;
mod work_lanes;
mod writing_instructions;
pub use writing_instructions::{WritingInstruction, WritingInstructions};
pub mod prose;
mod questions;
mod ready_sql;
mod roadmap;
mod schedules;
mod shares;
pub mod spec_history;
mod tags;
pub mod testruns;
mod tickets;
mod tokens;
pub mod trace;
mod transition;
mod users;
mod workflows;

pub use answer_grants::{DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS};
pub use checklist::{
    glob_matches, CaseFileOutcome, CaseInput, CheckCreate, CheckFilter, CheckPatch, PolicyInput,
    ReleasePush, VerdictInput, WorkItem, MAX_CASES_PAGE, MAX_CASES_PER_FILE, MAX_CHECKS_PAGE,
    MAX_CHECK_GLOBS, MAX_RELEASE_PATHS,
};
pub use claims::{
    ClaimMovement, ClaimStatus, ForcedRelease, ReadyFilter, DEFAULT_TTL_SECONDS, MAX_TTL_SECONDS,
};
pub use crdt::{
    CollabKind, CollabObject, CollabSession, COMPACT_AFTER_UPDATES, MAX_OBJECT_BYTES,
    MAX_UPDATE_BYTES, SESSION_TTL_SECONDS,
};
pub use docs::{
    DocumentCreate, DocumentFilter, DocumentPatch, MAX_DOCUMENTS_PAGE, MAX_DOCUMENTS_PER_PROJECT,
};
pub use environments::{
    EnvironmentCreate, EnvironmentFilter, EnvironmentPatch, MAX_ENVIRONMENTS_PAGE,
    MAX_ENVIRONMENTS_PER_PROJECT,
};
pub use events::EventFilter;
pub use initiatives::{
    DeletedInitiative, EntryCreate, InitiativeCreate, InitiativeListFilter, InitiativePatch,
    INITIATIVE_STATUSES, MAX_ENTRIES_PAGE, MAX_ENTRY_CONTENT_BYTES, MAX_INITIATIVES_PAGE,
    MAX_INITIATIVE_BYTES, MAX_INITIATIVE_ENTRIES,
};
pub use mindmaps::{
    validate_promotion_target, BranchPromotion, MindmapChange, MindmapCreate, MindmapListFilter,
    MindmapPatch, MAX_MINDMAPS_PAGE, MAX_MINDMAPS_PER_PROJECT, MINDMAP_STATUSES, PROMOTION_TARGETS,
};
pub use model::*;
pub use moves::{MoveOutcome, MoveRequest, MovedTicket, MAX_MOVE_TICKETS};
pub use oauth::{
    ACCESS_TOKEN_TTL_SECONDS, AUTH_CODE_TTL_SECONDS, MAX_REDIRECT_URIS, REFRESH_TOKEN_TTL_SECONDS,
    SPENT_CODE_RETENTION_SECONDS, UNUSED_CLIENT_RETENTION_SECONDS,
};
pub use projects::{
    normalize_answer_link_ttl, normalize_claim_ttls, normalize_style_guide, Conventions,
    DeletedCounts, ProjectCreateSettings, MAX_STYLE_GUIDE_CHARS,
};
pub use questions::{
    question_quality_hints, AnswerOutcome, Answerer, AskRequest, QuestionFilter, ResumeBlocked,
    ReviseOptionsRequest, TimeoutAction, MAX_QUESTIONS_PAGE, QUESTION_KINDS,
};
pub use schedules::{
    derive_outcome, ScheduleCreate, ScheduleListFilter, SchedulePatch, ScheduleTemplate,
    MAX_SCHEDULES_PAGE, MAX_SCHEDULES_PER_PROJECT, SCHEDULE_STATUSES,
};
pub use shares::{ShareKind, DEFAULT_SHARE_TTL_SECONDS, MAX_SHARE_TTL_SECONDS, SHARE_TICKETS_PAGE};
pub use tags::{normalize_tag_ref, validate_tag_kind, TagCreate, TagListFilter, TagPatch};
pub use tickets::{
    merge_patch, ArchivedFilter, DepDirection, TicketCreate, TicketListFilter, TicketPatch,
};
pub use trace::{TraceEntry, CLIENT_TRACE_KINDS, MAX_TRACE_PAGE, TRACE_KINDS};
pub use users::{validate_user_handle, UserCreate, UserListFilter, UserPatch, MAX_USERS_PAGE};

use crate::error::{ApiError, ApiResult};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// How many read-only companion connections back `with_conn`.
///
/// Small on purpose. They exist so a table scan (`/v1/export`, `/v1/metrics`, a
/// project roadmap, a transitive dep graph) cannot queue behind — or in front
/// of — a claim, not to scale reads horizontally: SQLite reads come off the page
/// cache and are CPU-bound, so more connections than cores buys nothing but file
/// handles and page caches. Four is enough that a slow export leaves readers for
/// the board's event polling.
const READ_CONNECTIONS: usize = 4;

pub struct Store {
    pub check_updates: tokio::sync::broadcast::Sender<(String, Vec<u8>)>,
    pub changes: tokio::sync::watch::Sender<u64>,
    /// **The** writer. Every mutation goes through `with_tx` and this mutex, and
    /// that serialization is the exactly-one-claimant guarantee for the ready
    /// queue. There is deliberately never a second writer: no call site in this
    /// codebase handles `SQLITE_BUSY` on a write, because with one writer it
    /// cannot happen.
    conn: Mutex<Connection>,
    /// Read-only companions for `with_conn`. Opened `SQLITE_OPEN_READ_ONLY` with
    /// `query_only` on top, so a write that strays onto the read path fails loudly
    /// instead of quietly becoming a second writer. Empty means "no reader" —
    /// see [`Store::open`] — and `with_conn` falls back to the writer.
    readers: Vec<Mutex<Connection>>,
    /// Round-robin cursor, used only when every reader is busy.
    next_reader: AtomicUsize,
    /// Where the database lives, when it lives anywhere a second connection can
    /// reach — `None` for the private databases [`is_private_db`] describes.
    /// Only [`Store::snapshot_into`] needs it: every other connection this store
    /// will ever open is opened in [`Store::open`], while a snapshot has to open
    /// one later, on demand, outside both the writer and the reader pool.
    path: Option<PathBuf>,
}

impl Store {
    /// Open (creating if needed) the database at `path` and initialize schema.
    ///
    /// Ordering matters: the writer runs `SCHEMA` and `migrate()` to completion
    /// **before** any read connection is opened, so a reader can never observe a
    /// half-migrated database (and never has to `CREATE`/`ALTER` anything itself
    /// — it could not, being read-only).
    pub fn open(path: impl AsRef<Path>) -> ApiResult<Store> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .map_err(|e| ApiError::internal(format!("cannot open database: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // BEFORE the schema, not after. `CREATE TABLE IF NOT EXISTS checks`
        // would cheerfully create an empty `checks` beside a populated `lanes`,
        // and the rename could then never run — the database would carry both,
        // with every row in the one nothing reads. This is the only migration
        // step that has to precede the schema batch.
        rename_lanes_to_checks(&conn)?;
        widen_doc_log_to_collab_objects(&conn)?;
        // Also before the batch: the batch indexes the column it adds.
        add_collab_session_minted_by(&conn)?;
        // Same reason: the batch indexes the column this adds. It must run after
        // `rename_lanes_to_checks`, because before that the table is `lanes`.
        add_check_node(&conn)?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(include_str!("agent_chat.sql"))?;
        conn.execute_batch(include_str!("work_lanes.sql"))?;
        migrate(&conn)?;
        checkcollab::seed_existing(&conn)?;
        // After the schema and the additive migrations, because it writes into
        // `crdt_updates` and reads the `nodes` column both of those provide.
        mindmaps::adopt_legacy_nodes(&conn)?;
        // After the schema and the migrations, before any reader opens: the
        // shipped workflows must be in the library for the same reason the
        // schema must exist, and a reader that saw the table without them would
        // report an empty library on a fresh database.
        Store::seed_builtin_workflows(&conn, crate::ids::now_ms())?;

        // A second connection to `:memory:` (or to a private temp database) is a
        // *different, empty* database, not a second view of this one. Nothing in
        // this repo opens the store that way, but rather than leave the trap
        // armed for whoever does, detect it and run without readers: `with_conn`
        // then falls back to the writer, which is exactly the old behavior.
        let private = is_private_db(path);
        let readers = if private {
            Vec::new()
        } else {
            let mut readers = Vec::with_capacity(READ_CONNECTIONS);
            for _ in 0..READ_CONNECTIONS {
                readers.push(Mutex::new(open_reader(path)?));
            }
            readers
        };

        Ok(Store {
            check_updates: tokio::sync::broadcast::channel(256).0,
            changes: tokio::sync::watch::channel(0).0,
            conn: Mutex::new(conn),
            readers,
            next_reader: AtomicUsize::new(0),
            path: (!private).then(|| path.to_path_buf()),
        })
    }

    /// Run `f` inside a single IMMEDIATE transaction. SQLite's single-writer
    /// model plus this process-wide mutex is the claim-serialization
    /// guarantee: every mutating operation is one atomic step.
    ///
    /// The closure is **synchronous, and must stay synchronous**: that is what
    /// makes holding this connection across an `.await` structurally impossible.
    /// An async variant would trade the latency problem `with_conn` used to have
    /// for a whole class of deadlocks.
    pub(crate) fn with_tx<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction) -> ApiResult<T>,
    ) -> ApiResult<T> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(ApiError::from)?;
        let out = f(&tx)?;
        tx.commit().map_err(ApiError::from)?;
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
        Ok(out)
    }

    /// Run `f` with a connection for reading.
    ///
    /// Reads run on a read-only companion connection, never on the writer, so a
    /// long scan cannot stall a claim, a transition or a heartbeat. WAL is what
    /// makes that safe: readers do not block the writer and the writer does not
    /// block readers.
    ///
    /// `f` runs inside a DEFERRED transaction, i.e. one WAL snapshot for the
    /// whole closure. That is not decoration — several reads here are
    /// multi-statement (the export scans tickets, then queries deps and comments
    /// per ticket), and on the shared mutex they used to be atomic against
    /// writers by accident. The snapshot keeps that property on purpose.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> ApiResult<T>) -> ApiResult<T> {
        if self.readers.is_empty() {
            let conn = self
                .conn
                .lock()
                .map_err(|_| ApiError::internal("store lock poisoned"))?;
            return f(&conn);
        }
        // Prefer any idle reader; only when all of them are busy pick one to
        // queue on, round-robin so concurrent readers spread out.
        for reader in &self.readers {
            if let Ok(mut conn) = reader.try_lock() {
                return read_snapshot(&mut conn, f);
            }
        }
        let idx = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        let mut conn = self.readers[idx]
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        read_snapshot(&mut conn, f)
    }

    /// Write a consistent snapshot of the WHOLE database to `dest` — every
    /// project, every table — as one self-contained SQLite file.
    ///
    /// `VACUUM INTO` rather than a file copy, because under WAL the `.db` file
    /// alone is a torn snapshot: committed data lives in the `-wal` sidecar
    /// until a checkpoint moves it. Verified, not assumed — 1000 rows committed
    /// on a connection left open (so nothing checkpoints) are all present in the
    /// snapshot. The output is `journal_mode=delete` with no sidecar of its own,
    /// which is what makes it a single file you can hand someone.
    ///
    /// Three constraints decide how it must be run, and each one rules out the
    /// connection you would otherwise reach for:
    ///
    /// - **Not inside a transaction** — `cannot VACUUM from within a
    ///   transaction`. That rules out [`Store::with_conn`], which wraps its
    ///   closure in a DEFERRED one.
    /// - **Not with `query_only`** — SQLite classifies VACUUM as a write and
    ///   refuses it with `attempt to write a readonly database`, even though the
    ///   only file written is `dest`. That rules out the pooled readers.
    /// - **Not on the writer** — it would hold the mutex every claim and
    ///   transition serializes behind for the length of a full-database scan,
    ///   which is the stall `long_export_does_not_stall_claims_and_heartbeats`
    ///   exists to prevent.
    ///
    /// What is left, and what this uses, is a dedicated connection opened
    /// `SQLITE_OPEN_READ_ONLY` *without* `query_only`: the open flag still
    /// refuses any write to the source, so the "never a second writer"
    /// invariant holds, while VACUUM INTO is free to write `dest`. It is opened
    /// per call rather than pooled because a dump is rare and long, and parking
    /// it in the pool would cost a reader the board's event polling needs.
    ///
    /// Where a caller should stage a [`Store::snapshot_into`] destination:
    /// beside the database itself, falling back to the system temp directory for
    /// a private database (which has no directory of its own).
    ///
    /// Beside the database on purpose. A snapshot is the size of the whole
    /// store, and the volume holding the database is the one an operator
    /// provisioned for exactly that much data — where `/tmp` is a tmpfs in RAM
    /// on a good many hosts, so staging there turns a large backup into memory
    /// pressure on the running server.
    pub fn snapshot_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir)
    }

    /// `dest` must not already exist; SQLite refuses to overwrite it.
    pub fn snapshot_into(&self, dest: &Path) -> ApiResult<()> {
        let dest = dest.to_str().ok_or_else(|| {
            ApiError::internal("snapshot destination path is not valid UTF-8".to_string())
        })?;
        match &self.path {
            Some(path) => {
                let conn = open_snapshot_reader(path)?;
                conn.execute("VACUUM INTO ?1", [dest])
                    .map_err(|e| ApiError::internal(format!("cannot write snapshot: {e}")))?;
            }
            // A private database has no path a second connection could reach, so
            // the writer is the only connection there is. Holding it is
            // acceptable precisely because nothing else can be reading it.
            None => {
                let conn = self
                    .conn
                    .lock()
                    .map_err(|_| ApiError::internal("store lock poisoned"))?;
                conn.execute("VACUUM INTO ?1", [dest])
                    .map_err(|e| ApiError::internal(format!("cannot write snapshot: {e}")))?;
            }
        }
        Ok(())
    }
}

/// Run a read closure inside a DEFERRED transaction — a stable WAL snapshot for
/// its whole duration — and end it without committing (there is nothing to
/// commit; the connection could not write if it tried).
fn read_snapshot<T>(
    conn: &mut Connection,
    f: impl FnOnce(&Connection) -> ApiResult<T>,
) -> ApiResult<T> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(ApiError::from)?;
    let out = f(&tx)?;
    drop(tx); // rollback: ends the read snapshot
    Ok(out)
}

/// A read-only companion connection to an already-migrated database.
///
/// Which pragmas a reader needs, and which it does not:
/// - `busy_timeout`: **needed**. WAL keeps readers off the writer's back for
///   normal commits, but a reader can still meet a busy database — a checkpoint
///   restarting the WAL, or another process recovering it. Without a timeout
///   that surfaces as an immediate `SQLITE_BUSY`; with one it just waits.
/// - `foreign_keys`: set for parity with the writer. Inert for pure reads (it
///   only governs constraint enforcement on writes), but it costs nothing and
///   keeps the two connections from differing in a way someone would later have
///   to reason about.
/// - `query_only`: belt to `SQLITE_OPEN_READ_ONLY`'s braces. The open flag
///   already refuses writes; this refuses them one layer earlier, so a stray
///   write on the read path is a loud error rather than a silent second writer.
/// - `journal_mode`: **not** set. It is a property of the database file, already
///   WAL, and setting it is itself a write — a read-only connection would fail.
/// - `synchronous`: **not** set. It only governs fsync on commit, and this
///   connection never commits.
fn open_reader(path: &Path) -> ApiResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ApiError::internal(format!("cannot open read-only database connection: {e}")))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(conn)
}

/// A connection for [`Store::snapshot_into`], and for nothing else.
///
/// Deliberately NOT [`open_reader`]: it differs in exactly one pragma, and that
/// pragma is the whole point. `query_only` would refuse VACUUM INTO, because
/// SQLite classifies VACUUM as a write regardless of which file it writes.
/// `SQLITE_OPEN_READ_ONLY` still holds the line that matters — the source
/// database cannot be modified through this connection — so dropping
/// `query_only` widens what is permitted from "no writes at all" to "no writes
/// except a new file", which is precisely the operation being authorized.
///
/// The busy timeout is longer than a reader's. A snapshot reads the entire
/// database, so its odds of meeting a checkpoint restarting the WAL are far
/// higher than a point read's, and waiting is the right answer where a fast
/// `SQLITE_BUSY` is not.
fn open_snapshot_reader(path: &Path) -> ApiResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ApiError::internal(format!("cannot open database for snapshot: {e}")))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(conn)
}

/// True when `path` names a database no second connection can reach: SQLite's
/// in-memory database, or the anonymous on-disk temp database an empty path
/// asks for. Both are private per connection.
fn is_private_db(path: &Path) -> bool {
    match path.to_str() {
        Some(s) => {
            s.is_empty()
                || s == ":memory:"
                || (s.starts_with("file:")
                    && s.contains("mode=memory")
                    && !s.contains("cache=shared"))
        }
        None => false,
    }
}

/// Does this database already have a table (or view) by this name?
fn has_table(conn: &Connection, name: &str) -> ApiResult<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Does this table carry this column?
fn has_column(conn: &Connection, table: &str, column: &str) -> ApiResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Rename the checklist `lanes` concept to `checks`, on a database that predates
/// the rename. Runs BEFORE the schema batch — see `Store::open`.
///
/// Why the concept was renamed at all: `lane` already meant "the initiative a
/// feature is worked in" on the roadmap and in `/initiatives`, so one product
/// carried two unrelated lanes. The verification one became `check`.
///
/// The column is `check_id`, not `check`, because `CHECK` is a SQL keyword.
///
/// Each step is guarded on "the old name is here and the new one is not", so
/// this is a no-op on a fresh database and on every boot after the first. Ids
/// are deliberately NOT rewritten: an existing row keeps its `lane-…` primary
/// key, because an id is opaque and rewriting primary keys to make a prefix
/// pretty is the one part of this rename that could lose data.
/// Widen the document update log and sync tickets to any collaborative object.
///
/// `/documents` shipped first, so both tables were written in terms of
/// documents, with a hard `REFERENCES documents(id)` on the owning column.
/// Mindmaps need the same log, and SQLite cannot drop a foreign key without
/// rebuilding the table — so this rebuilds them once, here, rather than leaving
/// two parallel logs to drift.
///
/// **It runs BEFORE the `CREATE TABLE IF NOT EXISTS` batch, and that ordering is
/// half the correctness argument** — the same one `rename_lanes_to_checks`
/// below makes. Run it after, and `CREATE TABLE IF NOT EXISTS crdt_updates`
/// would already have made an empty table beside the populated `doc_updates`;
/// this function would then see its target present, decline to copy, and every
/// existing document would come back blank.
///
/// **The other half is that it is ONE transaction, and that is not decoration.**
/// `execute_batch` prepares and steps each statement in turn with no implicit
/// `BEGIN`, so an unwrapped version autocommits the `CREATE` and then copies
/// hundreds of megabytes of prose in a second statement. Lose the process in
/// that window — an OOM kill, a full disk, a container eviction — and the
/// database is left with both tables present, which the guard reads as "already
/// done". The next start serves every document blank, having said nothing, with
/// the real bytes still sitting in a table nothing reads. That failure is
/// indistinguishable from success from the outside, which is exactly what makes
/// it worth a transaction.
///
/// Copy order is preserved; the `seq` values themselves are not. Replay reads
/// `ORDER BY seq`, so what matters is the sequence, and letting the new table
/// allocate its own keys avoids any chance of colliding with a row written by
/// another kind.
///
/// **This is one-way.** Once the old tables are gone, an older binary starting
/// against this database will create them empty and serve every document blank.
/// There is no downgrade path; roll back by restoring the database, not the
/// binary.
fn widen_doc_log_to_collab_objects(conn: &Connection) -> ApiResult<()> {
    if !has_table(conn, "doc_updates")? && !has_table(conn, "doc_sessions")? {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = widen_inside_transaction(conn);
    match outcome {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            // Best-effort: if the rollback itself fails there is nothing left to
            // try, and the original error is the one worth reporting.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The body of [`widen_doc_log_to_collab_objects`], with the transaction held by
/// the caller so that any error unwinds the whole move.
fn widen_inside_transaction(conn: &Connection) -> ApiResult<()> {
    if has_table(conn, "doc_updates")? {
        if !has_table(conn, "crdt_updates")? {
            conn.execute_batch(
                "CREATE TABLE crdt_updates (
                   seq         INTEGER PRIMARY KEY AUTOINCREMENT,
                   object_kind TEXT NOT NULL,
                   object_id   TEXT NOT NULL,
                   blob        BLOB NOT NULL,
                   bytes       INTEGER NOT NULL,
                   created_by  TEXT NOT NULL,
                   created_at  INTEGER NOT NULL
                 )",
            )?;
        }
        // A database that already holds document rows here, while the old table
        // still exists, is a state this function cannot produce. Refuse rather
        // than guess: skipping would strand the prose, and copying might double
        // it.
        let already: i64 = conn.query_row(
            "SELECT COUNT(*) FROM crdt_updates WHERE object_kind = 'document'",
            [],
            |r| r.get(0),
        )?;
        if already > 0 {
            return Err(ApiError::internal(
                "cannot widen the document log: crdt_updates already holds document rows while doc_updates still exists. Restore this database from a backup rather than starting against it.",
            ));
        }
        conn.execute_batch(
            "INSERT INTO crdt_updates (object_kind, object_id, blob, bytes, created_by, created_at)
               SELECT 'document', document, blob, bytes, created_by, created_at
               FROM doc_updates ORDER BY seq;
             DROP TABLE doc_updates;",
        )?;
    }

    if has_table(conn, "doc_sessions")? {
        if !has_table(conn, "crdt_sessions")? {
            conn.execute_batch(
                "CREATE TABLE crdt_sessions (
                   id          TEXT PRIMARY KEY,
                   token_hash  TEXT NOT NULL UNIQUE,
                   object_kind TEXT NOT NULL,
                   object_id   TEXT NOT NULL,
                   project     TEXT NOT NULL,
                   actor       TEXT NOT NULL,
                   \"user\"      TEXT REFERENCES users(id),
                   display     TEXT NOT NULL,
                   can_write   INTEGER NOT NULL,
                   expires_at  INTEGER NOT NULL,
                   created_at  INTEGER NOT NULL,
                   revoked_at  INTEGER
                 )",
            )?;
        }
        conn.execute_batch(
            "INSERT OR IGNORE INTO crdt_sessions (id, token_hash, object_kind, object_id, project, actor,
                                        \"user\", display, can_write, expires_at, created_at, revoked_at)
               SELECT id, token_hash, 'document', document, project, actor,
                      \"user\", display, can_write, expires_at, created_at, revoked_at
               FROM doc_sessions;
             DROP TABLE doc_sessions;",
        )?;
    }

    Ok(())
}

fn rename_lanes_to_checks(conn: &Connection) -> ApiResult<()> {
    if has_table(conn, "lanes")? && !has_table(conn, "checks")? {
        // SQLite updates the REFERENCES clauses in other tables for us, so
        // `cases.lane REFERENCES lanes(id)` follows the table to its new name.
        conn.execute("ALTER TABLE lanes RENAME TO checks", [])?;
    }
    if has_table(conn, "lane_globs")? && !has_table(conn, "check_globs")? {
        conn.execute("ALTER TABLE lane_globs RENAME TO check_globs", [])?;
    }
    if has_table(conn, "check_globs")?
        && has_column(conn, "check_globs", "lane")?
        && !has_column(conn, "check_globs", "check_id")?
    {
        conn.execute("ALTER TABLE check_globs RENAME COLUMN lane TO check_id", [])?;
    }
    if has_table(conn, "cases")?
        && has_column(conn, "cases", "lane")?
        && !has_column(conn, "cases", "check_id")?
    {
        conn.execute("ALTER TABLE cases RENAME COLUMN lane TO check_id", [])?;
    }
    // The old indexes survive a table rename under their old names, and the
    // schema batch is about to create identically-shaped ones under the new
    // names. Drop the old names rather than carry two indexes over one column.
    for stale in ["idx_lanes_project", "idx_lanes_epic", "idx_cases_lane"] {
        conn.execute(&format!("DROP INDEX IF EXISTS {stale}"), [])?;
    }
    Ok(())
}

/// Idempotent, additive, non-destructive startup migrations. Runs after the
/// `CREATE TABLE IF NOT EXISTS` schema on every open. It only ever ADDs missing
/// columns/indexes on a database that predates them — it never drops, rewrites,
/// or recreates existing data, so it is safe to run against a populated live DB
/// on every boot.
///
/// The one step that does not fit that description is `rename_lanes_to_checks`,
/// which is why it lives in its own function and runs before the schema.
/// Add `crdt_sessions.minted_by` to a database that predates it.
///
/// BEFORE the schema batch, not in `migrate` with the other additive columns,
/// because the batch also creates an INDEX on this column: on an older database
/// `CREATE TABLE IF NOT EXISTS crdt_sessions` correctly does nothing, and the
/// index then refers to a column that is not there yet and the open fails. The
/// same ordering trap `rename_lanes_to_checks` carries a note about.
fn add_collab_session_minted_by(conn: &Connection) -> ApiResult<()> {
    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(crdt_sessions)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    // Empty means the table does not exist yet — a fresh database, where the
    // schema batch is about to create it with the column already on it.
    if !columns.is_empty() && !columns.iter().any(|c| c == "minted_by") {
        conn.execute("ALTER TABLE crdt_sessions ADD COLUMN minted_by TEXT", [])?;
        // Every ticket outstanding at the moment of the upgrade has no answer to
        // "which token minted this", so `revoke_collab_sessions_of_token` can
        // never match it: revoking a leaked credential would leave its ticket
        // working for the rest of its 12 hours, which is the exact case the
        // cascade was added for. They are revoked here instead. The cost is that
        // an upgrade asks open browsers for a new ticket, which they do anyway
        // on reconnect.
        conn.execute(
            "UPDATE crdt_sessions SET revoked_at = ?1 WHERE revoked_at IS NULL",
            [crate::ids::now_ms()],
        )?;
    }
    Ok(())
}

/// Add `checks.node` to a database that predates it.
///
/// BEFORE the schema batch, for the reason `add_collab_session_minted_by` gives
/// and I got wrong here first: the batch creates an INDEX on this column, and on
/// an existing `checks` table `CREATE TABLE IF NOT EXISTS` correctly does
/// nothing — so the index would name a column that is not there yet and the open
/// would fail. Caught by the test that opens a pre-rename database.
fn add_check_node(conn: &Connection) -> ApiResult<()> {
    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(checks)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if !columns.is_empty() && !columns.iter().any(|c| c == "node") {
        conn.execute("ALTER TABLE checks ADD COLUMN node TEXT", [])?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> ApiResult<()> {
    // archived_at (nullable) separates archived tickets from active ones. Older
    // databases predate the column; add it only when PRAGMA table_info shows it
    // absent. `CREATE TABLE IF NOT EXISTS` above already carries it for a fresh
    // DB, so on those this ALTER is skipped.
    let trace_columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(plan_trace)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if !trace_columns.is_empty() && !trace_columns.iter().any(|c| c == "text") {
        conn.execute("ALTER TABLE plan_trace ADD COLUMN text TEXT", [])?;
    }

    let mindmap_columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(mindmaps)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if !mindmap_columns.iter().any(|c| c == "nodes") {
        conn.execute(
            "ALTER TABLE mindmaps ADD COLUMN nodes INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    // Sweep plan history whose project or map is already gone.
    //
    // Deleting a project or a map used to leave `plan_trace` behind, and that
    // table carries `text` — what each section SAID — so the prose outlived the
    // delete that was supposed to remove it. The delete paths take it now; this
    // clears what earlier ones left. Unreachable rows either way: the routes
    // that read them resolve the map first.
    conn.execute(
        "DELETE FROM plan_trace WHERE project NOT IN (SELECT id FROM projects) \
         OR mindmap NOT IN (SELECT id FROM mindmaps)",
        [],
    )?;

    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(tickets)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !columns.iter().any(|c| c == "archived_at") {
        conn.execute("ALTER TABLE tickets ADD COLUMN archived_at TEXT", [])?;
    }
    // tickets.tags: canonical `kind:handle` references into the project tag
    // registry. Additive; older ticket tables predate it. Defaults to the empty
    // JSON array, matching the `labels` precedent. The `tags` table itself is in
    // SCHEMA (CREATE TABLE IF NOT EXISTS), so it appears on old DBs automatically.
    if !columns.iter().any(|c| c == "tags") {
        conn.execute(
            "ALTER TABLE tickets ADD COLUMN tags TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    // tickets.lapsed_claim_holder: who held the lease that most recently expired
    // here, so the lapsed holder can resume it in place rather than walking the
    // ticket back through the ready queue (takomo-jb5i). Additive and nullable;
    // older ticket tables predate it, and NULL is exactly the right value for
    // every existing row — "no lease lapsed here that nothing has superseded".
    if !columns.iter().any(|c| c == "lapsed_claim_holder") {
        conn.execute(
            "ALTER TABLE tickets ADD COLUMN lapsed_claim_holder TEXT",
            [],
        )?;
    }
    // Partial index to keep `archived=only` and the default `archived_at IS
    // NULL` filter cheap. Created after the column is guaranteed to exist.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tickets_archived ON tickets(archived_at) WHERE archived_at IS NOT NULL",
        [],
    )?;
    // questions.mode distinguishes blocking (parks/resumes the ticket) from
    // advisory (routed + recorded, no state change). Older question tables
    // predate it; add it defaulting to 'blocking' (the original behavior). Only
    // ALTERs when PRAGMA shows it absent; a fresh DB already carries it.
    let question_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(questions)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "mode") {
        conn.execute(
            "ALTER TABLE questions ADD COLUMN mode TEXT NOT NULL DEFAULT 'blocking'",
            [],
        )?;
    }
    // questions.awaiting tracks whose turn it is on a question's follow-up thread
    // ('human' by default; 'agent' after a human bounces it back for research).
    // Older tables predate it; add it defaulting to 'human'.
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "awaiting") {
        conn.execute(
            "ALTER TABLE questions ADD COLUMN awaiting TEXT NOT NULL DEFAULT 'human'",
            [],
        )?;
    }
    // Richer question fields the inbox UI renders (all optional/additive):
    // recommendation confidence (1-4), a recommendation rationale, a one-line
    // list summary, and per-option descriptions (parallel to `options`).
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "confidence") {
        conn.execute("ALTER TABLE questions ADD COLUMN confidence INTEGER", [])?;
    }
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "recommended_note") {
        conn.execute("ALTER TABLE questions ADD COLUMN recommended_note TEXT", [])?;
    }
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "summary") {
        conn.execute("ALTER TABLE questions ADD COLUMN summary TEXT", [])?;
    }
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "option_notes") {
        conn.execute(
            "ALTER TABLE questions ADD COLUMN option_notes TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    // Multi-select choose: `multi` marks a choose question that takes several
    // options; `recommended_multi` is the suggested set.
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "multi") {
        conn.execute(
            "ALTER TABLE questions ADD COLUMN multi INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !question_cols.is_empty() && !question_cols.iter().any(|c| c == "recommended_multi") {
        conn.execute(
            "ALTER TABLE questions ADD COLUMN recommended_multi TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    // projects.question_language: the human-facing language agents should phrase
    // ask-a-human questions in for this project (nullable = no preference).
    // Additive; older project tables predate it.
    let project_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(projects)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "question_language") {
        conn.execute("ALTER TABLE projects ADD COLUMN question_language TEXT", [])?;
    }
    // projects.style_guide: the project's house style for the text agents write
    // — ticket titles/bodies and human-facing questions (nullable = no
    // preference). Additive; older project tables predate it.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "style_guide") {
        conn.execute("ALTER TABLE projects ADD COLUMN style_guide TEXT", [])?;
    }
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "document_appearance_json") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN document_appearance_json TEXT",
            [],
        )?;
    }
    // projects.workflow_layout_json: where the editor's nodes sit for THIS
    // project's workflow. Nullable = never opened in the editor, so the client
    // lays it out from the graph.
    //
    // A column beside the workflow rather than a key inside it, because
    // `Workflow` is `deny_unknown_fields`: a `positions` key in the document
    // would be rejected the moment it was PUT back. Separate storage also means
    // dragging a node is not a workflow change — it emits no `workflow_changed`
    // event and wakes no long-poller.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "workflow_layout_json") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN workflow_layout_json TEXT",
            [],
        )?;
    }
    // projects.answer_link_ttl_seconds: this project's default lifetime for an
    // answer link (nullable = unset, so minting uses DEFAULT_ANSWER_TTL_SECONDS).
    // Additive, and deliberately NOT backfilled with the built-in default: a row
    // that says nothing keeps tracking the default if it ever moves again, while
    // a backfilled row would freeze today's number into every existing project.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "answer_link_ttl_seconds") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN answer_link_ttl_seconds INTEGER",
            [],
        )?;
    }
    // projects.claim_ttl_seconds / max_claim_ttl_seconds: this project's lease
    // policy (nullable = unset, so claiming uses DEFAULT_TTL_SECONDS and the cap
    // is MAX_TTL_SECONDS). A *different* setting from answer_link_ttl_seconds
    // above, which is about a credential handed outside the org; these two are
    // about how long a worker may hold a ticket. Same not-backfilled reasoning as
    // that column: silence keeps tracking the built-in default.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "claim_ttl_seconds") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN claim_ttl_seconds INTEGER",
            [],
        )?;
    }
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "max_claim_ttl_seconds") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN max_claim_ttl_seconds INTEGER",
            [],
        )?;
    }
    // projects.schedule_approval: whether a schedule an AGENT proposes over MCP
    // must be activated by a human before it fires. Defaults to 1 (on), because
    // the safe value is the one that surprises nobody — a schedule outlives the
    // token that created it, so letting a `write` credential make one that fires
    // forever is an escalation an operator should opt into deliberately.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "schedule_approval") {
        conn.execute(
            "ALTER TABLE projects ADD COLUMN schedule_approval INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    // projects.archived_at: when the project was archived (NULL = live). The
    // gate that freezes every write under a project while leaving reads alone.
    // Additive and nullable, and NULL is exactly right for every existing row —
    // a database that predates the column has no archived project in it.
    if !project_cols.is_empty() && !project_cols.iter().any(|c| c == "archived_at") {
        conn.execute("ALTER TABLE projects ADD COLUMN archived_at INTEGER", [])?;
    }
    // tickets.schedule / occurrence / expires_at: where a scheduled ticket came
    // from and how long it counts as live. All three nullable, and NULL is
    // exactly right for every existing row — "nothing made this on a cadence".
    if !columns.iter().any(|c| c == "schedule") {
        conn.execute("ALTER TABLE tickets ADD COLUMN schedule TEXT", [])?;
    }
    if !columns.iter().any(|c| c == "occurrence") {
        conn.execute("ALTER TABLE tickets ADD COLUMN occurrence INTEGER", [])?;
    }
    if !columns.iter().any(|c| c == "expires_at") {
        conn.execute("ALTER TABLE tickets ADD COLUMN expires_at INTEGER", [])?;
    }
    // tickets.claim_since: when the current lease was granted, so "how long has
    // this been held" is answerable — the question an indefinite epic claim is
    // judged by, since it has no expiry to read. Nullable; NULL on old rows
    // means "granted before the column existed", which the status endpoint
    // reports as unknown rather than inventing a timestamp.
    if !columns.iter().any(|c| c == "claim_since") {
        conn.execute("ALTER TABLE tickets ADD COLUMN claim_since INTEGER", [])?;
    }
    let idempotency_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(idempotency)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !idempotency_cols.is_empty() && !idempotency_cols.iter().any(|c| c == "body_hash") {
        conn.execute("ALTER TABLE idempotency ADD COLUMN body_hash TEXT", [])?;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS comment_idempotency (
            actor TEXT NOT NULL,
            key TEXT NOT NULL,
            comment TEXT NOT NULL,
            body_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (actor, key)
        )",
        [],
    )?;
    // Created after the columns are guaranteed to exist. This index IS the
    // exactly-once guarantee, so it is added on an old database too.
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_tickets_occurrence ON tickets(schedule, occurrence) WHERE schedule IS NOT NULL",
        [],
    )?;
    // tokens.user: the directory person this credential belongs to (src/store/users.rs).
    // Nullable, and NULL is exactly right for every existing row — a database that
    // predates the directory has no people in it, and an unbound token behaves
    // precisely as it always did. The `users` table itself is in SCHEMA
    // (CREATE TABLE IF NOT EXISTS), so it appears on an old DB automatically.
    let token_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(tokens)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !token_cols.is_empty() && !token_cols.iter().any(|c| c == "user") {
        conn.execute("ALTER TABLE tokens ADD COLUMN \"user\" TEXT", [])?;
    }
    // The consent snapshot carries the person too, so an OAuth-issued token is the
    // same human as the credential it was approved with rather than an anonymous
    // copy of their scopes.
    for table in ["oauth_codes", "oauth_refresh"] {
        let cols: Vec<String> = {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let cols = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            cols
        };
        if !cols.is_empty() && !cols.iter().any(|c| c == "user") {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN \"user\" TEXT"), [])?;
        }
    }
    // case_verdicts.environment: where the verdict was observed. Nullable, and
    // NULL is exactly right for every existing row — a verdict recorded before
    // environments existed states no environment, which is not the same as an
    // unknown one. Not back-filled: inventing a location for a past observation
    // would be inventing evidence.
    let verdict_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(case_verdicts)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !verdict_cols.is_empty() && !verdict_cols.iter().any(|c| c == "environment") {
        conn.execute("ALTER TABLE case_verdicts ADD COLUMN environment TEXT", [])?;
    }
    // checks.initiative: which initiative's conversation agreed this check should
    // exist. Nullable, and NULL is exactly right for every existing row — a check
    // filed before the link existed belongs to no initiative, which is a fact
    // rather than a gap. Not back-filled from the epic's `initiative:` tag: that
    // would assert a link nobody made.
    let check_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(checks)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !check_cols.is_empty() && !check_cols.iter().any(|c| c == "initiative") {
        conn.execute("ALTER TABLE checks ADD COLUMN initiative TEXT", [])?;
    }
    // After the column is guaranteed to exist.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_checks_initiative ON checks(initiative) WHERE initiative IS NOT NULL",
        [],
    )?;
    // answer_grants.user: which person an answer link was minted FOR. Nullable —
    // an older grant, and any grant handed to an outside expert, carries only its
    // free-form `actor`. Non-NULL is what lets the grant satisfy an assignee-gated
    // approval by identity instead of a synthesized scope.
    let grant_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(answer_grants)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !grant_cols.is_empty() && !grant_cols.iter().any(|c| c == "user") {
        conn.execute("ALTER TABLE answer_grants ADD COLUMN \"user\" TEXT", [])?;
    }
    let share_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(shares)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !share_cols.is_empty() && !share_cols.iter().any(|c| c == "last_used_at") {
        conn.execute("ALTER TABLE shares ADD COLUMN last_used_at INTEGER", [])?;
    }
    if !grant_cols.is_empty() && !grant_cols.iter().any(|c| c == "last_used_at") {
        conn.execute(
            "ALTER TABLE answer_grants ADD COLUMN last_used_at INTEGER",
            [],
        )?;
    }
    // The person behind a verdict, in the three places a verdict is recorded: the
    // append-only history (`case_verdicts.user`, the permanent record) and the two
    // mirrors of the latest human verdict. All nullable, and NULL is right for
    // every existing row — those verdicts were recorded before a credential could
    // name anybody, and inventing a person for them would be worse than admitting
    // the gap.
    for (table, column) in [
        ("cases", "human_user"),
        ("case_environments", "human_user"),
        ("case_verdicts", "\"user\""),
    ] {
        let cols: Vec<String> = {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let cols = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            cols
        };
        let bare = column.trim_matches('"');
        if !cols.is_empty() && !cols.iter().any(|c| c == bare) {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"), [])?;
        }
    }
    // questions.assignee: the person this decision is waiting on. Nullable, and
    // NULL on every existing row is right — they were routed by expertise alone.
    let question_assignee_cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(questions)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        cols
    };
    if !question_assignee_cols.is_empty() && !question_assignee_cols.iter().any(|c| c == "assignee")
    {
        conn.execute("ALTER TABLE questions ADD COLUMN assignee TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_questions_assignee ON questions(assignee) WHERE assignee IS NOT NULL",
        [],
    )?;
    testruns::import_legacy(conn, None)?;
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id                      TEXT PRIMARY KEY,
  name                    TEXT NOT NULL,
  workflow_json           TEXT NOT NULL,
  question_language       TEXT,
  style_guide             TEXT,
  -- Default lifetime, in seconds, of an answer link minted for one of this
  -- project's questions. NULL = unset; minting then falls back to the built-in
  -- DEFAULT_ANSWER_TTL_SECONDS. Bounded exactly like an explicit `ttl_seconds`.
  answer_link_ttl_seconds INTEGER,
  -- This project's lease policy, both NULL = unset (DEFAULT_TTL_SECONDS /
  -- MAX_TTL_SECONDS). `claim_ttl_seconds` is what a claim that names no
  -- ttl_seconds gets; `max_claim_ttl_seconds` is the ceiling an explicit one is
  -- checked against. Unrelated to answer_link_ttl_seconds above — that bounds a
  -- credential, these bound how long work may be held.
  claim_ttl_seconds       INTEGER,
  max_claim_ttl_seconds   INTEGER,
  -- When this project was archived, or NULL for a live project. Archiving is a
  -- GATE, not a state: while it is set, every write under the project is
  -- refused with a teaching 409 (`project.archived`) and the ready queue stops
  -- offering its tickets, while every read keeps working exactly as before.
  -- Reversible on purpose — clearing this column puts the project straight back
  -- to work, which is what makes archiving safe to reach for instead of DELETE.
  archived_at             INTEGER,
  created_at              INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS project_writing_instructions (
  project TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  settings_json TEXT NOT NULL
);

-- Denormalized view of each project's workflow states so queue/blocking
-- queries can join on claimable/terminal without parsing JSON.
CREATE TABLE IF NOT EXISTS workflow_states (
  project   TEXT NOT NULL,
  state     TEXT NOT NULL,
  category  TEXT NOT NULL,
  claimable INTEGER NOT NULL DEFAULT 0,
  terminal  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (project, state)
);

CREATE TABLE IF NOT EXISTS tickets (
  id               TEXT PRIMARY KEY,
  project          TEXT NOT NULL REFERENCES projects(id),
  type             TEXT NOT NULL DEFAULT 'task',
  parent           TEXT REFERENCES tickets(id),
  title            TEXT NOT NULL,
  body             TEXT NOT NULL DEFAULT '',
  state            TEXT NOT NULL,
  priority         TEXT NOT NULL DEFAULT 'normal',
  labels           TEXT NOT NULL DEFAULT '[]',
  tags             TEXT NOT NULL DEFAULT '[]',
  metadata         TEXT NOT NULL DEFAULT '{}',
  links            TEXT NOT NULL DEFAULT '{}',
  claim_holder     TEXT,
  -- When the current lease expires, or NULL while claim_holder is set for a
  -- claim with NO expiry — an epic claim taken without a TTL, held until
  -- released (or force-released). The sweeper's `<= now` predicate skips NULL
  -- naturally, which is exactly the point: nothing expires it.
  claim_expires_at INTEGER,
  -- When the current lease was granted. Cleared with the claim; what the
  -- claim-status endpoint reads to answer "held for how long".
  claim_since      INTEGER,
  -- The actor whose lease on this ticket ended by *expiry*, kept so the lapsed
  -- holder can resume it in place instead of walking the ticket back through the
  -- ready queue (takomo-jb5i). Set when an expired claim is cleared; cleared
  -- again by anything else that starts or ends a lease (a new claim, a voluntary
  -- release, an admin force-release, a transition that auto-releases). So
  -- non-NULL means exactly: "the last lease here lapsed, and nothing has taken,
  -- released or revoked one since" — which is why it never coexists with an
  -- active claim, and why `fence_seq` is still the lapse-time fence.
  lapsed_claim_holder TEXT,
  fence_seq        INTEGER NOT NULL DEFAULT 0,
  version          INTEGER NOT NULL DEFAULT 1,
  created_by       TEXT NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  archived_at      TEXT,
  -- Where this ticket came from, when a schedule made it. Provenance and a link
  -- back, NOT a relationship: two occurrences of one schedule have no edge
  -- between them. Deliberately no REFERENCES, so deleting the rule leaves the
  -- work and the record of its origin intact.
  schedule         TEXT,
  -- The calendar slot this ticket stands for, as a unix-ms instant.
  occurrence       INTEGER,
  -- When this ticket stops counting as LIVE work: the moment its schedule's next
  -- occurrence comes due, stamped once at creation from the cadence alone. Never
  -- read from a sibling ticket, which is what keeps occurrences independent.
  --
  -- Expiry changes no state. The ticket is not archived, cancelled or
  -- transitioned — it drops out of the ready queue and reads as not_fulfilled.
  -- Closing it out is a maintenance agent's job, through the ordinary API.
  expires_at       INTEGER
);
CREATE INDEX IF NOT EXISTS idx_tickets_project_state ON tickets(project, state);
CREATE INDEX IF NOT EXISTS idx_tickets_parent ON tickets(parent);
CREATE INDEX IF NOT EXISTS idx_tickets_claim ON tickets(claim_holder) WHERE claim_holder IS NOT NULL;

CREATE TABLE IF NOT EXISTS deps (
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  blocked_by TEXT NOT NULL REFERENCES tickets(id),
  PRIMARY KEY (ticket, blocked_by)
);
CREATE INDEX IF NOT EXISTS idx_deps_blocked_by ON deps(blocked_by);

-- "Ask a human" board. A question is an agent's request for a human decision
-- (confirm / choose / clarify / approve) tied to a ticket it parked in a
-- blocked state. `expertise` is a JSON array of routing tags (e.g.
-- ["domain:billing"]); `options` a JSON array for choose-kind; `answer` the
-- recorded human response (JSON) once resolved. Lifecycle in `status`:
-- open -> answered | withdrawn | expired. The append-only event log carries the
-- same transitions (question_asked / question_answered / ...); this table is the
-- queryable read-model the inbox and expiry sweep run against.
CREATE TABLE IF NOT EXISTS questions (
  id           TEXT PRIMARY KEY,
  project      TEXT NOT NULL REFERENCES projects(id),
  ticket       TEXT NOT NULL REFERENCES tickets(id),
  asked_by     TEXT NOT NULL,
  mode         TEXT NOT NULL DEFAULT 'blocking',
  kind         TEXT NOT NULL,
  title        TEXT NOT NULL,
  body         TEXT NOT NULL DEFAULT '',
  options      TEXT NOT NULL DEFAULT '[]',
  recommended  TEXT,
  expertise    TEXT NOT NULL DEFAULT '[]',
  -- The person this decision is waiting on (users.id), or NULL for the open pool.
  -- Orthogonal to `expertise`: that says what a qualified answerer must be, this
  -- says who was asked. Both may be set.
  assignee     TEXT REFERENCES users(id),
  urgency      TEXT NOT NULL DEFAULT 'normal',
  status       TEXT NOT NULL DEFAULT 'open',
  answer       TEXT,
  answered_by  TEXT,
  answered_at  INTEGER,
  resolved_to  TEXT,
  expires_at   INTEGER,
  on_timeout   TEXT,
  awaiting     TEXT NOT NULL DEFAULT 'human',
  confidence   INTEGER,
  recommended_note TEXT,
  summary      TEXT,
  option_notes TEXT NOT NULL DEFAULT '[]',
  multi        INTEGER NOT NULL DEFAULT 0,
  recommended_multi TEXT NOT NULL DEFAULT '[]',
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  version      INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS idx_questions_project ON questions(project);
CREATE INDEX IF NOT EXISTS idx_questions_ticket ON questions(ticket);

-- Project-scoped tag registry: named entities of some free-form `kind`
-- (person, component, team, …) that tickets reference by `kind:handle`. Generic
-- by design — a new kind needs no schema change — with per-kind attributes in
-- the free-form `meta` JSON object. Identity is (project, kind, handle); tagging
-- a ticket stores the canonical `kind:handle` string in tickets.tags.
CREATE TABLE IF NOT EXISTS tags (
  id         TEXT PRIMARY KEY,
  project    TEXT NOT NULL REFERENCES projects(id),
  kind       TEXT NOT NULL,
  handle     TEXT NOT NULL,
  label      TEXT NOT NULL DEFAULT '',
  meta       TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (project, kind, handle)
);
CREATE INDEX IF NOT EXISTS idx_tags_project_kind ON tags(project, kind);

-- The people directory: one row per human, global to the server. What makes
-- "this decision is waiting on Ada" expressible, where `expertise` could only say
-- "waiting on whoever holds expert:domain:billing".
--
-- Global, not project-scoped like `tags` above, for the same reason `tokens` is:
-- a person is not per-project, so every `answered_by`/`assignee` reference
-- resolves to the same human from anywhere. `user_projects` bounds where they can
-- be handed work.
--
-- `handle` is validated by the TAG handle rule (store::tags::handle_shape_ok), so
-- `person:<handle>` stays a legal tag reference and the convention that already
-- exists on tickets and initiatives converges on this row instead of forking from
-- it.
--
-- NOT a credential. Nothing here authenticates; scopes remain what a token may
-- do. See docs/users.md for the one place that boundary deliberately bends.
--
-- No DELETE, ever: `disabled_at` is a gate in the `projects.archived_at` idiom.
-- A disabled person cannot be newly assigned and cannot exercise assignee
-- authority, while every past record naming them still resolves — dropping the
-- row would make an answered question's `answered_by` unreadable, which is the
-- one thing an audit trail may not do.
CREATE TABLE IF NOT EXISTS users (
  id          TEXT PRIMARY KEY,
  handle      TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL DEFAULT '',
  email       TEXT,
  meta        TEXT NOT NULL DEFAULT '{}',
  disabled_at INTEGER,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- Who is assignable where. Directory scoping, NOT access control: a token's
-- `projects` allowlist is still the only thing that decides what a credential may
-- read or write. Because a named assignee may answer an `approve`, this is also a
-- second fence in front of that authority — you cannot hand a decision to
-- someone who was never put on the project.
CREATE TABLE IF NOT EXISTS user_projects (
  "user"     TEXT NOT NULL REFERENCES users(id),
  project    TEXT NOT NULL REFERENCES projects(id),
  added_by   TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY ("user", project)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_user_projects_project ON user_projects(project);

-- The follow-up thread on a question: a human can bounce a question back to the
-- asking agent for more research (role='human'), and the agent replies
-- (role='agent'), before the human answers. Append-only.
CREATE TABLE IF NOT EXISTS question_messages (
  id          TEXT PRIMARY KEY,
  question    TEXT NOT NULL REFERENCES questions(id),
  author      TEXT NOT NULL,
  role        TEXT NOT NULL,
  body        TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_question_messages_question ON question_messages(question);

CREATE TABLE IF NOT EXISTS comments (
  id         TEXT PRIMARY KEY,
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  author     TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_comments_ticket ON comments(ticket);

CREATE TABLE IF NOT EXISTS events (
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  ticket  TEXT,
  project TEXT,
  actor   TEXT NOT NULL,
  kind    TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_ticket ON events(ticket);
CREATE INDEX IF NOT EXISTS idx_events_project ON events(project);

CREATE TABLE IF NOT EXISTS tokens (
  id           TEXT PRIMARY KEY,
  hash         TEXT NOT NULL UNIQUE,
  actor        TEXT NOT NULL,
  scopes       TEXT NOT NULL,
  projects     TEXT NOT NULL DEFAULT '*',
  rate_limit   INTEGER NOT NULL DEFAULT 120,
  -- Which person holds this credential (users.id), NULL for a machine token.
  -- An authorization fact, not a label: it is the proof that satisfies an
  -- assignee-gated approval, so only an admin sets it, and only at mint.
  "user"       TEXT REFERENCES users(id),
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,
  revoked_at   INTEGER,
  last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS idempotency (
  actor      TEXT NOT NULL,
  key        TEXT NOT NULL,
  ticket     TEXT NOT NULL,
  body_hash  TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (actor, key)
);

CREATE TABLE IF NOT EXISTS comment_idempotency (
  actor      TEXT NOT NULL,
  key        TEXT NOT NULL,
  comment    TEXT NOT NULL,
  body_hash  TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (actor, key)
);

-- Shareable read-only web links. A share mints a bearer token (stored only as a
-- SHA-256 hash, exactly like normal tokens) that grants a scoped, read-only,
-- auto-expiring view of the board. `kind` is 'project' (all tickets in
-- `project`) or 'subtree' (the `ref` ticket plus its full recursive descendant
-- subtree). `ref` is the project id or the root ticket id; `project` is the
-- denormalized scope used to bound every query the share token can run.
CREATE TABLE IF NOT EXISTS shares (
  id          TEXT PRIMARY KEY,
  token_hash  TEXT NOT NULL UNIQUE,
  kind        TEXT NOT NULL,
  "ref"       TEXT NOT NULL,
  project     TEXT NOT NULL,
  expires_at  INTEGER NOT NULL,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  revoked_at  INTEGER,
  last_used_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_shares_project ON shares(project);
CREATE INDEX IF NOT EXISTS idx_shares_created_by ON shares(created_by);

-- Per-question answer grants. A grant mints a bearer token (`tka_`, hashed at
-- rest like every token) that authorizes exactly ONE write — answering the one
-- referenced question — and nothing else. It is the "answer link" handed to an
-- outside domain expert who should not hold a standing token: scoped to a single
-- question, auto-expiring, and write-once (spent once the question leaves the
-- open state). Validated on a distinct auth path (auth::answer_auth) that reaches
-- only /v1/answer/self*.
CREATE TABLE IF NOT EXISTS answer_grants (
  id          TEXT PRIMARY KEY,
  token_hash  TEXT NOT NULL UNIQUE,
  question    TEXT NOT NULL REFERENCES questions(id),
  project     TEXT NOT NULL,
  actor       TEXT NOT NULL,
  -- The directory person this link was minted FOR (users.id), or NULL for the
  -- outside-expert case where only the free-form `actor` names the answerer.
  -- Non-NULL is what lets the grant satisfy an assignee-gated approval by
  -- identity rather than by a synthesized expert scope.
  "user"      TEXT REFERENCES users(id),
  expires_at  INTEGER NOT NULL,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  used_at     INTEGER,
  revoked_at  INTEGER,
  last_used_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_answer_grants_question ON answer_grants(question);

-- Initiatives: a durable home for an idea that is not yet work. A product idea,
-- a direction, the residue of a good conversation — something nurtured over time
-- rather than claimed and closed. Deliberately NOT a ticket: there is no
-- workflow, no state machine, no claim, no lease and no ready queue here, because
-- none of those describe an idea being fed by several people and agents. `status`
-- is a plain lifecycle label (see INITIATIVE_STATUSES), not a workflow state.
--
-- The metadata a reader wants at a glance — how many entries, how many
-- characters, how many bytes, how many attachments — is deliberately NOT stored
-- here. It is derived from `initiative_entries` on read (see
-- `initiatives::load_rollup`), so it cannot drift from the entries it counts.
CREATE TABLE IF NOT EXISTS initiatives (
  id         TEXT PRIMARY KEY,
  project    TEXT NOT NULL REFERENCES projects(id),
  title      TEXT NOT NULL,
  summary    TEXT NOT NULL DEFAULT '',
  status     TEXT NOT NULL DEFAULT 'open',
  labels     TEXT NOT NULL DEFAULT '[]',
  -- Canonical `kind:handle` references into the project tag registry, exactly as
  -- on tickets — so `person:ada` on an initiative means the same thing, and the
  -- same registry answers who that is.
  tags       TEXT NOT NULL DEFAULT '[]',
  metadata   TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  version    INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_initiatives_project ON initiatives(project, status);

-- One contribution to an initiative: a note, a research finding, a colleague's
-- feedback, a conversation transcript, an uploaded document. Append-only — the
-- point is the accumulated record, so nothing here is edited in place.
--
-- Generic on purpose, in two directions. `kind` is a free-form slug, so a new
-- sort of input needs no schema change; and every entry can carry text, an
-- attachment, or both: `text` is the markdown a reader (and the UI) can always
-- show, `content` the raw bytes of a document when there is a file. `content` is
-- the ONLY blob in this schema and is never selected by the list or detail
-- queries — it is fetched by itself, by id.
--
-- Provenance is first-class rather than left to a free-form note: `source` says
-- where the input came from (an agent, a person, a conversation), `source_uri`
-- points at it, and `origin_at` records when the content was *created* as opposed
-- to `created_at`, when it landed here. A transcript pasted in a week later has
-- two different, both correct, timestamps.
--
-- The three size columns are computed once, on append, from what was actually
-- stored: `chars` counts characters of `text` (what a human means by "how long"),
-- `text_bytes` its UTF-8 length, `content_bytes` the attachment's length. Summing
-- them at read time is what makes the rollup free of drift.
CREATE TABLE IF NOT EXISTS initiative_entries (
  id            TEXT PRIMARY KEY,
  initiative    TEXT NOT NULL REFERENCES initiatives(id),
  -- Denormalized from the initiative so every scope check and project filter is
  -- one query, matching the precedent set by shares and promotions.
  project       TEXT NOT NULL,
  kind          TEXT NOT NULL,
  title         TEXT,
  text          TEXT NOT NULL DEFAULT '',
  content       BLOB,
  mime          TEXT,
  filename      TEXT,
  chars         INTEGER NOT NULL DEFAULT 0,
  text_bytes    INTEGER NOT NULL DEFAULT 0,
  content_bytes INTEGER NOT NULL DEFAULT 0,
  source        TEXT NOT NULL,
  source_uri    TEXT,
  origin_at     INTEGER,
  meta          TEXT NOT NULL DEFAULT '{}',
  author        TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_initiative_entries_initiative ON initiative_entries(initiative);

-- Mindmaps: the ten minutes BEFORE any of this is an idea, let alone work.
--
-- A tree you grow at conversation speed — six words a node, split one in two the
-- moment it turns out to be two thoughts — whose branches can graduate into epics
-- and initiatives afterwards. It is a brainstorming method and nothing more, so
-- there is no workflow, no claim, no lease, no ready queue, no assignment and no
-- attachments here.
--
-- What separates it from an initiative is the right to be thrown away: an
-- initiative is nurtured, a mindmap is scratch, and DELETE is an ordinary thing to
-- do to one (nodes cascade). What separates it from tickets is that nothing in it
-- is work until somebody says so.
--
-- `title` is the root. A map starts from one thing and everything hangs off it, so
-- the root is the map rather than a node inside it. `status` is a plain label —
-- the same three an initiative uses, where `distilled` means its branches have
-- graduated.
CREATE TABLE IF NOT EXISTS mindmaps (
  id         TEXT PRIMARY KEY,
  project    TEXT NOT NULL REFERENCES projects(id),
  title      TEXT NOT NULL,
  summary    TEXT NOT NULL DEFAULT '',
  status     TEXT NOT NULL DEFAULT 'open',
  metadata   TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  version    INTEGER NOT NULL DEFAULT 1,
  -- How many nodes the map's document holds, denormalised.
  --
  -- The nodes live in a CRDT, not in rows, so an honest count means replaying
  -- the document — affordable for one map, not for a list of two hundred. This
  -- column is written by whoever last had the replica open: the flusher every
  -- couple of seconds, and each API write immediately. So a list is at worst one
  -- flush behind, and the single-map read counts the live replica instead of
  -- reading this at all.
  nodes      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_mindmaps_project ON mindmaps(project, status);

-- One thought. Capped short on purpose (see MAX_NODE_TEXT): a sentence or two IS
-- the method, and a node that outgrows it has stopped being a brainstorm node and
-- wants to be an initiative.
-- What happened to a section of the plan, who did it, and when.
--
-- The plan is one thing the map and the document render two ways, and this is
-- its history. Not the CRDT update log — that is the mechanism which rebuilds
-- the text, is written per flush, and is REWRITTEN by compaction. This is the
-- record of acts somebody would name: a thought written, renamed, moved,
-- reviewed, a proposal accepted.
--
-- In SQL rather than in the document for three reasons: it references a real
-- person and wants a real foreign key; "everything Ada reviewed this week" is a
-- query and not a document walk; and it has to survive compaction.
--
-- SPARSE by construction. Keystrokes reach nothing, the same rule the event log
-- follows — an entry per keystroke-batch buries every entry worth reading. The
-- document view posts one when an edit settles, not while somebody types.
CREATE TABLE IF NOT EXISTS plan_trace (
  id       TEXT PRIMARY KEY,
  project  TEXT NOT NULL,
  mindmap  TEXT NOT NULL,
  -- The section. NULL for something that happened to the plan as a whole.
  node     TEXT,
  kind     TEXT NOT NULL,
  -- The free-form actor string the credential carried.
  actor    TEXT NOT NULL,
  -- WHICH PERSON, when the credential was bound to one (`users.id`). This is
  -- the column an audit reads: two `human:alice` tokens are indistinguishable,
  -- and an actor string does not survive somebody leaving. Kept for an agent's
  -- entry too — an agent token can belong to somebody's own automation, and
  -- whose agent is worth knowing.
  "user"   TEXT REFERENCES users(id),
  note     TEXT,
  -- What the section SAID at that moment, as plain text.
  --
  -- This is what makes a diff possible at all. The CRDT log cannot answer it:
  -- compaction rewrites it into one blob, so "what did 2.1 say last Tuesday" has
  -- no answer there by design. Keeping the text on the acts that changed it is
  -- cheap precisely because the trace is sparse — an entry per keystroke would
  -- make this ruinous, which is the same reason the trace is sparse anyway.
  --
  -- Null for an act that did not change the prose, and for anything past the
  -- cap: a diff is worth having, an unbounded copy of every revision is not.
  text     TEXT,
  at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_plan_trace_map ON plan_trace(mindmap, at);
CREATE INDEX IF NOT EXISTS idx_plan_trace_node ON plan_trace(node, at) WHERE node IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_plan_trace_user ON plan_trace("user", at) WHERE "user" IS NOT NULL;

CREATE TABLE IF NOT EXISTS mindmap_nodes (
  id            TEXT PRIMARY KEY,
  mindmap       TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,
  -- NULL = a first-ring branch off the root. The same edge shape as
  -- `tickets.parent`, which is this store's precedent for a tree.
  parent        TEXT REFERENCES mindmap_nodes(id),
  text          TEXT NOT NULL,
  -- Order among siblings, gapped (1000, 2000, …) so inserting between two nodes
  -- is one write rather than a renumber of the whole ring.
  position      INTEGER NOT NULL,
  -- Hand placement; NULL = wherever the layout puts it. Nullable on purpose: a map
  -- nobody has dragged stays tidy as it grows at typing speed, and one that has
  -- been arranged stays exactly where it was left.
  x             REAL,
  y             REAL,
  -- What this branch became once it graduated: ('epic', 'tp-a1d8') or
  -- ('initiative', 'ini-9f3k'). Deliberately no REFERENCES — the same reason
  -- `tickets.schedule` carries none: deleting the map must leave the work, and
  -- deleting the work must leave the record of where it came from.
  --
  -- This pair is what lets a map stay useful after the brainstorm, as a picture of
  -- what the thinking turned into. Promotion never moves a node: the map is the
  -- record of how you got there.
  promoted_kind TEXT,
  promoted_id   TEXT,
  created_by    TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_mindmap_nodes_map ON mindmap_nodes(mindmap, position);
CREATE INDEX IF NOT EXISTS idx_mindmap_nodes_parent ON mindmap_nodes(parent);

-- Promotions: an append-only record that a ticket's work reached some named
-- target/stage — "staging", "production", "published", "delivered", whatever
-- the team uses. Deliberately free-form (`target` is any string) so takomo is
-- not tied to software deployment. History is kept; the latest per ticket drives
-- the board badge.
CREATE TABLE IF NOT EXISTS promotions (
  id          TEXT PRIMARY KEY,
  ticket      TEXT NOT NULL REFERENCES tickets(id),
  project     TEXT NOT NULL,
  target      TEXT NOT NULL,
  url         TEXT,
  ref         TEXT,
  note        TEXT,
  actor       TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_promotions_ticket ON promotions(ticket);
CREATE INDEX IF NOT EXISTS idx_promotions_project ON promotions(project);

-- A recurrence rule that materializes ordinary tickets. See
-- spec/schedule-format.md and src/store/schedules.rs.
--
-- `cadence` is the rule as JSON (every/interval/on/day/at/tz), validated by
-- src/schedule.rs on the way in; `template` is a ticket create minus the
-- project. Both are stored as text because they are authored documents, not
-- query targets: nothing filters on the inside of either.
CREATE TABLE IF NOT EXISTS schedules (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL REFERENCES projects(id),
  name        TEXT NOT NULL,
  cadence     TEXT NOT NULL,
  template    TEXT NOT NULL,
  -- pending | active | paused | rejected | retired. `pending` is what an
  -- agent-proposed schedule lands in when the project requires approval.
  status      TEXT NOT NULL DEFAULT 'active',
  proposed_by TEXT,
  rationale   TEXT,
  -- The next slot to fire, NULL unless status = 'active'. That invariant is what
  -- makes a pending or paused schedule inert BY CONSTRUCTION rather than by a
  -- check somewhere: the partial index below cannot see it, so the sweep cannot
  -- either.
  next_slot   INTEGER,
  -- The interval anchor and the earliest slot. Anchoring `interval` here rather
  -- than on calendar parity is what keeps "every 2 weeks" landing on the same
  -- weeks a year later.
  starts_at   INTEGER NOT NULL,
  ends_at     INTEGER,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  version     INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_schedules_project ON schedules(project, status);
CREATE INDEX IF NOT EXISTS idx_schedules_due ON schedules(next_slot) WHERE next_slot IS NOT NULL;

-- OAuth 2.1 authorization server (src/store/oauth.rs). Present in every database
-- but inert unless the operator sets a public base URL: with none, the OAuth
-- routes are not mounted at all, so nothing ever writes here.
--
-- Clients register themselves (RFC 7591) and are always PUBLIC clients — hence no
-- client_secret column. `redirect_uris` is a JSON array, matched literally: the
-- exact-match check against it is the only thing standing between this endpoint
-- and an open redirect, so it is stored in a form that cannot be split by a comma
-- inside a URI. Registration being unauthenticated by specification, a row that
-- never produced a code or a refresh token is swept on age — that, not the rate
-- limit, is what bounds this table.
CREATE TABLE IF NOT EXISTS oauth_clients (
  client_id     TEXT PRIMARY KEY,
  client_name   TEXT NOT NULL DEFAULT '',
  redirect_uris TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);

-- Authorization codes: single-use, ~60s, PKCE-bound. Columns actor..granted_by
-- are the consent snapshot (see model::GrantedAccess) — the slice of authority
-- the human handed this client, frozen so a later change to their own token can
-- neither widen it nor resurrect it. `issued_family` records which refresh-token
-- family the code produced, so a replay can revoke everything it bought — which is
-- why a spent row outlives its 60s expiry by an hour instead of being swept at once.
CREATE TABLE IF NOT EXISTS oauth_codes (
  code_hash      TEXT PRIMARY KEY,
  client_id      TEXT NOT NULL REFERENCES oauth_clients(client_id),
  redirect_uri   TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  resource       TEXT,
  actor          TEXT NOT NULL,
  scopes         TEXT NOT NULL,
  projects       TEXT NOT NULL DEFAULT '*',
  rate_limit     INTEGER NOT NULL,
  -- The person who consented, carried so the issued token is the same human and
  -- not an anonymous copy of their scopes. Inherited, never granted.
  "user"         TEXT,
  scope          TEXT NOT NULL,
  granted_by     TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  expires_at     INTEGER NOT NULL,
  used_at        INTEGER,
  issued_family  TEXT
);

-- Refresh tokens, hashed at rest like every credential here. Rotated on every
-- use: the presented row gets `rotated_at` and a successor is inserted with the
-- same `family`. Presenting a row that already has `rotated_at` or `revoked_at`
-- is reuse, and revokes the whole family.
CREATE TABLE IF NOT EXISTS oauth_refresh (
  token_hash TEXT PRIMARY KEY,
  family     TEXT NOT NULL,
  client_id  TEXT NOT NULL,
  actor      TEXT NOT NULL,
  scopes     TEXT NOT NULL,
  projects   TEXT NOT NULL DEFAULT '*',
  rate_limit INTEGER NOT NULL,
  -- Same consent snapshot as oauth_codes: the person survives every rotation, so
  -- a connection approved by Ada keeps answering as Ada a month later.
  "user"     TEXT,
  scope      TEXT NOT NULL,
  granted_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  rotated_at INTEGER,
  revoked_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_refresh_family ON oauth_refresh(family);

-- Ledger of access tokens this authorization server minted. An OAuth access
-- token is an ordinary `tokens` row (that is the point — one auth path, not two),
-- which leaves no way to tell it apart from one an operator minted by hand. This
-- table is that way: it is what lets the sweeper delete only OAuth-issued tokens,
-- and what lets a detected replay find and revoke the tokens a compromised
-- credential already produced.
CREATE TABLE IF NOT EXISTS oauth_issued (
  token_id     TEXT PRIMARY KEY,
  client_id    TEXT NOT NULL,
  family       TEXT NOT NULL,
  refresh_hash TEXT NOT NULL,
  created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_oauth_issued_family ON oauth_issued(family);
CREATE INDEX IF NOT EXISTS idx_oauth_issued_refresh ON oauth_issued(refresh_hash);

-- Checklist. A release is an ordered marker in a project's history, pushed by the
-- agent that merged the work; `seq` is monotonic per project so a release-count
-- expiry policy ("retest every 5 releases") is arithmetic rather than a date
-- comparison. `ref` is the tag or full sha, unique per project so pushing the same
-- release twice is a conflict rather than a silent duplicate.
CREATE TABLE IF NOT EXISTS releases (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  ref TEXT NOT NULL,
  seq INTEGER NOT NULL,
  note TEXT,
  pushed_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(project, ref),
  UNIQUE(project, seq)
);
CREATE INDEX IF NOT EXISTS idx_releases_project_seq ON releases(project, seq);

-- The paths the release's diff touched. Supplied by the pusher (it has the tree
-- checked out; the server does not clone anything) and intersected against check
-- globs to decide what went stale.
CREATE TABLE IF NOT EXISTS release_paths (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  PRIMARY KEY (release, path)
) WITHOUT ROWID;

-- Check globs that matched NO file in this release's tree. An orphaned glob is the
-- feature's worst failure mode — it reads as "still covered" while covering
-- nothing — so it is recorded per release and excluded from coverage rather than
-- counted.
CREATE TABLE IF NOT EXISTS release_orphan_globs (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  glob TEXT NOT NULL,
  PRIMARY KEY (release, glob)
) WITHOUT ROWID;

-- Inherited checklist policy. `epic = ''` is the project-level default; a row with
-- an epic ticket id overrides it for that epic's checks. Empty string rather than
-- NULL because SQLite treats NULLs as distinct in a UNIQUE index, which would
-- allow two project-level defaults.
CREATE TABLE IF NOT EXISTS checklist_policies (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  epic TEXT NOT NULL DEFAULT '',
  verification TEXT,
  expiry_days INTEGER,
  expiry_releases INTEGER,
  updated_at INTEGER NOT NULL,
  UNIQUE(project, epic)
);

-- A check is one action with one entry precondition at one layer. `body` is
-- free-form prose an agent or a human can follow — there is deliberately no step
-- model and no dependency graph, because the precondition is a statement about
-- data state, which is what keeps checks independently runnable.
CREATE TABLE IF NOT EXISTS checks (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  epic TEXT,
  -- The initiative whose conversation agreed this check should exist.
  --
  -- A DIRECT reference rather than one derived through the epic's
  -- `initiative:<id>` tag, because the moment a characterisation test gets
  -- agreed is a conversation about the feature, which is usually BEFORE any
  -- epic exists to hang it from. Deriving it would make the link unstateable
  -- exactly when it is being made.
  --
  -- No REFERENCES clause, matching `epic` directly above: validity is enforced
  -- in Rust so a wrong id gets a teaching 422 instead of an opaque FOREIGN KEY
  -- failure, and a dangling reference stays readable rather than blocking the
  -- row.
  initiative TEXT,
  -- The mindmap node this check verifies.
  --
  -- The same shape as `epic` and `initiative` above, and for the same reason: a
  -- check is agreed while somebody is looking at a part of the plan, and the
  -- plan's parts are nodes. Without this the tests view can say what a check is
  -- FOR only in prose, and "which parts of this plan are actually verified" has
  -- no answer the software can give.
  --
  -- No REFERENCES clause, matching the two above: validity is checked in Rust so
  -- a wrong id is a teaching 422 rather than an opaque FOREIGN KEY failure, and
  -- a node deleted from a brainstorm leaves the check readable rather than
  -- taking it with it. Deleting a map is ordinary; losing its verification
  -- record is not.
  node TEXT,
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  precondition TEXT NOT NULL DEFAULT '',
  layer TEXT NOT NULL DEFAULT 'api',
  severity TEXT NOT NULL DEFAULT 'advisory',
  verification TEXT,
  expiry_days INTEGER,
  expiry_releases INTEGER,
  cost_agent_minutes INTEGER,
  cost_human_minutes INTEGER,
  metadata TEXT NOT NULL DEFAULT 'null',
  version INTEGER NOT NULL DEFAULT 1,
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  archived_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_checks_project ON checks(project);
CREATE INDEX IF NOT EXISTS idx_checks_epic ON checks(epic) WHERE epic IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_checks_node ON checks(node) WHERE node IS NOT NULL;
-- The index on `initiative` is created in `migrate()`, NOT here. This batch runs
-- before the ALTER that adds the column to a pre-rename database, so indexing it
-- here fails on exactly the databases the migration exists for.

-- Which paths of the application under test a check claims to exercise. Declared by
-- hand and known to rot; `release_orphan_globs` is how the rot becomes visible.
CREATE TABLE IF NOT EXISTS check_globs (
  check_id TEXT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  glob TEXT NOT NULL,
  PRIMARY KEY (check_id, glob)
) WITHOUT ROWID;

-- One executable case: a check crossed with one parameter assignment. `key` is a
-- stable identity derived from that assignment, so regenerating a model after
-- adding a parameter matches surviving cases and keeps their history instead of
-- orphaning it. A case dropped by regeneration is `retired_at`-stamped, never
-- deleted, so its verdicts remain auditable.
CREATE TABLE IF NOT EXISTS cases (
  id TEXT PRIMARY KEY,
  check_id TEXT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  key TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '',
  assignment TEXT NOT NULL DEFAULT '{}',
  seeded INTEGER NOT NULL DEFAULT 0,
  agent_verdict TEXT,
  agent_at INTEGER,
  agent_by TEXT,
  agent_release TEXT,
  human_verdict TEXT,
  human_at INTEGER,
  human_by TEXT,
  -- WHICH PERSON approved it (users.id), where `human_by` is only the free-form
  -- actor string the credential carried. "A person approved this case" is the
  -- strongest claim this table makes, and an unresolvable name is a poor way to
  -- make it: two `human:alice` tokens are indistinguishable, and nothing survives
  -- somebody leaving. Nullable, because a verdict from a credential bound to
  -- nobody is still a verdict.
  --
  -- A mirror of the last human verdict, like the columns around it;
  -- `case_verdicts.user` is the permanent per-verdict record.
  human_user TEXT REFERENCES users(id),
  human_release TEXT,
  stale_since TEXT,
  retired_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(check_id, key)
);
CREATE INDEX IF NOT EXISTS idx_cases_check ON cases(check_id);
CREATE INDEX IF NOT EXISTS idx_cases_live ON cases(check_id) WHERE retired_at IS NULL;

-- Append-only verdict history. The `cases` row carries the LAST agent verdict and
-- the LAST human verdict as separate columns because they are separate facts — a
-- case can be agent-verified and human-approved, and a policy may require both —
-- while this table keeps every verdict ever recorded.
CREATE TABLE IF NOT EXISTS case_verdicts (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
  actor_kind TEXT NOT NULL,
  actor TEXT NOT NULL,
  -- The person behind the credential that recorded this verdict (users.id), or
  -- NULL for a machine token. THIS is the permanent record of who: the mirrors on
  -- `cases` and `case_environments` only hold the latest, while this table is
  -- append-only and is what an audit reads.
  --
  -- Kept for an agent verdict too, not just a human one. An agent token can
  -- belong to somebody's own automation, and "whose agent" is worth knowing.
  "user" TEXT REFERENCES users(id),
  verdict TEXT NOT NULL,
  note TEXT,
  release TEXT,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_case_verdicts_case ON case_verdicts(case_id);


-- Editable checks/cases are definitions. Runs pin immutable snapshots; observations
-- never overwrite the definition or a previous attempt.
CREATE TABLE IF NOT EXISTS test_definition_revisions (
  id TEXT PRIMARY KEY,
  check_id TEXT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  snapshot TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_test_revisions_check ON test_definition_revisions(check_id, created_at);
CREATE TABLE IF NOT EXISTS test_specification_revisions (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  snapshot TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS test_runs (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL DEFAULT 'execution' CHECK(kind IN ('execution','legacy')),
  status TEXT NOT NULL CHECK(status IN ('queued','running','completed','cancelled')),
  environment TEXT,
  environment_snapshot TEXT,
  code_ref TEXT,
  retry_of TEXT REFERENCES test_runs(id) ON DELETE SET NULL,
  created_by TEXT NOT NULL,
  executor TEXT,
  created_at INTEGER NOT NULL,
  started_at INTEGER,
  finished_at INTEGER,
  idempotency_key TEXT,
  request_hash TEXT,
  UNIQUE(project, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_test_runs_project ON test_runs(project, created_at DESC, id);
CREATE TABLE IF NOT EXISTS test_run_cases (
  run_id TEXT NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
  case_id TEXT NOT NULL,
  check_id TEXT NOT NULL,
  definition_revision TEXT REFERENCES test_definition_revisions(id),
  specification_revision TEXT REFERENCES test_specification_revisions(id),
  case_snapshot TEXT,
  PRIMARY KEY(run_id, case_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_test_run_cases_check ON test_run_cases(check_id, run_id);
CREATE TABLE IF NOT EXISTS test_run_results (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  actor_kind TEXT NOT NULL CHECK(actor_kind IN ('agent','human')),
  actor TEXT NOT NULL,
  user_id TEXT,
  verdict TEXT NOT NULL CHECK(verdict IN ('pass','fail','blocked','unreachable')),
  note TEXT,
  evidence TEXT NOT NULL DEFAULT '[]',
  recorded_at INTEGER NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  legacy_verdict TEXT UNIQUE,
  FOREIGN KEY(run_id, case_id) REFERENCES test_run_cases(run_id, case_id) ON DELETE CASCADE,
  UNIQUE(run_id, idempotency_key),
  UNIQUE(run_id, case_id, actor_kind)
);

-- Which environments a check must be verified in.
--
-- EMPTY is a legitimate steady state, not a gap: a check can be genuinely
-- environment-agnostic, and every check filed before this existed is one. A
-- check that declares nothing keeps using the verdict columns on `cases`; a
-- check that declares anything uses `case_environments` instead, and nothing
-- writes both.
CREATE TABLE IF NOT EXISTS check_environments (
  check_id    TEXT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  environment TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, environment)
) WITHOUT ROWID;

-- How one case stands in ONE environment.
--
-- The nine columns are lifted verbatim from `cases`, because they are the same
-- nine facts asked in a narrower scope. A row exists only once something has
-- been recorded: a pair nobody has run has no row and reads `never`, which is
-- both correct and free. Creating them eagerly would fan `file_cases` out to
-- cases x environments inserts — a 5,000-case check across four environments is
-- 20,000 rows in one transaction holding the write mutex every claim waits on.
CREATE TABLE IF NOT EXISTS case_environments (
  case_id       TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
  environment   TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
  agent_verdict TEXT,
  agent_at      INTEGER,
  agent_by      TEXT,
  agent_release TEXT,
  human_verdict TEXT,
  human_at      INTEGER,
  human_by      TEXT,
  -- Which person approved it HERE. The same mirror `cases.human_user` is, per
  -- place the check must pass: an approval in staging and one in production are
  -- separate claims, and so is who made each.
  human_user    TEXT REFERENCES users(id),
  human_release TEXT,
  stale_since   TEXT,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (case_id, environment)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_case_environments_env ON case_environments(environment);

-- Where a check can actually be run: a named, project-scoped environment.
--
-- A verdict is only as good as the thing it was taken against, and until now
-- Takomo had no way to say what that thing was. An agent handed "re-verify this"
-- had to be told the URL out of band, which is exactly the kind of context that
-- goes stale silently.
--
-- Takomo stores; the agent computes — the same rule the rest of Checklist runs
-- on. Nothing here is executed, polled or health-checked: `bring_up` and
-- `teardown` are prose an agent reads, not a command the server runs, which is
-- why they are free text rather than a structured spec. Structure would be a
-- promise the store cannot keep.
--
-- `credentials_hint` is a POINTER and never a secret — an env-var name, a vault
-- path, a runbook URL. The name is chosen to refuse on sight, because any token
-- with `read` can see this column.
CREATE TABLE IF NOT EXISTS environments (
  id               TEXT PRIMARY KEY,
  project          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  -- The handle an agent types. Not patchable: checks and tool calls carry it,
  -- and a silent rename would break every one of them.
  slug             TEXT NOT NULL,
  name             TEXT NOT NULL,
  -- An enum, not free text, or one project grows prod/production/Prod. The kind
  -- is also what makes an unverified flow on a scratch box a different finding
  -- from the same flow on production.
  kind             TEXT NOT NULL DEFAULT 'other',
  base_url         TEXT,
  bring_up         TEXT NOT NULL DEFAULT '',
  -- The half nobody writes down, and the half agents get wrong: who releases the
  -- lease when the run is over.
  teardown         TEXT NOT NULL DEFAULT '',
  data_state       TEXT NOT NULL DEFAULT 'unknown',
  -- ADVISORY. Takomo executes nothing and cannot enforce this; it is what an
  -- agent reads before running a destructive case, not a guarantee.
  writable         INTEGER NOT NULL DEFAULT 1,
  credentials_hint TEXT,
  notes            TEXT NOT NULL DEFAULT '',
  metadata         TEXT NOT NULL DEFAULT 'null',
  version          INTEGER NOT NULL DEFAULT 1,
  created_by       TEXT NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  -- Archive, never delete: a decommissioned box is still the evidence for every
  -- verdict ever taken there.
  archived_at      INTEGER,
  UNIQUE(project, slug)
);
CREATE INDEX IF NOT EXISTS idx_environments_project ON environments(project);

-- Collaborative documents: prose humans and agents edit AT THE SAME TIME.
--
-- Note what is not here: a `body`. The prose lives in a Yjs CRDT whose update log
-- is `doc_updates` below, because a text column means last-write-wins, and
-- last-write-wins is the exact failure this surface exists to remove — an agent
-- that spends nine seconds thinking must not overwrite what was typed during
-- them.
--
-- `version` therefore counts METADATA edits only. CRDT updates arrive by the
-- thousand and would make an `If-Match` precondition meaningless.
CREATE TABLE IF NOT EXISTS documents (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title       TEXT NOT NULL,
  -- Folder, `/`-separated. The same "a folder exists because a document names
  -- it" convention `/initiatives` derives its tree from, which needs no folder
  -- table and has no orphaned-directory problem.
  path        TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'draft',
  -- The initiative this was distilled from, if any. No REFERENCES clause,
  -- matching `checks.initiative`: validity is enforced in Rust so a wrong id
  -- gets a teaching 422 instead of an opaque FOREIGN KEY failure, and a dangling
  -- reference stays readable rather than blocking the row.
  initiative  TEXT,
  metadata    TEXT NOT NULL DEFAULT 'null',
  version     INTEGER NOT NULL DEFAULT 1,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  archived_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project);

-- The CRDT update log: one row per flush, replayed in `seq` order to rebuild a
-- document.
--
-- **There is no separate snapshot table, and that is a property of Yjs rather
-- than a shortcut.** A Yjs document's entire state serializes as a single,
-- ordinary update, so compacting a log is `DELETE` the rows and `INSERT` the
-- merged blob — the same shape and the same format as an increment. A second
-- table would only be a second thing to keep consistent.
--
-- Rows are appended by a DEBOUNCED flush, never per keystroke. Every write in
-- this store goes through one `IMMEDIATE` transaction behind a process-wide
-- mutex, and that serialization *is* the exactly-one-claimant guarantee for the
-- ready queue — so a per-keystroke insert would put every claim, transition and
-- heartbeat behind someone's typing. Live collaboration is served from memory;
-- see `src/store/docs.rs`.
CREATE TABLE IF NOT EXISTS crdt_updates (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  -- 'document' or 'mindmap'. Carried as a column even though the id prefix
  -- already says it, because a log nobody can read without knowing the id
  -- scheme is a log that outlives its reader.
  object_kind TEXT NOT NULL,
  -- No REFERENCES clause, and it cannot have one: this points at whichever of
  -- two tables the kind names. Validity is enforced in Rust — the same call
  -- `checks.epic` and `documents.initiative` already make, for the same reason
  -- (a teaching 422 rather than an opaque FOREIGN KEY failure). The cascade the
  -- FK used to provide is `Store::purge_collab`, called from each kind's own
  -- delete path.
  object_id   TEXT NOT NULL,
  blob        BLOB NOT NULL,
  bytes       INTEGER NOT NULL,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_crdt_updates_object ON crdt_updates(object_id, seq);

-- Immutable specification history; deliberately separate from the compactable live log.
CREATE TABLE IF NOT EXISTS specification_history_heads (
  mindmap TEXT PRIMARY KEY REFERENCES mindmaps(id) ON DELETE CASCADE,
  insertions BLOB NOT NULL,
  deletions BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS specification_versions (
  mindmap TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,
  version INTEGER NOT NULL,
  blob BLOB NOT NULL,
  state BLOB,
  kind TEXT NOT NULL,
  recorded_at INTEGER NOT NULL,
  recorded_by TEXT,
  PRIMARY KEY(mindmap,version)
);
CREATE TABLE IF NOT EXISTS specification_checkpoints (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  mindmap TEXT NOT NULL,
  version INTEGER NOT NULL,
  name TEXT NOT NULL,
  actor TEXT NOT NULL,
  user TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(mindmap,name),
  FOREIGN KEY(mindmap,version) REFERENCES specification_versions(mindmap,version) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_specification_checkpoints_version ON specification_checkpoints(mindmap,version);

-- A short-lived credential for ONE document's sync session.
--
-- The fifth credential shape in this store, and the reason is narrow: a browser
-- `WebSocket` cannot set an `Authorization` header (the same limitation that
-- keeps `/board` polling `/v1/events` rather than using SSE), so the credential
-- has to ride the handshake. A real `tk_` token in a query string would put the
-- org's actual credential in every access log; a ticket that expires and reaches
-- exactly one document bounds what such a leak is worth.
--
-- It authenticates and nothing more. `can_write` is copied from the minting
-- token's scopes at mint time, so a `read`-only reader joins as a read-only peer.
CREATE TABLE IF NOT EXISTS crdt_sessions (
  id          TEXT PRIMARY KEY,
  token_hash  TEXT NOT NULL UNIQUE,
  object_kind TEXT NOT NULL,
  object_id   TEXT NOT NULL,
  project     TEXT NOT NULL,
  actor       TEXT NOT NULL,
  -- The directory person this session belongs to, when the minting token was
  -- bound to one. It is what collaborators see next to a caret.
  "user"      TEXT REFERENCES users(id),
  display     TEXT NOT NULL,
  can_write   INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  revoked_at  INTEGER,
  -- The `tk_` token this session was derived from.
  --
  -- Without it, revoking a leaked token could not reach the sessions minted
  -- from it: a `tkd_` ticket kept opening a socket and writing for its whole
  -- life, which made it MORE permissive than the token it came from — the one
  -- thing it is not allowed to be.
  -- ON DELETE CASCADE, and both halves matter. Without any action the OAuth
  -- sweep's `DELETE FROM tokens` hits this constraint and rolls back its WHOLE
  -- transaction, so expired tokens, spent codes and retired refresh rows all
  -- stop being reaped for as long as one session references a swept token —
  -- which is reachable, because token retention (24h) is shorter than a
  -- session's life (12h TTL plus its sweep grace). Cascading is also the right
  -- semantic: the credential this was derived from no longer exists, so neither
  -- should this.
  minted_by   TEXT REFERENCES tokens(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_crdt_sessions_object ON crdt_sessions(object_id);
CREATE INDEX IF NOT EXISTS idx_crdt_sessions_minted_by ON crdt_sessions(minted_by);
CREATE TRIGGER IF NOT EXISTS cleanup_check_crdt AFTER DELETE ON checks
BEGIN
 DELETE FROM crdt_updates WHERE object_id=OLD.id;
 DELETE FROM crdt_sessions WHERE object_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS cleanup_project_crdt_sessions AFTER DELETE ON projects
BEGIN
 DELETE FROM crdt_sessions WHERE project=OLD.id;
END;


-- Named workflows that can be applied to any project.
--
-- A workflow has always been a COLUMN on `projects`, which makes each project's
-- state machine private to it: two projects that want the same lifecycle each
-- carry their own copy, and improving one improves neither the other nor the
-- next project created. This table is the shared shelf they can be taken from.
--
-- It is a SOURCE of documents, not a second way to apply one. Applying still
-- goes through `PUT /v1/projects/{p}/workflow`, so the check that refuses to
-- strand tickets has exactly one code path — a library that could write a
-- project's workflow directly would be a second door into the one operation
-- here that can break a live project.
--
-- `layout_json` holds node positions for the editor. It is deliberately OUTSIDE
-- `workflow_json`: `Workflow` is `deny_unknown_fields`, so a `positions` key
-- inside the document would 422 on the way back in.
CREATE TABLE IF NOT EXISTS workflow_library (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  description   TEXT,
  workflow_json TEXT NOT NULL,
  layout_json   TEXT,
  -- 1 for the workflows this binary ships. They are reseeded on every open, so
  -- editing or deleting one would be undone silently at the next restart; the
  -- API refuses both instead.
  builtin       INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  created_by    TEXT NOT NULL,
  updated_at    INTEGER NOT NULL
);
"#;

#[cfg(test)]
mod read_connection_tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(tmp.path().join("read.db")).expect("open store");
        (tmp, store)
    }

    /// The read path must be genuinely read-only. A write that strays onto it
    /// has to fail loudly — a silent second writer would invalidate the
    /// single-writer assumption every claim rests on.
    #[test]
    fn read_connection_refuses_writes() {
        let (_tmp, store) = temp_store();
        // The raw SQLite error is inspected inside the closure: `ApiError::from`
        // deliberately flattens every database error to one generic message, so
        // an API-level assertion could not tell "refused the write" from any
        // other failure.
        let sqlite_error = store
            .with_conn(|conn| {
                Ok(conn
                    .execute(
                        "INSERT INTO projects (id, name, workflow_json, created_at) VALUES ('x', 'x', '{}', 0)",
                        [],
                    )
                    .expect_err("a write through with_conn must be refused")
                    .to_string()
                    .to_lowercase())
            })
            .expect("the closure itself runs");
        assert!(
            sqlite_error.contains("readonly") || sqlite_error.contains("read-only"),
            "expected a read-only refusal, got: {sqlite_error}"
        );
        // And nothing landed.
        let rows: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
                    .map_err(ApiError::from)
            })
            .expect("read");
        assert_eq!(rows, 0, "the refused write must not have been applied");
    }

    /// A reader is a separate connection, so read-your-writes is worth pinning:
    /// each `with_conn` opens a fresh snapshot and must see everything committed
    /// by `with_tx` before it.
    #[test]
    fn read_connection_sees_committed_writes() {
        let (_tmp, store) = temp_store();
        store
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO projects (id, name, workflow_json, created_at) VALUES ('p1', 'P', '{}', 0)",
                    [],
                )?;
                Ok(())
            })
            .expect("write");
        let seen: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM projects WHERE id = 'p1'", [], |r| {
                    r.get(0)
                })
                .map_err(ApiError::from)
            })
            .expect("read");
        assert_eq!(seen, 1, "a reader must see writes the writer committed");
    }

    /// An in-memory path gets no readers (a second connection to `:memory:` is a
    /// different database), and `with_conn` still works by falling back to the
    /// writer.
    #[test]
    fn in_memory_store_falls_back_to_the_writer() {
        let store = Store::open(":memory:").expect("open in-memory store");
        assert!(
            store.readers.is_empty(),
            "no readers for a private database"
        );
        let n: i64 = store
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
                    .map_err(ApiError::from)
            })
            .expect("read via the writer");
        assert_eq!(n, 0);
    }
}
