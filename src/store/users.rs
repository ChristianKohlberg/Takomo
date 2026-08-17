//! The people directory: one row per human, global to the server, plus which
//! projects each is a member of.
//!
//! **A user says who work is waiting on. A scope says what a credential may do.**
//! Nothing in this module authenticates anything — takomo has four independent
//! token paths and no login, and a directory of people must not become a fifth
//! credential type. What it adds is *addressing*: a question can be waiting on
//! Ada rather than on whoever happens to hold `expert:domain:billing`.
//!
//! Two places that boundary is load-bearing, and one place it deliberately bends:
//!
//! - Membership ([`Store::add_member`]) decides who may be *handed* work in a
//!   project. It is not access control — a token's own `projects` allowlist still
//!   decides what that credential may read or write.
//! - `disabled_at` is a gate, in the same idiom as `projects.archived_at`: a
//!   disabled person cannot be newly assigned, while every past record naming
//!   them keeps resolving. There is deliberately no delete.
//! - The bend: a named assignee may answer an `approve` question (see
//!   `super::questions`). That is why `tokens.user` is admin-set and why
//!   [`Store::assignable_user`] refuses a disabled person or a non-member —
//!   assignment is the only route by which this directory confers authority, so
//!   both fences live in front of it.

use super::helpers::{emit_event, ensure_project_writable};
use super::merge_patch;
use super::model::{User, UserMembership, MAX_METADATA};
use super::tags::{handle_shape_ok, HANDLE_MAX};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, user_id};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

const MAX_NAME: usize = 200;
const MAX_EMAIL: usize = 320;

/// Page ceiling for `GET /v1/users`, matching the other keyed listings.
pub const MAX_USERS_PAGE: i64 = 200;

const USER_COLS: &str =
    "id, handle, name, email, meta, disabled_at, created_by, created_at, updated_at";

#[derive(Debug, Clone, Default)]
pub struct UserCreate {
    pub handle: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub meta: Option<Value>,
    /// Projects to make this person a member of at creation. Convenience only:
    /// the same rows [`Store::add_member`] writes, in the same transaction, so a
    /// person and their first project land together or not at all.
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UserPatch {
    pub name: Option<String>,
    /// `Some(None)` clears the address; `None` leaves it alone. Absent and null
    /// mean different things on the wire here for the same reason they do on a
    /// checklist policy override.
    pub email: Option<Option<String>>,
    pub meta_merge: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct UserListFilter {
    /// Case-insensitive substring match on handle, name or email.
    pub q: Option<String>,
    /// Only people who are members of this project.
    pub project: Option<String>,
    /// Include disabled people. Off by default: the common read is "who can I
    /// hand this to", and a directory that leads with people who left answers a
    /// question nobody asked.
    pub include_disabled: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Validate a user `handle`. Deliberately the *tag* handle rule
/// ([`handle_shape_ok`]) so `person:<handle>` stays a legal tag reference — the
/// one thing that keeps the existing `person:ada` convention pointing at this
/// directory instead of forking away from it.
pub fn validate_user_handle(handle: &str) -> ApiResult<()> {
    if handle_shape_ok(handle) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.user_handle",
        format!(
            "User handle '{handle}' is invalid. Use 1-{HANDLE_MAX} lowercase chars matching ^[a-z0-9][a-z0-9._-]*$ (e.g. 'ada', 'j.chen'). It is the person's stable identity and must also be usable as a 'person:<handle>' tag, so the real name goes in 'name'."
        ),
    ))
}

fn validate_name(name: &str) -> ApiResult<()> {
    if name.chars().count() > MAX_NAME {
        return Err(ApiError::validation(
            "validation.user_name",
            format!("User name must be at most {MAX_NAME} characters."),
        ));
    }
    Ok(())
}

fn validate_email(email: &str) -> ApiResult<()> {
    if email.len() > MAX_EMAIL {
        return Err(ApiError::validation(
            "validation.user_email",
            format!("User email must be at most {MAX_EMAIL} characters."),
        ));
    }
    // Deliberately shallow. takomo never sends mail, so an address here is a note
    // for a human reader; a strict RFC 5322 check would refuse valid addresses to
    // protect nothing.
    if !email.is_empty() && !email.contains('@') {
        return Err(ApiError::validation(
            "validation.user_email",
            format!("'{email}' does not look like an email address (no '@'). Omit the field if you do not have one."),
        ));
    }
    Ok(())
}

fn validate_meta(meta: &Value) -> ApiResult<()> {
    if !meta.is_object() {
        return Err(ApiError::validation(
            "validation.user_meta",
            "User meta must be a JSON object (free-form attributes like {\"timezone\": \"Europe/Berlin\"}).",
        ));
    }
    let size = serde_json::to_string(meta).map(|s| s.len()).unwrap_or(0);
    if size > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.user_meta_size",
            format!("User meta is {size} bytes serialized; the cap is {MAX_METADATA}."),
        ));
    }
    Ok(())
}

fn row_to_user(r: &rusqlite::Row) -> rusqlite::Result<User> {
    let meta_raw: String = r.get("meta")?;
    Ok(User {
        id: r.get("id")?,
        handle: r.get("handle")?,
        name: r.get("name")?,
        email: r.get("email")?,
        meta: serde_json::from_str(&meta_raw).unwrap_or(Value::Null),
        disabled_at: r.get("disabled_at")?,
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        projects: Vec::new(),
    })
}

/// Load one user by id **or** handle, inside an existing transaction.
///
/// Both spellings, because both are in circulation and neither is wrong: an API
/// caller writes `ada` (what a person types), while a stored reference is a
/// `usr-…` id (what survives a rename). Resolving them in one place is what keeps
/// every call site from having to guess which it holds.
pub(crate) fn lookup_user(conn: &Connection, id_or_handle: &str) -> ApiResult<Option<User>> {
    let user = conn
        .query_row(
            &format!("SELECT {USER_COLS} FROM users WHERE id = ?1 OR handle = ?1"),
            params![id_or_handle],
            row_to_user,
        )
        .optional()?;
    Ok(user)
}

/// Load a user by id or handle, or a teaching 404. The message names the create
/// route rather than only reporting absence, because there is no lazy-create
/// here: a directory that grows a new person from a typo is worse than one that
/// refuses, since the typo would then be assignable and look real.
pub(crate) fn require_user(conn: &Connection, id_or_handle: &str) -> ApiResult<User> {
    lookup_user(conn, id_or_handle)?.ok_or_else(|| {
        ApiError::not_found("user", id_or_handle).remedy(
            "List the directory with GET /v1/users, or add the person with POST /v1/users (admin) — a user is never created implicitly.",
        )
    })
}

fn memberships_of(conn: &Connection, user: &str) -> ApiResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT project FROM user_projects WHERE \"user\" = ?1 ORDER BY project")?;
    let rows = stmt
        .query_map(params![user], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn is_member(conn: &Connection, user: &str, project: &str) -> ApiResult<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM user_projects WHERE \"user\" = ?1 AND project = ?2",
            params![user, project],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

impl Store {
    pub fn create_user(&self, req: &UserCreate, actor: &str) -> ApiResult<User> {
        validate_user_handle(&req.handle)?;
        let name = req.name.clone().unwrap_or_default();
        validate_name(&name)?;
        if let Some(email) = &req.email {
            validate_email(email)?;
        }
        let meta = req.meta.clone().unwrap_or_else(|| json!({}));
        validate_meta(&meta)?;
        let now = now_ms();
        self.with_tx(|tx| {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM users WHERE handle = ?1",
                    params![req.handle],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Err(ApiError::conflict(
                    "user.exists",
                    format!(
                        "A user with handle '{}' already exists ({id}). PATCH /v1/users/{} to change their name, email or meta.",
                        req.handle, req.handle
                    ),
                ));
            }
            let id = user_id();
            tx.execute(
                "INSERT INTO users (id, handle, name, email, meta, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, req.handle, name, req.email, meta.to_string(), actor, now],
            )?;
            emit_event(
                tx,
                None,
                None,
                actor,
                "user_created",
                json!({ "user": id, "handle": req.handle }),
                now,
            )?;
            let mut projects = Vec::new();
            for project in &req.projects {
                insert_membership(tx, &id, project, actor, now)?;
                projects.push(project.clone());
            }
            projects.sort();
            projects.dedup();
            Ok(User {
                id,
                handle: req.handle.clone(),
                name,
                email: req.email.clone(),
                meta,
                disabled_at: None,
                created_by: actor.to_string(),
                created_at: now,
                updated_at: now,
                projects,
            })
        })
    }

    /// One page of the directory, plus how many matched. The count uses the *same*
    /// predicate as the page for the reason `api::paged` exists: a reader — usually
    /// an agent — must be able to tell a page from the whole directory.
    pub fn list_users(&self, filter: &UserListFilter) -> ApiResult<(Vec<User>, i64)> {
        self.with_conn(|conn| {
            let mut where_sql = String::from(" WHERE 1=1");
            let mut binds: Vec<rusqlite::types::Value> = Vec::new();
            if !filter.include_disabled {
                where_sql.push_str(" AND u.disabled_at IS NULL");
            }
            if let Some(project) = &filter.project {
                where_sql.push_str(
                    " AND EXISTS (SELECT 1 FROM user_projects m WHERE m.\"user\" = u.id AND m.project = ?)",
                );
                binds.push(rusqlite::types::Value::Text(project.clone()));
            }
            if let Some(q) = &filter.q {
                where_sql.push_str(
                    " AND (LOWER(u.handle) LIKE ? OR LOWER(u.name) LIKE ? OR LOWER(COALESCE(u.email, '')) LIKE ?)",
                );
                let needle = format!("%{}%", q.to_lowercase());
                for _ in 0..3 {
                    binds.push(rusqlite::types::Value::Text(needle.clone()));
                }
            }

            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM users u{where_sql}"),
                rusqlite::params_from_iter(binds.clone()),
                |r| r.get(0),
            )?;

            let cols = USER_COLS
                .split(", ")
                .map(|c| format!("u.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut sql = format!("SELECT {cols} FROM users u{where_sql} ORDER BY u.handle");
            sql.push_str(" LIMIT ? OFFSET ?");
            let mut page_binds = binds;
            page_binds.push(rusqlite::types::Value::Integer(filter.limit));
            page_binds.push(rusqlite::types::Value::Integer(filter.offset));

            let mut stmt = conn.prepare(&sql)?;
            let mut users = stmt
                .query_map(rusqlite::params_from_iter(page_binds), row_to_user)?
                .collect::<Result<Vec<_>, _>>()?;
            // Memberships for the page only, never for the whole table: this read is
            // what the assignee picker calls, so its cost has to scale with the page.
            for user in &mut users {
                user.projects = memberships_of(conn, &user.id)?;
            }
            Ok((users, total))
        })
    }

    pub fn get_user(&self, id_or_handle: &str) -> ApiResult<Option<User>> {
        self.with_conn(|conn| {
            let Some(mut user) = lookup_user(conn, id_or_handle)? else {
                return Ok(None);
            };
            user.projects = memberships_of(conn, &user.id)?;
            Ok(Some(user))
        })
    }

    /// Resolve many `person:<handle>` references (or bare handles, or ids) to the
    /// people behind them, keyed by the spelling that was asked for.
    ///
    /// One statement per distinct reference, and the caller is expected to hand it
    /// a bounded set — a ticket's tags, a page of initiative entries — for the same
    /// reason `ensure_tags_exist` is capped: an unbounded loop here would run under
    /// the read connection while a page waits on it.
    ///
    /// A reference with no matching person is simply absent from the map. That is
    /// the degradation the display path wants: an unresolved `person:someone`
    /// renders as the slug it always did, exactly as it did before the directory
    /// existed.
    pub fn resolve_users(&self, refs: &[String]) -> ApiResult<HashMap<String, User>> {
        if refs.is_empty() {
            return Ok(HashMap::new());
        }
        self.with_conn(|conn| {
            let mut out = HashMap::new();
            for raw in refs {
                let key = raw.strip_prefix("person:").unwrap_or(raw);
                if out.contains_key(raw) {
                    continue;
                }
                if let Some(user) = lookup_user(conn, key)? {
                    out.insert(raw.clone(), user);
                }
            }
            Ok(out)
        })
    }

    pub fn patch_user(
        &self,
        id_or_handle: &str,
        patch: &UserPatch,
        actor: &str,
    ) -> ApiResult<User> {
        if let Some(name) = &patch.name {
            validate_name(name)?;
        }
        if let Some(Some(email)) = &patch.email {
            validate_email(email)?;
        }
        let now = now_ms();
        self.with_tx(|tx| {
            let mut user = require_user(tx, id_or_handle)?;
            if let Some(name) = &patch.name {
                user.name = name.clone();
            }
            if let Some(email) = &patch.email {
                user.email = email.clone();
            }
            if let Some(m) = &patch.meta_merge {
                if !m.is_object() {
                    return Err(ApiError::validation(
                        "validation.user_meta",
                        "meta_merge must be a JSON object (keys set to null are removed).",
                    ));
                }
                merge_patch(&mut user.meta, m);
                validate_meta(&user.meta)?;
            }
            tx.execute(
                "UPDATE users SET name = ?2, email = ?3, meta = ?4, updated_at = ?5 WHERE id = ?1",
                params![user.id, user.name, user.email, user.meta.to_string(), now],
            )?;
            user.updated_at = now;
            emit_event(
                tx,
                None,
                None,
                actor,
                "user_updated",
                json!({ "user": user.id, "handle": user.handle }),
                now,
            )?;
            user.projects = memberships_of(tx, &user.id)?;
            Ok(user)
        })
    }

    /// Disable or re-enable a person. The reversible counterpart to the delete this
    /// module does not have.
    ///
    /// Disabling revokes nothing: their tokens keep working, because a credential's
    /// authority is its scopes and taking someone out of the directory is not a
    /// statement about their credentials. What it stops is *new* work being
    /// addressed to them, and the assignee route to approving — which is the only
    /// authority this directory grants. Revoke the token to end access.
    pub fn set_user_disabled(
        &self,
        id_or_handle: &str,
        disabled: bool,
        actor: &str,
    ) -> ApiResult<User> {
        let now = now_ms();
        self.with_tx(|tx| {
            let mut user = require_user(tx, id_or_handle)?;
            if user.active() != disabled {
                // Already in the requested state — report it rather than emitting a
                // second event that reads as a change.
                user.projects = memberships_of(tx, &user.id)?;
                return Ok(user);
            }
            let disabled_at = if disabled { Some(now) } else { None };
            tx.execute(
                "UPDATE users SET disabled_at = ?2, updated_at = ?3 WHERE id = ?1",
                params![user.id, disabled_at, now],
            )?;
            user.disabled_at = disabled_at;
            user.updated_at = now;
            emit_event(
                tx,
                None,
                None,
                actor,
                if disabled {
                    "user_disabled"
                } else {
                    "user_enabled"
                },
                json!({ "user": user.id, "handle": user.handle }),
                now,
            )?;
            user.projects = memberships_of(tx, &user.id)?;
            Ok(user)
        })
    }

    pub fn add_member(
        &self,
        id_or_handle: &str,
        project: &str,
        actor: &str,
    ) -> ApiResult<UserMembership> {
        let now = now_ms();
        self.with_tx(|tx| {
            ensure_project_writable(tx, project)?;
            let user = require_user(tx, id_or_handle)?;
            insert_membership(tx, &user.id, project, actor, now)?;
            Ok(UserMembership {
                user: user.id,
                project: project.to_string(),
                added_by: actor.to_string(),
                created_at: now,
            })
        })
    }

    /// Remove a membership. Returns false when there was none, so the handler can
    /// answer 404 rather than pretending it removed something.
    ///
    /// Existing assignments are left alone on purpose: a question already waiting
    /// on this person stays waiting on them, because retracting it silently would
    /// leave a decision nobody is looking at. They simply cannot be handed
    /// anything new here.
    pub fn remove_member(&self, id_or_handle: &str, project: &str, actor: &str) -> ApiResult<bool> {
        let now = now_ms();
        self.with_tx(|tx| {
            ensure_project_writable(tx, project)?;
            let user = require_user(tx, id_or_handle)?;
            let removed = tx.execute(
                "DELETE FROM user_projects WHERE \"user\" = ?1 AND project = ?2",
                params![user.id, project],
            )?;
            if removed == 0 {
                return Ok(false);
            }
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "user_project_removed",
                json!({ "user": user.id, "handle": user.handle }),
                now,
            )?;
            Ok(true)
        })
    }

    /// Resolve a person work is about to be addressed to, in `project`, or refuse
    /// with the reason.
    ///
    /// The single gate in front of every assignment, and the reason it is one
    /// function: assignment is the only route by which this directory confers
    /// authority (a named assignee may answer an `approve`), so "is this person
    /// real, active, and on this project" must have exactly one answer no matter
    /// which surface asked. Runs inside the caller's transaction.
    pub(crate) fn assignable_user(
        conn: &Connection,
        id_or_handle: &str,
        project: &str,
    ) -> ApiResult<User> {
        let user = require_user(conn, id_or_handle)?;
        if !user.active() {
            return Err(ApiError::conflict(
                "user.disabled",
                format!(
                    "'{}' is disabled, so new work cannot be addressed to them. Their past answers and decisions are untouched.",
                    user.handle
                ),
            )
            .remedy(format!(
                "Assign someone else, or re-enable them with POST /v1/users/{}/enable (admin).",
                user.handle
            )));
        }
        if !is_member(conn, &user.id, project)? {
            return Err(ApiError::conflict(
                "user.not_member",
                format!(
                    "'{}' is not a member of project '{project}', so work here cannot be addressed to them. Membership is what says who may be handed work in a project — it is separate from what their token may read or write.",
                    user.handle
                ),
            )
            .remedy(format!(
                "Add them with POST /v1/users/{}/projects {{\"project\":\"{project}\"}} (admin), or assign a member.",
                user.handle
            )));
        }
        Ok(user)
    }
}

/// Insert one membership row, idempotently, emitting an event only when it is new.
///
/// `ON CONFLICT DO NOTHING` collapses check-then-write into one atomic statement,
/// the same shape `ensure_tags_exist` uses, so re-adding a member is a no-op
/// rather than a 409 — the caller asked for a state, and that state holds.
fn insert_membership(
    tx: &rusqlite::Transaction,
    user: &str,
    project: &str,
    actor: &str,
    now: i64,
) -> ApiResult<()> {
    let project_exists: Option<String> = tx
        .query_row(
            "SELECT id FROM projects WHERE id = ?1",
            params![project],
            |r| r.get(0),
        )
        .optional()?;
    if project_exists.is_none() {
        return Err(ApiError::not_found("project", project));
    }
    let created = tx.execute(
        "INSERT INTO user_projects (\"user\", project, added_by, created_at) \
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT (\"user\", project) DO NOTHING",
        params![user, project, actor, now],
    )?;
    if created > 0 {
        emit_event(
            tx,
            None,
            Some(project),
            actor,
            "user_project_added",
            json!({ "user": user }),
            now,
        )?;
    }
    Ok(())
}
