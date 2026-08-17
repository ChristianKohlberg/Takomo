//! Projects and workflow management.

use super::answer_grants::MAX_ANSWER_TTL_SECONDS;
use super::helpers::{emit_event, ensure_project_writable, get_workflow, sync_workflow_states};
use super::model::Project;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::workflow::Workflow;
use rusqlite::{params, Connection, OptionalExtension};

/// The columns every `Project` read selects, in the order `row_to_project`
/// expects. One literal so a new setting cannot be added to one query and
/// forgotten in the other.
const PROJECT_COLS: &str =
    "id, name, workflow_json, question_language, style_guide, answer_link_ttl_seconds, \
     claim_ttl_seconds, max_claim_ttl_seconds, archived_at, created_at";

/// Cap on a project's style guide. It is attached to work-loop responses an
/// agent reads constantly, so it has to stay a glanceable convention rather
/// than a second copy of the project's documentation.
pub const MAX_STYLE_GUIDE_CHARS: usize = 2000;

/// Normalize a style-guide value: trim it, treat blank as "clear it", and refuse
/// anything over [`MAX_STYLE_GUIDE_CHARS`] with a teaching error.
///
/// Exposed so the create-project path can reject an oversized guide *before*
/// inserting the project, rather than leaving a half-configured project behind.
pub fn normalize_style_guide(style: Option<&str>) -> ApiResult<Option<String>> {
    let style = style.map(str::trim).filter(|s| !s.is_empty());
    let Some(s) = style else { return Ok(None) };
    let chars = s.chars().count();
    if chars > MAX_STYLE_GUIDE_CHARS {
        return Err(ApiError::validation(
            "project.style_guide_too_long",
            format!(
                "The style guide is {chars} characters; the limit is {MAX_STYLE_GUIDE_CHARS}. It rides along on every work-loop response, so keep it to the few conventions that change how an agent writes — put anything longer in a ticket or your repo docs and reference it here."
            ),
        )
        .details(serde_json::json!({
            "chars": chars,
            "max_chars": MAX_STYLE_GUIDE_CHARS,
        })));
    }
    Ok(Some(s.to_string()))
}

/// A project's advisory writing conventions: the house style for the text
/// agents write, and the human-facing language questions belong in. Both
/// optional; a project that sets neither yields the default, and the work loop
/// then sends no hint at all rather than an empty one.
#[derive(Debug, Clone, Default)]
pub struct Conventions {
    pub question_language: Option<String>,
    pub style_guide: Option<String>,
}

impl Conventions {
    /// True when the project sets neither convention.
    pub fn is_empty(&self) -> bool {
        self.question_language.is_none() && self.style_guide.is_none()
    }
}

/// Validate a project's answer-link default lifetime: `None` (or a cleared
/// setting) means "unset — fall back to [`DEFAULT_ANSWER_TTL_SECONDS`]".
///
/// Bounded by exactly the rule that bounds an explicit `ttl_seconds` on a mint
/// call — a positive integer no larger than [`MAX_ANSWER_TTL_SECONDS`] — so the
/// project default can never express a lifetime a per-call `--ttl` would be
/// refused, and there is one number to remember instead of two. The cap is the
/// point: an answer link is a credential handed to someone outside the org, and
/// an unbounded default would make it a standing one.
///
/// [`DEFAULT_ANSWER_TTL_SECONDS`]: super::DEFAULT_ANSWER_TTL_SECONDS
pub fn normalize_answer_link_ttl(secs: Option<i64>) -> ApiResult<Option<i64>> {
    let Some(s) = secs else { return Ok(None) };
    if s <= 0 || s > MAX_ANSWER_TTL_SECONDS {
        return Err(ApiError::validation(
            "project.answer_link_ttl",
            format!(
                "answer_link_ttl_seconds is {s}; it must be a positive number of seconds no larger than {MAX_ANSWER_TTL_SECONDS} (30 days). That is the same bound an explicit ttl_seconds on POST /v1/questions/{{id}}/answer-link carries, because an answer link is a credential handed outside the org. Send null to clear the default and fall back to 7 days."
            ),
        )
        .details(serde_json::json!({
            "answer_link_ttl_seconds": s,
            "max_seconds": MAX_ANSWER_TTL_SECONDS,
        })));
    }
    Ok(Some(s))
}

/// Validate a project's lease policy — the pair `(claim_ttl_seconds,
/// max_claim_ttl_seconds)`, each `None` meaning "unset, fall back to the
/// built-in" ([`DEFAULT_TTL_SECONDS`] / [`MAX_TTL_SECONDS`]).
///
/// Validated as a pair on purpose. They are not independent: a default above the
/// ceiling would be silently clamped on every claim, so the project would be
/// configured to something it never gets. Catching that here is the difference
/// between a 422 that names both numbers and a lease policy that quietly does
/// something else. Two separate endpoints could not check it without an ordering
/// trap (raise the cap first, or the default is refused), which is why there is
/// one endpoint taking both.
///
/// **No upper bound on the ceiling** (takomo-2ztv, decided deliberately). A
/// deployment may set whatever its fleet needs. What that costs is worth keeping
/// in view: this number *is* the ready queue's recovery time, because the sweeper
/// frees only expired leases — so a crashed worker parks its ticket for exactly
/// this long, and an absurd value parks it effectively forever. `i64` seconds is
/// the only limit.
///
/// [`DEFAULT_TTL_SECONDS`]: super::DEFAULT_TTL_SECONDS
/// [`MAX_TTL_SECONDS`]: super::MAX_TTL_SECONDS
pub fn normalize_claim_ttls(
    default_secs: Option<i64>,
    max_secs: Option<i64>,
) -> ApiResult<(Option<i64>, Option<i64>)> {
    for (label, v) in [
        ("claim_ttl_seconds", default_secs),
        ("max_claim_ttl_seconds", max_secs),
    ] {
        if let Some(s) = v {
            if s <= 0 {
                return Err(ApiError::validation(
                    "project.claim_ttl",
                    format!(
                        "{label} is {s}; it must be a positive number of seconds. Send null to \
                         clear it and fall back to the built-in ({} default, {} max).",
                        super::DEFAULT_TTL_SECONDS,
                        super::MAX_TTL_SECONDS
                    ),
                )
                .details(serde_json::json!({ label: s })));
            }
        }
    }
    // Compare each against whichever ceiling will actually apply — the project's
    // own if it sets one, else the built-in. Otherwise a project could set a
    // 7200s default while leaving the cap unset, and every claim would silently
    // come back with 3600.
    let effective_max = max_secs.unwrap_or(super::MAX_TTL_SECONDS);
    if let Some(d) = default_secs {
        if d > effective_max {
            return Err(ApiError::validation(
                "project.claim_ttl",
                format!(
                    "claim_ttl_seconds is {d}, above the {effective_max}s ceiling that would \
                     apply{}, so every claim would be clamped and no claim would ever get the \
                     default you set. Raise max_claim_ttl_seconds in the same request, or lower \
                     the default.",
                    if max_secs.is_some() {
                        " (max_claim_ttl_seconds)"
                    } else {
                        " (the built-in maximum, since max_claim_ttl_seconds is unset)"
                    }
                ),
            )
            .details(serde_json::json!({
                "claim_ttl_seconds": d,
                "max_claim_ttl_seconds": max_secs,
                "effective_max_seconds": effective_max,
            })));
        }
    }
    Ok((default_secs, max_secs))
}

/// Treat a stored blank as unset: a guide cleared to whitespace must read as
/// "no preference", not as an empty hint an agent has to interpret.
fn nonblank(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// One project row as [`PROJECT_COLS`] selects it, before the workflow JSON is
/// parsed.
type ProjectRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
);

fn project_row(r: &rusqlite::Row) -> rusqlite::Result<ProjectRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
    ))
}

fn hydrate_project(row: ProjectRow) -> ApiResult<Project> {
    let (
        id,
        name,
        wf_raw,
        question_language,
        style_guide,
        answer_link_ttl_seconds,
        claim_ttl_seconds,
        max_claim_ttl_seconds,
        archived_at,
        created_at,
    ) = row;
    let workflow = serde_json::from_str(&wf_raw)
        .map_err(|e| ApiError::internal(format!("stored workflow for '{id}' is corrupt: {e}")))?;
    Ok(Project {
        id,
        name,
        workflow,
        question_language,
        style_guide,
        answer_link_ttl_seconds,
        claim_ttl_seconds,
        max_claim_ttl_seconds,
        archived_at,
        created_at,
    })
}

/// Which questions belong to a project, as a reusable SQL predicate over `?1`.
/// `questions` carries both a `project` and a `ticket` column and both are
/// foreign keys, so the delete cascade has to clear a row that matches *either*
/// — otherwise a stray row survives and aborts `DELETE FROM projects`/`tickets`
/// on the immediate FK check. It is a fixed literal (no interpolated data), so
/// splicing it into a query string is injection-free.
const QUESTIONS_OF_PROJECT: &str =
    "project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)";

/// Row counts removed by a cascade project delete, for the audit trail and the
/// CLI's "what was deleted" summary.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeletedCounts {
    pub tickets: i64,
    pub comments: i64,
    pub deps: i64,
    pub events: i64,
    pub questions: i64,
    pub question_messages: i64,
    pub answer_grants: i64,
    pub tags: i64,
    pub promotions: i64,
}

fn project_id_valid(id: &str) -> bool {
    let bytes = id.as_bytes();
    (2..=16).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

fn states_in_use(conn: &Connection, project: &str) -> ApiResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT state FROM tickets WHERE project = ?1")?;
    let states = stmt
        .query_map(params![project], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(states)
}

fn validate_workflow(wf: &Workflow, existing_states: &[String]) -> ApiResult<()> {
    let problems = wf.validate(existing_states);
    if problems.is_empty() {
        return Ok(());
    }
    Err(ApiError::validation(
        "workflow.invalid",
        format!(
            "The workflow definition is invalid: {}. Fix the definition and retry; see workflow-format.md for the format.",
            problems.join("; ")
        ),
    )
    .details(serde_json::json!({ "problems": problems })))
}

/// Optional per-project settings applied atomically at creation time.
#[derive(Debug, Clone, Default)]
pub struct ProjectCreateSettings {
    pub question_language: Option<String>,
    pub style_guide: Option<String>,
    pub answer_link_ttl_seconds: Option<i64>,
    pub claim_ttl_seconds: Option<i64>,
    pub max_claim_ttl_seconds: Option<i64>,
}

impl Store {
    pub fn create_project(
        &self,
        id: &str,
        name: &str,
        workflow: Option<Workflow>,
        actor: &str,
    ) -> ApiResult<Project> {
        if !project_id_valid(id) {
            return Err(ApiError::validation(
                "project.id",
                format!(
                    "Project id '{id}' is invalid. Use 2-16 chars matching ^[a-z][a-z0-9-]{{1,15}}$; it becomes the ticket id prefix."
                ),
            ));
        }
        let wf = workflow.unwrap_or_else(crate::workflow::factory_default);
        validate_workflow(&wf, &[])?;
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_some() {
                return Err(ApiError::conflict(
                    "project.exists",
                    format!("Project '{id}' already exists. Choose a different id, or GET /v1/projects/{id}/workflow to inspect it."),
                ));
            }
            tx.execute(
                "INSERT INTO projects (id, name, workflow_json, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, name, serde_json::to_string(&wf).unwrap(), now],
            )?;
            sync_workflow_states(tx, id, &wf)?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "workflow_changed",
                serde_json::json!({ "workflow": wf.name, "on": "project_created" }),
                now,
            )?;
            Ok(Project {
                id: id.to_string(),
                name: name.to_string(),
                workflow: wf,
                question_language: None,
                style_guide: None,
                answer_link_ttl_seconds: None,
                claim_ttl_seconds: None,
                max_claim_ttl_seconds: None,
                archived_at: None,
                created_at: now,
            })
        })
    }

    /// Create a project and apply optional per-project settings in one
    /// transaction. Callers must validate each setting (style guide length,
    /// claim-ttl pairing, …) before calling — same contract as the HTTP
    /// handler's pre-insert checks.
    pub fn create_project_with_settings(
        &self,
        id: &str,
        name: &str,
        workflow: Option<Workflow>,
        actor: &str,
        settings: &ProjectCreateSettings,
    ) -> ApiResult<Project> {
        if !project_id_valid(id) {
            return Err(ApiError::validation(
                "project.id",
                format!(
                    "Project id '{id}' is invalid. Use 2-16 chars matching ^[a-z][a-z0-9-]{{1,15}}$; it becomes the ticket id prefix."
                ),
            ));
        }
        let wf = workflow.unwrap_or_else(crate::workflow::factory_default);
        validate_workflow(&wf, &[])?;
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_some() {
                return Err(ApiError::conflict(
                    "project.exists",
                    format!("Project '{id}' already exists. Choose a different id, or GET /v1/projects/{id}/workflow to inspect it."),
                ));
            }
            tx.execute(
                "INSERT INTO projects (id, name, workflow_json, question_language, style_guide, \
                 answer_link_ttl_seconds, claim_ttl_seconds, max_claim_ttl_seconds, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    name,
                    serde_json::to_string(&wf).unwrap(),
                    settings.question_language,
                    settings.style_guide,
                    settings.answer_link_ttl_seconds,
                    settings.claim_ttl_seconds,
                    settings.max_claim_ttl_seconds,
                    now,
                ],
            )?;
            sync_workflow_states(tx, id, &wf)?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "workflow_changed",
                serde_json::json!({ "workflow": wf.name, "on": "project_created" }),
                now,
            )?;
            let has_settings = settings.question_language.is_some()
                || settings.style_guide.is_some()
                || settings.answer_link_ttl_seconds.is_some()
                || settings.claim_ttl_seconds.is_some()
                || settings.max_claim_ttl_seconds.is_some();
            if has_settings {
                emit_event(
                    tx,
                    None,
                    Some(id),
                    actor,
                    "project_updated",
                    serde_json::json!({
                        "question_language": settings.question_language,
                        "style_guide": settings.style_guide,
                        "answer_link_ttl_seconds": settings.answer_link_ttl_seconds,
                        "claim_ttl_seconds": settings.claim_ttl_seconds,
                        "max_claim_ttl_seconds": settings.max_claim_ttl_seconds,
                    }),
                    now,
                )?;
            }
            Ok(Project {
                id: id.to_string(),
                name: name.to_string(),
                workflow: wf,
                question_language: settings.question_language.clone(),
                style_guide: settings.style_guide.clone(),
                answer_link_ttl_seconds: settings.answer_link_ttl_seconds,
                claim_ttl_seconds: settings.claim_ttl_seconds,
                max_claim_ttl_seconds: settings.max_claim_ttl_seconds,
                archived_at: None,
                created_at: now,
            })
        })
    }

    pub fn list_projects(&self) -> ApiResult<Vec<Project>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {PROJECT_COLS} FROM projects ORDER BY id"))?;
            let rows = stmt.query_map([], project_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(hydrate_project(row?)?);
            }
            Ok(out)
        })
    }

    pub fn get_project(&self, id: &str) -> ApiResult<Option<Project>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"),
                    params![id],
                    project_row,
                )
                .optional()?;
            match row {
                None => Ok(None),
                Some(row) => Ok(Some(hydrate_project(row)?)),
            }
        })
    }

    /// A project's advisory writing conventions, for attaching to a work-loop
    /// response.
    ///
    /// Deliberately not `get_project`: every work-loop call would otherwise
    /// deserialize the whole stored workflow document to read two strings. Two
    /// columns, one row, no JSON parse. An unknown project yields the empty set
    /// rather than an error — a hint is advisory and must never be the thing
    /// that fails a claim.
    pub fn project_conventions(&self, project: &str) -> ApiResult<Conventions> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT question_language, style_guide FROM projects WHERE id = ?1",
                    params![project],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()?;
            let Some((question_language, style_guide)) = row else {
                return Ok(Conventions::default());
            };
            Ok(Conventions {
                question_language: nonblank(question_language),
                style_guide: nonblank(style_guide),
            })
        })
    }

    /// Set (or clear, with None) a project's human-facing question language.
    pub fn set_question_language(
        &self,
        id: &str,
        language: Option<&str>,
        actor: &str,
    ) -> ApiResult<Project> {
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", id));
            }
            // A project setting is a write like any other: an archived project is
            // frozen as it stood, not reconfigurable in place.
            ensure_project_writable(tx, id)?;
            tx.execute(
                "UPDATE projects SET question_language = ?2 WHERE id = ?1",
                params![id, language],
            )?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "project_updated",
                serde_json::json!({ "question_language": language }),
                now,
            )?;
            Ok(())
        })?;
        self.get_project(id)?
            .ok_or_else(|| ApiError::not_found("project", id))
    }

    /// Set (or clear, with None) a project's style guide — the house style for
    /// the text agents write on this project.
    ///
    /// A whitespace-only value clears it, so callers never have to distinguish
    /// "" from null. Refuses anything over [`MAX_STYLE_GUIDE_CHARS`]: this is a
    /// short convention an agent reads on every work-loop call, not a place to
    /// park project documentation.
    pub fn set_style_guide(
        &self,
        id: &str,
        style: Option<&str>,
        actor: &str,
    ) -> ApiResult<Project> {
        let style = normalize_style_guide(style)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", id));
            }
            // A project setting is a write like any other: an archived project is
            // frozen as it stood, not reconfigurable in place.
            ensure_project_writable(tx, id)?;
            tx.execute(
                "UPDATE projects SET style_guide = ?2 WHERE id = ?1",
                params![id, style],
            )?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "project_updated",
                serde_json::json!({ "style_guide": style }),
                now,
            )?;
            Ok(())
        })?;
        self.get_project(id)?
            .ok_or_else(|| ApiError::not_found("project", id))
    }

    /// A project's default answer-link lifetime in seconds, or `None` when the
    /// project sets none (the caller then uses
    /// [`DEFAULT_ANSWER_TTL_SECONDS`](super::DEFAULT_ANSWER_TTL_SECONDS)).
    ///
    /// Deliberately not `get_project`, for the same reason as
    /// [`Store::project_conventions`]: minting a link would otherwise
    /// deserialize the whole stored workflow document to read one integer.
    ///
    /// An unknown project yields `None` rather than an error — the mint path has
    /// already resolved and authorized the question's project, so a missing row
    /// here can only mean it was deleted underneath us, and falling back to the
    /// built-in default is the safe answer.
    pub fn answer_link_ttl(&self, project: &str) -> ApiResult<Option<i64>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT answer_link_ttl_seconds FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?;
            Ok(row.flatten())
        })
    }

    /// Set (or clear, with None) how long an answer link minted for one of this
    /// project's questions stays valid.
    ///
    /// Unlike the language and style settings this one is *enforced*: it decides
    /// the lifetime of a bearer credential handed to someone outside the org, so
    /// it is validated against [`normalize_answer_link_ttl`] before it is stored
    /// and again nowhere else — a value in the column is always mintable.
    pub fn set_answer_link_ttl(
        &self,
        id: &str,
        ttl_seconds: Option<i64>,
        actor: &str,
    ) -> ApiResult<Project> {
        let ttl = normalize_answer_link_ttl(ttl_seconds)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", id));
            }
            // A project setting is a write like any other: an archived project is
            // frozen as it stood, not reconfigurable in place.
            ensure_project_writable(tx, id)?;
            tx.execute(
                "UPDATE projects SET answer_link_ttl_seconds = ?2 WHERE id = ?1",
                params![id, ttl],
            )?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "project_updated",
                serde_json::json!({ "answer_link_ttl_seconds": ttl }),
                now,
            )?;
            Ok(())
        })?;
        self.get_project(id)?
            .ok_or_else(|| ApiError::not_found("project", id))
    }

    /// Set (or clear, with None) this project's lease policy: the default lease a
    /// claim gets, and the ceiling an explicit `ttl_seconds` is checked against.
    ///
    /// Both in one call because [`normalize_claim_ttls`] validates them as a pair
    /// — see there for why splitting them would create an ordering trap.
    pub fn set_claim_ttls(
        &self,
        id: &str,
        default_secs: Option<i64>,
        max_secs: Option<i64>,
        actor: &str,
    ) -> ApiResult<Project> {
        let (default_ttl, max_ttl) = normalize_claim_ttls(default_secs, max_secs)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", id));
            }
            // A project setting is a write like any other: an archived project is
            // frozen as it stood, not reconfigurable in place.
            ensure_project_writable(tx, id)?;
            tx.execute(
                "UPDATE projects SET claim_ttl_seconds = ?2, max_claim_ttl_seconds = ?3 WHERE id = ?1",
                params![id, default_ttl, max_ttl],
            )?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "project_updated",
                serde_json::json!({
                    "claim_ttl_seconds": default_ttl,
                    "max_claim_ttl_seconds": max_ttl,
                }),
                now,
            )?;
            Ok(())
        })?;
        self.get_project(id)?
            .ok_or_else(|| ApiError::not_found("project", id))
    }

    /// Archive a project (`archived = true`) or put it back to work
    /// (`archived = false`).
    ///
    /// This is the gate, and it is deliberately the *only* thing about a project
    /// that archiving changes. Nothing is moved, rewritten, or deleted: the
    /// tickets keep their states, claims, questions and history, and every read
    /// answers exactly as it did before. What stops is writing —
    /// [`ensure_project_writable`](super::helpers::ensure_project_writable)
    /// refuses every mutation under the project — and the ready queue, which
    /// stops offering its tickets so no agent is handed work it would then be
    /// refused permission to do.
    ///
    /// **Reversible, and that is the point.** The existing way to retire a
    /// project was `delete_project`, which cascades away every ticket, comment,
    /// question and event and cannot be undone. Someone who only wants "stop
    /// working on this" should not have to choose between that and leaving a dead
    /// project in the ready queue. Unarchiving restores the project unchanged
    /// because archiving never changed it.
    ///
    /// Idempotent in both directions: archiving an archived project keeps the
    /// original `archived_at` and emits no second event, so the audit trail
    /// records when the project was frozen rather than when someone last said so.
    ///
    /// Refuses with a teaching 409 when a ticket holds an active lease, unless
    /// `force` — archiving under a live claim would strand that worker, whose
    /// every next call (heartbeat, done, even release) would be refused. With
    /// `force` those leases are released here rather than abandoned, bumping each
    /// ticket's fence exactly as an admin force-release does, so a displaced
    /// worker gets the fencing 409 it already knows how to read instead of a
    /// ticket frozen mid-claim.
    pub fn set_project_archived(
        &self,
        id: &str,
        archived: bool,
        force: bool,
        actor: &str,
    ) -> ApiResult<Project> {
        let now = now_ms();
        self.with_tx(|tx| {
            let current: Option<Option<i64>> = tx
                .query_row(
                    "SELECT archived_at FROM projects WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(archived_at) = current else {
                return Err(ApiError::not_found("project", id));
            };
            // Already where the caller wants it: nothing to write, nothing to
            // announce.
            if archived_at.is_some() == archived {
                return Ok(());
            }
            if !archived {
                tx.execute(
                    "UPDATE projects SET archived_at = NULL WHERE id = ?1",
                    params![id],
                )?;
                emit_event(
                    tx,
                    None,
                    Some(id),
                    actor,
                    "project_unarchived",
                    serde_json::json!({}),
                    now,
                )?;
                return Ok(());
            }

            // Archiving: refuse under a live lease unless forced.
            let mut stmt = tx.prepare(
                "SELECT id, claim_holder, fence_seq FROM tickets \
                 WHERE project = ?1 AND claim_holder IS NOT NULL AND (claim_expires_at IS NULL OR claim_expires_at > ?2) \
                 ORDER BY id",
            )?;
            let claimed: Vec<(String, String, i64)> = stmt
                .query_map(params![id, now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            if !claimed.is_empty() && !force {
                let holders: Vec<&str> = claimed.iter().map(|(_, h, _)| h.as_str()).collect();
                return Err(ApiError::conflict(
                    "project.active_claims",
                    format!(
                        "Project '{id}' has {} ticket(s) with an active (unexpired) claim; archiving now would freeze those workers mid-lease — their next heartbeat, done or release would be refused with nothing they could do about it. Wait for the leases to expire or be released, or re-issue the archive with force=true to release them (bumping each ticket's fence, so the displaced worker gets a fencing 409 it can act on) and archive anyway.",
                        claimed.len()
                    ),
                )
                .details(serde_json::json!({
                    "active_claims": claimed.len(),
                    "tickets": claimed.iter().map(|(t, _, _)| t.clone()).collect::<Vec<_>>(),
                    "holders": holders,
                })));
            }
            for (ticket, holder, fence) in &claimed {
                tx.execute(
                    "UPDATE tickets SET claim_holder = NULL, claim_expires_at = NULL, claim_since = NULL, \
                     lapsed_claim_holder = NULL, fence_seq = fence_seq + 1, \
                     version = version + 1, updated_at = ?2 WHERE id = ?1",
                    params![ticket, now],
                )?;
                emit_event(
                    tx,
                    Some(ticket),
                    Some(id),
                    actor,
                    "released",
                    serde_json::json!({
                        "holder": holder,
                        "fence": fence + 1,
                        "forced": true,
                        "reason": "project archived",
                    }),
                    now,
                )?;
            }
            tx.execute(
                "UPDATE projects SET archived_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            emit_event(
                tx,
                None,
                Some(id),
                actor,
                "project_archived",
                serde_json::json!({
                    "forced": force,
                    "released_claims": claimed.len(),
                }),
                now,
            )?;
            Ok(())
        })?;
        self.get_project(id)?
            .ok_or_else(|| ApiError::not_found("project", id))
    }

    /// Cascade-delete a project and everything under it (tickets, comments,
    /// deps, events, idempotency records, questions with their follow-up
    /// threads and answer grants, promotions, and the tag registry), in one
    /// transaction.
    ///
    /// Refuses with a teaching 409 if any ticket carries an active (unexpired)
    /// claim, unless `force` is set — deleting under a live lease would yank
    /// work out from under a running worker. Tokens scoped to this project are
    /// deliberately left untouched: they simply stop resolving once the project
    /// is gone, and an admin can revoke them separately.
    ///
    /// Returns the counts removed. A store-level `project_deleted` audit event
    /// (with a null project, so per-project event queries stay empty) records
    /// the deletion.
    pub fn delete_project(&self, id: &str, force: bool, actor: &str) -> ApiResult<DeletedCounts> {
        let now = now_ms();
        self.with_tx(|tx| {
            // 404 if the project does not exist.
            let exists: Option<String> = tx
                .query_row("SELECT id FROM projects WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", id));
            }

            // Guard: refuse while any ticket holds an active (unexpired) lease,
            // unless the caller forces it.
            let active_claims: i64 = tx.query_row(
                "SELECT COUNT(*) FROM tickets WHERE project = ?1 AND claim_holder IS NOT NULL AND (claim_expires_at IS NULL OR claim_expires_at > ?2)",
                params![id, now],
                |r| r.get(0),
            )?;
            if active_claims > 0 && !force {
                return Err(ApiError::conflict(
                    "project.active_claims",
                    format!(
                        "Project '{id}' has {active_claims} ticket(s) with an active (unexpired) claim; deleting it now would yank work out from under a live worker. Wait for those leases to expire or be released, or re-issue the delete with ?force=true to abandon them and delete anyway."
                    ),
                )
                .details(serde_json::json!({ "active_claims": active_claims })));
            }

            // Counts captured before deletion, for the audit event + response.
            let counts = DeletedCounts {
                tickets: tx.query_row(
                    "SELECT COUNT(*) FROM tickets WHERE project = ?1",
                    params![id],
                    |r| r.get(0),
                )?,
                comments: tx.query_row(
                    "SELECT COUNT(*) FROM comments WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                    params![id],
                    |r| r.get(0),
                )?,
                deps: tx.query_row(
                    "SELECT COUNT(*) FROM deps WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1) OR blocked_by IN (SELECT id FROM tickets WHERE project = ?1)",
                    params![id],
                    |r| r.get(0),
                )?,
                events: tx.query_row(
                    "SELECT COUNT(*) FROM events WHERE project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                    params![id],
                    |r| r.get(0),
                )?,
                questions: tx.query_row(
                    &format!("SELECT COUNT(*) FROM questions WHERE {QUESTIONS_OF_PROJECT}"),
                    params![id],
                    |r| r.get(0),
                )?,
                question_messages: tx.query_row(
                    &format!("SELECT COUNT(*) FROM question_messages WHERE question IN (SELECT id FROM questions WHERE {QUESTIONS_OF_PROJECT})"),
                    params![id],
                    |r| r.get(0),
                )?,
                answer_grants: tx.query_row(
                    &format!("SELECT COUNT(*) FROM answer_grants WHERE project = ?1 OR question IN (SELECT id FROM questions WHERE {QUESTIONS_OF_PROJECT})"),
                    params![id],
                    |r| r.get(0),
                )?,
                tags: tx.query_row(
                    "SELECT COUNT(*) FROM tags WHERE project = ?1",
                    params![id],
                    |r| r.get(0),
                )?,
                promotions: tx.query_row(
                    "SELECT COUNT(*) FROM promotions WHERE project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                    params![id],
                    |r| r.get(0),
                )?,
            };

            // Cascade in FK-safe order: children referencing tickets first, then
            // the tickets, then the project row itself. Immediate foreign keys
            // are checked at the end of each statement, so deleting all of a
            // project's tickets (including parent/child pairs) in one statement
            // leaves no dangling reference. deps are cleared in both directions
            // because a blocked_by edge may originate in another project.
            //
            // The order below is load-bearing, not cosmetic — every one of these
            // tables holds a real REFERENCES, so a statement run too early aborts
            // the whole transaction with a 500:
            //   question_messages, answer_grants -> questions(id)
            //   questions                        -> projects(id) AND tickets(id)
            //   promotions                       -> tickets(id)
            //   tags                             -> projects(id)
            // `shares` is deliberately NOT swept here: shares.project is a plain
            // TEXT column with no REFERENCES, so it leaks orphan rows rather than
            // blocking the delete. That is a separate bug with its own ticket.
            tx.execute(
                "DELETE FROM deps WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1) OR blocked_by IN (SELECT id FROM tickets WHERE project = ?1)",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM comment_idempotency WHERE comment IN (SELECT id FROM comments WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1))",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM comments WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM idempotency WHERE ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM events WHERE project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                params![id],
            )?;
            // The question thread and its answer links, before the questions.
            tx.execute(
                &format!("DELETE FROM question_messages WHERE question IN (SELECT id FROM questions WHERE {QUESTIONS_OF_PROJECT})"),
                params![id],
            )?;
            tx.execute(
                &format!("DELETE FROM answer_grants WHERE project = ?1 OR question IN (SELECT id FROM questions WHERE {QUESTIONS_OF_PROJECT})"),
                params![id],
            )?;
            tx.execute(
                &format!("DELETE FROM questions WHERE {QUESTIONS_OF_PROJECT}"),
                params![id],
            )?;
            tx.execute(
                "DELETE FROM promotions WHERE project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)",
                params![id],
            )?;
            tx.execute("DELETE FROM tickets WHERE project = ?1", params![id])?;
            tx.execute("DELETE FROM tags WHERE project = ?1", params![id])?;
            tx.execute("DELETE FROM workflow_states WHERE project = ?1", params![id])?;
            tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;

            emit_event(
                tx,
                None,
                None,
                actor,
                "project_deleted",
                serde_json::json!({
                    "project": id,
                    "forced": force,
                    "deleted": {
                        "tickets": counts.tickets,
                        "comments": counts.comments,
                        "deps": counts.deps,
                        "events": counts.events,
                        "questions": counts.questions,
                        "question_messages": counts.question_messages,
                        "answer_grants": counts.answer_grants,
                        "tags": counts.tags,
                        "promotions": counts.promotions,
                    }
                }),
                now,
            )?;
            Ok(counts)
        })
    }

    /// What [`Store::put_workflow`] would complain about, WITHOUT writing.
    ///
    /// The same `validate` against the same live `states_in_use`, so a draft the
    /// editor calls clean cannot be refused by the PUT a moment later. Empty
    /// means it would be accepted. Runs on a read connection: it is a read.
    pub fn workflow_problems(&self, project: &str, wf: &Workflow) -> ApiResult<Vec<String>> {
        self.with_conn(|conn| {
            // 404 if the project does not exist, exactly as the write path does.
            get_workflow(conn, project)?;
            let in_use = states_in_use(conn, project)?;
            Ok(wf.validate(&in_use))
        })
    }

    /// Replace a project's workflow (PUT). Must remain valid for existing tickets.
    pub fn put_workflow(&self, project: &str, wf: Workflow, actor: &str) -> ApiResult<Workflow> {
        let now = now_ms();
        self.with_tx(|tx| {
            // 404 if the project does not exist.
            get_workflow(tx, project)?;
            // The state machine is the one thing an archived project must not
            // change: its tickets are frozen where they stand, and a workflow
            // that no longer describes them would strand them there for good.
            ensure_project_writable(tx, project)?;
            let in_use = states_in_use(tx, project)?;
            validate_workflow(&wf, &in_use)?;
            tx.execute(
                "UPDATE projects SET workflow_json = ?2 WHERE id = ?1",
                params![project, serde_json::to_string(&wf).unwrap()],
            )?;
            sync_workflow_states(tx, project, &wf)?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "workflow_changed",
                serde_json::json!({ "workflow": wf.name }),
                now,
            )?;
            Ok(wf)
        })
    }
}
