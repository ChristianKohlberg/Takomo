//! Repository layer. All SQL lives under this module; handlers never touch the
//! database directly. SQLite (WAL) is the only backend in v0; the surface is
//! kept narrow and connection-agnostic so a Postgres implementation could be
//! added behind the same methods later.

mod answer_grants;
mod checklist;
mod claims;
mod events;
mod helpers;
mod initiatives;
mod metrics;
mod model;
mod moves;
mod oauth;
mod projects;
mod questions;
mod roadmap;
mod schedules;
mod shares;
mod tags;
mod tickets;
mod tokens;
mod transition;
mod workflows;

pub use answer_grants::{DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS};
pub use checklist::{
    glob_matches, CaseFileOutcome, CaseInput, LaneCreate, LaneFilter, LanePatch, PolicyInput,
    ReleasePush, WorkItem, MAX_CASES_PAGE, MAX_CASES_PER_FILE, MAX_LANES_PAGE, MAX_LANE_GLOBS,
    MAX_RELEASE_PATHS,
};
pub use claims::{ForcedRelease, ReadyFilter, DEFAULT_TTL_SECONDS, MAX_TTL_SECONDS};
pub use events::EventFilter;
pub use initiatives::{
    EntryCreate, InitiativeCreate, InitiativeListFilter, InitiativePatch, INITIATIVE_STATUSES,
    MAX_ENTRIES_PAGE, MAX_ENTRY_CONTENT_BYTES, MAX_INITIATIVES_PAGE, MAX_INITIATIVE_BYTES,
    MAX_INITIATIVE_ENTRIES,
};
pub use model::*;
pub use moves::{MoveOutcome, MoveRequest, MovedTicket, MAX_MOVE_TICKETS};
pub use oauth::{
    ACCESS_TOKEN_TTL_SECONDS, AUTH_CODE_TTL_SECONDS, MAX_REDIRECT_URIS, REFRESH_TOKEN_TTL_SECONDS,
    SPENT_CODE_RETENTION_SECONDS, UNUSED_CLIENT_RETENTION_SECONDS,
};
pub use projects::{
    normalize_answer_link_ttl, normalize_claim_ttls, normalize_style_guide, Conventions,
    DeletedCounts, MAX_STYLE_GUIDE_CHARS,
};
pub use questions::{
    question_quality_hints, AnswerOutcome, AskRequest, QuestionFilter, ResumeBlocked,
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
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
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

/// Idempotent, additive, non-destructive startup migrations. Runs after the
/// `CREATE TABLE IF NOT EXISTS` schema on every open. It only ever ADDs missing
/// columns/indexes on a database that predates them — it never drops, rewrites,
/// or recreates existing data, so it is safe to run against a populated live DB
/// on every boot.
fn migrate(conn: &Connection) -> ApiResult<()> {
    // archived_at (nullable) separates archived tickets from active ones. Older
    // databases predate the column; add it only when PRAGMA table_info shows it
    // absent. `CREATE TABLE IF NOT EXISTS` above already carries it for a fresh
    // DB, so on those this ALTER is skipped.
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
    // Created after the columns are guaranteed to exist. This index IS the
    // exactly-once guarantee, so it is added on an old database too.
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_tickets_occurrence ON tickets(schedule, occurrence) WHERE schedule IS NOT NULL",
        [],
    )?;
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
  claim_expires_at INTEGER,
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
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,
  revoked_at   INTEGER,
  last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS idempotency (
  actor      TEXT NOT NULL,
  key        TEXT NOT NULL,
  ticket     TEXT NOT NULL,
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
  revoked_at  INTEGER
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
  expires_at  INTEGER NOT NULL,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  used_at     INTEGER,
  revoked_at  INTEGER
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
-- checked out; the server does not clone anything) and intersected against lane
-- globs to decide what went stale.
CREATE TABLE IF NOT EXISTS release_paths (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  PRIMARY KEY (release, path)
) WITHOUT ROWID;

-- Lane globs that matched NO file in this release's tree. An orphaned glob is the
-- feature's worst failure mode — it reads as "still covered" while covering
-- nothing — so it is recorded per release and excluded from coverage rather than
-- counted.
CREATE TABLE IF NOT EXISTS release_orphan_globs (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  glob TEXT NOT NULL,
  PRIMARY KEY (release, glob)
) WITHOUT ROWID;

-- Inherited checklist policy. `epic = ''` is the project-level default; a row with
-- an epic ticket id overrides it for that epic's lanes. Empty string rather than
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

-- A lane is one action with one entry precondition at one layer. `body` is
-- free-form prose an agent or a human can follow — there is deliberately no step
-- model and no dependency graph, because the precondition is a statement about
-- data state, which is what keeps lanes independently runnable.
CREATE TABLE IF NOT EXISTS lanes (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  epic TEXT,
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
CREATE INDEX IF NOT EXISTS idx_lanes_project ON lanes(project);
CREATE INDEX IF NOT EXISTS idx_lanes_epic ON lanes(epic) WHERE epic IS NOT NULL;

-- Which paths of the application under test a lane claims to exercise. Declared by
-- hand and known to rot; `release_orphan_globs` is how the rot becomes visible.
CREATE TABLE IF NOT EXISTS lane_globs (
  lane TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
  glob TEXT NOT NULL,
  PRIMARY KEY (lane, glob)
) WITHOUT ROWID;

-- One executable case: a lane crossed with one parameter assignment. `key` is a
-- stable identity derived from that assignment, so regenerating a model after
-- adding a parameter matches surviving cases and keeps their history instead of
-- orphaning it. A case dropped by regeneration is `retired_at`-stamped, never
-- deleted, so its verdicts remain auditable.
CREATE TABLE IF NOT EXISTS cases (
  id TEXT PRIMARY KEY,
  lane TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
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
  human_release TEXT,
  stale_since TEXT,
  retired_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(lane, key)
);
CREATE INDEX IF NOT EXISTS idx_cases_lane ON cases(lane);
CREATE INDEX IF NOT EXISTS idx_cases_live ON cases(lane) WHERE retired_at IS NULL;

-- Append-only verdict history. The `cases` row carries the LAST agent verdict and
-- the LAST human verdict as separate columns because they are separate facts — a
-- case can be agent-verified and human-approved, and a policy may require both —
-- while this table keeps every verdict ever recorded.
CREATE TABLE IF NOT EXISTS case_verdicts (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
  actor_kind TEXT NOT NULL,
  actor TEXT NOT NULL,
  verdict TEXT NOT NULL,
  note TEXT,
  release TEXT,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_case_verdicts_case ON case_verdicts(case_id);

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
