-- Postgres translation of the core of SCHEMA in src/store/mod.rs.
--
-- SCOPE: the five tables the ready queue and the claim path touch. The other 26
-- tables are mechanical once the rules below are settled, and are deliberately
-- not here yet — the point of this file is to pin the *decisions*, not to grind
-- out DDL before they are made.
--
-- Translation rules established here, and why:
--
--  1. `rowid` -> an explicit `seq BIGINT GENERATED ALWAYS AS IDENTITY`.
--     SQLite's implicit rowid is used across the store as a monotonic insertion
--     counter (ordering tiebreaks, keyset pagination, MAX(rowid) "newest row").
--     Postgres has nothing implicit that means this, so it becomes a real column.
--     Note it is NOT the primary key: ids stay TEXT, exactly as today.
--
--  2. INTEGER-as-boolean -> BOOLEAN. SQLite has no bool, so workflow_states
--     carries `claimable INTEGER NOT NULL DEFAULT 0` and every predicate reads
--     `ws.claimable = 1`. Postgres has a real BOOLEAN and the predicate becomes
--     `ws.claimable`. This is the one translation that changes query text in a
--     way a reader will notice.
--
--  3. INTEGER timestamps -> BIGINT. They are unix seconds (and unix-ms for
--     `occurrence`); SQLite's INTEGER is 64-bit, Postgres's INTEGER is 32-bit, so
--     a literal translation would silently truncate in 2038.
--
--  4. TEXT-holding-JSON -> JSONB. `labels`, `tags`, `metadata`, `links` are JSON
--     in TEXT columns because that is all SQLite offers. JSONB makes the label
--     filter an indexable containment query instead of a scan over json_each.
--
--  5. No `IF NOT EXISTS` idempotent-schema trick. SQLite re-runs SCHEMA on every
--     open and relies on IF NOT EXISTS to make that a no-op; a Postgres port
--     should use a real migration tool with versioned, ordered files. This being
--     0001_ is that decision.

CREATE TABLE projects (
  id                      TEXT PRIMARY KEY,
  name                    TEXT NOT NULL,
  workflow_json           TEXT NOT NULL,
  question_language       TEXT,
  style_guide             TEXT,
  answer_link_ttl_seconds BIGINT,
  claim_ttl_seconds       BIGINT,
  max_claim_ttl_seconds   BIGINT,
  created_at              BIGINT NOT NULL
);

-- Denormalized view of each project's workflow states so queue/blocking queries
-- can join on claimable/terminal without parsing JSON.
CREATE TABLE workflow_states (
  project   TEXT    NOT NULL,
  state     TEXT    NOT NULL,
  category  TEXT    NOT NULL,
  claimable BOOLEAN NOT NULL DEFAULT FALSE,   -- rule 2
  terminal  BOOLEAN NOT NULL DEFAULT FALSE,   -- rule 2
  PRIMARY KEY (project, state)
);

CREATE TABLE tickets (
  id                  TEXT PRIMARY KEY,
  -- rule 1: the insertion counter that replaces rowid.
  seq                 BIGINT GENERATED ALWAYS AS IDENTITY,
  project             TEXT NOT NULL REFERENCES projects(id),
  type                TEXT NOT NULL DEFAULT 'task',
  parent              TEXT REFERENCES tickets(id),
  title               TEXT NOT NULL,
  body                TEXT NOT NULL DEFAULT '',
  state               TEXT NOT NULL,
  priority            TEXT NOT NULL DEFAULT 'normal',
  labels              JSONB NOT NULL DEFAULT '[]'::jsonb,   -- rule 4
  tags                JSONB NOT NULL DEFAULT '[]'::jsonb,   -- rule 4
  metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,   -- rule 4
  links               JSONB NOT NULL DEFAULT '{}'::jsonb,   -- rule 4
  claim_holder        TEXT,
  claim_expires_at    BIGINT,
  lapsed_claim_holder TEXT,
  fence_seq           BIGINT NOT NULL DEFAULT 0,
  version             BIGINT NOT NULL DEFAULT 1,
  created_by          TEXT NOT NULL,
  created_at          BIGINT NOT NULL,
  updated_at          BIGINT NOT NULL,
  archived_at         TEXT,
  schedule            TEXT,
  occurrence          BIGINT,
  expires_at          BIGINT
);
CREATE UNIQUE INDEX idx_tickets_seq ON tickets(seq);
CREATE INDEX idx_tickets_project_state ON tickets(project, state);
CREATE INDEX idx_tickets_parent ON tickets(parent);
CREATE INDEX idx_tickets_claim ON tickets(claim_holder) WHERE claim_holder IS NOT NULL;
-- rule 4 pays off here: containment on a GIN index instead of a scan.
CREATE INDEX idx_tickets_labels ON tickets USING GIN (labels);
CREATE INDEX idx_tickets_tags ON tickets USING GIN (tags);

CREATE TABLE deps (
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  blocked_by TEXT NOT NULL REFERENCES tickets(id),
  PRIMARY KEY (ticket, blocked_by)
);
CREATE INDEX idx_deps_blocked_by ON deps(blocked_by);

CREATE TABLE events (
  id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project    TEXT NOT NULL,
  ticket     TEXT,
  kind       TEXT NOT NULL,
  actor      TEXT,
  data       JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at BIGINT NOT NULL
);
CREATE INDEX idx_events_project ON events(project, id);
