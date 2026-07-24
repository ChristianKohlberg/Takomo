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
mod tickets;
mod tokens;
mod transition;

pub use answer_grants::{DEFAULT_ANSWER_TTL_SECONDS, MAX_ANSWER_TTL_SECONDS};
pub use claims::{ReadyFilter, DEFAULT_TTL_SECONDS, MAX_TTL_SECONDS};
pub use events::EventFilter;
pub use model::*;
pub use projects::DeletedCounts;
pub use questions::{AskRequest, QuestionFilter, TimeoutAction, QUESTION_KINDS};
pub use shares::{ShareKind, DEFAULT_SHARE_TTL_SECONDS, MAX_SHARE_TTL_SECONDS};
pub use tickets::{
    merge_patch, ArchivedFilter, DepDirection, TicketCreate, TicketListFilter, TicketPatch,
};

use crate::error::{ApiError, ApiResult};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the database at `path` and initialize schema.
    pub fn open(path: impl AsRef<Path>) -> ApiResult<Store> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| ApiError::internal(format!("cannot open database: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// Run `f` inside a single IMMEDIATE transaction. SQLite's single-writer
    /// model plus this process-wide mutex is the claim-serialization
    /// guarantee: every mutating operation is one atomic step.
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

    /// Run `f` with a plain connection (reads).
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> ApiResult<T>) -> ApiResult<T> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        f(&conn)
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
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id                TEXT PRIMARY KEY,
  name              TEXT NOT NULL,
  workflow_json     TEXT NOT NULL,
  question_language TEXT,
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
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  version      INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_questions_status ON questions(status);
CREATE INDEX IF NOT EXISTS idx_questions_project ON questions(project);
CREATE INDEX IF NOT EXISTS idx_questions_ticket ON questions(ticket);

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
"#;
