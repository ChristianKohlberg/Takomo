//! Checklist: the verification surface. Releases, checks, the cases generated
//! beneath them, and the verdicts recorded against those cases.
//!
//! The division of labour is deliberate and load-bearing: **Takomo stores, the
//! agent computes.** Nothing here generates a combinatorial model, validates one,
//! or checks whether a coverage claim is true. The agent owns the recipe and its
//! correctness; this module owns persistence, policy resolution, expiry
//! arithmetic and history. A wrong model is stored faithfully — the alternative
//! is Takomo growing an opinion about every application under test.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::{
    Case, CaseVerdict, Check, CheckCounts, Release, ReleaseImpact, ResolvedPolicy, CASE_VERDICTS,
    CHECK_LAYERS, CHECK_SEVERITIES, MAX_BODY, MAX_METADATA, MAX_TITLE, VERIFICATION_LEVELS,
};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{case_id, check_id, checklist_policy_id, now_ms, release_id, verdict_id};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Ceiling on the paths one release push may carry. A release diff is bounded in
/// practice by what a human reviews; this exists so one request cannot hold the
/// process-wide write mutex — the mutex whose single-writer serialization *is*
/// the exactly-one-claimant guarantee for the ready queue — for an unbounded
/// time. A diff larger than this is a repository-wide sweep, and the honest
/// answer there is "retest everything", not "enumerate 200k paths".
pub const MAX_RELEASE_PATHS: usize = 20_000;

/// Ceiling on globs one check may claim. A check covers one action; a handful of
/// path patterns describes it. Anything needing fifty is really several checks.
pub const MAX_CHECK_GLOBS: usize = 50;

/// Ceiling on cases filed in one call. The measured pairwise model for a large
/// real form produced 76; 3-way produced 503. 5000 leaves room for a pathological
/// model while still bounding the transaction.
pub const MAX_CASES_PER_FILE: usize = 5_000;

/// Page ceilings for the two checklist lists that grow with the work rather
/// than with the project. Both used to return everything they had: a check per
/// action per layer, and up to [`MAX_CASES_PER_FILE`] cases *per check* — so a
/// single unfiltered read could hand an agent a five-thousand-row reply that
/// buries whatever it was actually looking for. The counts are still reported in
/// full, so a capped read is visible as one rather than looking complete.
pub const MAX_CHECKS_PAGE: i64 = 200;
pub const MAX_CASES_PAGE: i64 = 500;

const MAX_KEY: usize = 200;
const MAX_LABEL: usize = 300;
const MAX_GLOB: usize = 400;
const MAX_NOTE: usize = 4_000;
const MAX_REF: usize = 200;

/// Default verification level when nothing up the chain sets one. `agent` and not
/// `human`: the point of the feature is that agents clear the bulk of the work.
const DEFAULT_VERIFICATION: &str = "agent";

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// Does `pat` match `path`? `**` spans path separators, `*` does not.
///
/// Hand-rolled rather than a dependency: the whole matcher is thirty lines, the
/// SPAs are dependency-free on purpose and the same instinct applies here.
/// Recursion depth is bounded by the pattern length.
pub fn glob_matches(pat: &str, path: &str) -> bool {
    matches_bytes(pat.as_bytes(), path.as_bytes())
}

fn matches_bytes(pat: &[u8], path: &[u8]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat.starts_with(b"**") {
        let rest = &pat[2..];
        // `a/**/b` must also match `a/b` — zero intervening segments.
        if let Some(after_slash) = rest.strip_prefix(b"/") {
            if matches_bytes(after_slash, path) {
                return true;
            }
        }
        for i in 0..=path.len() {
            if matches_bytes(rest, &path[i..]) {
                return true;
            }
        }
        return false;
    }
    if pat[0] == b'*' {
        let rest = &pat[1..];
        let mut i = 0usize;
        loop {
            if matches_bytes(rest, &path[i..]) {
                return true;
            }
            // A single star stops at a separator.
            if i >= path.len() || path[i] == b'/' {
                return false;
            }
            i += 1;
        }
    }
    if !path.is_empty() && pat[0] == path[0] {
        return matches_bytes(&pat[1..], &path[1..]);
    }
    false
}

// ---------------------------------------------------------------------------
// Request shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ReleasePush {
    pub project: String,
    pub reference: String,
    pub note: Option<String>,
    /// Paths the release's diff touched, supplied by the pusher.
    pub touched_paths: Vec<String>,
    /// Check globs that matched NO file in this tree.
    pub orphan_globs: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckCreate {
    pub project: String,
    pub epic: Option<String>,
    pub title: String,
    pub body: String,
    pub precondition: String,
    pub layer: Option<String>,
    pub severity: Option<String>,
    pub verification: Option<String>,
    pub expiry_days: Option<i64>,
    pub expiry_releases: Option<i64>,
    pub cost_agent_minutes: Option<i64>,
    pub cost_human_minutes: Option<i64>,
    pub globs: Vec<String>,
    pub metadata: Option<Value>,
}

/// A check patch. The doubly-wrapped fields are override slots: `None` means "not
/// mentioned", `Some(None)` means "clear it and inherit again". Collapsing those
/// two into one would make an inherited policy impossible to restore.
#[derive(Debug, Clone, Default)]
pub struct CheckPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub precondition: Option<String>,
    pub layer: Option<String>,
    pub severity: Option<String>,
    pub verification: Option<Option<String>>,
    pub expiry_days: Option<Option<i64>>,
    pub expiry_releases: Option<Option<i64>>,
    pub cost_agent_minutes: Option<Option<i64>>,
    pub cost_human_minutes: Option<Option<i64>>,
    pub globs: Option<Vec<String>>,
    pub metadata_merge: Option<Value>,
    pub epic: Option<Option<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct CaseInput {
    pub key: String,
    pub label: String,
    pub assignment: Value,
    pub seeded: bool,
}

/// What filing a set of cases did. `retired` counts live cases the new set no
/// longer contains; they keep their verdict history rather than being deleted.
#[derive(Debug, Clone, Default)]
pub struct CaseFileOutcome {
    pub added: i64,
    pub updated: i64,
    pub retired: i64,
    pub revived: i64,
    pub live: i64,
}

impl CaseFileOutcome {
    pub fn to_json(&self) -> Value {
        json!({
            "added": self.added,
            "updated": self.updated,
            "retired": self.retired,
            "revived": self.revived,
            "live": self.live,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct PolicyInput {
    pub verification: Option<Option<String>>,
    pub expiry_days: Option<Option<i64>>,
    pub expiry_releases: Option<Option<i64>>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckFilter {
    pub project: String,
    pub epic: Option<String>,
    pub severity: Option<String>,
    pub layer: Option<String>,
    pub include_archived: bool,
    pub with_policy: bool,
    /// Page size; `None` means [`MAX_CHECKS_PAGE`], which is also the ceiling.
    pub limit: Option<i64>,
}

/// One thing that needs re-verifying, and why.
#[derive(Debug, Clone)]
pub struct WorkItem {
    pub check: String,
    pub check_title: String,
    pub severity: String,
    pub layer: String,
    pub case: String,
    pub case_key: String,
    pub case_label: String,
    pub reason: &'static str,
    pub verification: String,
    pub cost_minutes: Option<i64>,
}

impl WorkItem {
    pub fn to_json(&self) -> Value {
        json!({
            "check": self.check,
            "check_title": self.check_title,
            "severity": self.severity,
            "layer": self.layer,
            "case": self.case,
            "case_key": self.case_key,
            "case_label": self.case_label,
            "reason": self.reason,
            "verification": self.verification,
            "cost_minutes": self.cost_minutes,
        })
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

// The three `msg_*` helpers build the wording; each *check* below then constructs
// its `ApiError` with a **literal** code. That split is deliberate: passing the
// code through a shared helper would hide it from the error-code scan in
// `tests/api.rs`, and a code the scan cannot read is a code that can silently
// leave the documented vocabulary. Verbose here buys a contract that stays honest.

fn msg_too_long(field: &str, actual: usize, max: usize) -> (String, String) {
    (
        format!("'{field}' is {actual} characters; the maximum is {max}."),
        format!("Shorten '{field}' to {max} characters or fewer."),
    )
}

fn msg_bad_enum(field: &str, value: &str, allowed: &[&str]) -> (String, String) {
    (
        format!(
            "'{value}' is not a valid '{field}'. Valid values: {}.",
            allowed.join(", ")
        ),
        format!("Send one of: {}.", allowed.join(", ")),
    )
}

fn msg_negative(field: &str, value: i64) -> (String, String) {
    (
        format!("'{field}' is {value}; it cannot be negative."),
        format!("Send a non-negative '{field}', or null to inherit."),
    )
}

// Written out one function per field rather than generated: the error-code scan in
// `tests/api.rs` reads source text, so a macro body or a forwarded parameter would
// hide the code from it — and a code the scan cannot read is one that can quietly
// drift out of the documented vocabulary.

fn validate_release_ref(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_REF {
        let (m, r) = msg_too_long("ref", n, MAX_REF);
        return Err(ApiError::validation("validation.release_ref", m).remedy(r));
    }
    Ok(())
}

fn validate_release_note(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_NOTE {
        let (m, r) = msg_too_long("note", n, MAX_NOTE);
        return Err(ApiError::validation("validation.release_note", m).remedy(r));
    }
    Ok(())
}

fn validate_check_title(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_TITLE {
        let (m, r) = msg_too_long("title", n, MAX_TITLE);
        return Err(ApiError::validation("validation.check_title", m).remedy(r));
    }
    Ok(())
}

fn validate_check_body(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_BODY {
        let (m, r) = msg_too_long("body", n, MAX_BODY);
        return Err(ApiError::validation("validation.check_body", m).remedy(r));
    }
    Ok(())
}

fn validate_check_precondition(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_BODY {
        let (m, r) = msg_too_long("precondition", n, MAX_BODY);
        return Err(ApiError::validation("validation.check_precondition", m).remedy(r));
    }
    Ok(())
}

fn validate_glob_len(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_GLOB {
        let (m, r) = msg_too_long("glob", n, MAX_GLOB);
        return Err(ApiError::validation("validation.check_globs", m).remedy(r));
    }
    Ok(())
}

fn validate_case_key_len(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_KEY {
        let (m, r) = msg_too_long("key", n, MAX_KEY);
        return Err(ApiError::validation("validation.case_key", m).remedy(r));
    }
    Ok(())
}

fn validate_case_label(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_LABEL {
        let (m, r) = msg_too_long("label", n, MAX_LABEL);
        return Err(ApiError::validation("validation.case_label", m).remedy(r));
    }
    Ok(())
}

fn validate_verdict_note(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_NOTE {
        let (m, r) = msg_too_long("note", n, MAX_NOTE);
        return Err(ApiError::validation("validation.verdict_note", m).remedy(r));
    }
    Ok(())
}

fn validate_layer(value: &str) -> ApiResult<()> {
    let allowed: &[&str] = &CHECK_LAYERS;
    if !allowed.contains(&value) {
        let (m, r) = msg_bad_enum("layer", value, allowed);
        return Err(ApiError::validation("validation.check_layer", m).remedy(r));
    }
    Ok(())
}

fn validate_severity(value: &str) -> ApiResult<()> {
    let allowed: &[&str] = &CHECK_SEVERITIES;
    if !allowed.contains(&value) {
        let (m, r) = msg_bad_enum("severity", value, allowed);
        return Err(ApiError::validation("validation.check_severity", m).remedy(r));
    }
    Ok(())
}

fn validate_verification(value: &str) -> ApiResult<()> {
    let allowed: &[&str] = &VERIFICATION_LEVELS;
    if !allowed.contains(&value) {
        let (m, r) = msg_bad_enum("verification", value, allowed);
        return Err(ApiError::validation("validation.verification", m).remedy(r));
    }
    Ok(())
}

fn validate_verdict(value: &str) -> ApiResult<()> {
    let allowed: &[&str] = &CASE_VERDICTS;
    if !allowed.contains(&value) {
        let (m, r) = msg_bad_enum("verdict", value, allowed);
        return Err(ApiError::validation("validation.verdict", m).remedy(r));
    }
    Ok(())
}

fn validate_actor_kind(value: &str) -> ApiResult<()> {
    let allowed: &[&str] = &["agent", "human"];
    if !allowed.contains(&value) {
        let (m, r) = msg_bad_enum("actor_kind", value, allowed);
        return Err(ApiError::validation("validation.actor_kind", m).remedy(r));
    }
    Ok(())
}

fn validate_check_number(field: &str, value: i64) -> ApiResult<()> {
    if value < 0 {
        let (m, r) = msg_negative(field, value);
        return Err(ApiError::validation("validation.check_numbers", m).remedy(r));
    }
    Ok(())
}

fn validate_policy_number(field: &str, value: i64) -> ApiResult<()> {
    if value < 0 {
        let (m, r) = msg_negative(field, value);
        return Err(ApiError::validation("validation.policy_numbers", m).remedy(r));
    }
    Ok(())
}

fn normalize_globs(globs: &[String]) -> ApiResult<Vec<String>> {
    if globs.len() > MAX_CHECK_GLOBS {
        return Err(ApiError::validation(
            "validation.check_globs",
            format!(
                "A check claims {} globs; the maximum is {MAX_CHECK_GLOBS}.",
                globs.len()
            ),
        )
        .remedy(
            "A check covers one action, which a handful of path patterns describes. \
             Split it into several checks."
                .to_string(),
        ));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for g in globs {
        let g = g.trim();
        if g.is_empty() {
            return Err(ApiError::validation(
                "validation.check_globs",
                "A check glob cannot be empty.",
            )
            .remedy("Drop the empty entry, or send a pattern like 'src/claims/**'."));
        }
        validate_glob_len(g)?;
        if seen.insert(g.to_string()) {
            out.push(g.to_string());
        }
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

/// An epic parent must exist, live in the same project, and actually be an epic.
/// Checklist reuses `type: epic` tickets for grouping so the vocabulary matches
/// tickets and the roadmap rollup keeps working.
fn validate_epic(conn: &Connection, project: &str, epic: &str) -> ApiResult<()> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT project, type FROM tickets WHERE id = ?1",
            params![epic],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        None => Err(ApiError::not_found("ticket", epic)),
        Some((p, _)) if p != project => Err(ApiError::validation(
            "validation.check_epic",
            format!("Ticket '{epic}' belongs to project '{p}', not '{project}'."),
        )
        .remedy("Pick an epic in this project, or omit 'epic'.".to_string())),
        Some((_, ty)) if ty != "epic" => Err(ApiError::validation(
            "validation.check_epic",
            format!(
                "Ticket '{epic}' has type '{ty}'; a check groups under a ticket of type 'epic'."
            ),
        )
        .remedy("Pass an epic's id, or omit 'epic' to leave the check ungrouped.".to_string())),
        Some(_) => Ok(()),
    }
}

fn row_to_check(row: &Row) -> rusqlite::Result<Check> {
    let metadata_raw: String = row.get("metadata")?;
    Ok(Check {
        id: row.get("id")?,
        project: row.get("project")?,
        epic: row.get("epic")?,
        title: row.get("title")?,
        body: row.get("body")?,
        precondition: row.get("precondition")?,
        layer: row.get("layer")?,
        severity: row.get("severity")?,
        verification: row.get("verification")?,
        expiry_days: row.get("expiry_days")?,
        expiry_releases: row.get("expiry_releases")?,
        cost_agent_minutes: row.get("cost_agent_minutes")?,
        cost_human_minutes: row.get("cost_human_minutes")?,
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        version: row.get("version")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
        globs: Vec::new(),
        counts: CheckCounts::default(),
        orphan_globs: Vec::new(),
        policy: None,
    })
}

fn row_to_case(row: &Row) -> rusqlite::Result<Case> {
    let assignment_raw: String = row.get("assignment")?;
    let seeded: i64 = row.get("seeded")?;
    Ok(Case {
        id: row.get("id")?,
        // The column is `check_id`; `CHECK` is a SQL keyword.
        check: row.get("check_id")?,
        key: row.get("key")?,
        label: row.get("label")?,
        assignment: serde_json::from_str(&assignment_raw).unwrap_or(Value::Null),
        seeded: seeded != 0,
        agent_verdict: row.get("agent_verdict")?,
        agent_at: row.get("agent_at")?,
        agent_by: row.get("agent_by")?,
        agent_release: row.get("agent_release")?,
        human_verdict: row.get("human_verdict")?,
        human_at: row.get("human_at")?,
        human_by: row.get("human_by")?,
        human_release: row.get("human_release")?,
        stale_since: row.get("stale_since")?,
        retired_at: row.get("retired_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

const CHECK_COLS: &str = "id, project, epic, title, body, precondition, layer, severity, \
    verification, expiry_days, expiry_releases, cost_agent_minutes, cost_human_minutes, \
    metadata, version, created_by, created_at, updated_at, archived_at";

const CASE_COLS: &str = "id, check_id, key, label, assignment, seeded, agent_verdict, agent_at, \
    agent_by, agent_release, human_verdict, human_at, human_by, human_release, stale_since, \
    retired_at, created_at, updated_at";

fn load_globs(conn: &Connection, check: &str) -> ApiResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT glob FROM check_globs WHERE check_id = ?1 ORDER BY glob")?;
    let out = stmt
        .query_map(params![check], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Live cases of a check, counted by state.
fn load_counts(conn: &Connection, check: &str) -> ApiResult<CheckCounts> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CASE_COLS} FROM cases WHERE check_id = ?1 AND retired_at IS NULL"
    ))?;
    let cases = stmt
        .query_map(params![check], row_to_case)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut counts = CheckCounts::default();
    for c in &cases {
        counts.add(c.state());
    }
    Ok(counts)
}

/// Globs of this check that matched nothing in the newest release. Reported by the
/// pusher, stored per release; surfaced on the check so the rot is visible where
/// the claim is made.
fn load_orphan_globs(conn: &Connection, project: &str, check: &str) -> ApiResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT g.glob FROM check_globs g
         WHERE g.check_id = ?1
           AND EXISTS (
             SELECT 1 FROM release_orphan_globs o
             JOIN releases r ON r.id = o.release
             WHERE o.glob = g.glob AND r.project = ?2
               AND r.seq = (SELECT MAX(seq) FROM releases WHERE project = ?2)
           )
         ORDER BY g.glob",
    )?;
    let out = stmt
        .query_map(params![check, project], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// One stored policy row's three override slots, each independently unset.
type PolicyRow = (Option<String>, Option<i64>, Option<i64>);

/// Resolve verification + expiry down project → epic → check.
fn resolve_policy(conn: &Connection, check: &Check) -> ApiResult<ResolvedPolicy> {
    let load = |epic: &str| -> ApiResult<Option<PolicyRow>> {
        let mut s = conn.prepare(
            "SELECT verification, expiry_days, expiry_releases FROM checklist_policies
             WHERE project = ?1 AND epic = ?2",
        )?;
        let row = s
            .query_row(params![check.project, epic], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        Ok(row)
    };

    let project_policy = load("")?;
    let epic_policy = match &check.epic {
        Some(e) => load(e)?,
        None => None,
    };

    let mut verification = DEFAULT_VERIFICATION.to_string();
    let mut verification_from = "default";
    let mut expiry_days = None;
    let mut expiry_releases = None;
    let mut expiry_from = "default";

    for (source, policy) in [("project", &project_policy), ("epic", &epic_policy)] {
        if let Some((v, d, r)) = policy {
            if let Some(v) = v {
                verification = v.clone();
                verification_from = source;
            }
            if d.is_some() || r.is_some() {
                expiry_days = *d;
                expiry_releases = *r;
                expiry_from = source;
            }
        }
    }
    if let Some(v) = &check.verification {
        verification = v.clone();
        verification_from = "check";
    }
    if check.expiry_days.is_some() || check.expiry_releases.is_some() {
        expiry_days = check.expiry_days;
        expiry_releases = check.expiry_releases;
        expiry_from = "check";
    }

    Ok(ResolvedPolicy {
        verification,
        verification_from: verification_from.to_string(),
        expiry_days,
        expiry_releases,
        expiry_from: expiry_from.to_string(),
    })
}

/// Has a verified case aged out? Time-based and release-count expiry both apply;
/// whichever trips first wins, which is why this is an `||` and not a choice.
fn case_expired(
    case: &Case,
    policy: &ResolvedPolicy,
    now: i64,
    latest_seq: Option<i64>,
    release_seq: &HashMap<String, i64>,
) -> bool {
    // Only something previously verified can expire. "Never verified" is its own
    // state and reporting it as expired would double-count the same gap.
    let last_at = match (case.human_at, case.agent_at) {
        (Some(h), Some(a)) => Some(h.max(a)),
        (Some(h), None) => Some(h),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    let Some(last_at) = last_at else {
        return false;
    };
    if let Some(days) = policy.expiry_days {
        if days > 0 && now.saturating_sub(last_at) >= days.saturating_mul(86_400_000) {
            return true;
        }
    }
    if let Some(limit) = policy.expiry_releases {
        if limit > 0 {
            // Take the release belonging to whichever verdict is the more recent,
            // not whichever column happens to be populated. A human approval at r1
            // followed by an agent re-run at r5 was last verified at r5; reading r1
            // would age the case out four releases early.
            let newest_release = if case.human_at >= case.agent_at {
                case.human_release
                    .as_deref()
                    .or(case.agent_release.as_deref())
            } else {
                case.agent_release
                    .as_deref()
                    .or(case.human_release.as_deref())
            };
            let verified_at_release = newest_release.and_then(|r| release_seq.get(r).copied());
            if let (Some(seq), Some(latest)) = (verified_at_release, latest_seq) {
                if latest.saturating_sub(seq) >= limit {
                    return true;
                }
            }
        }
    }
    false
}

impl Store {
    // -----------------------------------------------------------------------
    // Releases
    // -----------------------------------------------------------------------

    /// Record a release and apply its consequences: cases whose check claims a
    /// touched path go stale, and globs the pusher found empty are recorded so
    /// those checks stop counting as covered.
    ///
    /// The pusher supplies the diff and the empty globs because it has the tree
    /// checked out — the server clones nothing. That is the cheapest possible
    /// place to learn the truth, and it keeps releases something an agent pushes
    /// rather than an integration Takomo owns.
    pub fn push_release(
        &self,
        req: &ReleasePush,
        actor: &str,
    ) -> ApiResult<(Release, ReleaseImpact)> {
        let reference = req.reference.trim().to_string();
        if reference.is_empty() {
            return Err(ApiError::validation(
                "validation.release_ref",
                "A release needs a 'ref' — a tag or the full commit sha it stands for.",
            )
            .remedy("Send {\"ref\": \"v1.4.0\"} or the full sha.".to_string()));
        }
        validate_release_ref(&reference)?;
        if let Some(note) = &req.note {
            validate_release_note(note)?;
        }
        if req.touched_paths.len() > MAX_RELEASE_PATHS {
            return Err(ApiError::validation(
                "validation.release_paths",
                format!(
                    "This push carries {} touched paths; the maximum is {MAX_RELEASE_PATHS}.",
                    req.touched_paths.len()
                ),
            )
            .remedy(
                "A diff that large is a repository-wide sweep. Push the release without \
                 'touched_paths' and retest every check instead."
                    .to_string(),
            ));
        }

        let now = now_ms();
        let project = req.project.clone();
        let paths: Vec<String> = req
            .touched_paths
            .iter()
            .map(|p| p.trim().trim_start_matches("./").to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let orphans: Vec<String> = req
            .orphan_globs
            .iter()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect();

        self.with_tx(|tx| {
            project_exists(tx, &project)?;
            ensure_project_writable(tx, &project)?;

            let taken: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM releases WHERE project = ?1 AND ref = ?2",
                    params![project, reference],
                    |r| r.get(0),
                )
                .optional()?;
            if taken.is_some() {
                return Err(ApiError::conflict(
                    "conflict.release_exists",
                    format!("Release '{reference}' is already recorded for project '{project}'."),
                )
                .remedy(
                    "Releases are immutable markers. Push the next ref, or read the \
                     existing one instead of re-pushing it."
                        .to_string(),
                ));
            }

            let next_seq: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM releases WHERE project = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .unwrap_or(1);

            let id = release_id();
            tx.execute(
                "INSERT INTO releases (id, project, ref, seq, note, pushed_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, project, reference, next_seq, req.note, actor, now],
            )?;
            for p in &paths {
                tx.execute(
                    "INSERT OR IGNORE INTO release_paths (release, path) VALUES (?1, ?2)",
                    params![id, p],
                )?;
            }
            for g in &orphans {
                tx.execute(
                    "INSERT OR IGNORE INTO release_orphan_globs (release, glob) VALUES (?1, ?2)",
                    params![id, g],
                )?;
            }

            // Which checks does this diff touch? One pass over the project's globs
            // rather than a query per path.
            let mut check_stmt = tx.prepare(&format!(
                "SELECT {CHECK_COLS} FROM checks WHERE project = ?1 AND archived_at IS NULL"
            ))?;
            let checks = check_stmt
                .query_map(params![project], row_to_check)?
                .collect::<Result<Vec<_>, _>>()?;
            drop(check_stmt);

            let orphan_set: HashSet<&str> = orphans.iter().map(String::as_str).collect();
            let mut impact = ReleaseImpact::default();
            let release_seq = self.release_seq_map(tx, &project)?;

            for check in &checks {
                let globs = load_globs(tx, &check.id)?;
                let touched = globs
                    .iter()
                    .any(|g| paths.iter().any(|p| glob_matches(g, p)));
                if !globs.is_empty() && globs.iter().all(|g| orphan_set.contains(g.as_str())) {
                    impact.orphaned_checks.push(check.id.clone());
                }
                if touched {
                    // Only something previously verified can go stale. A case that
                    // was never verified stays `never`: calling it stale would
                    // report "needs re-testing" for work nobody has done once, and
                    // quietly shrink the never-tested gap this feature exists to
                    // show.
                    let n = tx.execute(
                        "UPDATE cases SET stale_since = ?1, updated_at = ?2
                         WHERE check_id = ?3 AND retired_at IS NULL AND stale_since IS NULL
                           AND (agent_verdict IS NOT NULL OR human_verdict IS NOT NULL)",
                        params![id, now, check.id],
                    )?;
                    if n > 0 {
                        impact.stale_cases += n as i64;
                        impact.stale_checks.push(check.id.clone());
                    }
                    continue;
                }
                // Not touched by the diff, but its policy clock may have run out.
                let policy = resolve_policy(tx, check)?;
                if policy.expiry_days.is_none() && policy.expiry_releases.is_none() {
                    continue;
                }
                let mut stmt = tx.prepare(&format!(
                    "SELECT {CASE_COLS} FROM cases WHERE check_id = ?1 AND retired_at IS NULL"
                ))?;
                let cases = stmt
                    .query_map(params![check.id], row_to_case)?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                let expired: Vec<&Case> = cases
                    .iter()
                    .filter(|c| {
                        c.stale_since.is_none()
                            && case_expired(c, &policy, now, Some(next_seq), &release_seq)
                    })
                    .collect();
                if !expired.is_empty() {
                    for c in &expired {
                        tx.execute(
                            "UPDATE cases SET stale_since = ?1, updated_at = ?2 WHERE id = ?3",
                            params![id, now, c.id],
                        )?;
                    }
                    impact.stale_cases += expired.len() as i64;
                    impact.expired_checks.push(check.id.clone());
                }
            }

            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "release.pushed",
                json!({
                    "release": id,
                    "ref": reference,
                    "seq": next_seq,
                    "touched_paths": paths.len(),
                    "orphan_globs": orphans.len(),
                    "impact": impact.to_json(),
                }),
                now,
            )?;

            Ok((
                Release {
                    id,
                    project: project.clone(),
                    reference: reference.clone(),
                    seq: next_seq,
                    note: req.note.clone(),
                    pushed_by: actor.to_string(),
                    created_at: now,
                    path_count: paths.len() as i64,
                    orphan_globs: orphans.clone(),
                },
                impact,
            ))
        })
    }

    fn release_seq_map(&self, conn: &Connection, project: &str) -> ApiResult<HashMap<String, i64>> {
        let mut stmt = conn.prepare("SELECT id, seq FROM releases WHERE project = ?1")?;
        let rows = stmt
            .query_map(params![project], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().collect())
    }

    pub fn list_releases(&self, project: &str, limit: usize) -> ApiResult<Vec<Release>> {
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let mut stmt = conn.prepare(
                "SELECT id, project, ref, seq, note, pushed_by, created_at,
                        (SELECT COUNT(*) FROM release_paths WHERE release = releases.id) AS path_count
                 FROM releases WHERE project = ?1 ORDER BY seq DESC LIMIT ?2",
            )?;
            let mut out = stmt
                .query_map(params![project, limit as i64], |r| {
                    Ok(Release {
                        id: r.get(0)?,
                        project: r.get(1)?,
                        reference: r.get(2)?,
                        seq: r.get(3)?,
                        note: r.get(4)?,
                        pushed_by: r.get(5)?,
                        created_at: r.get(6)?,
                        path_count: r.get(7)?,
                        orphan_globs: Vec::new(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for rel in &mut out {
                let mut s = conn
                    .prepare("SELECT glob FROM release_orphan_globs WHERE release = ?1 ORDER BY glob")?;
                rel.orphan_globs = s
                    .query_map(params![rel.id], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Ok(out)
        })
    }

    // -----------------------------------------------------------------------
    // Checks
    // -----------------------------------------------------------------------

    pub fn create_check(&self, req: &CheckCreate, actor: &str) -> ApiResult<Check> {
        let title = req.title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::validation(
                "validation.check_title",
                "A check needs a 'title' naming the one action it verifies.",
            )
            .remedy("Send {\"title\": \"Create a claim\"}.".to_string()));
        }
        validate_check_title(&title)?;
        validate_check_body(&req.body)?;
        validate_check_precondition(&req.precondition)?;
        let layer = req.layer.clone().unwrap_or_else(|| "api".to_string());
        validate_layer(&layer)?;
        let severity = req
            .severity
            .clone()
            .unwrap_or_else(|| "advisory".to_string());
        validate_severity(&severity)?;
        if let Some(v) = &req.verification {
            validate_verification(v)?;
        }
        for (f, v) in [
            ("expiry_days", req.expiry_days),
            ("expiry_releases", req.expiry_releases),
            ("cost_agent_minutes", req.cost_agent_minutes),
            ("cost_human_minutes", req.cost_human_minutes),
        ] {
            if let Some(v) = v {
                validate_check_number(f, v)?;
            }
        }
        let globs = normalize_globs(&req.globs)?;
        let metadata = req.metadata.clone().unwrap_or(Value::Null);
        let metadata_raw = metadata.to_string();
        if metadata_raw.len() > MAX_METADATA {
            return Err(ApiError::validation(
                "validation.check_metadata_size",
                format!(
                    "'metadata' is {} bytes; the maximum is {MAX_METADATA}.",
                    metadata_raw.len()
                ),
            )
            .remedy("Move the bulk into the check 'body'.".to_string()));
        }

        let now = now_ms();
        let id = check_id();
        let project = req.project.clone();
        let epic = req.epic.clone();

        self.with_tx(|tx| {
            project_exists(tx, &project)?;
            ensure_project_writable(tx, &project)?;
            if let Some(e) = &epic {
                validate_epic(tx, &project, e)?;
            }
            tx.execute(
                "INSERT INTO checks (id, project, epic, title, body, precondition, layer, severity,
                    verification, expiry_days, expiry_releases, cost_agent_minutes,
                    cost_human_minutes, metadata, version, created_by, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1,?15,?16,?16)",
                params![
                    id,
                    project,
                    epic,
                    title,
                    req.body,
                    req.precondition,
                    layer,
                    severity,
                    req.verification,
                    req.expiry_days,
                    req.expiry_releases,
                    req.cost_agent_minutes,
                    req.cost_human_minutes,
                    metadata_raw,
                    actor,
                    now,
                ],
            )?;
            for g in &globs {
                tx.execute(
                    "INSERT OR IGNORE INTO check_globs (check_id, glob) VALUES (?1, ?2)",
                    params![id, g],
                )?;
            }
            emit_event(
                tx,
                epic.as_deref(),
                Some(&project),
                actor,
                "checklist.check_created",
                json!({ "check": id, "title": title, "layer": layer, "severity": severity }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let mut check = stmt.query_row(params![id], row_to_check)?;
            check.globs = globs.clone();
            check.policy = Some(resolve_policy(tx, &check)?);
            Ok(check)
        })
    }

    /// Checks for a project, plus how many matched before the page size applied.
    ///
    /// The cap is not paranoia: a project accumulates a check per action per
    /// layer, each hydrated below with three further queries, and the whole set
    /// used to be returned and hydrated however large it grew.
    ///
    /// `severity` and `layer` are filtered in Rust rather than SQL (they live
    /// behind `row_to_check`), so the page size is applied *after* that filter and
    /// not as a `LIMIT` on the query — a `LIMIT` would cap the rows before
    /// narrowing them and hand back a page shorter than the caller asked for,
    /// with no way to tell that from the end of the list. Hydration then runs
    /// only over the rows actually returned, so asking for ten checks out of two
    /// hundred costs ten checks' worth of queries instead of two hundred.
    pub fn list_checks(&self, filter: &CheckFilter) -> ApiResult<(Vec<Check>, i64)> {
        let limit = filter
            .limit
            .unwrap_or(MAX_CHECKS_PAGE)
            .clamp(1, MAX_CHECKS_PAGE);
        self.with_conn(|conn| {
            project_exists(conn, &filter.project)?;
            let mut sql = format!("SELECT {CHECK_COLS} FROM checks WHERE project = ?1");
            if !filter.include_archived {
                sql.push_str(" AND archived_at IS NULL");
            }
            if let Some(e) = &filter.epic {
                sql.push_str(if e.is_empty() {
                    " AND epic IS NULL"
                } else {
                    " AND epic = ?2"
                });
            }
            sql.push_str(" ORDER BY COALESCE(epic, ''), title");
            let mut stmt = conn.prepare(&sql)?;
            let mut checks: Vec<Check> = match &filter.epic {
                Some(e) if !e.is_empty() => stmt
                    .query_map(params![filter.project, e], row_to_check)?
                    .collect::<Result<Vec<_>, _>>()?,
                _ => stmt
                    .query_map(params![filter.project], row_to_check)?
                    .collect::<Result<Vec<_>, _>>()?,
            };
            checks.retain(|l| {
                filter.severity.as_deref().is_none_or(|s| s == l.severity)
                    && filter.layer.as_deref().is_none_or(|s| s == l.layer)
            });
            let total = checks.len() as i64;
            checks.truncate(limit as usize);
            for check in &mut checks {
                check.globs = load_globs(conn, &check.id)?;
                check.counts = load_counts(conn, &check.id)?;
                check.orphan_globs = load_orphan_globs(conn, &filter.project, &check.id)?;
                if filter.with_policy {
                    check.policy = Some(resolve_policy(conn, check)?);
                }
            }
            Ok((checks, total))
        })
    }

    pub fn get_check(&self, id: &str) -> ApiResult<Check> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let mut check = stmt
                .query_row(params![id], row_to_check)
                .optional()?
                .ok_or_else(|| ApiError::not_found("check", id))?;
            check.globs = load_globs(conn, id)?;
            check.counts = load_counts(conn, id)?;
            check.orphan_globs = load_orphan_globs(conn, &check.project, id)?;
            check.policy = Some(resolve_policy(conn, &check)?);
            Ok(check)
        })
    }

    pub fn patch_check(&self, id: &str, patch: &CheckPatch, actor: &str) -> ApiResult<Check> {
        if let Some(t) = &patch.title {
            if t.trim().is_empty() {
                return Err(ApiError::validation(
                    "validation.check_title",
                    "'title' cannot be blanked; a check must name the action it verifies.",
                )
                .remedy("Send a non-empty title, or omit the field.".to_string()));
            }
            validate_check_title(t)?;
        }
        if let Some(b) = &patch.body {
            validate_check_body(b)?;
        }
        if let Some(p) = &patch.precondition {
            validate_check_precondition(p)?;
        }
        if let Some(l) = &patch.layer {
            validate_layer(l)?;
        }
        if let Some(s) = &patch.severity {
            validate_severity(s)?;
        }
        if let Some(Some(v)) = &patch.verification {
            validate_verification(v)?;
        }
        let globs = match &patch.globs {
            Some(g) => Some(normalize_globs(g)?),
            None => None,
        };

        let now = now_ms();
        let id = id.to_string();
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let existing = stmt
                .query_row(params![id], row_to_check)
                .optional()?
                .ok_or_else(|| ApiError::not_found("check", &id))?;
            drop(stmt);
            ensure_project_writable(tx, &existing.project)?;

            if let Some(Some(e)) = &patch.epic {
                validate_epic(tx, &existing.project, e)?;
            }

            macro_rules! set_plain {
                ($field:ident, $col:literal) => {
                    if let Some(v) = &patch.$field {
                        tx.execute(
                            concat!("UPDATE checks SET ", $col, " = ?1 WHERE id = ?2"),
                            params![v, id],
                        )?;
                    }
                };
            }
            set_plain!(title, "title");
            set_plain!(body, "body");
            set_plain!(precondition, "precondition");
            set_plain!(layer, "layer");
            set_plain!(severity, "severity");

            macro_rules! set_override {
                ($field:ident, $col:literal) => {
                    if let Some(v) = &patch.$field {
                        tx.execute(
                            concat!("UPDATE checks SET ", $col, " = ?1 WHERE id = ?2"),
                            params![v, id],
                        )?;
                    }
                };
            }
            set_override!(verification, "verification");
            set_override!(expiry_days, "expiry_days");
            set_override!(expiry_releases, "expiry_releases");
            set_override!(cost_agent_minutes, "cost_agent_minutes");
            set_override!(cost_human_minutes, "cost_human_minutes");
            set_override!(epic, "epic");

            if let Some(merge) = &patch.metadata_merge {
                let mut merged = existing.metadata.clone();
                super::merge_patch(&mut merged, merge);
                let raw = merged.to_string();
                if raw.len() > MAX_METADATA {
                    return Err(ApiError::validation(
                        "validation.check_metadata_size",
                        format!(
                            "'metadata' would be {} bytes; the maximum is {MAX_METADATA}.",
                            raw.len()
                        ),
                    )
                    .remedy("Move the bulk into the check 'body'.".to_string()));
                }
                tx.execute(
                    "UPDATE checks SET metadata = ?1 WHERE id = ?2",
                    params![raw, id],
                )?;
            }
            if let Some(globs) = &globs {
                tx.execute("DELETE FROM check_globs WHERE check_id = ?1", params![id])?;
                for g in globs {
                    tx.execute(
                        "INSERT OR IGNORE INTO check_globs (check_id, glob) VALUES (?1, ?2)",
                        params![id, g],
                    )?;
                }
            }
            tx.execute(
                "UPDATE checks SET version = version + 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            emit_event(
                tx,
                None,
                Some(&existing.project),
                actor,
                "checklist.check_updated",
                json!({ "check": id }),
                now,
            )?;

            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let mut check = stmt.query_row(params![id], row_to_check)?;
            drop(stmt);
            check.globs = load_globs(tx, &id)?;
            check.counts = load_counts(tx, &id)?;
            check.policy = Some(resolve_policy(tx, &check)?);
            Ok(check)
        })
    }

    /// Archive a check. Its cases and their verdict history stay: a check that is no
    /// longer worth running is still evidence of what was once verified.
    pub fn archive_check(&self, id: &str, actor: &str) -> ApiResult<Check> {
        let now = now_ms();
        let id = id.to_string();
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let existing = stmt
                .query_row(params![id], row_to_check)
                .optional()?
                .ok_or_else(|| ApiError::not_found("check", &id))?;
            drop(stmt);
            ensure_project_writable(tx, &existing.project)?;
            tx.execute(
                "UPDATE checks SET archived_at = ?1, updated_at = ?1, version = version + 1
                 WHERE id = ?2 AND archived_at IS NULL",
                params![now, id],
            )?;
            emit_event(
                tx,
                None,
                Some(&existing.project),
                actor,
                "checklist.check_archived",
                json!({ "check": id }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let mut check = stmt.query_row(params![id], row_to_check)?;
            drop(stmt);
            check.globs = load_globs(tx, &id)?;
            check.counts = load_counts(tx, &id)?;
            Ok(check)
        })
    }

    // -----------------------------------------------------------------------
    // Cases
    // -----------------------------------------------------------------------

    /// File the generated case set for a check.
    ///
    /// Upsert by `key`, which the agent derives from the parameter assignment. A
    /// case still present keeps its id and its verdicts; one that disappeared is
    /// retired rather than deleted; one that reappears is revived. That is what
    /// makes regenerating a model after adding a parameter safe — history follows
    /// the assignment, not the row order.
    ///
    /// `prune` false files additions without retiring anything, for an agent that
    /// is extending a set rather than replacing it.
    pub fn file_cases(
        &self,
        check: &str,
        cases: &[CaseInput],
        prune: bool,
        actor: &str,
    ) -> ApiResult<CaseFileOutcome> {
        if cases.len() > MAX_CASES_PER_FILE {
            return Err(ApiError::validation(
                "validation.case_count",
                format!(
                    "This call files {} cases; the maximum is {MAX_CASES_PER_FILE}.",
                    cases.len()
                ),
            )
            .remedy(
                "A pairwise model for a large form lands near 76 cases. If you have \
                 thousands, the parameter set is probably wrong — most form fields are \
                 inert and do not belong in the model."
                    .to_string(),
            ));
        }
        let mut seen = HashSet::new();
        for c in cases {
            let key = c.key.trim();
            if key.is_empty() {
                return Err(ApiError::validation(
                    "validation.case_key",
                    "Every case needs a 'key' — a stable identity derived from its parameter assignment.",
                )
                .remedy(
                    "Derive the key from the assignment (a hash or a canonical join), so \
                     regenerating the model matches existing cases instead of orphaning them."
                        .to_string(),
                ));
            }
            validate_case_key_len(key)?;
            validate_case_label(&c.label)?;
            if !seen.insert(key.to_string()) {
                return Err(ApiError::validation(
                    "validation.case_key",
                    format!("Case key '{key}' appears more than once in this call."),
                )
                .remedy(
                    "Keys identify a case within its check, so they must be unique. Two \
                     identical assignments are one case."
                        .to_string(),
                ));
            }
        }

        let now = now_ms();
        let check = check.to_string();
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(&format!("SELECT {CHECK_COLS} FROM checks WHERE id = ?1"))?;
            let check_row = stmt
                .query_row(params![check], row_to_check)
                .optional()?
                .ok_or_else(|| ApiError::not_found("check", &check))?;
            ensure_project_writable(tx, &check_row.project)?;
            drop(stmt);

            let mut existing: HashMap<String, (String, Option<i64>)> = HashMap::new();
            {
                let mut s =
                    tx.prepare("SELECT key, id, retired_at FROM cases WHERE check_id = ?1")?;
                for row in s.query_map(params![check], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                    ))
                })? {
                    let (k, id, retired) = row?;
                    existing.insert(k, (id, retired));
                }
            }

            let mut out = CaseFileOutcome::default();
            for c in cases {
                let key = c.key.trim();
                let assignment = if c.assignment.is_null() {
                    json!({})
                } else {
                    c.assignment.clone()
                };
                let assignment_raw = assignment.to_string();
                match existing.get(key) {
                    Some((id, retired)) => {
                        tx.execute(
                            "UPDATE cases SET label = ?1, assignment = ?2, seeded = ?3,
                                 retired_at = NULL, updated_at = ?4 WHERE id = ?5",
                            params![c.label, assignment_raw, c.seeded as i64, now, id],
                        )?;
                        if retired.is_some() {
                            out.revived += 1;
                        } else {
                            out.updated += 1;
                        }
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO cases (id, check_id, key, label, assignment, seeded,
                                 created_at, updated_at)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
                            params![
                                case_id(),
                                check,
                                key,
                                c.label,
                                assignment_raw,
                                c.seeded as i64,
                                now
                            ],
                        )?;
                        out.added += 1;
                    }
                }
            }

            if prune {
                let filed: HashSet<&str> = cases.iter().map(|c| c.key.trim()).collect();
                for (key, (id, retired)) in &existing {
                    if retired.is_none() && !filed.contains(key.as_str()) {
                        tx.execute(
                            "UPDATE cases SET retired_at = ?1, updated_at = ?1 WHERE id = ?2",
                            params![now, id],
                        )?;
                        out.retired += 1;
                    }
                }
            }

            out.live = tx.query_row(
                "SELECT COUNT(*) FROM cases WHERE check_id = ?1 AND retired_at IS NULL",
                params![check],
                |r| r.get(0),
            )?;

            emit_event(
                tx,
                None,
                Some(&check_row.project),
                actor,
                "checklist.cases_filed",
                json!({ "check": check, "outcome": out.to_json() }),
                now,
            )?;
            Ok(out)
        })
    }

    /// One page of a check's cases, plus how many the check holds in total.
    ///
    /// Bounded because cases are *generated*: a PICT model over a form of any
    /// size lands up to [`MAX_CASES_PER_FILE`] rows under one check, and this used
    /// to return all of them. `limit` is `None` for the full page size, which is
    /// also the ceiling ([`MAX_CASES_PAGE`]); `offset` pages through the rest.
    ///
    /// An offset cursor is honest here in a way it is not for the ready queue:
    /// cases are ordered by `key`, which is stable and does not reshuffle as
    /// other agents work, so page 2 means the same thing on the second read.
    pub fn list_cases(
        &self,
        check: &str,
        include_retired: bool,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> ApiResult<(Vec<Case>, i64)> {
        let limit = limit.unwrap_or(MAX_CASES_PAGE).clamp(1, MAX_CASES_PAGE);
        let offset = offset.unwrap_or(0).max(0);
        self.with_conn(|conn| {
            let exists: Option<i64> = conn
                .query_row("SELECT 1 FROM checks WHERE id = ?1", params![check], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("check", check));
            }
            let retired_clause = if include_retired {
                ""
            } else {
                " AND retired_at IS NULL"
            };
            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM cases WHERE check_id = ?1{retired_clause}"),
                params![check],
                |r| r.get(0),
            )?;
            let sql = format!(
                "SELECT {CASE_COLS} FROM cases WHERE check_id = ?1{retired_clause} \
                 ORDER BY key LIMIT ?2 OFFSET ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let out = stmt
                .query_map(params![check, limit, offset], row_to_case)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((out, total))
        })
    }

    pub fn get_case(&self, id: &str) -> ApiResult<(Case, Vec<CaseVerdict>)> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!("SELECT {CASE_COLS} FROM cases WHERE id = ?1"))?;
            let case = stmt
                .query_row(params![id], row_to_case)
                .optional()?
                .ok_or_else(|| ApiError::not_found("case", id))?;
            drop(stmt);
            let mut stmt = conn.prepare(
                // Newest first, tiebroken on rowid — i.e. insertion order.
                //
                // NOT on `id`: a verdict id is `cv-` plus eight RANDOM base36
                // characters, so tiebreaking on it was a coin flip rather than an
                // order. Two verdicts recorded in the same millisecond — which is
                // exactly what an agent pass followed by a human pass does — came
                // back in either order, so "newest first" was only true about half
                // the time.
                "SELECT id, case_id, actor_kind, actor, verdict, note, release, at
                 FROM case_verdicts WHERE case_id = ?1 ORDER BY at DESC, rowid DESC",
            )?;
            let history = stmt
                .query_map(params![id], |r| {
                    Ok(CaseVerdict {
                        id: r.get(0)?,
                        case_id: r.get(1)?,
                        actor_kind: r.get(2)?,
                        actor: r.get(3)?,
                        verdict: r.get(4)?,
                        note: r.get(5)?,
                        release: r.get(6)?,
                        at: r.get(7)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((case, history))
        })
    }

    // -----------------------------------------------------------------------
    // Verdicts
    // -----------------------------------------------------------------------

    /// Record a verdict. The agent's and the human's are separate facts, kept in
    /// separate columns: a policy of `agent_then_human` needs both, and collapsing
    /// them would make "a person looked at this" unrecoverable.
    ///
    /// A verdict clears `stale_since` — that is the whole point of re-running it.
    pub fn record_verdict(
        &self,
        case: &str,
        actor_kind: &str,
        actor: &str,
        verdict: &str,
        note: Option<&str>,
        release: Option<&str>,
    ) -> ApiResult<Case> {
        validate_verdict(verdict)?;
        validate_actor_kind(actor_kind)?;
        if let Some(n) = note {
            validate_verdict_note(n)?;
        }
        if verdict == "fail" && note.map(str::trim).unwrap_or("").is_empty() {
            return Err(ApiError::validation(
                "validation.verdict_note",
                "A 'fail' verdict needs a 'note' saying what went wrong.",
            )
            .remedy(
                "Describe what you observed. A bare fail is a claim the next reader \
                 cannot act on."
                    .to_string(),
            ));
        }

        let now = now_ms();
        let case = case.to_string();
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(&format!("SELECT {CASE_COLS} FROM cases WHERE id = ?1"))?;
            let existing = stmt
                .query_row(params![case], row_to_case)
                .optional()?
                .ok_or_else(|| ApiError::not_found("case", &case))?;
            drop(stmt);
            // A case names its check, and the check names the project — the only
            // hop between a verdict and the gate it has to pass.
            let case_project: String = tx.query_row(
                "SELECT project FROM checks WHERE id = ?1",
                params![existing.check],
                |r| r.get(0),
            )?;
            ensure_project_writable(tx, &case_project)?;
            if existing.retired_at.is_some() {
                return Err(ApiError::conflict(
                    "conflict.case_retired",
                    format!(
                        "Case '{case}' was retired by a later regeneration; it is history, not work."
                    ),
                )
                .remedy(
                    "File the current case set for this check and record verdicts against \
                     a live case."
                        .to_string(),
                ));
            }
            let project: String = tx.query_row(
                "SELECT project FROM checks WHERE id = ?1",
                params![existing.check],
                |r| r.get(0),
            )?;
            if let Some(rel) = release {
                let ok: Option<i64> = tx
                    .query_row(
                        "SELECT 1 FROM releases WHERE id = ?1 AND project = ?2",
                        params![rel, project],
                        |r| r.get(0),
                    )
                    .optional()?;
                if ok.is_none() {
                    return Err(ApiError::not_found("release", rel));
                }
            }

            if actor_kind == "agent" {
                tx.execute(
                    "UPDATE cases SET agent_verdict = ?1, agent_at = ?2, agent_by = ?3,
                         agent_release = ?4, stale_since = NULL, updated_at = ?2 WHERE id = ?5",
                    params![verdict, now, actor, release, case],
                )?;
            } else {
                tx.execute(
                    "UPDATE cases SET human_verdict = ?1, human_at = ?2, human_by = ?3,
                         human_release = ?4, stale_since = NULL, updated_at = ?2 WHERE id = ?5",
                    params![verdict, now, actor, release, case],
                )?;
            }
            tx.execute(
                "INSERT INTO case_verdicts (id, case_id, actor_kind, actor, verdict, note, release, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![verdict_id(), case, actor_kind, actor, verdict, note, release, now],
            )?;
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "checklist.verdict",
                json!({
                    "case": case,
                    "check": existing.check,
                    "actor_kind": actor_kind,
                    "verdict": verdict,
                    "release": release,
                }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!("SELECT {CASE_COLS} FROM cases WHERE id = ?1"))?;
            let out = stmt.query_row(params![case], row_to_case)?;
            Ok(out)
        })
    }

    // -----------------------------------------------------------------------
    // Policy
    // -----------------------------------------------------------------------

    /// Set the project-level default (`epic` empty) or an epic-level override.
    pub fn set_checklist_policy(
        &self,
        project: &str,
        epic: Option<&str>,
        input: &PolicyInput,
        actor: &str,
    ) -> ApiResult<Value> {
        if let Some(Some(v)) = &input.verification {
            validate_verification(v)?;
        }
        for (f, v) in [
            ("expiry_days", &input.expiry_days),
            ("expiry_releases", &input.expiry_releases),
        ] {
            if let Some(Some(v)) = v {
                validate_policy_number(f, *v)?;
            }
        }
        let now = now_ms();
        let project = project.to_string();
        let scope = epic.unwrap_or("").to_string();
        self.with_tx(|tx| {
            project_exists(tx, &project)?;
            ensure_project_writable(tx, &project)?;
            if !scope.is_empty() {
                validate_epic(tx, &project, &scope)?;
            }
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM checklist_policies WHERE project = ?1 AND epic = ?2",
                    params![project, scope],
                    |r| r.get(0),
                )
                .optional()?;
            let id = existing.unwrap_or_else(checklist_policy_id);
            tx.execute(
                "INSERT INTO checklist_policies (id, project, epic, verification, expiry_days,
                     expiry_releases, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(project, epic) DO UPDATE SET
                     verification = COALESCE(excluded.verification, verification),
                     expiry_days = excluded.expiry_days,
                     expiry_releases = excluded.expiry_releases,
                     updated_at = excluded.updated_at",
                params![
                    id,
                    project,
                    scope,
                    input.verification.clone().flatten(),
                    input.expiry_days.flatten(),
                    input.expiry_releases.flatten(),
                    now
                ],
            )?;
            // An explicit null clears the override rather than inheriting the old value.
            if let Some(None) = input.verification {
                tx.execute(
                    "UPDATE checklist_policies SET verification = NULL WHERE id = ?1",
                    params![id],
                )?;
            }
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "checklist.policy_set",
                json!({ "epic": if scope.is_empty() { Value::Null } else { json!(scope) } }),
                now,
            )?;
            let row = tx.query_row(
                "SELECT verification, expiry_days, expiry_releases FROM checklist_policies
                 WHERE id = ?1",
                params![id],
                |r| {
                    Ok(json!({
                        "project": project,
                        "epic": if scope.is_empty() { Value::Null } else { json!(scope) },
                        "verification": r.get::<_, Option<String>>(0)?,
                        "expiry_days": r.get::<_, Option<i64>>(1)?,
                        "expiry_releases": r.get::<_, Option<i64>>(2)?,
                    }))
                },
            )?;
            Ok(row)
        })
    }

    pub fn list_checklist_policies(&self, project: &str) -> ApiResult<Vec<Value>> {
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let mut stmt = conn.prepare(
                "SELECT epic, verification, expiry_days, expiry_releases FROM checklist_policies
                 WHERE project = ?1 ORDER BY epic",
            )?;
            let out = stmt
                .query_map(params![project], |r| {
                    let epic: String = r.get(0)?;
                    Ok(json!({
                        "project": project,
                        "epic": if epic.is_empty() { Value::Null } else { json!(epic) },
                        "verification": r.get::<_, Option<String>>(1)?,
                        "expiry_days": r.get::<_, Option<i64>>(2)?,
                        "expiry_releases": r.get::<_, Option<i64>>(3)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(out)
        })
    }

    // -----------------------------------------------------------------------
    // Coverage, worklist, gate
    // -----------------------------------------------------------------------

    /// Coverage for a project, rolled up per epic.
    ///
    /// `unreachable` is counted apart from both covered and uncovered on purpose:
    /// calling it a gap reports work nobody can do, and calling it covered claims
    /// verification of code no path reaches.
    pub fn checklist_coverage(&self, project: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let mut stmt = conn.prepare(&format!(
                "SELECT {CHECK_COLS} FROM checks WHERE project = ?1 AND archived_at IS NULL"
            ))?;
            let mut checks = stmt
                .query_map(params![project], row_to_check)?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            for check in &mut checks {
                check.globs = load_globs(conn, &check.id)?;
                check.counts = load_counts(conn, &check.id)?;
                check.orphan_globs = load_orphan_globs(conn, project, &check.id)?;
                check.policy = Some(resolve_policy(conn, check)?);
            }

            let latest: Option<(String, String, i64)> = conn
                .query_row(
                    "SELECT id, ref, seq FROM releases WHERE project = ?1 ORDER BY seq DESC LIMIT 1",
                    params![project],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;

            let mut total = CheckCounts::default();
            let mut by_epic: HashMap<String, (CheckCounts, i64, Vec<String>)> = HashMap::new();
            for check in &checks {
                let key = check.epic.clone().unwrap_or_default();
                let slot = by_epic.entry(key).or_insert((CheckCounts::default(), 0, Vec::new()));
                slot.1 += 1;
                slot.0.total += check.counts.total;
                slot.0.approved += check.counts.approved;
                slot.0.verified += check.counts.verified;
                slot.0.stale += check.counts.stale;
                slot.0.failed += check.counts.failed;
                slot.0.unreachable += check.counts.unreachable;
                slot.0.never += check.counts.never;
                slot.2.extend(check.orphan_globs.iter().cloned());
                total.total += check.counts.total;
                total.approved += check.counts.approved;
                total.verified += check.counts.verified;
                total.stale += check.counts.stale;
                total.failed += check.counts.failed;
                total.unreachable += check.counts.unreachable;
                total.never += check.counts.never;
            }

            let mut epics: Vec<Value> = by_epic
                .into_iter()
                .map(|(epic, (counts, check_count, orphans))| {
                    let title: Option<String> = if epic.is_empty() {
                        None
                    } else {
                        conn.query_row(
                            "SELECT title FROM tickets WHERE id = ?1",
                            params![epic],
                            |r| r.get(0),
                        )
                        .optional()
                        .ok()
                        .flatten()
                    };
                    json!({
                        "epic": if epic.is_empty() { Value::Null } else { json!(epic) },
                        "title": title,
                        "checks": check_count,
                        "cases": counts.to_json(),
                        "percent": percent(&counts),
                        "orphan_globs": orphans,
                    })
                })
                .collect();
            epics.sort_by(|a, b| {
                a["epic"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["epic"].as_str().unwrap_or(""))
            });

            Ok(json!({
                "project": project,
                "release": latest.as_ref().map(|(id, r, seq)| json!({ "id": id, "ref": r, "seq": seq })),
                "checks": checks.len(),
                "cases": total.to_json(),
                "percent": percent(&total),
                "epics": epics,
                "check_detail": checks.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
            }))
        })
    }

    /// What must be re-verified, split by who can clear it.
    ///
    /// Human time is the scarce resource — a hundred cases cost an agent minutes
    /// and cost a person a day — so the split is the product, not a formatting
    /// choice.
    pub fn checklist_worklist(&self, project: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            project_exists(conn, project)?;
            let mut stmt = conn.prepare(&format!(
                "SELECT {CHECK_COLS} FROM checks WHERE project = ?1 AND archived_at IS NULL"
            ))?;
            let mut checks = stmt
                .query_map(params![project], row_to_check)?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);

            let latest: Option<(String, i64)> = conn
                .query_row(
                    "SELECT id, seq FROM releases WHERE project = ?1 ORDER BY seq DESC LIMIT 1",
                    params![project],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let latest_seq = latest.as_ref().map(|(_, s)| *s);
            let release_seq = self.release_seq_map(conn, project)?;
            let now = now_ms();

            let mut agent_items = Vec::new();
            let mut human_items = Vec::new();
            for check in &mut checks {
                let policy = resolve_policy(conn, check)?;
                let mut stmt = conn.prepare(&format!(
                    "SELECT {CASE_COLS} FROM cases WHERE check_id = ?1 AND retired_at IS NULL ORDER BY key"
                ))?;
                let cases = stmt
                    .query_map(params![check.id], row_to_case)?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for case in &cases {
                    let expired = case_expired(case, &policy, now, latest_seq, &release_seq);
                    let reason = if case.stale_since.is_some() {
                        "stale"
                    } else if expired {
                        "expired"
                    } else if case.state() == "failed" {
                        "failed"
                    } else if !case.satisfies(&policy) {
                        if case.state() == "never" {
                            "never"
                        } else {
                            "awaiting_human"
                        }
                    } else {
                        continue;
                    };
                    // Who can clear it: anything needing a human's signature goes to
                    // the human list, and a stale case under agent_then_human needs
                    // the agent first, so it appears on the agent list until it has
                    // a fresh agent verdict.
                    let needs_agent_first = policy.verification == "agent_then_human"
                        && (case.stale_since.is_some()
                            || expired
                            || case.agent_verdict.as_deref() != Some("pass"));
                    let to_human = policy.needs_human() && !needs_agent_first;
                    let item = WorkItem {
                        check: check.id.clone(),
                        check_title: check.title.clone(),
                        severity: check.severity.clone(),
                        layer: check.layer.clone(),
                        case: case.id.clone(),
                        case_key: case.key.clone(),
                        case_label: case.label.clone(),
                        reason,
                        verification: policy.verification.clone(),
                        cost_minutes: if to_human {
                            check.cost_human_minutes
                        } else {
                            check.cost_agent_minutes
                        },
                    };
                    if to_human {
                        human_items.push(item);
                    } else {
                        agent_items.push(item);
                    }
                }
            }

            let rank = |s: &str| match s {
                "blocking" => 0,
                "advisory" => 1,
                _ => 2,
            };
            for list in [&mut agent_items, &mut human_items] {
                list.sort_by(|a, b| {
                    rank(&a.severity)
                        .cmp(&rank(&b.severity))
                        .then_with(|| a.check_title.cmp(&b.check_title))
                        .then_with(|| a.case_key.cmp(&b.case_key))
                });
            }

            Ok(json!({
                "project": project,
                "release": latest.as_ref().map(|(id, seq)| json!({ "id": id, "seq": seq })),
                "agent": {
                    "cases": agent_items.len(),
                    "minutes": sum_minutes(&agent_items),
                    "items": agent_items.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
                },
                "human": {
                    "cases": human_items.len(),
                    "minutes": sum_minutes(&human_items),
                    "items": human_items.iter().map(|i| i.to_json()).collect::<Vec<_>>(),
                },
            }))
        })
    }

    /// Is the project's verification good enough to ship?
    ///
    /// Only `blocking` severity blocks. Advisory and low checks nag: an unverified
    /// low-severity flow should not stop a release, or the gate gets overridden
    /// out of habit and stops meaning anything.
    pub fn checklist_gate(&self, project: &str) -> ApiResult<Value> {
        let worklist = self.checklist_worklist(project)?;
        let mut blocking_agent = Vec::new();
        let mut blocking_human = Vec::new();
        for (bucket, sink) in [
            ("agent", &mut blocking_agent),
            ("human", &mut blocking_human),
        ] {
            if let Some(items) = worklist[bucket]["items"].as_array() {
                for i in items {
                    if i["severity"] == "blocking" {
                        sink.push(i.clone());
                    }
                }
            }
        }
        let blocked = !blocking_agent.is_empty() || !blocking_human.is_empty();
        Ok(json!({
            "project": project,
            "release": worklist["release"].clone(),
            "blocked": blocked,
            "blocking": {
                "agent_cases": blocking_agent.len(),
                "human_cases": blocking_human.len(),
                "items": blocking_agent.iter().chain(blocking_human.iter()).cloned().collect::<Vec<_>>(),
            },
            "advisory_outstanding": worklist["agent"]["cases"].as_i64().unwrap_or(0)
                + worklist["human"]["cases"].as_i64().unwrap_or(0)
                - blocking_agent.len() as i64
                - blocking_human.len() as i64,
        }))
    }
}

fn sum_minutes(items: &[WorkItem]) -> Option<i64> {
    let mut any = false;
    let mut total = 0;
    for i in items {
        if let Some(m) = i.cost_minutes {
            any = true;
            total += m;
        }
    }
    any.then_some(total)
}

/// Share of *verifiable* cases that are currently verified or approved.
///
/// Stale, failed and never are excluded from the numerator — a percentage that
/// counts a case nobody has re-run since the code changed is the exact comfort
/// this feature exists to remove.
///
/// `unreachable` comes out of the **denominator** too, which is what "counted
/// apart from both covered and uncovered" has to mean arithmetically. Leaving it
/// in would cap a fully-verified project below 100% forever with no action that
/// could ever close the gap, and a ceiling nobody can reach is a number people
/// learn to ignore. The count itself stays on the report as its own finding.
fn percent(counts: &CheckCounts) -> i64 {
    let verifiable = counts.total - counts.unreachable;
    if verifiable <= 0 {
        return 0;
    }
    (counts.approved + counts.verified) * 100 / verifiable
}

#[cfg(test)]
mod glob_tests {
    use super::glob_matches;

    /// The matcher is pure logic with no HTTP surface, so it is unit-tested here
    /// (the same exception `workflow.rs` takes) rather than only through a release
    /// push, where a miss would surface as "the wrong cases went stale" with no
    /// indication of why.
    #[test]
    fn double_star_spans_separators_and_single_star_does_not() {
        assert!(glob_matches("src/claims/**", "src/claims/mod.rs"));
        assert!(glob_matches(
            "src/claims/**",
            "src/claims/deep/nested/file.rs"
        ));
        assert!(!glob_matches("src/claims/*", "src/claims/deep/nested.rs"));
        assert!(glob_matches("src/claims/*.rs", "src/claims/mod.rs"));
    }

    /// A prefix must not leak into a sibling directory: this is the difference
    /// between invalidating one check and invalidating the wrong one.
    #[test]
    fn prefixes_do_not_bleed_across_directories() {
        assert!(!glob_matches("src/claims/**", "src/claimsx/mod.rs"));
        assert!(!glob_matches("src/claims/**", "other/src/claims/mod.rs"));
    }

    /// `a/**/b` has to match `a/b` too, or every "anything in between" pattern
    /// silently misses the zero-segment case.
    #[test]
    fn double_star_matches_zero_segments() {
        assert!(glob_matches("src/**/mod.rs", "src/mod.rs"));
        assert!(glob_matches("src/**/mod.rs", "src/a/b/mod.rs"));
    }

    #[test]
    fn exact_patterns_still_work() {
        assert!(glob_matches("Cargo.toml", "Cargo.toml"));
        assert!(!glob_matches("Cargo.toml", "Cargo.lock"));
    }
}
