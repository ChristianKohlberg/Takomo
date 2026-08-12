-- Postgres translation of SCHEMA in src/store/mod.rs. All 29 tables.
--
-- Verified: loads into Postgres 16 under ON_ERROR_STOP, and the resulting
-- pg_tables set is identical to the CREATE TABLE set in src/store/mod.rs.
--
-- GOVERNING RULE: translate a type only where Postgres FORCES it.
--
-- The temptation in a port like this is to "improve" the schema on the way
-- across — JSONB for the JSON columns, BOOLEAN for the 0/1 integers, timestamptz
-- for the epoch seconds. Every one of those is a silent behaviour change in a
-- codebase that reads these columns back into concrete Rust types, and a port
-- whose failures are silent is worse than no port. Improvements are a later,
-- separate, individually-justified change. What follows is as close to a literal
-- transcription as the two dialects allow.
--
-- The six forced changes, each with the reason it is not optional:
--
--  1. `rowid` -> an explicit `seq BIGINT GENERATED ALWAYS AS IDENTITY`.
--     SQLite's implicit rowid is used across the store as a monotonic insertion
--     counter: ORDER BY tiebreaks (the ready queue, the roadmap, verdict
--     history), keyset pagination (`WHERE t.rowid < ?`), and MAX(rowid) to mean
--     "newest row" (promotions). Postgres has nothing implicit that means this.
--     Only `tickets` gets one here, because only `tickets` is paginated by it;
--     `promotions` and `case_verdicts` need one too and are marked TODO below
--     rather than guessed at. NOT the primary key — ids stay TEXT.
--
--  2. INTEGER -> BIGINT. These columns hold unix seconds (unix MILLIseconds for
--     `occurrence` and `next_slot`). SQLite's INTEGER is 64-bit; Postgres's is
--     32-bit, so a literal transcription would overflow in 2038.
--
--  3. BLOB -> BYTEA. `initiative_entries.content`, the only blob in the schema.
--
--  4. INTEGER PRIMARY KEY AUTOINCREMENT -> GENERATED ALWAYS AS IDENTITY.
--     `events.seq` only. This is the event-log cursor every long-poller and the
--     board's `?since=` reads, so it must stay gap-tolerant-but-monotonic, which
--     IDENTITY is.
--
--  5. WITHOUT ROWID -> dropped (3 tables). It is a SQLite storage-layout hint
--     for PK-only tables with no semantic content. Nothing to express.
--
--  6. `CREATE TABLE IF NOT EXISTS` -> plain CREATE. SQLite re-runs SCHEMA on
--     every open and leans on IF NOT EXISTS to make that a no-op. A Postgres
--     port wants versioned ordered migrations; this file being 0001_ is that
--     decision.
--
-- DELIBERATELY NOT CHANGED, with reasons, because each looks like an oversight:
--
--  * JSON columns stay TEXT. JSONB is not round-trip preserving — it reorders
--    object keys and strips whitespace. `schedules.cadence` and
--    `schedules.template` are authored documents whose comment in the source
--    says they are text precisely because nothing queries inside them;
--    `projects.workflow_json` is read back and parsed; `tokens.scopes` is not
--    even JSON, it is a CSV string split in Rust (`src/store/tokens.rs:15`).
--    The five columns that ARE queried inside — tickets.labels, tickets.tags,
--    questions.expertise, initiatives.labels, initiatives.tags — are handled at
--    the QUERY layer instead:
--        json_each(t.labels)  ->  jsonb_array_elements_text(t.labels::jsonb)
--    which works on a TEXT column and needs no schema change. Where that is hot
--    enough to matter, an expression index gets it back:
--        CREATE INDEX ... USING GIN ((labels::jsonb) jsonb_path_ops);
--
--  * 0/1 integers stay integers. `workflow_states.claimable`/`terminal`,
--    `questions.multi`, `cases.seeded`. BOOLEAN is the "right" type, but
--    `cases.seeded` is read as `row.get::<i64>` (src/store/checklist.rs:536) and
--    the predicates read `= 1`. Converting means touching both, for no
--    behavioural gain, in a change that is already large.
--
--  * Timestamps stay integers, not timestamptz. Same reason: they are read as
--    i64 everywhere and compared against `now` computed in Rust.
--
--  * `tickets.archived_at` is TEXT while `lanes.archived_at` is INTEGER. That
--    inconsistency is in the SQLite schema and is preserved. Fixing it is a
--    behaviour change and belongs in its own commit, on main, not smuggled into
--    a port.

CREATE TABLE projects (
  id                      TEXT PRIMARY KEY,
  name                    TEXT NOT NULL,
  workflow_json           TEXT NOT NULL,
  question_language       TEXT,
  style_guide             TEXT,
  answer_link_ttl_seconds BIGINT,
  claim_ttl_seconds       BIGINT,
  max_claim_ttl_seconds   BIGINT,
  -- Added by migrate() on SQLite rather than by SCHEMA, so it is easy to miss
  -- when translating: the real SQLite schema is SCHEMA *plus* every additive
  -- migration. Whether an agent may propose schedules for this project.
  schedule_approval       BIGINT NOT NULL DEFAULT 0,
  created_at              BIGINT NOT NULL
);

CREATE TABLE workflow_states (
  project   TEXT   NOT NULL,
  state     TEXT   NOT NULL,
  category  TEXT   NOT NULL,
  claimable BIGINT NOT NULL DEFAULT 0,
  terminal  BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (project, state)
);

CREATE TABLE tickets (
  -- Constraint NAME is load-bearing: schedules::is_primary_key_conflict matches
  -- the substring "tickets.id", and Postgres reports the constraint name where
  -- SQLite reports table.column. Default would be `tickets_pkey`.
  id                  TEXT CONSTRAINT "tickets.id" PRIMARY KEY,
  seq                 BIGINT GENERATED ALWAYS AS IDENTITY,   -- rule 1 (was rowid)
  project             TEXT NOT NULL REFERENCES projects(id),
  type                TEXT NOT NULL DEFAULT 'task',
  parent              TEXT REFERENCES tickets(id),
  title               TEXT NOT NULL,
  body                TEXT NOT NULL DEFAULT '',
  state               TEXT NOT NULL,
  priority            TEXT NOT NULL DEFAULT 'normal',
  labels              TEXT NOT NULL DEFAULT '[]',
  tags                TEXT NOT NULL DEFAULT '[]',
  metadata            TEXT NOT NULL DEFAULT '{}',
  links               TEXT NOT NULL DEFAULT '{}',
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

CREATE TABLE deps (
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  blocked_by TEXT NOT NULL REFERENCES tickets(id),
  PRIMARY KEY (ticket, blocked_by)
);
CREATE INDEX idx_deps_blocked_by ON deps(blocked_by);

CREATE TABLE questions (
  id                TEXT PRIMARY KEY,
  project           TEXT NOT NULL REFERENCES projects(id),
  ticket            TEXT NOT NULL REFERENCES tickets(id),
  asked_by          TEXT NOT NULL,
  mode              TEXT NOT NULL DEFAULT 'blocking',
  kind              TEXT NOT NULL,
  title             TEXT NOT NULL,
  body              TEXT NOT NULL DEFAULT '',
  options           TEXT NOT NULL DEFAULT '[]',
  recommended       TEXT,
  expertise         TEXT NOT NULL DEFAULT '[]',
  urgency           TEXT NOT NULL DEFAULT 'normal',
  status            TEXT NOT NULL DEFAULT 'open',
  answer            TEXT,
  answered_by       TEXT,
  answered_at       BIGINT,
  resolved_to       TEXT,
  expires_at        BIGINT,
  on_timeout        TEXT,
  awaiting          TEXT NOT NULL DEFAULT 'human',
  confidence        BIGINT,
  recommended_note  TEXT,
  summary           TEXT,
  option_notes      TEXT NOT NULL DEFAULT '[]',
  multi             BIGINT NOT NULL DEFAULT 0,
  recommended_multi TEXT NOT NULL DEFAULT '[]',
  created_at        BIGINT NOT NULL,
  updated_at        BIGINT NOT NULL,
  version           BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX idx_questions_status ON questions(status);
CREATE INDEX idx_questions_project ON questions(project);
CREATE INDEX idx_questions_ticket ON questions(ticket);

CREATE TABLE tags (
  id         TEXT PRIMARY KEY,
  project    TEXT NOT NULL REFERENCES projects(id),
  kind       TEXT NOT NULL,
  handle     TEXT NOT NULL,
  label      TEXT NOT NULL DEFAULT '',
  meta       TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL,
  UNIQUE (project, kind, handle)
);
CREATE INDEX idx_tags_project_kind ON tags(project, kind);

CREATE TABLE question_messages (
  id         TEXT PRIMARY KEY,
  question   TEXT NOT NULL REFERENCES questions(id),
  author     TEXT NOT NULL,
  role       TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at BIGINT NOT NULL
);
CREATE INDEX idx_question_messages_question ON question_messages(question);

CREATE TABLE comments (
  id         TEXT PRIMARY KEY,
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  author     TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at BIGINT NOT NULL
);
CREATE INDEX idx_comments_ticket ON comments(ticket);

-- rule 4: the event-log cursor.
CREATE TABLE events (
  seq     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  ticket  TEXT,
  project TEXT,
  actor   TEXT NOT NULL,
  kind    TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  at      BIGINT NOT NULL
);
CREATE INDEX idx_events_ticket ON events(ticket);
CREATE INDEX idx_events_project ON events(project);

CREATE TABLE tokens (
  id           TEXT PRIMARY KEY,
  hash         TEXT NOT NULL UNIQUE,
  actor        TEXT NOT NULL,
  scopes       TEXT NOT NULL,
  projects     TEXT NOT NULL DEFAULT '*',
  rate_limit   BIGINT NOT NULL DEFAULT 120,
  created_at   BIGINT NOT NULL,
  expires_at   BIGINT,
  revoked_at   BIGINT,
  last_used_at BIGINT
);

CREATE TABLE idempotency (
  actor      TEXT NOT NULL,
  key        TEXT NOT NULL,
  ticket     TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  PRIMARY KEY (actor, key)
);

CREATE TABLE shares (
  id         TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  kind       TEXT NOT NULL,
  "ref"      TEXT NOT NULL,
  project    TEXT NOT NULL,
  expires_at BIGINT NOT NULL,
  created_by TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  revoked_at BIGINT
);
CREATE INDEX idx_shares_project ON shares(project);
CREATE INDEX idx_shares_created_by ON shares(created_by);

CREATE TABLE answer_grants (
  id         TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  question   TEXT NOT NULL REFERENCES questions(id),
  project    TEXT NOT NULL,
  actor      TEXT NOT NULL,
  expires_at BIGINT NOT NULL,
  created_by TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  used_at    BIGINT,
  revoked_at BIGINT
);
CREATE INDEX idx_answer_grants_question ON answer_grants(question);

-- rule 1: paginated by rowid (src/store/initiatives.rs:604,651).
CREATE TABLE initiatives (
  id         TEXT PRIMARY KEY,
  seq        BIGINT GENERATED ALWAYS AS IDENTITY,
  project    TEXT NOT NULL REFERENCES projects(id),
  title      TEXT NOT NULL,
  summary    TEXT NOT NULL DEFAULT '',
  status     TEXT NOT NULL DEFAULT 'open',
  labels     TEXT NOT NULL DEFAULT '[]',
  tags       TEXT NOT NULL DEFAULT '[]',
  metadata   TEXT NOT NULL DEFAULT '{}',
  created_by TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL,
  version    BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX idx_initiatives_project ON initiatives(project, status);

-- rule 1: paginated by rowid (src/store/initiatives.rs:881,888).
CREATE TABLE initiative_entries (
  id            TEXT PRIMARY KEY,
  seq           BIGINT GENERATED ALWAYS AS IDENTITY,
  initiative    TEXT NOT NULL REFERENCES initiatives(id),
  project       TEXT NOT NULL,
  kind          TEXT NOT NULL,
  title         TEXT,
  text          TEXT NOT NULL DEFAULT '',
  content       BYTEA,                      -- rule 3 (was BLOB)
  mime          TEXT,
  filename      TEXT,
  chars         BIGINT NOT NULL DEFAULT 0,
  text_bytes    BIGINT NOT NULL DEFAULT 0,
  content_bytes BIGINT NOT NULL DEFAULT 0,
  source        TEXT NOT NULL,
  source_uri    TEXT,
  origin_at     BIGINT,
  meta          TEXT NOT NULL DEFAULT '{}',
  author        TEXT NOT NULL,
  created_at    BIGINT NOT NULL
);
CREATE INDEX idx_initiative_entries_initiative ON initiative_entries(initiative);

-- rule 1: read with MAX(rowid) to pick the newest promotion per ticket
-- (src/store/tickets.rs:1248-1255) and ordered by rowid DESC.
CREATE TABLE promotions (
  id         TEXT PRIMARY KEY,
  seq        BIGINT GENERATED ALWAYS AS IDENTITY,
  ticket     TEXT NOT NULL REFERENCES tickets(id),
  project    TEXT NOT NULL,
  target     TEXT NOT NULL,
  url        TEXT,
  ref        TEXT,
  note       TEXT,
  actor      TEXT NOT NULL,
  created_at BIGINT NOT NULL
);
CREATE INDEX idx_promotions_ticket ON promotions(ticket);
CREATE INDEX idx_promotions_project ON promotions(project);

CREATE TABLE schedules (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL REFERENCES projects(id),
  name        TEXT NOT NULL,
  cadence     TEXT NOT NULL,
  template    TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'active',
  proposed_by TEXT,
  rationale   TEXT,
  next_slot   BIGINT,
  starts_at   BIGINT NOT NULL,
  ends_at     BIGINT,
  created_by  TEXT NOT NULL,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  version     BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX idx_schedules_project ON schedules(project, status);
-- The partial index that makes a pending/paused schedule inert by construction.
CREATE INDEX idx_schedules_due ON schedules(next_slot) WHERE next_slot IS NOT NULL;

CREATE TABLE oauth_clients (
  client_id     TEXT PRIMARY KEY,
  client_name   TEXT NOT NULL DEFAULT '',
  redirect_uris TEXT NOT NULL,
  created_at    BIGINT NOT NULL
);

CREATE TABLE oauth_codes (
  code_hash      TEXT PRIMARY KEY,
  client_id      TEXT NOT NULL REFERENCES oauth_clients(client_id),
  redirect_uri   TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  resource       TEXT,
  actor          TEXT NOT NULL,
  scopes         TEXT NOT NULL,
  projects       TEXT NOT NULL DEFAULT '*',
  rate_limit     BIGINT NOT NULL,
  scope          TEXT NOT NULL,
  granted_by     TEXT NOT NULL,
  created_at     BIGINT NOT NULL,
  expires_at     BIGINT NOT NULL,
  used_at        BIGINT,
  issued_family  TEXT
);

CREATE TABLE oauth_refresh (
  token_hash TEXT PRIMARY KEY,
  family     TEXT NOT NULL,
  client_id  TEXT NOT NULL,
  actor      TEXT NOT NULL,
  scopes     TEXT NOT NULL,
  projects   TEXT NOT NULL DEFAULT '*',
  rate_limit BIGINT NOT NULL,
  scope      TEXT NOT NULL,
  granted_by TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  expires_at BIGINT NOT NULL,
  rotated_at BIGINT,
  revoked_at BIGINT
);
CREATE INDEX idx_oauth_refresh_family ON oauth_refresh(family);

CREATE TABLE oauth_issued (
  token_id     TEXT PRIMARY KEY,
  client_id    TEXT NOT NULL,
  family       TEXT NOT NULL,
  refresh_hash TEXT NOT NULL,
  created_at   BIGINT NOT NULL
);
CREATE INDEX idx_oauth_issued_family ON oauth_issued(family);
CREATE INDEX idx_oauth_issued_refresh ON oauth_issued(refresh_hash);

-- NOTE: `releases.seq` is NOT rule 1. It is an explicit, application-assigned
-- per-project counter that predates this port, and it is what makes a
-- "retest every 5 releases" policy arithmetic. Unrelated to rowid.
CREATE TABLE releases (
  id         TEXT PRIMARY KEY,
  project    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  ref        TEXT NOT NULL,
  seq        BIGINT NOT NULL,
  note       TEXT,
  pushed_by  TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  UNIQUE(project, ref),
  UNIQUE(project, seq)
);
CREATE INDEX idx_releases_project_seq ON releases(project, seq);

-- rule 5: WITHOUT ROWID dropped here and on the next two.
CREATE TABLE release_paths (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  path    TEXT NOT NULL,
  PRIMARY KEY (release, path)
);

CREATE TABLE release_orphan_globs (
  release TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  glob    TEXT NOT NULL,
  PRIMARY KEY (release, glob)
);

-- `epic = ''` is the project-level default. The source comment explains the
-- empty string rather than NULL: SQLite treats NULLs as distinct in a UNIQUE
-- index, which would allow two project-level defaults. Postgres behaves the
-- SAME way here, so the workaround is still required — this is one of the few
-- places the two dialects agree on something non-obvious.
CREATE TABLE checklist_policies (
  id              TEXT PRIMARY KEY,
  project         TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  epic            TEXT NOT NULL DEFAULT '',
  verification    TEXT,
  expiry_days     BIGINT,
  expiry_releases BIGINT,
  updated_at      BIGINT NOT NULL,
  UNIQUE(project, epic)
);

CREATE TABLE lanes (
  id                 TEXT PRIMARY KEY,
  project            TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  epic               TEXT,
  title              TEXT NOT NULL,
  body               TEXT NOT NULL DEFAULT '',
  precondition       TEXT NOT NULL DEFAULT '',
  layer              TEXT NOT NULL DEFAULT 'api',
  severity           TEXT NOT NULL DEFAULT 'advisory',
  verification       TEXT,
  expiry_days        BIGINT,
  expiry_releases    BIGINT,
  cost_agent_minutes BIGINT,
  cost_human_minutes BIGINT,
  metadata           TEXT NOT NULL DEFAULT 'null',
  version            BIGINT NOT NULL DEFAULT 1,
  created_by         TEXT NOT NULL,
  created_at         BIGINT NOT NULL,
  updated_at         BIGINT NOT NULL,
  archived_at        BIGINT
);
CREATE INDEX idx_lanes_project ON lanes(project);
CREATE INDEX idx_lanes_epic ON lanes(epic) WHERE epic IS NOT NULL;

CREATE TABLE lane_globs (
  lane TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
  glob TEXT NOT NULL,
  PRIMARY KEY (lane, glob)
);

CREATE TABLE cases (
  id            TEXT PRIMARY KEY,
  lane          TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
  key           TEXT NOT NULL,
  label         TEXT NOT NULL DEFAULT '',
  assignment    TEXT NOT NULL DEFAULT '{}',
  seeded        BIGINT NOT NULL DEFAULT 0,
  agent_verdict TEXT,
  agent_at      BIGINT,
  agent_by      TEXT,
  agent_release TEXT,
  human_verdict TEXT,
  human_at      BIGINT,
  human_by      TEXT,
  human_release TEXT,
  stale_since   TEXT,
  retired_at    BIGINT,
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL,
  UNIQUE(lane, key)
);
CREATE INDEX idx_cases_lane ON cases(lane);
CREATE INDEX idx_cases_live ON cases(lane) WHERE retired_at IS NULL;

-- rule 1: ordered by rowid to mean insertion order — this is the table commit
-- 1c58e1c fixed ("order verdict history by insertion, not by a random id"), so
-- an ordering that is merely plausible here is the bug returning.
CREATE TABLE case_verdicts (
  id         TEXT PRIMARY KEY,
  seq        BIGINT GENERATED ALWAYS AS IDENTITY,
  case_id    TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
  actor_kind TEXT NOT NULL,
  actor      TEXT NOT NULL,
  verdict    TEXT NOT NULL,
  note       TEXT,
  release    TEXT,
  at         BIGINT NOT NULL
);
CREATE INDEX idx_case_verdicts_case ON case_verdicts(case_id);

-- ---------------------------------------------------------------------------
-- GLOB, which Postgres does not have.
--
-- The store matches checklist coverage paths with SQLite's GLOB operator, and
-- `store::sql` rewrites `A GLOB B` into `glob(A, B)`. LIKE is not a substitute:
-- glob is anchored, `*` spans separators, `?` is exactly one character, and
-- `[abc]` is a class. Translating to a POSIX regex preserves all four.
-- ---------------------------------------------------------------------------
CREATE FUNCTION glob(subject TEXT, pattern TEXT) RETURNS BOOLEAN AS $$
DECLARE
  re   TEXT := '^';
  i    INT  := 1;
  ch   TEXT;
BEGIN
  IF subject IS NULL OR pattern IS NULL THEN RETURN NULL; END IF;
  WHILE i <= length(pattern) LOOP
    ch := substr(pattern, i, 1);
    IF ch = '*' THEN
      re := re || '.*';
    ELSIF ch = '?' THEN
      re := re || '.';
    ELSIF ch = '[' THEN
      -- Character class: copy through to the closing bracket verbatim.
      DECLARE j INT := i + 1; BEGIN
        WHILE j <= length(pattern) AND substr(pattern, j, 1) <> ']' LOOP
          j := j + 1;
        END LOOP;
        re := re || substr(pattern, i, j - i + 1);
        i := j;
      END;
    ELSE
      -- Everything else is literal; escape any regex metacharacter.
      -- Membership test, not a regex: PostgreSQL's advanced regex DOES treat
      -- backslash as special inside brackets, so a class ending in \] is
      -- unterminated. strpos sidesteps the question. chr(92) likewise, because
      -- with standard_conforming_strings on (the default) '\\' is TWO
      -- backslashes and would escape the wrong thing.
      IF strpos('.^$+(){}|]' || chr(92), ch) > 0 THEN
        re := re || chr(92) || ch;
      ELSE
        re := re || ch;
      END IF;
    END IF;
    i := i + 1;
  END LOOP;
  RETURN subject ~ (re || '$');
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- The unique index behind the schedules exactly-once guarantee. Its NAME is
-- load-bearing: `store::sql::Error::is_constraint_on` matches the substring
-- "tickets.occurrence", and Postgres reports the index name where SQLite
-- reports table.column. Renaming this silently turns "one ticket per slot" into
-- a 500. See src/store/schedules.rs::is_occurrence_conflict.
CREATE UNIQUE INDEX "tickets.occurrence"
  ON tickets(schedule, occurrence)
  WHERE schedule IS NOT NULL AND occurrence IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Two SQLite built-ins Postgres lacks, supplied as functions so the store's SQL
-- needs no rewriting. Defining them here rather than translating at the query
-- layer keeps one body of SQL that still reads as the original.
-- ---------------------------------------------------------------------------

-- SQLite has no boolean: `SUM(x = 'done')` sums 1s and 0s. Postgres returns a
-- real boolean from the comparison and has no sum() that accepts one.
CREATE FUNCTION bool_sum_add(state BIGINT, value BOOLEAN) RETURNS BIGINT AS $$
  SELECT state + CASE WHEN value THEN 1 ELSE 0 END;
$$ LANGUAGE sql IMMUTABLE;

CREATE AGGREGATE sum(BOOLEAN) (
  SFUNC = bool_sum_add,
  STYPE = BIGINT,
  INITCOND = '0'
);

-- SQLite's json_extract(doc, '$.path'). The path syntax is already valid
-- jsonpath, so this is a direct forward; `#>> '{}'` unwraps the jsonb to text
-- the way SQLite returns a scalar rather than a quoted JSON string.
CREATE FUNCTION json_extract(doc TEXT, path TEXT) RETURNS TEXT AS $$
  SELECT jsonb_path_query_first(doc::jsonb, path::jsonpath) #>> '{}';
$$ LANGUAGE sql IMMUTABLE;
