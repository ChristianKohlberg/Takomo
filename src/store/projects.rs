//! Projects and workflow management.

use super::helpers::{emit_event, get_workflow, sync_workflow_states};
use super::model::Project;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use crate::workflow::Workflow;
use rusqlite::{params, Connection, OptionalExtension};

/// Row counts removed by a cascade project delete, for the audit trail and the
/// CLI's "what was deleted" summary.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeletedCounts {
    pub tickets: i64,
    pub comments: i64,
    pub deps: i64,
    pub events: i64,
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
                created_at: now,
            })
        })
    }

    pub fn list_projects(&self) -> ApiResult<Vec<Project>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, workflow_json, question_language, created_at FROM projects ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, name, wf_raw, question_language, created_at) = row?;
                let workflow = serde_json::from_str(&wf_raw).map_err(|e| {
                    ApiError::internal(format!("stored workflow for '{id}' is corrupt: {e}"))
                })?;
                out.push(Project {
                    id,
                    name,
                    workflow,
                    question_language,
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
                    "SELECT id, name, workflow_json, question_language, created_at FROM projects WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()?;
            match row {
                None => Ok(None),
                Some((id, name, wf_raw, question_language, created_at)) => {
                    let workflow = serde_json::from_str(&wf_raw).map_err(|e| {
                        ApiError::internal(format!("stored workflow for '{id}' is corrupt: {e}"))
                    })?;
                    Ok(Some(Project {
                        id,
                        name,
                        workflow,
                        question_language,
                        created_at,
                    }))
                }
            }
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

    /// Cascade-delete a project and everything under it (tickets, comments,
    /// deps, events, and idempotency records), in one transaction.
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
            };

            // Cascade in FK-safe order: children referencing tickets first, then
            // the tickets, then the project row itself. Immediate foreign keys
            // are checked at the end of each statement, so deleting all of a
            // project's tickets (including parent/child pairs) in one statement
            // leaves no dangling reference. deps are cleared in both directions
            // because a blocked_by edge may originate in another project.
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
            tx.execute("DELETE FROM tickets WHERE project = ?1", params![id])?;
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
