//! Verification: behaviors, the tests linked to them, and the runs that report
//! results. See `docs/verification.md`.
//!
//! A **behavior** is what the software must do, written for people. A **test**
//! is external and exists here only as a key its reporter chose; Takomo never
//! stores or runs test code. A **run** is one report from CI or an agent: the
//! commit it ran against, an optional note on why these tests were chosen, and
//! one pass/fail **result** per test key.
//!
//! A behavior's status is computed on read from the latest result of each linked
//! key and never stored, so it cannot drift from the evidence:
//!
//! - `failing`  — some linked key's latest result is a fail
//! - `verified` — otherwise, some linked key passed within [`FRESH_DAYS`]
//! - `stale`    — otherwise, a linked key has a result, but none is fresh
//! - `untested` — no linked key has ever reported
//!
//! One fresh pass is enough for `verified` on purpose: which variants to run is
//! the reporter's judgement, recorded in the run's note, and a failure anywhere
//! still wins.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::MAX_TITLE;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{behavior_id, iso, now_ms, sha256_hex, verification_run_id};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// How long a pass counts as current evidence. One server-wide number rather
/// than a setting: a per-project knob is easy to add once someone needs one.
pub const FRESH_DAYS: i64 = 14;
const DAY_MS: i64 = 86_400_000;

pub const MAX_BEHAVIORS_PAGE: i64 = 500;
pub const MAX_RUNS_PAGE: i64 = 200;
/// A behavior backed by more than this is several behaviors.
pub const MAX_TESTS_PER_BEHAVIOR: usize = 100;
/// Bounds how long one report can hold the write lock; a larger suite reports
/// in several runs.
pub const MAX_RESULTS_PER_RUN: usize = 10_000;
const MAX_STATEMENT: usize = 20_000;
const MAX_TEST_KEY: usize = 500;
const MAX_DETAIL: usize = 4_000;
const MAX_NOTE: usize = 4_000;
const MAX_COMMIT: usize = 100;
const MAX_IDEMPOTENCY_KEY: usize = 200;
const HISTORY_LIMIT: i64 = 50;
const UNLINKED_LIMIT: i64 = 50;

pub const BEHAVIOR_STATUSES: [&str; 4] = ["verified", "failing", "stale", "untested"];
pub const OUTCOMES: [&str; 2] = ["pass", "fail"];

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

pub struct BehaviorCreate {
    pub project: String,
    pub title: String,
    pub statement: String,
    pub section: Option<String>,
    pub tests: Vec<String>,
}

/// Absent fields are left alone. `section: Some(None)` clears the link;
/// `tests: Some(..)` replaces the whole list.
#[derive(Default)]
pub struct BehaviorPatch {
    pub title: Option<String>,
    pub statement: Option<String>,
    pub section: Option<Option<String>>,
    pub tests: Option<Vec<String>>,
}

pub struct BehaviorFilter {
    pub project: String,
    /// `Some("")` means "linked to no section".
    pub section: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct ResultInput {
    pub test: String,
    pub outcome: String,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct RunReport {
    #[serde(skip)]
    pub project: String,
    pub commit: Option<String>,
    pub note: Option<String>,
    pub results: Vec<ResultInput>,
    #[serde(skip)]
    pub idempotency_key: Option<String>,
    #[serde(skip)]
    pub user: Option<String>,
}

// ---------------------------------------------------------------------------
// Read models
// ---------------------------------------------------------------------------

/// The latest result of one test key.
#[derive(Debug, Clone)]
pub struct Latest {
    pub test: String,
    pub outcome: String,
    pub detail: Option<String>,
    pub at: i64,
    pub commit: Option<String>,
    pub run: String,
    pub actor: String,
}

impl Latest {
    fn to_json(&self) -> Value {
        json!({
            "test": self.test,
            "outcome": self.outcome,
            "detail": self.detail,
            "at": iso(self.at),
            "commit": self.commit,
            "run": self.run,
            "actor": self.actor,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Behavior {
    pub id: String,
    pub project: String,
    pub section: Option<String>,
    pub title: String,
    pub statement: String,
    pub tests: Vec<String>,
    pub status: &'static str,
    pub last_result: Option<Latest>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Behavior {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "project": self.project,
            "section": self.section,
            "title": self.title,
            "statement": self.statement,
            "tests": self.tests,
            "status": self.status,
            "last_result": self.last_result.as_ref().map(|l| json!({
                "test": l.test,
                "outcome": l.outcome,
                "at": iso(l.at),
                "commit": l.commit,
                "run": l.run,
            })),
            "created_by": self.created_by,
            "created_at": iso(self.created_at),
            "updated_at": iso(self.updated_at),
        })
    }
}

/// Status from the latest result of each linked key. See the module docs.
fn status_of<'a>(latest: impl IntoIterator<Item = &'a Latest>, now: i64) -> &'static str {
    let cutoff = now - FRESH_DAYS * DAY_MS;
    let (mut any, mut fresh_pass) = (false, false);
    for l in latest {
        if l.outcome == "fail" {
            return "failing";
        }
        any = true;
        fresh_pass |= l.at >= cutoff;
    }
    match (any, fresh_pass) {
        (_, true) => "verified",
        (true, false) => "stale",
        (false, _) => "untested",
    }
}

/// The result shown on a behavior: its failure if it has one, otherwise the
/// most recent result.
fn headline<'a>(latest: impl IntoIterator<Item = &'a Latest>) -> Option<Latest> {
    let all: Vec<&Latest> = latest.into_iter().collect();
    all.iter()
        .filter(|l| l.outcome == "fail")
        .max_by_key(|l| l.at)
        .or_else(|| all.iter().max_by_key(|l| l.at))
        .map(|l| (*l).clone())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn too_long(field: &str, actual: usize, max: usize) -> (String, String) {
    (
        format!("'{field}' is {actual} characters; the maximum is {max}."),
        format!("Shorten '{field}' to at most {max} characters."),
    )
}

/// Trimmed text, `None` when empty; `Err` carries the too-long message.
fn optional_text(
    value: &Option<String>,
    field: &str,
    max: usize,
) -> Result<Option<String>, (String, String)> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) => {
            let n = v.chars().count();
            if n > max {
                return Err(too_long(field, n, max));
            }
            Ok(Some(v.to_string()))
        }
    }
}

fn validate_title(title: &str) -> ApiResult<String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::validation(
            "validation.behavior_title",
            "A behavior needs a 'title' saying what the software does.",
        )
        .remedy("Send {\"title\": \"A failed save keeps edits and allows retry\"}.".to_string()));
    }
    let n = title.chars().count();
    if n > MAX_TITLE {
        let (msg, remedy) = too_long("title", n, MAX_TITLE);
        return Err(ApiError::validation("validation.behavior_title", msg).remedy(remedy));
    }
    Ok(title)
}

fn validate_statement(statement: &str) -> ApiResult<()> {
    let n = statement.chars().count();
    if n > MAX_STATEMENT {
        let (msg, remedy) = too_long("statement", n, MAX_STATEMENT);
        return Err(ApiError::validation("validation.behavior_statement", msg).remedy(remedy));
    }
    Ok(())
}

fn validate_test_key(key: &str) -> ApiResult<String> {
    let key = key.trim().to_string();
    if key.is_empty() || key.chars().any(char::is_control) {
        return Err(ApiError::validation(
            "validation.test_key",
            "A test key must be non-empty text without control characters.",
        )
        .remedy(
            "Use the identifier your runner reports, e.g. \"playwright:editor.spec.ts › keeps edits\"."
                .to_string(),
        ));
    }
    let n = key.chars().count();
    if n > MAX_TEST_KEY {
        let (msg, remedy) = too_long("test", n, MAX_TEST_KEY);
        return Err(ApiError::validation("validation.test_key", msg).remedy(remedy));
    }
    Ok(key)
}

fn normalize_tests(tests: &[String]) -> ApiResult<Vec<String>> {
    let mut out = Vec::new();
    for t in tests {
        let key = validate_test_key(t)?;
        if !out.contains(&key) {
            out.push(key);
        }
    }
    if out.len() > MAX_TESTS_PER_BEHAVIOR {
        return Err(ApiError::validation(
            "validation.behavior_tests",
            format!(
                "A behavior may link at most {MAX_TESTS_PER_BEHAVIOR} tests; this request links {}.",
                out.len()
            ),
        )
        .remedy("Split it into narrower behaviors.".to_string()));
    }
    out.sort();
    Ok(out)
}

fn project_exists(conn: &Connection, project: &str) -> ApiResult<()> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM projects WHERE id = ?1",
            params![project],
            |r| r.get(0),
        )
        .optional()?;
    if found.is_none() {
        return Err(ApiError::not_found("project", project));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

const BEHAVIOR_COLS: &str =
    "id, project, section, title, statement, created_by, created_at, updated_at";

fn row_to_behavior(row: &Row) -> rusqlite::Result<Behavior> {
    Ok(Behavior {
        id: row.get("id")?,
        project: row.get("project")?,
        section: row.get("section")?,
        title: row.get("title")?,
        statement: row.get("statement")?,
        tests: Vec::new(),
        status: "untested",
        last_result: None,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_latest(row: &Row) -> rusqlite::Result<Latest> {
    Ok(Latest {
        test: row.get("test_key")?,
        outcome: row.get("outcome")?,
        detail: row.get("detail")?,
        at: row.get("at")?,
        commit: row.get("commit")?,
        run: row.get("run")?,
        actor: row.get("actor")?,
    })
}

/// The latest result of every key in `project` that satisfies `key_filter`
/// (an SQL predicate over `r.test_key`). Two results at the same millisecond
/// resolve to the fail, so a tie can never hide one.
fn latest_results(
    conn: &Connection,
    project: &str,
    key_filter: &str,
) -> ApiResult<HashMap<String, Latest>> {
    let sql = format!(
        "SELECT r.test_key, r.outcome, r.detail, r.at, v.\"commit\" AS \"commit\", r.run, v.actor
         FROM verification_results r JOIN verification_runs v ON v.id = r.run
         WHERE r.project = ?1 AND {key_filter}
           AND r.at = (SELECT MAX(x.at) FROM verification_results x
                       WHERE x.project = r.project AND x.test_key = r.test_key)"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project], row_to_latest)?;
    let mut out: HashMap<String, Latest> = HashMap::new();
    for row in rows {
        let l = row?;
        match out.get(&l.test) {
            Some(prev) if prev.outcome == "fail" => {}
            _ => {
                out.insert(l.test.clone(), l);
            }
        }
    }
    Ok(out)
}

/// Every behavior of a project with its tests and computed status.
fn load_project(conn: &Connection, project: &str) -> ApiResult<Vec<Behavior>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {BEHAVIOR_COLS} FROM behaviors WHERE project = ?1
         ORDER BY title COLLATE NOCASE, id"
    ))?;
    let mut behaviors = stmt
        .query_map(params![project], row_to_behavior)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut links: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT bt.behavior, bt.test_key FROM behavior_tests bt
         JOIN behaviors b ON b.id = bt.behavior WHERE b.project = ?1 ORDER BY bt.test_key",
    )?;
    for row in stmt.query_map(params![project], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (b, k) = row?;
        links.entry(b).or_default().push(k);
    }
    let latest = latest_results(
        conn,
        project,
        "r.test_key IN (SELECT bt.test_key FROM behavior_tests bt
                        JOIN behaviors b ON b.id = bt.behavior WHERE b.project = ?1)",
    )?;
    let now = now_ms();
    for b in &mut behaviors {
        b.tests = links.remove(&b.id).unwrap_or_default();
        hydrate(b, &latest, now);
    }
    Ok(behaviors)
}

fn hydrate(b: &mut Behavior, latest: &HashMap<String, Latest>, now: i64) {
    let mine: Vec<&Latest> = b.tests.iter().filter_map(|k| latest.get(k)).collect();
    b.status = status_of(mine.iter().copied(), now);
    b.last_result = headline(mine);
}

fn load_one(conn: &Connection, id: &str) -> ApiResult<Behavior> {
    let mut b = conn
        .query_row(
            &format!("SELECT {BEHAVIOR_COLS} FROM behaviors WHERE id = ?1"),
            params![id],
            row_to_behavior,
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("behavior", id))?;
    b.tests = load_tests(conn, id)?;
    let latest = latest_for_keys(conn, &b.project, &b.tests)?;
    hydrate(&mut b, &latest, now_ms());
    Ok(b)
}

/// `latest_results` for an explicit key list (the one-behavior read).
fn latest_for_keys(
    conn: &Connection,
    project: &str,
    keys: &[String],
) -> ApiResult<HashMap<String, Latest>> {
    let mut out = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT r.test_key, r.outcome, r.detail, r.at, v.\"commit\" AS \"commit\", r.run, v.actor
         FROM verification_results r JOIN verification_runs v ON v.id = r.run
         WHERE r.project = ?1 AND r.test_key = ?2
         ORDER BY r.at DESC, CASE r.outcome WHEN 'fail' THEN 0 ELSE 1 END LIMIT 1",
    )?;
    for k in keys {
        if let Some(l) = stmt
            .query_row(params![project, k], row_to_latest)
            .optional()?
        {
            out.insert(k.clone(), l);
        }
    }
    Ok(out)
}

fn load_tests(conn: &Connection, id: &str) -> ApiResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT test_key FROM behavior_tests WHERE behavior = ?1 ORDER BY test_key")?;
    let keys = stmt
        .query_map(params![id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(keys)
}

fn replace_tests(conn: &Connection, id: &str, tests: &[String]) -> ApiResult<()> {
    conn.execute(
        "DELETE FROM behavior_tests WHERE behavior = ?1",
        params![id],
    )?;
    for t in tests {
        conn.execute(
            "INSERT INTO behavior_tests (behavior, test_key) VALUES (?1, ?2)",
            params![id, t],
        )?;
    }
    Ok(())
}

fn project_of(conn: &Connection, id: &str) -> ApiResult<String> {
    conn.query_row(
        "SELECT project FROM behaviors WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("behavior", id))
}

fn run_json(conn: &Connection, id: &str) -> ApiResult<Value> {
    conn.query_row(
        "SELECT v.id, v.project, v.\"commit\", v.note, v.actor, v.at,
                (SELECT COUNT(*) FROM verification_results r WHERE r.run = v.id AND r.outcome = 'pass'),
                (SELECT COUNT(*) FROM verification_results r WHERE r.run = v.id AND r.outcome = 'fail')
         FROM verification_runs v WHERE v.id = ?1",
        params![id],
        row_to_run_json,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("run", id))
}

fn row_to_run_json(r: &Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?,
        "project": r.get::<_, String>(1)?,
        "commit": r.get::<_, Option<String>>(2)?,
        "note": r.get::<_, Option<String>>(3)?,
        "actor": r.get::<_, String>(4)?,
        "at": iso(r.get::<_, i64>(5)?),
        "passed": r.get::<_, i64>(6)?,
        "failed": r.get::<_, i64>(7)?,
    }))
}

// ---------------------------------------------------------------------------
// Store API
// ---------------------------------------------------------------------------

impl Store {
    pub fn create_behavior(&self, req: &BehaviorCreate, actor: &str) -> ApiResult<Behavior> {
        let title = validate_title(&req.title)?;
        validate_statement(&req.statement)?;
        let tests = normalize_tests(&req.tests)?;
        let id = behavior_id();
        let now = now_ms();
        self.with_tx(|tx| {
            project_exists(tx, &req.project)?;
            ensure_project_writable(tx, &req.project)?;
            tx.execute(
                "INSERT INTO behaviors (id, project, section, title, statement, created_by,
                    created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    id,
                    req.project,
                    req.section,
                    title,
                    req.statement,
                    actor,
                    now
                ],
            )?;
            replace_tests(tx, &id, &tests)?;
            emit_event(
                tx,
                None,
                Some(&req.project),
                actor,
                "verification.behavior_created",
                json!({ "behavior": id, "title": title, "tests": tests.len() }),
                now,
            )?;
            load_one(tx, &id)
        })
    }

    pub fn get_behavior(&self, id: &str) -> ApiResult<Behavior> {
        self.with_conn(|conn| load_one(conn, id))
    }

    /// The behavior plus, per linked test, its latest result, and the most
    /// recent results across all of them.
    pub fn behavior_detail(&self, id: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            let b = load_one(conn, id)?;
            let latest = latest_for_keys(conn, &b.project, &b.tests)?;
            let mut out = b.to_json();
            out["test_results"] = json!(b
                .tests
                .iter()
                .map(|k| json!({ "test": k, "latest": latest.get(k).map(Latest::to_json) }))
                .collect::<Vec<_>>());
            let mut stmt = conn.prepare(
                "SELECT r.test_key, r.outcome, r.detail, r.at, v.\"commit\", r.run, v.note, v.actor
                 FROM verification_results r JOIN verification_runs v ON v.id = r.run
                 WHERE r.project = ?1
                   AND r.test_key IN (SELECT test_key FROM behavior_tests WHERE behavior = ?2)
                 ORDER BY r.at DESC, r.test_key LIMIT ?3",
            )?;
            let history = stmt
                .query_map(params![b.project, b.id, HISTORY_LIMIT], |r| {
                    Ok(json!({
                        "test": r.get::<_, String>(0)?,
                        "outcome": r.get::<_, String>(1)?,
                        "detail": r.get::<_, Option<String>>(2)?,
                        "at": iso(r.get::<_, i64>(3)?),
                        "commit": r.get::<_, Option<String>>(4)?,
                        "run": r.get::<_, String>(5)?,
                        "note": r.get::<_, Option<String>>(6)?,
                        "actor": r.get::<_, String>(7)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            out["history"] = json!(history);
            Ok(out)
        })
    }

    pub fn list_behaviors(&self, filter: &BehaviorFilter) -> ApiResult<(Vec<Behavior>, i64)> {
        if let Some(s) = &filter.status {
            if !BEHAVIOR_STATUSES.contains(&s.as_str()) {
                return Err(ApiError::validation(
                    "validation.behavior_status",
                    format!("Unknown status '{s}'."),
                )
                .remedy(format!("Use one of: {}.", BEHAVIOR_STATUSES.join(", "))));
            }
        }
        let limit = filter
            .limit
            .unwrap_or(MAX_BEHAVIORS_PAGE)
            .clamp(1, MAX_BEHAVIORS_PAGE) as usize;
        let offset = filter.offset.unwrap_or(0).max(0) as usize;
        let q = filter.q.as_deref().map(str::to_lowercase);
        self.with_conn(|conn| {
            project_exists(conn, &filter.project)?;
            let mut all = load_project(conn, &filter.project)?;
            all.retain(|b| {
                filter.section.as_deref().is_none_or(|s| {
                    if s.is_empty() {
                        b.section.is_none()
                    } else {
                        b.section.as_deref() == Some(s)
                    }
                }) && filter.status.as_deref().is_none_or(|s| b.status == s)
                    && q.as_deref().is_none_or(|q| {
                        b.title.to_lowercase().contains(q)
                            || b.statement.to_lowercase().contains(q)
                            || b.tests.iter().any(|t| t.to_lowercase().contains(q))
                    })
            });
            let total = all.len() as i64;
            Ok((all.into_iter().skip(offset).take(limit).collect(), total))
        })
    }

    pub fn patch_behavior(
        &self,
        id: &str,
        patch: &BehaviorPatch,
        actor: &str,
    ) -> ApiResult<Behavior> {
        let title = patch.title.as_deref().map(validate_title).transpose()?;
        if let Some(s) = &patch.statement {
            validate_statement(s)?;
        }
        let tests = patch.tests.as_deref().map(normalize_tests).transpose()?;
        let now = now_ms();
        self.with_tx(|tx| {
            let project = project_of(tx, id)?;
            ensure_project_writable(tx, &project)?;
            if let Some(t) = &title {
                tx.execute(
                    "UPDATE behaviors SET title = ?2 WHERE id = ?1",
                    params![id, t],
                )?;
            }
            if let Some(s) = &patch.statement {
                tx.execute(
                    "UPDATE behaviors SET statement = ?2 WHERE id = ?1",
                    params![id, s],
                )?;
            }
            if let Some(section) = &patch.section {
                tx.execute(
                    "UPDATE behaviors SET section = ?2 WHERE id = ?1",
                    params![id, section],
                )?;
            }
            if let Some(tests) = &tests {
                replace_tests(tx, id, tests)?;
            }
            tx.execute(
                "UPDATE behaviors SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            let mut changed = Vec::new();
            for (field, set) in [
                ("title", title.is_some()),
                ("statement", patch.statement.is_some()),
                ("section", patch.section.is_some()),
                ("tests", tests.is_some()),
            ] {
                if set {
                    changed.push(field);
                }
            }
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "verification.behavior_updated",
                json!({ "behavior": id, "fields": changed }),
                now,
            )?;
            load_one(tx, id)
        })
    }

    /// Deletes the behavior and its links. Results stay: they are evidence
    /// about tests, and a test may back another behavior.
    pub fn delete_behavior(&self, id: &str, actor: &str) -> ApiResult<()> {
        self.with_tx(|tx| {
            let project = project_of(tx, id)?;
            ensure_project_writable(tx, &project)?;
            tx.execute("DELETE FROM behaviors WHERE id = ?1", params![id])?;
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "verification.behavior_deleted",
                json!({ "behavior": id }),
                now_ms(),
            )?;
            Ok(())
        })
    }

    /// Record one run. Returns the run, the reported keys no behavior links,
    /// how many behaviors the report touched, and whether this was a replay of
    /// an earlier report with the same idempotency key.
    pub fn report_run(&self, req: &RunReport, actor: &str) -> ApiResult<(Value, bool)> {
        if req.results.is_empty() {
            return Err(ApiError::validation(
                "validation.run_results",
                "A run needs at least one result.",
            )
            .remedy(
                "Send {\"results\": [{\"test\": \"<key>\", \"outcome\": \"pass\"}]}.".to_string(),
            ));
        }
        if req.results.len() > MAX_RESULTS_PER_RUN {
            return Err(ApiError::validation(
                "validation.run_results",
                format!(
                    "A run may carry at most {MAX_RESULTS_PER_RUN} results; this one carries {}.",
                    req.results.len()
                ),
            )
            .remedy("Report the suite in several runs against the same commit.".to_string()));
        }
        let commit = optional_text(&req.commit, "commit", MAX_COMMIT)
            .map_err(|(m, r)| ApiError::validation("validation.run_commit", m).remedy(r))?;
        let note = optional_text(&req.note, "note", MAX_NOTE)
            .map_err(|(m, r)| ApiError::validation("validation.run_note", m).remedy(r))?;
        let idem = optional_text(&req.idempotency_key, "Idempotency-Key", MAX_IDEMPOTENCY_KEY)
            .map_err(|(m, r)| ApiError::validation("validation.idempotency_key", m).remedy(r))?;
        let mut results = Vec::with_capacity(req.results.len());
        let mut seen = HashSet::new();
        for r in &req.results {
            let key = validate_test_key(&r.test)?;
            if !OUTCOMES.contains(&r.outcome.as_str()) {
                return Err(ApiError::validation(
                    "validation.run_outcome",
                    format!("Result for '{key}' has outcome '{}'.", r.outcome),
                )
                .remedy(
                    "Use \"pass\" or \"fail\". Report a test that could not run as a fail \
                     with the reason in 'detail'; leave out a test that was skipped."
                        .to_string(),
                ));
            }
            if !seen.insert(key.clone()) {
                return Err(ApiError::validation(
                    "validation.run_duplicate_test",
                    format!("Test '{key}' appears more than once in this run."),
                )
                .remedy("Report each test once per run.".to_string()));
            }
            let detail = optional_text(&r.detail, "detail", MAX_DETAIL)
                .map_err(|(m, r)| ApiError::validation("validation.run_detail", m).remedy(r))?;
            results.push((key, r.outcome.clone(), detail));
        }
        let body_hash = sha256_hex(serde_json::to_string(req).unwrap_or_default().as_bytes());
        let now = now_ms();
        self.with_tx(|tx| {
            project_exists(tx, &req.project)?;
            ensure_project_writable(tx, &req.project)?;
            if let Some(key) = &idem {
                let existing: Option<(String, Option<String>)> = tx
                    .query_row(
                        "SELECT id, body_hash FROM verification_runs
                         WHERE project = ?1 AND actor = ?2 AND idempotency_key = ?3",
                        params![req.project, actor, key],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some((run, hash)) = existing {
                    if hash.as_deref() != Some(body_hash.as_str()) {
                        return Err(ApiError::conflict(
                            "conflict.idempotency_key",
                            "This Idempotency-Key was already used for a different report.",
                        )
                        .remedy(
                            "Use a fresh key for a new report, or resend the identical body."
                                .to_string(),
                        )
                        .details(json!({ "key": key, "run": run })));
                    }
                    return Ok((report_outcome(tx, &req.project, &run)?, true));
                }
            }
            let id = verification_run_id();
            tx.execute(
                "INSERT INTO verification_runs (id, project, \"commit\", note, actor, \"user\", at,
                    idempotency_key, body_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    req.project,
                    commit,
                    note,
                    actor,
                    req.user,
                    now,
                    idem,
                    body_hash
                ],
            )?;
            let mut stmt = tx.prepare(
                "INSERT INTO verification_results (run, project, test_key, outcome, detail, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (key, outcome, detail) in &results {
                stmt.execute(params![id, req.project, key, outcome, detail, now])?;
            }
            drop(stmt);
            let out = report_outcome(tx, &req.project, &id)?;
            emit_event(
                tx,
                None,
                Some(&req.project),
                actor,
                "verification.run_reported",
                json!({
                    "run": id,
                    "commit": commit,
                    "passed": out["run"]["passed"],
                    "failed": out["run"]["failed"],
                }),
                now,
            )?;
            Ok((out, false))
        })
    }

    pub fn list_runs(
        &self,
        project: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> ApiResult<(Vec<Value>, i64)> {
        let limit = limit.unwrap_or(50).clamp(1, MAX_RUNS_PAGE);
        let offset = offset.unwrap_or(0).max(0);
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM verification_runs WHERE project = ?1",
                params![project],
                |r| r.get(0),
            )?;
            let mut stmt = conn.prepare(
                "SELECT v.id, v.project, v.\"commit\", v.note, v.actor, v.at,
                    (SELECT COUNT(*) FROM verification_results r WHERE r.run = v.id AND r.outcome = 'pass'),
                    (SELECT COUNT(*) FROM verification_results r WHERE r.run = v.id AND r.outcome = 'fail')
                 FROM verification_runs v WHERE v.project = ?1
                 ORDER BY v.at DESC, v.id DESC LIMIT ?2 OFFSET ?3",
            )?;
            let runs = stmt
                .query_map(params![project, limit, offset], row_to_run_json)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((runs, total))
        })
    }

    /// Where verification stands for a project: status counts overall and per
    /// section, and the reported tests no behavior links.
    pub fn verification_summary(&self, project: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let behaviors = load_project(conn, project)?;
            let empty = || {
                json!({ "total": 0, "verified": 0, "failing": 0, "stale": 0, "untested": 0 })
            };
            let bump = |counts: &mut Value, status: &str| {
                counts["total"] = json!(counts["total"].as_i64().unwrap_or(0) + 1);
                counts[status] = json!(counts[status].as_i64().unwrap_or(0) + 1);
            };
            let mut summary = empty();
            let mut sections: BTreeMap<String, Value> = BTreeMap::new();
            let mut unsectioned = 0;
            for b in &behaviors {
                bump(&mut summary, b.status);
                match &b.section {
                    Some(s) => bump(sections.entry(s.clone()).or_insert_with(empty), b.status),
                    None => unsectioned += 1,
                }
            }
            let unlinked_filter = "r.test_key NOT IN (SELECT bt.test_key FROM behavior_tests bt
                                   JOIN behaviors b ON b.id = bt.behavior WHERE b.project = ?1)";
            let mut unlinked: Vec<Latest> =
                latest_results(conn, project, unlinked_filter)?.into_values().collect();
            unlinked.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.test.cmp(&b.test)));
            let unlinked_total = unlinked.len();
            let unlinked_items: Vec<Value> = unlinked
                .iter()
                .take(UNLINKED_LIMIT as usize)
                .map(|l| {
                    json!({ "test": l.test, "outcome": l.outcome, "at": iso(l.at), "commit": l.commit })
                })
                .collect();
            let latest_run: Option<String> = conn
                .query_row(
                    "SELECT id FROM verification_runs WHERE project = ?1
                     ORDER BY at DESC, id DESC LIMIT 1",
                    params![project],
                    |r| r.get(0),
                )
                .optional()?;
            let latest_run = latest_run.map(|id| run_json(conn, &id)).transpose()?;
            Ok(json!({
                "fresh_days": FRESH_DAYS,
                "summary": summary,
                "sections": sections,
                "unsectioned": unsectioned,
                "unlinked_tests": {
                    "items": unlinked_items,
                    "total": unlinked_total,
                    "limit": UNLINKED_LIMIT,
                },
                "latest_run": latest_run,
            }))
        })
    }
}

fn report_outcome(conn: &Connection, project: &str, run: &str) -> ApiResult<Value> {
    let mut stmt = conn.prepare(
        "SELECT r.test_key,
                EXISTS(SELECT 1 FROM behavior_tests bt JOIN behaviors b ON b.id = bt.behavior
                       WHERE b.project = ?1 AND bt.test_key = r.test_key)
         FROM verification_results r WHERE r.run = ?2 ORDER BY r.test_key",
    )?;
    let mut unlinked = Vec::new();
    for row in stmt.query_map(params![project, run], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
    })? {
        let (key, linked) = row?;
        if !linked {
            unlinked.push(key);
        }
    }
    let affected: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT bt.behavior) FROM behavior_tests bt
         JOIN behaviors b ON b.id = bt.behavior
         WHERE b.project = ?1
           AND bt.test_key IN (SELECT test_key FROM verification_results WHERE run = ?2)",
        params![project, run],
        |r| r.get(0),
    )?;
    Ok(json!({
        "run": run_json(conn, run)?,
        "unlinked": unlinked,
        "behaviors_affected": affected,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latest(outcome: &str, at: i64) -> Latest {
        Latest {
            test: format!("t{at}"),
            outcome: outcome.into(),
            detail: None,
            at,
            commit: None,
            run: "vrn-x".into(),
            actor: "a".into(),
        }
    }

    #[test]
    fn status_rules() {
        let now = 100 * DAY_MS;
        let fresh = now - DAY_MS;
        let old = now - (FRESH_DAYS + 1) * DAY_MS;
        assert_eq!(status_of([], now), "untested");
        assert_eq!(status_of(&[latest("pass", fresh)], now), "verified");
        assert_eq!(status_of(&[latest("pass", old)], now), "stale");
        // One fresh pass among old ones is enough.
        assert_eq!(
            status_of(&[latest("pass", old), latest("pass", fresh)], now),
            "verified"
        );
        // A failure wins, however old.
        assert_eq!(
            status_of(&[latest("pass", fresh), latest("fail", old)], now),
            "failing"
        );
    }

    #[test]
    fn headline_prefers_the_failure() {
        let a = latest("pass", 10);
        let b = latest("fail", 5);
        assert_eq!(headline([&a, &b]).unwrap().outcome, "fail");
        assert_eq!(headline([&a]).unwrap().at, 10);
        assert!(headline([]).is_none());
    }
}
