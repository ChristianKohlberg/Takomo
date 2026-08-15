//! Schedules: a cadence that materializes ordinary tickets.
//!
//! A schedule is a rule, not a kind of work. What it produces is an ordinary
//! ticket — claimable, leasable, fenced, transitioned through the project's own
//! workflow — carrying three extra columns that say where it came from
//! (`schedule`, `occurrence`) and how long it counts as live (`expires_at`).
//! Nothing about the board, the ready queue or the event log has to learn a new
//! shape.
//!
//! Two properties are load-bearing, and both are the reason this file is short.
//!
//! **Occurrences are independent of one another.** [`Store::materialize_due`]
//! never reads a previous occurrence, so there is no overlap policy, no
//! predecessor lookup, and no edge between the tickets one schedule produces.
//! The deadline that makes that safe is computed from the cadence alone
//! ([`crate::schedule`]), stamped once at creation, and never revisited.
//!
//! **Exactly one ticket per slot is structural, not defensive.**
//! `UNIQUE(tickets.schedule, tickets.occurrence)` is what guarantees it, so two
//! concurrent sweeps, a manual `/run` racing the timer, and a restart mid-tick
//! all converge on one ticket: the second INSERT cannot land. That is the same
//! shape as the ready queue's guarantee, where the process-wide write mutex —
//! not a check — is what makes a claim exclusive.
//!
//! Expiry deliberately changes no state. An expired occurrence is not archived,
//! cancelled or transitioned, because doing any of those would need a legal edge
//! in every project's workflow and a scheduler must not be able to hit an
//! illegal-transition wall. It simply stops being live work: it drops out of the
//! ready queue and reads as `not_fulfilled`. Closing it out is ordinary work for
//! a maintenance agent — which can itself be a schedule.

use super::helpers::{emit_event, ensure_project_writable};
use super::model::{Schedule, ScheduleOccurrence, MAX_TITLE, PRIORITIES, TICKET_TYPES};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, schedule_id, ticket_suffix};
use crate::schedule::Cadence;
use chrono::{DateTime, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// The lifecycle a schedule row moves through.
///
/// `pending` is the one that matters: a schedule an agent proposed over MCP
/// lands there with `next_slot = NULL`, so the sweep's index cannot see it. It is
/// inert by construction rather than by an `if` somewhere — which is the
/// difference between a safety property and a bug waiting for a refactor.
pub const SCHEDULE_STATUSES: [&str; 5] = ["pending", "active", "paused", "rejected", "retired"];

/// Cap on how many schedules one project may have in a non-terminal status.
/// Generous for a team, finite so a loop cannot fill the table.
pub const MAX_SCHEDULES_PER_PROJECT: i64 = 50;

/// Default/max page size when listing schedules or occurrences.
pub const MAX_SCHEDULES_PAGE: i64 = 200;

/// How many missed slots [`Store::materialize_due`] will skip past in one tick.
///
/// Only the most recent due slot fires, so a server that was down needs to walk
/// forward over the slots it missed to find it. A daily cadence down for a year
/// is 365 steps; this bounds the walk at a few years of them so a wildly stale
/// `starts_at` cannot make one sweep tick unbounded.
const MAX_MISSED_SLOTS: u32 = 2_000;

/// The ticket a schedule stamps out each occurrence.
///
/// A `TicketCreate` minus `project` (the schedule already knows it) and minus
/// anything that would tie occurrences together — no `parent`, no `blocked_by`.
/// Both are deliberate: an occurrence that inherited a dependency would stop
/// being independent of its siblings, which is the invariant the whole design
/// rests on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleTemplate {
    /// May contain the occurrence placeholders — see [`Cadence::render`].
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl ScheduleTemplate {
    /// Parse and validate, reporting every problem at once.
    pub fn parse(value: &Value) -> Result<ScheduleTemplate, String> {
        if !value.is_object() {
            return Err(format!(
                "template must be an object like {}. It is a ticket create minus the project: \
                 title (required, may use the {{date}} {{week}} {{month}} {{slot}} placeholders), \
                 body, type, priority, labels, tags.",
                r#"{"title":"Weekly review — {week}","labels":["ritual"]}"#
            ));
        }
        let t: ScheduleTemplate = serde_json::from_value(value.clone()).map_err(|e| {
            format!(
                "Could not read the template: {e}. Fields: title (required), body, type, \
                 priority, labels, tags. Unknown fields are refused — parent and blocked_by are \
                 absent on purpose, because an occurrence that inherited a dependency would stop \
                 being independent of its siblings."
            )
        })?;
        let mut problems: Vec<String> = Vec::new();
        if t.title.is_empty() || t.title.len() > MAX_TITLE {
            problems.push(format!("template.title must be 1-{MAX_TITLE} characters."));
        }
        if let Some(ty) = &t.ty {
            if !TICKET_TYPES.contains(&ty.as_str()) {
                problems.push(format!(
                    "template.type must be one of {}, got '{ty}'.",
                    TICKET_TYPES.join(", ")
                ));
            }
        }
        if let Some(p) = &t.priority {
            if !PRIORITIES.contains(&p.as_str()) {
                problems.push(format!(
                    "template.priority must be one of {}, got '{p}'.",
                    PRIORITIES.join(", ")
                ));
            }
        }
        if problems.is_empty() {
            Ok(t)
        } else {
            Err(problems.join(" "))
        }
    }
}

/// The store-side shape of a schedule create.
#[derive(Debug, Clone)]
pub struct ScheduleCreate {
    pub project: String,
    pub name: String,
    pub cadence: Cadence,
    pub template: ScheduleTemplate,
    /// The interval anchor, and the earliest slot. Defaults to now.
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    /// Why the proposer thinks this recurs; shown on the confirm row.
    pub rationale: Option<String>,
}

/// Fields a PATCH may change. `None` = absent.
#[derive(Debug, Clone, Default)]
pub struct SchedulePatch {
    pub name: Option<String>,
    pub cadence: Option<Cadence>,
    pub template: Option<ScheduleTemplate>,
    pub ends_at: Option<Option<i64>>,
}

#[derive(Debug, Clone, Default)]
pub struct ScheduleListFilter {
    pub project: Option<String>,
    pub status: Option<String>,
    /// Token project scoping. None = unrestricted.
    pub allowed_projects: Option<Vec<String>>,
}

const SCHEDULE_COLS: &str = "id, project, name, cadence, template, status, proposed_by, \
    rationale, next_slot, starts_at, ends_at, created_by, created_at, updated_at, version";

fn row_to_schedule(row: &rusqlite::Row) -> rusqlite::Result<Schedule> {
    let cadence_raw: String = row.get("cadence")?;
    let template_raw: String = row.get("template")?;
    Ok(Schedule {
        id: row.get("id")?,
        project: row.get("project")?,
        name: row.get("name")?,
        // A stored cadence was validated on the way in. If it will not parse now
        // the row is corrupt, and falling back to a default would invent a
        // cadence nobody asked for — so keep the raw text and let the caller
        // surface it.
        cadence: serde_json::from_str(&cadence_raw).ok(),
        cadence_raw,
        template: serde_json::from_str(&template_raw).unwrap_or(Value::Null),
        status: row.get("status")?,
        proposed_by: row.get("proposed_by")?,
        rationale: row.get("rationale")?,
        next_slot: row.get("next_slot")?,
        starts_at: row.get("starts_at")?,
        ends_at: row.get("ends_at")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        version: row.get("version")?,
    })
}

fn ms_to_utc(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// The next slot a schedule should fire, or `None` when it has run out of
/// window. Kept in one place because create, patch, resume and materialize all
/// have to agree on it — and on the `ends_at` check, which is what turns a
/// schedule `retired` rather than leaving it firing forever.
fn compute_next_slot(sched: &Schedule, after: i64) -> Option<i64> {
    let cadence = sched.cadence.as_ref()?;
    // Never before starts_at: the anchor is also the earliest slot, so a
    // schedule created with a future start does not fire immediately.
    let from = after.max(sched.starts_at - 1);
    let slot = cadence.next_slot_after(ms_to_utc(from), ms_to_utc(sched.starts_at))?;
    let ms = slot.timestamp_millis();
    match sched.ends_at {
        Some(end) if ms > end => None,
        _ => Some(ms),
    }
}

impl Store {
    /// Create a schedule.
    ///
    /// `needs_approval` decides whether it lands `pending` (inert) or `active`.
    /// The caller resolves it from the project flag and the credential's scope,
    /// because that is an auth question and this layer does not see scopes.
    pub fn create_schedule(
        &self,
        req: &ScheduleCreate,
        actor: &str,
        needs_approval: bool,
    ) -> ApiResult<Schedule> {
        if req.name.trim().is_empty() || req.name.len() > MAX_TITLE {
            return Err(ApiError::validation(
                "validation.schedule.name",
                format!("name must be 1-{MAX_TITLE} characters and not blank."),
            ));
        }
        if let (Some(start), Some(end)) = (req.starts_at, req.ends_at) {
            if end <= start {
                return Err(ApiError::validation(
                    "validation.schedule.window",
                    "ends_at must be after starts_at.",
                ));
            }
        }
        let now = now_ms();
        let starts_at = req.starts_at.unwrap_or(now);

        self.with_tx(|tx| {
            let exists: i64 = tx.query_row(
                "SELECT COUNT(*) FROM projects WHERE id = ?1",
                params![req.project],
                |r| r.get(0),
            )?;
            if exists == 0 {
                return Err(ApiError::not_found("project", &req.project));
            }
            ensure_project_writable(tx, &req.project)?;
            let live: i64 = tx.query_row(
                "SELECT COUNT(*) FROM schedules WHERE project = ?1 AND status IN ('pending','active','paused')",
                params![req.project],
                |r| r.get(0),
            )?;
            if live >= MAX_SCHEDULES_PER_PROJECT {
                return Err(ApiError::validation(
                    "validation.schedule.too_many",
                    format!(
                        "Project '{}' already has {live} schedules that are pending, active or paused, which is the cap ({MAX_SCHEDULES_PER_PROJECT}). Delete or reject one before adding another: GET /v1/schedules?project={} lists them.",
                        req.project, req.project
                    ),
                ));
            }

            let status = if needs_approval { "pending" } else { "active" };
            let id = schedule_id();
            tx.execute(
                "INSERT INTO schedules (id, project, name, cadence, template, status, proposed_by, rationale, next_slot, starts_at, ends_at, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, ?11, ?12, ?12)",
                params![
                    id,
                    req.project,
                    req.name.trim(),
                    serde_json::to_string(&req.cadence).unwrap(),
                    serde_json::to_string(&req.template).unwrap(),
                    status,
                    if needs_approval { Some(actor) } else { None },
                    req.rationale,
                    starts_at,
                    req.ends_at,
                    actor,
                    now,
                ],
            )?;

            // next_slot is set in a second step rather than inline, so the
            // "NULL unless active" invariant lives in exactly one place.
            let mut sched = load_schedule(tx, &id)?;
            if status == "active" {
                let slot = compute_next_slot(&sched, now);
                set_next_slot(tx, &id, slot, now)?;
                sched.next_slot = slot;
                if slot.is_none() {
                    // An active schedule whose window is already over never
                    // fires; say so at creation rather than looking healthy.
                    tx.execute(
                        "UPDATE schedules SET status = 'retired' WHERE id = ?1",
                        params![id],
                    )?;
                    sched.status = "retired".to_string();
                }
            }

            emit_event(
                tx,
                None,
                Some(&req.project),
                actor,
                if needs_approval {
                    "schedule_proposed"
                } else {
                    "schedule_created"
                },
                json!({
                    "schedule": id,
                    "name": sched.name,
                    "status": sched.status,
                    "cadence": req.cadence,
                }),
                now,
            )?;
            sched.version = 1;
            Ok(sched)
        })
    }

    pub fn get_schedule(&self, id: &str) -> ApiResult<Schedule> {
        self.with_conn(|conn| load_schedule(conn, id))
    }

    pub fn list_schedules(
        &self,
        filter: &ScheduleListFilter,
        limit: i64,
    ) -> ApiResult<Vec<Schedule>> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {SCHEDULE_COLS} FROM schedules WHERE 1=1");
            let mut args: Vec<SqlValue> = Vec::new();
            if let Some(p) = &filter.project {
                sql.push_str(" AND project = ?");
                args.push(SqlValue::Text(p.clone()));
            }
            if let Some(s) = &filter.status {
                sql.push_str(" AND status = ?");
                args.push(SqlValue::Text(s.clone()));
            }
            if let Some(allowed) = &filter.allowed_projects {
                sql.push_str(" AND project IN (");
                for (i, p) in allowed.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    args.push(SqlValue::Text(p.clone()));
                }
                sql.push(')');
            }
            // Waiting-for-you first, then running, then stopped — the order the
            // page renders, so the UI never has to re-sort.
            sql.push_str(
                " ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'active' THEN 1 WHEN 'paused' THEN 2 ELSE 3 END, created_at DESC LIMIT ?",
            );
            args.push(SqlValue::Integer(limit.clamp(1, MAX_SCHEDULES_PAGE)));
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args), row_to_schedule)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Edit a schedule behind an `If-Match` version, recomputing `next_slot`
    /// whenever the cadence or the window moved.
    pub fn patch_schedule(
        &self,
        id: &str,
        patch: &SchedulePatch,
        expected_version: Option<i64>,
        actor: &str,
    ) -> ApiResult<Schedule> {
        let now = now_ms();
        self.with_tx(|tx| {
            let mut sched = load_schedule(tx, id)?;
            ensure_project_writable(tx, &sched.project)?;
            if let Some(v) = expected_version {
                if v != sched.version {
                    return Err(ApiError::conflict(
                        "conflict.version",
                        format!(
                            "Schedule '{id}' is at version {} but If-Match said {v}. Re-read it (GET /v1/schedules/{id}) and retry with the current ETag.",
                            sched.version
                        ),
                    )
                    .current_version(sched.version));
                }
            }
            if let Some(name) = &patch.name {
                if name.trim().is_empty() || name.len() > MAX_TITLE {
                    return Err(ApiError::validation(
                        "validation.schedule.name",
                        format!("name must be 1-{MAX_TITLE} characters and not blank."),
                    ));
                }
                tx.execute(
                    "UPDATE schedules SET name = ?2 WHERE id = ?1",
                    params![id, name.trim()],
                )?;
                sched.name = name.trim().to_string();
            }
            if let Some(cadence) = &patch.cadence {
                tx.execute(
                    "UPDATE schedules SET cadence = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(cadence).unwrap()],
                )?;
                sched.cadence = Some(cadence.clone());
            }
            if let Some(template) = &patch.template {
                tx.execute(
                    "UPDATE schedules SET template = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(template).unwrap()],
                )?;
                sched.template = serde_json::to_value(template).unwrap_or(Value::Null);
            }
            if let Some(ends_at) = patch.ends_at {
                if let (Some(end), start) = (ends_at, sched.starts_at) {
                    if end <= start {
                        return Err(ApiError::validation(
                            "validation.schedule.window",
                            "ends_at must be after starts_at.",
                        ));
                    }
                }
                tx.execute(
                    "UPDATE schedules SET ends_at = ?2 WHERE id = ?1",
                    params![id, ends_at],
                )?;
                sched.ends_at = ends_at;
            }

            if sched.status == "active" {
                let slot = compute_next_slot(&sched, now);
                set_next_slot(tx, id, slot, now)?;
                sched.next_slot = slot;
            }
            let version = touch_schedule(tx, id, now)?;
            sched.version = version;
            emit_event(
                tx,
                None,
                Some(&sched.project),
                actor,
                "schedule_updated",
                json!({ "schedule": id, "version": version }),
                now,
            )?;
            Ok(sched)
        })
    }

    /// Move a schedule between statuses.
    ///
    /// One method for activate/reject/pause/resume because they differ only in
    /// which transitions are legal and whether `next_slot` comes back — and
    /// having one place own that keeps the "NULL unless active" invariant true
    /// by construction.
    pub fn set_schedule_status(&self, id: &str, to: &str, actor: &str) -> ApiResult<Schedule> {
        let now = now_ms();
        self.with_tx(|tx| {
            let mut sched = load_schedule(tx, id)?;
            ensure_project_writable(tx, &sched.project)?;
            let legal: &[&str] = match sched.status.as_str() {
                "pending" => &["active", "rejected"],
                "active" => &["paused"],
                "paused" => &["active"],
                // rejected and retired are terminal: a schedule that was turned
                // down or has run out its window is re-created, not revived, so
                // its history stays honest about what was agreed when.
                _ => &[],
            };
            if !legal.contains(&to) {
                return Err(ApiError::conflict(
                    "conflict.schedule.status",
                    format!(
                        "Schedule '{id}' is {} and cannot become {to}. Legal from here: {}.",
                        sched.status,
                        if legal.is_empty() {
                            "nothing — this status is terminal".to_string()
                        } else {
                            legal.join(", ")
                        }
                    ),
                )
                .details(json!({ "status": sched.status, "allowed": legal })));
            }

            tx.execute(
                "UPDATE schedules SET status = ?2 WHERE id = ?1",
                params![id, to],
            )?;
            sched.status = to.to_string();

            // Resume computes forward from now and never backfills the pause:
            // unpausing a long-paused schedule must not dump history into the
            // queue.
            let slot = if to == "active" {
                compute_next_slot(&sched, now)
            } else {
                None
            };
            set_next_slot(tx, id, slot, now)?;
            sched.next_slot = slot;
            if to == "active" && slot.is_none() {
                tx.execute(
                    "UPDATE schedules SET status = 'retired' WHERE id = ?1",
                    params![id],
                )?;
                sched.status = "retired".to_string();
            }

            let version = touch_schedule(tx, id, now)?;
            sched.version = version;
            emit_event(
                tx,
                None,
                Some(&sched.project),
                actor,
                match to {
                    "active" => "schedule_activated",
                    "rejected" => "schedule_rejected",
                    "paused" => "schedule_paused",
                    _ => "schedule_updated",
                },
                json!({ "schedule": id, "status": sched.status }),
                now,
            )?;
            Ok(sched)
        })
    }

    /// Delete the rule. The tickets it made stay: they are real work with real
    /// history, and a dangling `schedule` id on them is the correct record of
    /// where they came from — which is also why that column carries no foreign
    /// key.
    pub fn delete_schedule(&self, id: &str, actor: &str) -> ApiResult<()> {
        let now = now_ms();
        self.with_tx(|tx| {
            let sched = load_schedule(tx, id)?;
            ensure_project_writable(tx, &sched.project)?;
            tx.execute("DELETE FROM schedules WHERE id = ?1", params![id])?;
            emit_event(
                tx,
                None,
                Some(&sched.project),
                actor,
                "schedule_deleted",
                json!({ "schedule": id, "name": sched.name }),
                now,
            )?;
            Ok(())
        })
    }

    /// Fire a schedule now, off-cycle, for the slot it would next have used.
    ///
    /// This is what makes the feature testable and the seed demo-able. It does
    /// not disturb the cadence: `next_slot` still advances to the next real slot,
    /// so a manual run is an extra occurrence rather than a shifted one.
    pub fn run_schedule_now(&self, id: &str, actor: &str) -> ApiResult<Option<String>> {
        let now = now_ms();
        self.with_tx(|tx| {
            let sched = load_schedule(tx, id)?;
            ensure_project_writable(tx, &sched.project)?;
            if sched.status != "active" {
                return Err(ApiError::conflict(
                    "conflict.schedule.status",
                    format!(
                        "Schedule '{id}' is {} — only an active schedule can be run. {}",
                        sched.status,
                        match sched.status.as_str() {
                            "pending" => "Activate it first: POST /v1/schedules/{id}/activate.",
                            "paused" => "Resume it first: POST /v1/schedules/{id}/resume.",
                            _ => "This status is terminal; create a new schedule instead.",
                        }
                    ),
                ));
            }
            let slot = sched.next_slot.unwrap_or(now);
            let ticket = materialize_one(tx, &sched, slot, now, actor)?;
            Ok(ticket)
        })
    }

    /// The sweeper pass: fire every schedule whose slot has come.
    ///
    /// Each schedule is its own transaction, so one corrupt cadence cannot stop
    /// the rest, and a long list of due schedules does not hold the write mutex
    /// for the whole batch. Returns how many tickets were created.
    pub fn materialize_due(&self) -> ApiResult<usize> {
        let now = now_ms();
        let due: Vec<String> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                // Archived projects are skipped, not errored: a schedule that
                // fired would CREATE a ticket under a project that takes no
                // work. The schedule stays active and its slots keep advancing
                // on the next pass once the project is unarchived — nothing here
                // rewrites it, so unarchiving needs no repair.
                "SELECT s.id FROM schedules s \
                 JOIN projects p ON p.id = s.project \
                 WHERE s.status = 'active' AND s.next_slot IS NOT NULL AND s.next_slot <= ?1 \
                   AND p.archived_at IS NULL \
                 ORDER BY s.next_slot",
            )?;
            let ids = stmt
                .query_map(params![now], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })?;

        let mut created = 0usize;
        for id in due {
            let outcome = self.with_tx(|tx| {
                let sched = load_schedule(tx, &id)?;
                // Re-check under the write lock: another sweep or a /run may have
                // advanced this row between the scan and here.
                let Some(slot) = sched.next_slot else {
                    return Ok(None);
                };
                if sched.status != "active" || slot > now {
                    return Ok(None);
                }

                // Only the most recent due slot fires. Walk forward over the
                // ones that passed while nothing was running: materializing them
                // would create tickets that are already expired, which is work
                // with no output.
                let (fire_slot, missed) = latest_due_slot(&sched, slot, now);
                let ticket = materialize_one(tx, &sched, fire_slot, now, "system")?;
                if missed > 0 {
                    // The lineage has a hole here. Record it, so a gap in the
                    // history reads as "nothing was running" rather than
                    // "nothing was scheduled".
                    emit_event(
                        tx,
                        None,
                        Some(&sched.project),
                        "system",
                        "schedule_missed",
                        json!({
                            "schedule": sched.id,
                            "slots": missed,
                            "from": crate::ids::iso(slot),
                            "fired": crate::ids::iso(fire_slot),
                        }),
                        now,
                    )?;
                }
                Ok(ticket)
            })?;
            if outcome.is_some() {
                created += 1;
            }
        }
        Ok(created)
    }

    /// A schedule's occurrence history, newest first, with each outcome derived
    /// rather than stored.
    ///
    /// Derived is a deliberate trade: it costs nothing to keep in sync and can
    /// never disagree with the ticket, but it means re-opening a July ticket in
    /// October changes what July looks like. The alternative — freezing an
    /// outcome row per slot — was the occurrences table this design deleted.
    pub fn schedule_occurrences(&self, id: &str, limit: i64) -> ApiResult<Vec<ScheduleOccurrence>> {
        let now = now_ms();
        self.with_conn(|conn| {
            // Prove the schedule exists, so an empty list means "never fired"
            // rather than "no such schedule".
            load_schedule(conn, id)?;
            let mut stmt = conn.prepare(
                r#"
                SELECT t.id, t.occurrence, t.expires_at, t.title, t.state,
                       COALESCE(ws.terminal, 0) AS terminal,
                       COALESCE(ws.category, '') AS category,
                       t.claim_holder, t.lapsed_claim_holder, t.archived_at
                FROM tickets t
                LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
                WHERE t.schedule = ?1 AND t.occurrence IS NOT NULL
                ORDER BY t.occurrence DESC
                LIMIT ?2
                "#,
            )?;
            let rows = stmt
                .query_map(params![id, limit.clamp(1, MAX_SCHEDULES_PAGE)], |r| {
                    let terminal: i64 = r.get("terminal")?;
                    let expires_at: Option<i64> = r.get("expires_at")?;
                    let claim_holder: Option<String> = r.get("claim_holder")?;
                    let lapsed: Option<String> = r.get("lapsed_claim_holder")?;
                    Ok(ScheduleOccurrence {
                        ticket: r.get("id")?,
                        slot: r.get("occurrence")?,
                        expires_at,
                        title: r.get("title")?,
                        state: r.get("state")?,
                        state_category: r.get("category")?,
                        outcome: derive_outcome(terminal == 1, expires_at, now).to_string(),
                        claimed_by: claim_holder.or(lapsed),
                        archived_at: r.get("archived_at")?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The next `count` slots a schedule would use, so a create response can show
    /// the caller what they just bought and a confirm row can show a reviewer
    /// what they are approving.
    pub fn upcoming_slots(&self, sched: &Schedule, count: usize) -> Vec<i64> {
        let _ = self;
        upcoming(sched, count)
    }
}

/// `done` | `open` | `not_fulfilled` — the three outcomes, derived from the
/// ticket alone.
///
/// There is deliberately no fourth for "claimed then the lease lapsed": the
/// distinction that matters to a reader is finished versus nobody finished it.
/// `lapsed_claim_holder` still records who dropped it, so the outcome can grow a
/// fourth value later without a migration.
pub fn derive_outcome(terminal: bool, expires_at: Option<i64>, now: i64) -> &'static str {
    if terminal {
        return "done";
    }
    match expires_at {
        Some(exp) if exp <= now => "not_fulfilled",
        _ => "open",
    }
}

fn upcoming(sched: &Schedule, count: usize) -> Vec<i64> {
    let Some(cadence) = sched.cadence.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(count);
    let mut cursor = sched.next_slot.unwrap_or_else(now_ms).max(sched.starts_at) - 1;
    for _ in 0..count {
        match cadence.next_slot_after(ms_to_utc(cursor), ms_to_utc(sched.starts_at)) {
            Some(next) => {
                let ms = next.timestamp_millis();
                if matches!(sched.ends_at, Some(end) if ms > end) {
                    break;
                }
                out.push(ms);
                cursor = ms;
            }
            None => break,
        }
    }
    out
}

/// Walk forward from `slot` to the last slot that is still `<= now`, returning
/// it and how many were passed over.
fn latest_due_slot(sched: &Schedule, slot: i64, now: i64) -> (i64, u32) {
    let Some(cadence) = sched.cadence.as_ref() else {
        return (slot, 0);
    };
    let anchor = ms_to_utc(sched.starts_at);
    let (mut current, mut missed) = (slot, 0u32);
    while missed < MAX_MISSED_SLOTS {
        match cadence.next_slot_after(ms_to_utc(current), anchor) {
            Some(next) => {
                let ms = next.timestamp_millis();
                if ms > now || matches!(sched.ends_at, Some(end) if ms > end) {
                    break;
                }
                current = ms;
                missed += 1;
            }
            None => break,
        }
    }
    (current, missed)
}

fn load_schedule(conn: &Connection, id: &str) -> ApiResult<Schedule> {
    let sql = format!("SELECT {SCHEDULE_COLS} FROM schedules WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_schedule)
        .optional()?
        .ok_or_else(|| ApiError::not_found("schedule", id))
}

fn set_next_slot(conn: &Connection, id: &str, slot: Option<i64>, now: i64) -> ApiResult<()> {
    conn.execute(
        "UPDATE schedules SET next_slot = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, slot, now],
    )?;
    Ok(())
}

fn touch_schedule(conn: &Connection, id: &str, now: i64) -> ApiResult<i64> {
    conn.execute(
        "UPDATE schedules SET version = version + 1, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    let v: i64 = conn.query_row(
        "SELECT version FROM schedules WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(v)
}

/// Create the ticket for one occurrence and advance the schedule, in the
/// caller's transaction.
///
/// Returns `None` when the slot was already materialized. That is not an error
/// path to apologise for — it is the exactly-once guarantee doing its job, and
/// the caller (sweep or `/run`) treats it as "someone else got there first".
fn materialize_one(
    conn: &Connection,
    sched: &Schedule,
    slot: i64,
    now: i64,
    actor: &str,
) -> ApiResult<Option<String>> {
    let Some(cadence) = sched.cadence.as_ref() else {
        return Err(ApiError::internal(format!(
            "schedule '{}' has an unparseable cadence: {}",
            sched.id, sched.cadence_raw
        )));
    };
    let template: ScheduleTemplate =
        serde_json::from_value(sched.template.clone()).map_err(|e| {
            ApiError::internal(format!("schedule '{}' template is corrupt: {e}", sched.id))
        })?;

    let expires_at = cadence
        .next_slot_after(ms_to_utc(slot), ms_to_utc(sched.starts_at))
        .map(|d| d.timestamp_millis());
    let title = cadence.render(&template.title, ms_to_utc(slot));
    let body = cadence.render(&template.body, ms_to_utc(slot));
    let initial: String = conn.query_row(
        "SELECT json_extract(workflow_json, '$.initial') FROM projects WHERE id = ?1",
        params![sched.project],
        |r| r.get(0),
    )?;

    // Retry only for an id collision. A duplicate (schedule, occurrence) is the
    // guarantee, not bad luck, so it returns None instead of retrying.
    let mut ticket_id = String::new();
    let mut inserted = false;
    for attempt in 0..8 {
        let candidate = format!("{}-{}", sched.project, ticket_suffix(4 + attempt / 4));
        let res = conn.execute(
            "INSERT INTO tickets (id, project, type, title, body, state, priority, labels, tags, metadata, links, schedule, occurrence, expires_at, created_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', '{}', ?10, ?11, ?12, ?13, ?14, ?14)",
            params![
                candidate,
                sched.project,
                template.ty.clone().unwrap_or_else(|| "task".to_string()),
                title,
                body,
                initial,
                template.priority.clone().unwrap_or_else(|| "normal".to_string()),
                serde_json::to_string(&template.labels).unwrap(),
                serde_json::to_string(&template.tags).unwrap(),
                sched.id,
                slot,
                expires_at,
                format!("schedule:{}", sched.id),
                now,
            ],
        );
        match res {
            Ok(_) => {
                ticket_id = candidate;
                inserted = true;
                break;
            }
            Err(e) if is_occurrence_conflict(&e) => return Ok(None),
            Err(e) if is_primary_key_conflict(&e) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if !inserted {
        return Err(ApiError::internal(
            "could not allocate a ticket id for a scheduled occurrence",
        ));
    }

    // Advance past the slot we just fired, in the same transaction, so a crash
    // here re-fires the same slot and hits the unique index rather than skipping
    // an occurrence.
    let mut next = expires_at;
    if matches!((next, sched.ends_at), (Some(n), Some(end)) if n > end) {
        next = None;
    }
    set_next_slot(conn, &sched.id, next, now)?;
    if next.is_none() {
        conn.execute(
            "UPDATE schedules SET status = 'retired' WHERE id = ?1 AND status = 'active'",
            params![sched.id],
        )?;
    }

    // The ordinary ticket_created goes out too, so the board, the SSE stream and
    // every existing consumer need no change at all.
    emit_event(
        conn,
        Some(&ticket_id),
        Some(&sched.project),
        &format!("schedule:{}", sched.id),
        "ticket_created",
        json!({ "title": title, "state": initial, "schedule": sched.id }),
        now,
    )?;
    emit_event(
        conn,
        Some(&ticket_id),
        Some(&sched.project),
        actor,
        "schedule_fired",
        json!({
            "schedule": sched.id,
            "occurrence": crate::ids::iso(slot),
            "expires_at": expires_at.map(crate::ids::iso),
            "ticket": ticket_id,
        }),
        now,
    )?;
    Ok(Some(ticket_id))
}

/// True when the failure is the `(schedule, occurrence)` unique index — i.e. the
/// exactly-once guarantee refusing a second ticket for one slot.
fn is_occurrence_conflict(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, Some(msg))
            if f.code == rusqlite::ErrorCode::ConstraintViolation
                && msg.contains("tickets.occurrence")
    )
}

fn is_primary_key_conflict(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, Some(msg))
            if f.code == rusqlite::ErrorCode::ConstraintViolation
                && msg.contains("tickets.id")
    )
}

impl Store {
    /// Rewind a schedule's next slot to an earlier instant.
    ///
    /// **Fixture support only.** `crate::seed` uses it to build a believable
    /// occurrence history through the *real* firing path — rewind, fire, repeat —
    /// rather than inserting tickets that merely look like occurrences. It is
    /// `pub(crate)` on purpose: `next_slot` is server-owned, and no HTTP, MCP or
    /// CLI surface can reach this.
    pub(crate) fn rewind_next_slot(&self, id: &str, slot: i64) -> ApiResult<()> {
        let now = now_ms();
        self.with_tx(|tx| {
            let n = tx.execute(
                "UPDATE schedules SET next_slot = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, slot, now],
            )?;
            if n == 0 {
                return Err(ApiError::not_found("schedule", id));
            }
            Ok(())
        })
    }

    /// Whether this project requires a human to activate a schedule an agent
    /// proposed. Defaults to true for a project row that predates the column.
    ///
    /// Read here rather than in the API layer because the flag is project state;
    /// how it combines with the caller's scopes is the auth layer's business.
    pub fn schedule_approval_required(&self, project: &str) -> ApiResult<bool> {
        self.with_conn(|conn| {
            let flag: Option<i64> = conn
                .query_row(
                    "SELECT schedule_approval FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .optional()?;
            match flag {
                Some(v) => Ok(v != 0),
                None => Err(ApiError::not_found("project", project)),
            }
        })
    }

    /// Turn the approval requirement on or off. Admin-gated at the edge, like
    /// every other project setting.
    pub fn set_schedule_approval(
        &self,
        project: &str,
        required: bool,
        actor: &str,
    ) -> ApiResult<bool> {
        let now = now_ms();
        self.with_tx(|tx| {
            let exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }
            ensure_project_writable(tx, project)?;
            tx.execute(
                "UPDATE projects SET schedule_approval = ?2 WHERE id = ?1",
                params![project, required as i64],
            )?;
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "project_updated",
                json!({ "schedule_approval": required }),
                now,
            )?;
            Ok(required)
        })
    }
}
