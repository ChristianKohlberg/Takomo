//! Projects and workflow management.

use super::helpers::{emit_event, get_workflow, sync_workflow_states};
use super::model::Project;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::workflow::Workflow;
use rusqlite::{params, Connection, OptionalExtension};

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

/// Treat a stored blank as unset: a guide cleared to whitespace must read as
/// "no preference", not as an empty hint an agent has to interpret.
fn nonblank(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
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
                created_at: now,
            })
        })
    }

    pub fn list_projects(&self) -> ApiResult<Vec<Project>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, workflow_json, question_language, style_guide, created_at FROM projects ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, name, wf_raw, question_language, style_guide, created_at) = row?;
                let workflow = serde_json::from_str(&wf_raw).map_err(|e| {
                    ApiError::internal(format!("stored workflow for '{id}' is corrupt: {e}"))
                })?;
                out.push(Project {
                    id,
                    name,
                    workflow,
                    question_language,
                    style_guide,
                    created_at,
                });
            }
            Ok(out)
        })
    }

    pub fn get_project(&self, id: &str) -> ApiResult<Option<Project>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, name, workflow_json, question_language, style_guide, created_at FROM projects WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()?;
            match row {
                None => Ok(None),
                Some((id, name, wf_raw, question_language, style_guide, created_at)) => {
                    let workflow = serde_json::from_str(&wf_raw).map_err(|e| {
                        ApiError::internal(format!("stored workflow for '{id}' is corrupt: {e}"))
                    })?;
                    Ok(Some(Project {
                        id,
                        name,
                        workflow,
                        question_language,
                        style_guide,
                        created_at,
                    }))
                }
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
                "SELECT COUNT(*) FROM tickets WHERE project = ?1 AND claim_holder IS NOT NULL AND claim_expires_at > ?2",
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

    /// Replace a project's workflow (PUT). Must remain valid for existing tickets.
    pub fn put_workflow(&self, project: &str, wf: Workflow, actor: &str) -> ApiResult<Workflow> {
        let now = now_ms();
        self.with_tx(|tx| {
            // 404 if the project does not exist.
            get_workflow(tx, project)?;
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
