//! Environments: the places a check can actually be run.
//!
//! A verdict is a claim about a running system, and until this existed Takomo
//! stored the claim without ever recording what it was made against. An agent
//! handed "re-verify the six stale cases" had to be told the URL, the way to
//! bring the thing up, and whether it was safe to write to it — all out of band,
//! all going stale silently. This is that context, in the store, next to the
//! verdicts it qualifies.
//!
//! **Takomo stores; the agent computes**, exactly as the rest of Checklist does.
//! Nothing here is executed, polled, deployed or health-checked. `bring_up` and
//! `teardown` are prose an agent reads and runs in its own harness, which is why
//! they are free text: structuring them into a command spec would be a promise
//! the store cannot keep, since the store never runs them.
//!
//! Two rules are worth stating because they are the ones a future change is
//! likely to erode.
//!
//! **`credentials_hint` is a pointer and never a secret.** Any token with `read`
//! can list environments, so a secret here would be a secret handed to every
//! reader. The field name is chosen to refuse on sight, the validator rejects
//! anything shaped like a PEM block, and the cap is short enough that a pasted
//! key does not fit comfortably. None of that is real secret-detection, and it is
//! deliberately not: a heuristic that catches most pasted secrets is worse than
//! none, because it teaches people it works.
//!
//! **`slug` is immutable.** Checks and tool calls address an environment by
//! slug, so renaming one would silently break every reference to it. A new name
//! is a new environment, and the old one is archived.
//!
//! Archiving, not deletion, for the same reason a check is archived: a
//! decommissioned box is still the evidence behind every verdict ever taken
//! there, and deleting it would orphan that history.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::{Environment, ENVIRONMENT_DATA_STATES, ENVIRONMENT_KINDS, MAX_METADATA};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{environment_id, now_ms};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};

/// Cap on live environments per project. Generous for any real topology —
/// local, ephemeral, two staging tiers, production — and finite so a loop
/// cannot fill the table.
pub const MAX_ENVIRONMENTS_PER_PROJECT: i64 = 100;

/// Default and maximum page size when listing environments.
pub const MAX_ENVIRONMENTS_PAGE: i64 = 200;

const MAX_ENV_NAME: usize = 120;
const MAX_ENV_URL: usize = 2048;
const MAX_ENV_PROSE: usize = 4096;
const MAX_ENV_HINT: usize = 300;

const ENV_COLS: &str = "id, project, slug, name, kind, base_url, bring_up, teardown, \
    data_state, writable, credentials_hint, notes, metadata, version, created_by, \
    created_at, updated_at, archived_at";

/// What a caller sends to register an environment.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentCreate {
    pub project: String,
    pub slug: String,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub base_url: Option<String>,
    pub bring_up: Option<String>,
    pub teardown: Option<String>,
    pub data_state: Option<String>,
    pub writable: Option<bool>,
    pub credentials_hint: Option<String>,
    pub notes: Option<String>,
    pub metadata: Option<Value>,
}

/// A partial update. The nullable fields use an override slot — absent leaves
/// them alone, explicit null clears them — for the same reason a check's policy
/// overrides do: once a value is set, "unset it again" has to be expressible.
///
/// There is deliberately no `slug`.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentPatch {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub base_url: Option<Option<String>>,
    pub bring_up: Option<String>,
    pub teardown: Option<String>,
    pub data_state: Option<String>,
    pub writable: Option<bool>,
    pub credentials_hint: Option<Option<String>>,
    pub notes: Option<String>,
    pub metadata_merge: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentFilter {
    pub project: String,
    pub kind: Option<String>,
    pub include_archived: bool,
    pub limit: Option<i64>,
}

// Written one function per field rather than generated, for the same reason the
// checklist validators are: the error-code scan in `tests/api.rs` reads source
// text, so a code reached through a shared helper or a macro body is invisible
// to it — and a code the scan cannot see can drift out of the documented
// vocabulary without anything failing.

/// A slug is a handle a person types and an agent passes: same shape as a
/// project id, and bounded for the same reason.
fn validate_environment_slug(value: &str) -> ApiResult<()> {
    let bytes = value.as_bytes();
    let ok = (2..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if !ok {
        return Err(ApiError::validation(
            "validation.environment_slug",
            format!(
                "'{value}' is not a valid environment slug. Use 2-32 characters: a leading \
                 lowercase letter, then lowercase letters, digits or dashes."
            ),
        )
        .remedy("Send {\"slug\": \"staging\"}.".to_string()));
    }
    Ok(())
}

fn validate_environment_name(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n == 0 {
        return Err(ApiError::validation(
            "validation.environment_name",
            "An environment needs a non-empty 'name'.",
        )
        .remedy("Send {\"name\": \"Staging\"}, or omit it to reuse the slug.".to_string()));
    }
    if n > MAX_ENV_NAME {
        return Err(ApiError::validation(
            "validation.environment_name",
            format!("'name' is {n} characters; the maximum is {MAX_ENV_NAME}."),
        )
        .remedy(format!(
            "Shorten 'name' to {MAX_ENV_NAME} characters or fewer."
        )));
    }
    Ok(())
}

fn validate_environment_kind(value: &str) -> ApiResult<()> {
    if !ENVIRONMENT_KINDS.contains(&value) {
        return Err(ApiError::validation(
            "validation.environment_kind",
            format!(
                "'{value}' is not a valid 'kind'. Valid values: {}.",
                ENVIRONMENT_KINDS.join(", ")
            ),
        )
        .remedy(format!("Send one of: {}.", ENVIRONMENT_KINDS.join(", "))));
    }
    Ok(())
}

fn validate_environment_data_state(value: &str) -> ApiResult<()> {
    if !ENVIRONMENT_DATA_STATES.contains(&value) {
        return Err(ApiError::validation(
            "validation.environment_data_state",
            format!(
                "'{value}' is not a valid 'data_state'. Valid values: {}.",
                ENVIRONMENT_DATA_STATES.join(", ")
            ),
        )
        .remedy(format!(
            "Send one of: {}.",
            ENVIRONMENT_DATA_STATES.join(", ")
        )));
    }
    Ok(())
}

fn validate_environment_base_url(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_ENV_URL {
        return Err(ApiError::validation(
            "validation.environment_base_url",
            format!("'base_url' is {n} characters; the maximum is {MAX_ENV_URL}."),
        )
        .remedy("Send the origin only, e.g. https://staging.example.com.".to_string()));
    }
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(ApiError::validation(
            "validation.environment_base_url",
            format!("'base_url' is '{value}'; it must start with http:// or https://."),
        )
        .remedy(
            "Send an absolute URL, e.g. https://staging.example.com, or null if this \
             environment has no HTTP surface."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_environment_prose(field: &str, value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_ENV_PROSE {
        return Err(ApiError::validation(
            "validation.environment_prose",
            format!("'{field}' is {n} characters; the maximum is {MAX_ENV_PROSE}."),
        )
        .remedy(format!(
            "Keep '{field}' to the commands and caveats a runner needs; link out for the rest."
        )));
    }
    Ok(())
}

/// The one field with a content rule rather than only a length rule.
///
/// This is a POINTER to where a credential lives, and every `read` token can see
/// it. The PEM check is not secret detection and is not pretending to be — it
/// catches the single most common way a private key gets pasted into a text
/// field, and refuses it loudly enough that the next person reads the docs.
fn validate_environment_credentials_hint(value: &str) -> ApiResult<()> {
    let n = value.chars().count();
    if n > MAX_ENV_HINT {
        return Err(ApiError::validation(
            "validation.environment_credentials_hint",
            format!("'credentials_hint' is {n} characters; the maximum is {MAX_ENV_HINT}."),
        )
        .remedy(
            "This field points at where a credential lives — an env-var name, a vault \
             path, a runbook URL — so it should be short."
                .to_string(),
        ));
    }
    if value.contains("-----BEGIN") {
        return Err(ApiError::validation(
            "validation.environment_credentials_hint",
            "'credentials_hint' looks like a key, not a pointer to one.",
        )
        .remedy(
            "Takomo never stores credentials, and any token with 'read' can see this \
             field. Send where the credential lives instead, e.g. \"env:STAGING_TOKEN\" \
             or \"op://vault/staging/agent\"."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_environment_metadata(raw: &str) -> ApiResult<()> {
    if raw.len() > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.environment_metadata_size",
            format!(
                "'metadata' is {} bytes; the maximum is {MAX_METADATA}.",
                raw.len()
            ),
        )
        .remedy("Move the bulk into 'notes'.".to_string()));
    }
    Ok(())
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

fn row_to_environment(row: &Row) -> rusqlite::Result<Environment> {
    let metadata_raw: String = row.get("metadata")?;
    let writable: i64 = row.get("writable")?;
    Ok(Environment {
        id: row.get("id")?,
        project: row.get("project")?,
        slug: row.get("slug")?,
        name: row.get("name")?,
        kind: row.get("kind")?,
        base_url: row.get("base_url")?,
        bring_up: row.get("bring_up")?,
        teardown: row.get("teardown")?,
        data_state: row.get("data_state")?,
        writable: writable != 0,
        credentials_hint: row.get("credentials_hint")?,
        notes: row.get("notes")?,
        metadata: serde_json::from_str(&metadata_raw).unwrap_or(Value::Null),
        version: row.get("version")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
    })
}

impl Store {
    pub fn create_environment(
        &self,
        req: &EnvironmentCreate,
        actor: &str,
    ) -> ApiResult<Environment> {
        let slug = req.slug.trim().to_string();
        validate_environment_slug(&slug)?;
        // A name is a courtesy, not a requirement: the slug already identifies
        // the thing, so defaulting saves a caller from writing "staging" twice.
        let name = req
            .name
            .clone()
            .map(|n| n.trim().to_string())
            .unwrap_or_else(|| slug.clone());
        validate_environment_name(&name)?;
        let kind = req.kind.clone().unwrap_or_else(|| "other".to_string());
        validate_environment_kind(&kind)?;
        let data_state = req
            .data_state
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        validate_environment_data_state(&data_state)?;
        if let Some(u) = &req.base_url {
            validate_environment_base_url(u)?;
        }
        let bring_up = req.bring_up.clone().unwrap_or_default();
        let teardown = req.teardown.clone().unwrap_or_default();
        let notes = req.notes.clone().unwrap_or_default();
        validate_environment_prose("bring_up", &bring_up)?;
        validate_environment_prose("teardown", &teardown)?;
        validate_environment_prose("notes", &notes)?;
        if let Some(h) = &req.credentials_hint {
            validate_environment_credentials_hint(h)?;
        }
        let metadata = req.metadata.clone().unwrap_or(Value::Null);
        let metadata_raw = metadata.to_string();
        validate_environment_metadata(&metadata_raw)?;
        // Production defaults to read-only. It errs toward refusing a
        // destructive run rather than permitting one, and it is a default rather
        // than a rule so a project that really does test writes against
        // production can say so explicitly.
        let writable = req.writable.unwrap_or(kind != "production");

        let now = now_ms();
        let id = environment_id();
        let project = req.project.clone();

        self.with_tx(|tx| {
            project_exists(tx, &project)?;
            ensure_project_writable(tx, &project)?;

            let live: i64 = tx.query_row(
                "SELECT COUNT(*) FROM environments WHERE project = ?1 AND archived_at IS NULL",
                params![project],
                |r| r.get(0),
            )?;
            if live >= MAX_ENVIRONMENTS_PER_PROJECT {
                return Err(ApiError::validation(
                    "validation.environment_count",
                    format!(
                        "Project '{project}' already has {live} live environments; the maximum \
                         is {MAX_ENVIRONMENTS_PER_PROJECT}."
                    ),
                )
                .remedy(
                    "Archive an environment nobody runs against any more, with \
                     DELETE /v1/environments/{id}."
                        .to_string(),
                ));
            }

            let taken: Option<String> = tx
                .query_row(
                    "SELECT id FROM environments WHERE project = ?1 AND slug = ?2",
                    params![project, slug],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(existing) = taken {
                return Err(ApiError::conflict(
                    "conflict.environment_slug",
                    format!(
                        "Project '{project}' already has an environment with slug '{slug}' \
                         ({existing})."
                    ),
                )
                .remedy(
                    "Update that one with PATCH /v1/environments/{id}, or pick another slug. \
                     A slug is immutable because checks and tool calls address environments \
                     by it."
                        .to_string(),
                ));
            }

            tx.execute(
                "INSERT INTO environments (id, project, slug, name, kind, base_url, bring_up,
                    teardown, data_state, writable, credentials_hint, notes, metadata, version,
                    created_by, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,1,?14,?15,?15)",
                params![
                    id,
                    project,
                    slug,
                    name,
                    kind,
                    req.base_url,
                    bring_up,
                    teardown,
                    data_state,
                    i64::from(writable),
                    req.credentials_hint,
                    notes,
                    metadata_raw,
                    actor,
                    now,
                ],
            )?;
            emit_event(
                tx,
                None,
                Some(&project),
                actor,
                "environment.created",
                json!({ "environment": id, "slug": slug, "kind": kind, "writable": writable }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
            ))?;
            Ok(stmt.query_row(params![id], row_to_environment)?)
        })
    }

    pub fn list_environments(
        &self,
        filter: &EnvironmentFilter,
    ) -> ApiResult<(Vec<Environment>, i64)> {
        let limit = filter
            .limit
            .unwrap_or(MAX_ENVIRONMENTS_PAGE)
            .clamp(1, MAX_ENVIRONMENTS_PAGE);
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {ENV_COLS} FROM environments WHERE project = ?1");
            if !filter.include_archived {
                sql.push_str(" AND archived_at IS NULL");
            }
            if let Some(k) = &filter.kind {
                validate_environment_kind(k)?;
                sql.push_str(" AND kind = ?2");
            }
            // Archived last, then by slug: the list is short and read by a human
            // picking one, so a stable alphabetical order beats recency.
            sql.push_str(" ORDER BY archived_at IS NOT NULL, slug");

            let count_sql = sql
                .replacen(&format!("SELECT {ENV_COLS}"), "SELECT COUNT(*)", 1)
                .replace(" ORDER BY archived_at IS NOT NULL, slug", "");

            let (rows, total) = if let Some(k) = &filter.kind {
                let mut stmt = conn.prepare(&format!("{sql} LIMIT ?3"))?;
                let rows = stmt
                    .query_map(params![filter.project, k, limit], row_to_environment)?
                    .collect::<Result<Vec<_>, _>>()?;
                let total: i64 =
                    conn.query_row(&count_sql, params![filter.project, k], |r| r.get(0))?;
                (rows, total)
            } else {
                let mut stmt = conn.prepare(&format!("{sql} LIMIT ?2"))?;
                let rows = stmt
                    .query_map(params![filter.project, limit], row_to_environment)?
                    .collect::<Result<Vec<_>, _>>()?;
                let total: i64 =
                    conn.query_row(&count_sql, params![filter.project], |r| r.get(0))?;
                (rows, total)
            };
            Ok((rows, total))
        })
    }

    pub fn get_environment(&self, id: &str) -> ApiResult<Environment> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
            ))?;
            stmt.query_row(params![id], row_to_environment)
                .optional()?
                .ok_or_else(|| ApiError::not_found("environment", id))
        })
    }

    pub fn patch_environment(
        &self,
        id: &str,
        req: &EnvironmentPatch,
        actor: &str,
    ) -> ApiResult<Environment> {
        if let Some(n) = &req.name {
            validate_environment_name(n.trim())?;
        }
        if let Some(k) = &req.kind {
            validate_environment_kind(k)?;
        }
        if let Some(d) = &req.data_state {
            validate_environment_data_state(d)?;
        }
        if let Some(Some(u)) = &req.base_url {
            validate_environment_base_url(u)?;
        }
        if let Some(v) = &req.bring_up {
            validate_environment_prose("bring_up", v)?;
        }
        if let Some(v) = &req.teardown {
            validate_environment_prose("teardown", v)?;
        }
        if let Some(v) = &req.notes {
            validate_environment_prose("notes", v)?;
        }
        if let Some(Some(h)) = &req.credentials_hint {
            validate_environment_credentials_hint(h)?;
        }

        let now = now_ms();
        let id = id.to_string();
        self.with_tx(|tx| {
            let existing = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
                ))?;
                stmt.query_row(params![id], row_to_environment)
                    .optional()?
                    .ok_or_else(|| ApiError::not_found("environment", &id))?
            };
            ensure_project_writable(tx, &existing.project)?;

            macro_rules! set_col {
                ($val:expr, $col:literal) => {
                    if let Some(v) = $val {
                        tx.execute(
                            concat!("UPDATE environments SET ", $col, " = ?1 WHERE id = ?2"),
                            params![v, id],
                        )?;
                    }
                };
            }
            set_col!(req.name.as_ref().map(|n| n.trim().to_string()), "name");
            set_col!(req.kind.as_ref(), "kind");
            set_col!(req.bring_up.as_ref(), "bring_up");
            set_col!(req.teardown.as_ref(), "teardown");
            set_col!(req.data_state.as_ref(), "data_state");
            set_col!(req.notes.as_ref(), "notes");
            set_col!(req.writable.map(i64::from), "writable");
            // The two override slots: an explicit null clears the column.
            if let Some(v) = &req.base_url {
                tx.execute(
                    "UPDATE environments SET base_url = ?1 WHERE id = ?2",
                    params![v, id],
                )?;
            }
            if let Some(v) = &req.credentials_hint {
                tx.execute(
                    "UPDATE environments SET credentials_hint = ?1 WHERE id = ?2",
                    params![v, id],
                )?;
            }
            if let Some(merge) = &req.metadata_merge {
                let mut base = match existing.metadata.clone() {
                    Value::Object(m) => m,
                    _ => serde_json::Map::new(),
                };
                if let Value::Object(m) = merge {
                    for (k, v) in m {
                        if v.is_null() {
                            base.remove(k);
                        } else {
                            base.insert(k.clone(), v.clone());
                        }
                    }
                }
                let raw = Value::Object(base).to_string();
                validate_environment_metadata(&raw)?;
                tx.execute(
                    "UPDATE environments SET metadata = ?1 WHERE id = ?2",
                    params![raw, id],
                )?;
            }

            tx.execute(
                "UPDATE environments SET version = version + 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            emit_event(
                tx,
                None,
                Some(&existing.project),
                actor,
                "environment.updated",
                json!({ "environment": id }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
            ))?;
            Ok(stmt.query_row(params![id], row_to_environment)?)
        })
    }

    /// Archive an environment, and report the live checks still pointing at it.
    ///
    /// The count comes back in the response for the same reason a release push
    /// reports its impact: the caller learns the consequence of what it just did
    /// without having to know to ask. Nothing is detached — a check that names an
    /// archived environment is a finding, not a corruption.
    pub fn archive_environment(&self, id: &str, actor: &str) -> ApiResult<Environment> {
        let now = now_ms();
        let id = id.to_string();
        self.with_tx(|tx| {
            let existing = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
                ))?;
                stmt.query_row(params![id], row_to_environment)
                    .optional()?
                    .ok_or_else(|| ApiError::not_found("environment", &id))?
            };
            ensure_project_writable(tx, &existing.project)?;
            if existing.archived_at.is_some() {
                return Ok(existing);
            }
            tx.execute(
                "UPDATE environments SET archived_at = ?1, updated_at = ?1, version = version + 1 \
                 WHERE id = ?2",
                params![now, id],
            )?;
            emit_event(
                tx,
                None,
                Some(&existing.project),
                actor,
                "environment.archived",
                json!({ "environment": id, "slug": existing.slug }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
            ))?;
            Ok(stmt.query_row(params![id], row_to_environment)?)
        })
    }

    /// Bring an archived environment back. Archiving changed nothing else about
    /// it, so this is a pure reversal — the same property `unarchive` has for a
    /// project.
    pub fn unarchive_environment(&self, id: &str, actor: &str) -> ApiResult<Environment> {
        let now = now_ms();
        let id = id.to_string();
        self.with_tx(|tx| {
            let existing = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
                ))?;
                stmt.query_row(params![id], row_to_environment)
                    .optional()?
                    .ok_or_else(|| ApiError::not_found("environment", &id))?
            };
            ensure_project_writable(tx, &existing.project)?;
            if existing.archived_at.is_none() {
                return Ok(existing);
            }
            tx.execute(
                "UPDATE environments SET archived_at = NULL, updated_at = ?1, \
                 version = version + 1 WHERE id = ?2",
                params![now, id],
            )?;
            emit_event(
                tx,
                None,
                Some(&existing.project),
                actor,
                "environment.unarchived",
                json!({ "environment": id, "slug": existing.slug }),
                now,
            )?;
            let mut stmt = tx.prepare(&format!(
                "SELECT {ENV_COLS} FROM environments WHERE id = ?1"
            ))?;
            Ok(stmt.query_row(params![id], row_to_environment)?)
        })
    }
}
