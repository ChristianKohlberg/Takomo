//! Repository layer. All SQL lives under this module; handlers never touch the
//! database directly. SQLite (WAL) is the only backend in v0; the surface is
//! kept narrow and connection-agnostic so a Postgres implementation could be
//! added behind the same methods later.

mod answer_grants;
mod claims;
mod events;
mod helpers;
mod metrics;
mod model;
mod projects;
mod questions;
mod roadmap;
mod shares;
mod tags;
mod tickets;
mod tokens;
mod transition;

pub use answer_grants::{DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS};
pub use claims::{ReadyFilter, DEFAULT_TTL_SECONDS, MAX_TTL_SECONDS};
pub use events::EventFilter;
pub use model::*;
pub use projects::{normalize_style_guide, Conventions, DeletedCounts, MAX_STYLE_GUIDE_CHARS};
pub use questions::{
    question_quality_hints, AskRequest, QuestionFilter, ReviseOptionsRequest, TimeoutAction,
    MAX_QUESTIONS_PAGE, QUESTION_KINDS,
};
pub use shares::{ShareKind, DEFAULT_SHARE_TTL_SECONDS, MAX_SHARE_TTL_SECONDS};
pub use tags::{normalize_tag_ref, validate_tag_kind, TagCreate, TagListFilter, TagPatch};
pub use tickets::{
    merge_patch, ArchivedFilter, DepDirection, TicketCreate, TicketListFilter, TicketPatch,
};

use crate::error::{ApiError, ApiResult};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
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

        // A second connection to `:memory:` (or to a private temp database) is a
        // *different, empty* database, not a second view of this one. Nothing in
        // this repo opens the store that way, but rather than leave the trap
        // armed for whoever does, detect it and run without readers: `with_conn`
        // then falls back to the writer, which is exactly the old behavior.
        let readers = if is_private_db(path) {
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
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id                TEXT PRIMARY KEY,
  name              TEXT NOT NULL,
  workflow_json     TEXT NOT NULL,
  question_language TEXT,
  style_guide       TEXT,
  created_at        INTEGER NOT NULL
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
  fence_seq        INTEGER NOT NULL DEFAULT 0,
  version          INTEGER NOT NULL DEFAULT 1,
  created_by       TEXT NOT NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  archived_at      TEXT
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
