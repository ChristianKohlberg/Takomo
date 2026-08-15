//! The workflow library: named state machines that can be applied to any
//! project, plus the built-ins this binary ships.
//!
//! The library stores documents. It never applies one — that stays
//! `PUT /v1/projects/{p}/workflow`, so the check that refuses to strand tickets
//! has exactly one code path.

use super::{Store, WorkflowEntry};
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, workflow_entry_id};
use crate::workflow::{builtins, Workflow};
use rusqlite::{params, Connection};

/// Names reserved for the shipped workflows. A user entry may not take one:
/// the seed below would overwrite it on the next restart, which is a data loss
/// nobody asked for and could not see coming.
fn is_builtin_name(name: &str) -> bool {
    builtins().iter().any(|w| w.name == name)
}

fn row_to_entry(r: &rusqlite::Row) -> rusqlite::Result<WorkflowEntry> {
    Ok(WorkflowEntry {
        id: r.get("id")?,
        name: r.get("name")?,
        description: r.get("description")?,
        workflow: serde_json::from_str(&r.get::<_, String>("workflow_json")?)
            .unwrap_or(serde_json::Value::Null),
        layout: r
            .get::<_, Option<String>>("layout_json")?
            .and_then(|s| serde_json::from_str(&s).ok()),
        builtin: r.get::<_, i64>("builtin")? != 0,
        created_at: r.get("created_at")?,
        created_by: r.get("created_by")?,
        updated_at: r.get("updated_at")?,
    })
}

const COLS: &str =
    "id, name, description, workflow_json, layout_json, builtin, created_at, created_by, updated_at";

impl Store {
    /// Insert or refresh the shipped workflows.
    ///
    /// Runs on every open, like `migrate`, and is idempotent. It overwrites the
    /// DOCUMENT of a built-in row but keeps its id — so an operator who upgrades
    /// gets the new definition without the library growing a second
    /// `factory-default`, and nothing that referenced the row by id breaks.
    pub(crate) fn seed_builtin_workflows(conn: &Connection, now: i64) -> ApiResult<()> {
        for wf in builtins() {
            let json = serde_json::to_string(&wf).map_err(|e| {
                ApiError::internal(format!("cannot serialize built-in workflow: {e}"))
            })?;
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM workflow_library WHERE name = ?1",
                    params![wf.name],
                    |r| r.get(0),
                )
                .ok();
            match existing {
                Some(id) => {
                    conn.execute(
                        "UPDATE workflow_library
                            SET workflow_json = ?2, builtin = 1, updated_at = ?3
                          WHERE id = ?1",
                        params![id, json, now],
                    )?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO workflow_library
                             (id, name, description, workflow_json, layout_json, builtin,
                              created_at, created_by, updated_at)
                         VALUES (?1, ?2, NULL, ?3, NULL, 1, ?4, 'takomo', ?4)",
                        params![workflow_entry_id(), wf.name, json, now],
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Every library entry, built-ins first, then user entries by name.
    pub fn list_workflow_entries(&self) -> ApiResult<Vec<WorkflowEntry>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {COLS} FROM workflow_library ORDER BY builtin DESC, name COLLATE NOCASE"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map([], row_to_entry)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_workflow_entry(&self, id: &str) -> ApiResult<Option<WorkflowEntry>> {
        self.with_conn(|conn| {
            let sql = format!("SELECT {COLS} FROM workflow_library WHERE id = ?1");
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query_map(params![id], row_to_entry)?;
            match rows.next() {
                Some(r) => Ok(Some(r?)),
                None => Ok(None),
            }
        })
    }

    pub fn create_workflow_entry(
        &self,
        name: &str,
        description: Option<&str>,
        wf: &Workflow,
        layout: Option<&serde_json::Value>,
        actor: &str,
    ) -> ApiResult<WorkflowEntry> {
        if is_builtin_name(name) {
            return Err(name_reserved(name));
        }
        let id = workflow_entry_id();
        let now = now_ms();
        let json = serde_json::to_string(wf)
            .map_err(|e| ApiError::internal(format!("cannot serialize workflow: {e}")))?;
        let layout = layout.map(|l| l.to_string());
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO workflow_library
                     (id, name, description, workflow_json, layout_json, builtin,
                      created_at, created_by, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?6)",
                params![id, name, description, json, layout, now, actor],
            )
            .map_err(|e| duplicate_name_or(e, name))?;
            Ok(())
        })?;
        self.get_workflow_entry(&id)?
            .ok_or_else(|| ApiError::internal("workflow entry vanished after insert"))
    }

    /// Patch a user entry. `None` on a field leaves it alone.
    pub fn patch_workflow_entry(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        wf: Option<&Workflow>,
        layout: Option<Option<&serde_json::Value>>,
    ) -> ApiResult<WorkflowEntry> {
        if let Some(n) = name {
            if is_builtin_name(n) {
                return Err(name_reserved(n));
            }
        }
        let now = now_ms();
        let json = wf
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| ApiError::internal(format!("cannot serialize workflow: {e}")))?;
        let layout = layout.map(|l| l.map(|v| v.to_string()));

        self.with_tx(|tx| {
            let builtin: i64 = tx
                .query_row(
                    "SELECT builtin FROM workflow_library WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|_| ApiError::not_found("workflow", id))?;
            if builtin != 0 {
                return Err(builtin_immutable());
            }
            if let Some(n) = name {
                tx.execute(
                    "UPDATE workflow_library SET name = ?2 WHERE id = ?1",
                    params![id, n],
                )
                .map_err(|e| duplicate_name_or(e, n))?;
            }
            if let Some(d) = description {
                tx.execute(
                    "UPDATE workflow_library SET description = ?2 WHERE id = ?1",
                    params![id, d],
                )?;
            }
            if let Some(j) = &json {
                tx.execute(
                    "UPDATE workflow_library SET workflow_json = ?2 WHERE id = ?1",
                    params![id, j],
                )?;
            }
            if let Some(l) = &layout {
                tx.execute(
                    "UPDATE workflow_library SET layout_json = ?2 WHERE id = ?1",
                    params![id, l],
                )?;
            }
            tx.execute(
                "UPDATE workflow_library SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            Ok(())
        })?;
        self.get_workflow_entry(id)?
            .ok_or_else(|| ApiError::not_found("workflow", id))
    }

    /// Delete a user entry. Deleting does NOT touch any project already using
    /// the document — a project owns its workflow outright once applied, which
    /// is what keeps the library from being load-bearing at runtime.
    pub fn delete_workflow_entry(&self, id: &str) -> ApiResult<bool> {
        self.with_tx(|tx| {
            let builtin: Option<i64> = tx
                .query_row(
                    "SELECT builtin FROM workflow_library WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .ok();
            match builtin {
                None => Ok(false),
                Some(b) if b != 0 => Err(builtin_immutable()),
                Some(_) => {
                    tx.execute("DELETE FROM workflow_library WHERE id = ?1", params![id])?;
                    Ok(true)
                }
            }
        })
    }

    /// The stored node layout for a project's own workflow, if it has one.
    pub fn get_workflow_layout(&self, project: &str) -> ApiResult<Option<serde_json::Value>> {
        self.with_conn(|conn| {
            let raw: Option<String> = conn
                .query_row(
                    "SELECT workflow_layout_json FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .map_err(|_| ApiError::not_found("project", project))?;
            Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
        })
    }

    /// Store the node layout for a project's workflow.
    ///
    /// Emits NO event and does not wake long-pollers: moving a node is not a
    /// change to how the project behaves, and a board that re-fetched every time
    /// someone dragged a box would be reacting to nothing.
    pub fn put_workflow_layout(&self, project: &str, layout: &serde_json::Value) -> ApiResult<()> {
        let raw = layout.to_string();
        self.with_tx(|tx| {
            // Refused on an archived project like every other write, even though
            // this one is only cosmetic. The gate is worth more as a rule with no
            // exceptions to remember than as a rule with one harmless hole.
            super::helpers::ensure_project_writable(tx, project)?;
            let n = tx.execute(
                "UPDATE projects SET workflow_layout_json = ?2 WHERE id = ?1",
                params![project, raw],
            )?;
            if n == 0 {
                return Err(ApiError::not_found("project", project));
            }
            Ok(())
        })
    }
}

fn name_reserved(name: &str) -> ApiError {
    ApiError::validation(
        "workflow.name_reserved",
        format!(
            "'{name}' is the name of a workflow this server ships, and those are reseeded on every start — an entry using it would be overwritten. Pick another name."
        ),
    )
}

fn builtin_immutable() -> ApiError {
    ApiError::validation(
        "workflow.builtin",
        "This workflow ships with the server and is reseeded on every start, so editing or deleting it here would be undone silently. Copy it under a new name and change the copy.",
    )
}

/// A UNIQUE violation on `name` is a conflict the caller can fix, not a 500.
fn duplicate_name_or(e: rusqlite::Error, name: &str) -> ApiError {
    let msg = e.to_string();
    if msg.contains("UNIQUE") {
        return ApiError::conflict(
            "workflow.name_taken",
            format!(
                "A workflow named '{name}' already exists. Pick another name, or edit that one."
            ),
        );
    }
    ApiError::from(e)
}
