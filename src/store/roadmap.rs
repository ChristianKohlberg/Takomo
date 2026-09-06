//! Epic progress rollup: for each `epic` in a project, aggregate its full
//! descendant subtree (walked recursively via `parent`) into counts by state
//! and by category, plus a done-count and completion percent. Read-only; drives
//! `GET /v1/projects/{project}/roadmap` and `takomo roadmap`.
//!
//! The same rollup is also computed **per initiative**, which is what lets an
//! initiative be a long-lived lane rather than only a place ideas land. An epic
//! closes; an initiative never does. So a feature that ships as v1, then v1.1,
//! then v2 is one initiative with one epic per version filed under it, and the
//! numbers below are what make that readable: per-version progress from `epics`,
//! lane-lifetime progress from `initiatives`.
//!
//! Two things sit alongside the per-epic rollups so the response accounts for
//! all the work, not just the work someone remembered to file under an epic:
//!
//! - `unparented`: a rollup with the same shape as an epic's (`total`, `done`,
//!   `percent`, `by_state`, `by_category`, and no ticket identity) over every
//!   non-epic ticket in the project whose `parent` chain never reaches an
//!   `epic`. That covers a NULL parent, a chain of non-epic ancestors, and a
//!   dangling `parent` pointing at a row that no longer exists. Without it the
//!   percentages read as complete while real work is invisible.
//! - `flags` on each epic: short codes for an epic whose own state contradicts
//!   its children — `done_with_open_children`, `open_with_all_children_done`,
//!   `empty_epic`. Empty when the epic is consistent. `empty_epic` is a flag
//!   and not an error: an epic filed ahead of its work is legitimate, and the
//!   flag lets a client render it differently from a 0%-complete epic that
//!   does have children.
//!
//! The initiative side has the same pair, for the same reason: `uninitiated` is
//! the rollup over work no initiative owns, and `flags` marks an initiative
//! whose own status disagrees with its work. Both are project-wide statements,
//! so both are omitted — exactly as `unparented` is — when the report is
//! narrowed to one epic.
//!
//! **A ticket joins an initiative by the `initiative:<id>` tag** — the reference
//! the `/initiatives` page already writes when it files a passage as work — and
//! the set is grown downward through `parent` from there, so tagging the epic is
//! enough and every ticket beneath it counts. Two consequences worth knowing:
//!
//! - **Initiative rollups may overlap.** A ticket carrying two initiative tags,
//!   or tagged for one lane beneath an ancestor tagged for another, is counted by
//!   both. Unlike flat epics, `initiatives` therefore does NOT partition the
//!   project and its totals must not be summed. `uninitiated` stays well defined
//!   regardless: it is the work no tagged subtree reaches.
//! - **A tag naming an initiative that does not exist owns nothing.** The seed
//!   joins `initiatives`, so a stale `initiative:` reference leaves its ticket in
//!   `uninitiated` rather than disappearing from every bucket — the same rule
//!   `unparented` applies to a dangling `parent`.
//!
//! Both recursive walks use `WITH RECURSIVE ... UNION` (not `UNION ALL`), which
//! stops at an already-visited id — a malformed `parent` cycle terminates
//! rather than hanging the endpoint.

use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{iso, now_ms};
use rusqlite::{params, Connection, Statement};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The recursive CTEs readiness needs, in `ready_sql`'s NUMBERED form so every
/// `now` reference collapses to one bind. Every rollup query below opens with
/// `WITH RECURSIVE {…}, …` and appends its own CTEs after these.
///
/// `?1` is `now` in every rollup query here, which is why the numbered form is
/// the right one: the positional form's four-binds-in-declared-order contract
/// would have to be re-honoured by each of the four queries separately.
fn ready_ctes() -> String {
    super::ready_sql::ready_ctes("?1")
}

/// The two per-ticket predicates every rollup query selects alongside its counts.
///
/// `ready` is the ready queue's own predicate, taken from `store::ready_sql` —
/// the same text `store::claims` filters on, so this count cannot answer a
/// different question than the queue does. It used to be a copy annotated
/// "verbatim from the ready queue"; epic reservations were then added to the queue
/// and not to the copy, and the rollup went on reporting reserved work as
/// offerable. A comment cannot hold two copies together.
///
/// `awaiting` counts a ticket carrying at least one OPEN question, in ANY mode.
/// Advisory questions are included deliberately: the number answers "is a
/// decision outstanding here", which is what makes a queue entry misleading to
/// pick up, and a `blocking` question has already moved its ticket out of a
/// claimable state anyway — so restricting to blocking would report ~0 on the
/// very tickets this count exists to surface.
fn rollup_selects() -> String {
    format!(
        // Column ORDER is the contract with `collect_rollup`, which reads by
        // index: 3 = ready, 4 = awaiting, 5 = claimable_state,
        // 6 = last_activity.
        r#",
               SUM(CASE WHEN {ready} THEN 1 ELSE 0 END) AS ready,
               SUM(CASE WHEN EXISTS (SELECT 1 FROM questions q
                                      WHERE q.ticket = t.id AND q.status = 'open')
                        THEN 1 ELSE 0 END) AS awaiting,
               COALESCE(MAX(ws.claimable), 0) AS claimable_state,
               MAX(t.updated_at) AS last_activity"#,
        ready = super::ready_sql::ready_conditions("?1"),
    )
}

/// Aggregate over a set of tickets (an epic's descendant subtree, or the
/// unparented bucket).
struct Rollup {
    total: i64,
    done: i64,
    /// Claimable, unclaimed and unblocked — what `takomo ready` would hand out.
    ready: i64,
    /// In a claimable state but NOT ready: blocked by a dep, or already claimed.
    /// `ready + backlog` is the whole claimable category, so the two split it.
    backlog: i64,
    /// Carrying at least one open question. An OVERLAY, not a partition — such a
    /// ticket is also counted in its own state, and may or may not be `ready`.
    awaiting_answer: i64,
    /// Newest `updated_at` in the set, or None when the set is empty. Free — the
    /// aggregate already walks these rows — and it is what lets an epic claim be
    /// judged by movement without a second pass over the event log.
    last_activity: Option<i64>,
    by_state: Map<String, Value>,
    by_category: Map<String, Value>,
}

impl Rollup {
    /// `done/total` rounded to a whole percent (0 when the set is empty).
    fn percent(&self) -> i64 {
        if self.total > 0 {
            ((self.done as f64 / self.total as f64) * 100.0).round() as i64
        } else {
            0
        }
    }
}

/// Fold `(state, category, count, ready, awaiting, claimable, last_activity)`
/// rows — the shape every rollup query below returns — into a `Rollup`.
fn collect_rollup(stmt: &mut Statement, args: &[&dyn rusqlite::ToSql]) -> ApiResult<Rollup> {
    let rows = stmt
        .query_map(args, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<i64>>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut total = 0i64;
    let mut done = 0i64;
    let mut ready = 0i64;
    let mut claimable = 0i64;
    let mut awaiting_answer = 0i64;
    let mut last_activity: Option<i64> = None;
    let mut by_state: Map<String, Value> = Map::new();
    let mut by_category: Map<String, Value> = Map::new();
    for (st, cat, n, rdy, awaiting, is_claimable, touched) in rows {
        // Newest write anywhere in the set. Grouped by state, so the max has to
        // be folded across groups rather than read off one row.
        if let Some(ts) = touched {
            last_activity = Some(last_activity.map_or(ts, |cur: i64| cur.max(ts)));
        }
        total += n;
        by_state.insert(st, json!(n));
        if cat == "done" {
            done += n;
        }
        // The CLAIMABLE flag, not the `todo` CATEGORY: `draft` is category
        // `todo` and deliberately not claimable, so counting the category would
        // put an un-pickable draft in the backlog and make ready+backlog
        // disagree with what the queue can actually hand out.
        if is_claimable == 1 {
            claimable += n;
        }
        ready += rdy;
        awaiting_answer += awaiting;
        if !cat.is_empty() {
            let prev = by_category.get(&cat).and_then(Value::as_i64).unwrap_or(0);
            by_category.insert(cat, json!(prev + n));
        }
    }
    Ok(Rollup {
        total,
        done,
        ready,
        // Never negative: `ready` is a subset of the claimable rows by
        // construction (its CASE requires `ws.claimable = 1`).
        backlog: (claimable - ready).max(0),
        awaiting_answer,
        last_activity,
        by_state,
        by_category,
    })
}

/// One epic plus the rollup over its descendants. `total`/`done`/`percent`
/// count the whole subtree beneath the epic (the epic itself is the container,
/// not counted). `done` is the number of descendants whose state category is
/// `done`; `percent` is `done/total` rounded to a whole percent (0 when empty).
fn rollup_for_epic(conn: &Connection, epic_id: &str, now: i64) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE {ctes},
        sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?2
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM sub
        JOIN tickets t ON t.id = sub.id
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        JOIN projects p ON p.id = t.project
        GROUP BY t.state
        "#,
        ctes = ready_ctes(),
        selects = rollup_selects(),
    );
    let mut stmt = conn.prepare(&sql)?;
    collect_rollup(&mut stmt, params![now, epic_id])
}

/// Rollup over the project's non-epic tickets that no epic owns: the recursive
/// term grows the set of tickets reachable *downward* from any epic, and the
/// outer select keeps everything else. A ticket is excluded exactly when its
/// `parent` chain reaches an epic, so a NULL parent, an all-non-epic ancestor
/// chain, and a dangling parent id all land in the bucket.
fn rollup_unparented(conn: &Connection, project: &str, now: i64) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE {ctes},
        owned(id) AS (
            SELECT t.id FROM tickets t
              JOIN tickets par ON t.parent = par.id
             WHERE t.project = ?2 AND par.type = 'epic'
            UNION
            SELECT t.id FROM tickets t JOIN owned ON t.parent = owned.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM tickets t
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        JOIN projects p ON p.id = t.project
        WHERE t.project = ?2
          AND t.type <> 'epic'
          AND t.id NOT IN (SELECT id FROM owned)
        GROUP BY t.state
        "#,
        ctes = ready_ctes(),
        selects = rollup_selects(),
    );
    let mut stmt = conn.prepare(&sql)?;
    collect_rollup(&mut stmt, params![now, project])
}

/// How a ticket says which initiative it belongs to: a `kind:handle` reference
/// into the project tag registry whose handle is the initiative id. Not a new
/// mechanism — `/initiatives` already writes exactly this tag when it files a
/// passage as work, so the rollup reads a link that is already being created.
const INITIATIVE_TAG: &str = "initiative:";

/// Rollup over one initiative's work: every ticket in `project` carrying
/// `initiative:<id>`, plus everything beneath those tickets via `parent`. Tagging
/// the version epic is therefore enough to count its whole subtree.
///
/// Epics are excluded from the counts for the same reason `rollup_unparented`
/// excludes them: under this model an epic is the container for a version, not a
/// unit of work, and counting it would inflate every lane by one per version.
fn rollup_for_initiative(
    conn: &Connection,
    project: &str,
    initiative_id: &str,
    now: i64,
) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE {ctes},
        sub(id) AS (
            SELECT t.id FROM tickets t
             WHERE t.project = ?2
               AND EXISTS (SELECT 1 FROM json_each(t.tags) WHERE json_each.value = ?3)
            UNION
            SELECT c.id FROM tickets c JOIN sub ON c.parent = sub.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM sub
        JOIN tickets t ON t.id = sub.id
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        JOIN projects p ON p.id = t.project
        WHERE t.type <> 'epic'
        GROUP BY t.state
        "#,
        ctes = ready_ctes(),
        selects = rollup_selects(),
    );
    let mut stmt = conn.prepare(&sql)?;
    let tag = format!("{INITIATIVE_TAG}{initiative_id}");
    collect_rollup(&mut stmt, params![now, project, tag])
}

/// Rollup over the project's non-epic tickets that no initiative owns — the
/// initiative-side twin of `rollup_unparented`, and the reason the per-initiative
/// percentages cannot read as complete while real work sits outside every lane.
///
/// The seed JOINs `initiatives`, so a tag naming an initiative that no longer
/// exists does not silently remove its ticket from the accounting: that ticket is
/// unowned, and lands here.
fn rollup_uninitiated(conn: &Connection, project: &str, now: i64) -> ApiResult<Rollup> {
    let sql = format!(
        r#"
        WITH RECURSIVE {ctes},
        owned(id) AS (
            SELECT t.id FROM tickets t
             WHERE t.project = ?2
               AND EXISTS (
                   SELECT 1 FROM json_each(t.tags) je
                     JOIN initiatives i ON i.id = substr(je.value, {handle_at})
                    WHERE je.value LIKE '{INITIATIVE_TAG}%'
                      AND i.project = t.project
               )
            UNION
            SELECT c.id FROM tickets c JOIN owned ON c.parent = owned.id
        )
        SELECT t.state,
               COALESCE(ws.category, '') AS category,
               COUNT(*) AS n{selects}
        FROM tickets t
        LEFT JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        JOIN projects p ON p.id = t.project
        WHERE t.project = ?2
          AND t.type <> 'epic'
          AND t.id NOT IN (SELECT id FROM owned)
        GROUP BY t.state
        "#,
        ctes = ready_ctes(),
        // 1-based, so the handle starts one past the prefix. Derived from the
        // constant rather than written as a literal, so the two cannot drift.
        handle_at = INITIATIVE_TAG.len() + 1,
        selects = rollup_selects(),
    );
    let mut stmt = conn.prepare(&sql)?;
    collect_rollup(&mut stmt, params![now, project])
}

/// Which epics are filed under which initiative, as one query rather than one
/// per initiative — the join that makes "initiative is the lane, epic is the
/// version" renderable from a single response.
///
/// Directly tagged epics only. The rollup above walks *downward* into a tagged
/// ticket's subtree, but this list answers a narrower question — "which versions
/// were filed under this lane" — and an epic that merely inherits a lane from an
/// ancestor was not filed as one of its versions.
fn epics_by_initiative(
    conn: &Connection,
    project: &str,
) -> ApiResult<HashMap<String, Vec<String>>> {
    let sql = format!(
        r#"
        SELECT substr(je.value, {handle_at}) AS initiative, t.id
          FROM tickets t, json_each(t.tags) je
         WHERE t.project = ?1
           AND t.type = 'epic'
           AND je.value LIKE '{INITIATIVE_TAG}%'
         ORDER BY t.created_at ASC, t.rowid ASC
        "#,
        handle_at = INITIATIVE_TAG.len() + 1,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![project], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (initiative, epic) in rows {
        out.entry(initiative).or_default().push(epic);
    }
    Ok(out)
}

/// Which initiatives each epic is filed under — `epics_by_initiative` inverted.
///
/// The lane→versions direction answers "where is this feature", and this one
/// answers "what is this version part of". An epic-first reader needs the second,
/// and inverting a whole project's lanes client-side to get it is work the server
/// has already done.
fn initiatives_by_epic(epics_for: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    // Sorted, so an epic in two lanes reports them in a stable order rather than
    // whatever the hash iteration gave this time.
    let mut lanes: Vec<&String> = epics_for.keys().collect();
    lanes.sort();
    for lane in lanes {
        for epic in &epics_for[lane] {
            out.entry(epic.clone()).or_default().push(lane.clone());
        }
    }
    out
}

/// An epic's claim, as much of it as the row itself can answer.
///
/// Deliberately NOT `GET /v1/tickets/{id}/claim`. That route walks the event log
/// to break movement down into created/closed/in_progress/blocked, which is worth
/// a request for one epic and would be N event-log walks here — on an endpoint
/// that already runs a query per epic. So this is the cheap half: who holds it,
/// how long they have, and whether anything under them has moved since.
///
/// `idle_seconds` is anchored at the claim when nothing has moved, so a fresh
/// claim reads as idle 0 rather than as idle-since-whenever. It is derived from
/// `updated_at` rather than from events, which makes it a near-equivalent of that
/// route's number and not the identical one — close enough to sort a list by,
/// and the route remains the precise answer for one epic.
///
/// `None` when there is no active claim: an expired lease is not a hold.
fn claim_summary(
    holder: Option<String>,
    since: Option<i64>,
    expires_at: Option<i64>,
    last_activity: Option<i64>,
    now: i64,
) -> Option<Value> {
    let holder = holder?;
    if expires_at.is_some_and(|e| e <= now) {
        return None;
    }
    let anchor = match (last_activity, since) {
        (Some(a), Some(s)) => Some(a.max(s)),
        (a, s) => a.or(s),
    };
    Some(json!({
        "holder": holder,
        "held_since": since.map(iso),
        "held_for_seconds": since.map(|s| ((now - s) / 1000).max(0)),
        // An epic claimed without a TTL is held until released — there is no
        // expiry to judge it by, which is exactly why movement is reported.
        "indefinite": expires_at.is_none(),
        "expires_at": expires_at.map(iso),
        "last_activity_at": last_activity.map(iso),
        "idle_seconds": anchor.map(|a| ((now - a) / 1000).max(0)),
    }))
}

/// Contradiction codes for an initiative whose own status disagrees with the work
/// filed under it. Pure derivation over the rollup — no extra query.
///
/// There is no initiative equivalent of `done_with_open_children`, and that is
/// not an omission: an initiative's `status` is a label, not a state machine, and
/// `distilled` means "its substance became tickets" — which is precisely when
/// open work is expected, not a contradiction.
fn initiative_flags(status: &str, r: &Rollup) -> Vec<&'static str> {
    let mut flags = Vec::new();
    if r.total == 0 {
        flags.push("empty_initiative");
    }
    // Parking is deliberate ("set aside, still readable"), so a parked lane whose
    // tickets the queue is still handing out is worth surfacing: the decision to
    // stop and what the fleet will actually do have come apart.
    if status == "parked" && r.ready > 0 {
        flags.push("parked_with_ready_work");
    }
    flags
}

/// Every initiative in the project with its lane rollup, in creation order so a
/// client renders a stable list. One extra query per initiative, matching what
/// the per-epic rollups already cost.
fn initiative_rollups(
    conn: &Connection,
    project: &str,
    now: i64,
    epics_for: &HashMap<String, Vec<String>>,
) -> ApiResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, title, status
        FROM initiatives
        WHERE project = ?1
        ORDER BY created_at ASC, rowid ASC
        "#,
    )?;
    let rows = stmt
        .query_map(params![project], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, title, status) in rows {
        let r = rollup_for_initiative(conn, project, &id, now)?;
        let flags = initiative_flags(&status, &r);
        let percent = r.percent();
        out.push(json!({
            "id": id,
            "title": title,
            "status": status,
            "total": r.total,
            "done": r.done,
            "percent": percent,
            "ready": r.ready,
            "backlog": r.backlog,
            "awaiting_answer": r.awaiting_answer,
            "by_state": Value::Object(r.by_state),
            "by_category": Value::Object(r.by_category),
            "epics": epics_for.get(&id).cloned().unwrap_or_default(),
            "flags": flags,
        }));
    }
    Ok(out)
}

/// Contradiction codes for an epic whose own state disagrees with its subtree.
/// Pure derivation over the epic's `state_category` and its rollup counts — no
/// extra query per epic.
fn epic_flags(state_category: &str, r: &Rollup) -> Vec<&'static str> {
    let mut flags = Vec::new();
    if state_category == "done" && r.done < r.total {
        flags.push("done_with_open_children");
    }
    if state_category != "done" && r.total > 0 && r.done == r.total {
        flags.push("open_with_all_children_done");
    }
    if r.total == 0 {
        flags.push("empty_epic");
    }
    flags
}

impl Store {
    /// Roadmap rollup for every epic in `project`, plus the `unparented` bucket
    /// for work no epic owns. Returns a 404 for an unknown project. Each epic
    /// carries its own metadata, a subtree rollup, and contradiction `flags`.
    /// `epic` narrows the report to ONE epic. The `unparented` bucket is then
    /// omitted rather than returned empty: it is a project-wide statement about
    /// work no epic owns, and repeating it under a single-epic view would read
    /// as "this epic has no unparented work", which is not a thing an epic can
    /// have. An unknown epic 404s instead of returning an empty list, for the
    /// same reason an unknown project does.
    pub fn roadmap(&self, project: &str, epic: Option<&str>) -> ApiResult<Value> {
        let now = now_ms();
        self.with_conn(|conn| {
            // 404 for an unknown project, so a scoped caller gets a clean error
            // rather than an empty list that hides a typo.
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM projects WHERE id = ?1",
                    params![project],
                    |r| r.get(0),
                )
                .ok();
            if exists.is_none() {
                return Err(ApiError::not_found("project", project));
            }

            // A filter naming a ticket that is not an epic OF THIS PROJECT is a
            // 404, not an empty report — the three ways to get it wrong (typo,
            // wrong project, naming a task) are indistinguishable in an empty
            // list and all of them are worth telling the caller about.
            if let Some(e) = epic {
                let ok: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM tickets WHERE id = ?1 AND project = ?2 AND type = 'epic'",
                        params![e, project],
                        |r| r.get(0),
                    )
                    .ok();
                if ok.is_none() {
                    return Err(ApiError::not_found("epic", e));
                }
            }

            let mut stmt = conn.prepare(
                r#"
                SELECT t.id, t.title, t.state, t.priority,
                       COALESCE((SELECT ws.category FROM workflow_states ws
                                 WHERE ws.project = t.project AND ws.state = t.state), '') AS category,
                       t.claim_holder, t.claim_since, t.claim_expires_at, t.updated_at,
                       (SELECT COUNT(*) FROM questions q WHERE q.ticket = t.id AND q.status = 'open')
                FROM tickets t
                WHERE t.project = ?1 AND t.type = 'epic'
                  AND (?2 IS NULL OR t.id = ?2)
                ORDER BY t.created_at ASC, t.rowid ASC
                "#,
            )?;
            let epics = stmt
                .query_map(params![project, epic], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, Option<i64>>(7)?,
                        r.get::<_, i64>(8)?,
                        r.get::<_, i64>(9)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // Which lanes each epic belongs to. One query for the whole project,
            // shared by both directions of the join below.
            let epics_for = epics_by_initiative(conn, project)?;
            let lanes_of = initiatives_by_epic(&epics_for);

            let mut out = Vec::with_capacity(epics.len());
            for (id, title, st, priority, category, holder, since, expires, updated, own_open_questions) in epics {
                let r = rollup_for_epic(conn, &id, now)?;
                let flags = epic_flags(&category, &r);
                let percent = r.percent();
                let claim = claim_summary(holder, since, expires, r.last_activity, now);
                out.push(json!({
                    "id": id,
                    "title": title,
                    "state": st,
                    "state_category": category,
                    "priority": priority,
                    // Available regardless of a claim, including epics with no tasks.
                    "last_activity_at": iso(r.last_activity.map_or(updated, |at| at.max(updated))),
                    "own_open_questions": own_open_questions,
                    "total": r.total,
                    "done": r.done,
                    "percent": percent,
                    "ready": r.ready,
                    "backlog": r.backlog,
                    "awaiting_answer": r.awaiting_answer,
                    "by_state": Value::Object(r.by_state),
                    "by_category": Value::Object(r.by_category),
                    // The inverse of a lane's `epics`: what this version is part
                    // of. Empty for an epic filed under no initiative.
                    "initiatives": lanes_of.get(&id).cloned().unwrap_or_default(),
                    "claim": claim,
                    "flags": flags,
                }));
            }

            let mut body = json!({
                "project": project,
                "generated_at": iso(now),
                "epics": out,
            });
            if let Some(e) = epic {
                // Echo the filter so a stored response says what it is a view of.
                body["epic"] = json!(e);
            } else {
                // The initiative side, and both project-wide buckets. All of it
                // is omitted under a single-epic view for the same reason
                // `unparented` is: a lane spans versions, so reporting lanes
                // beside ONE version would invite reading the lane's numbers as
                // that version's.
                body["initiatives"] =
                    Value::Array(initiative_rollups(conn, project, now, &epics_for)?);

                let ui = rollup_uninitiated(conn, project, now)?;
                body["uninitiated"] = json!({
                    "total": ui.total,
                    "done": ui.done,
                    "percent": ui.percent(),
                    "ready": ui.ready,
                    "backlog": ui.backlog,
                    "awaiting_answer": ui.awaiting_answer,
                    "by_state": Value::Object(ui.by_state),
                    "by_category": Value::Object(ui.by_category),
                });

                let u = rollup_unparented(conn, project, now)?;
                body["unparented"] = json!({
                    "total": u.total,
                    "done": u.done,
                    "percent": u.percent(),
                    "ready": u.ready,
                    "backlog": u.backlog,
                    "awaiting_answer": u.awaiting_answer,
                    "by_state": Value::Object(u.by_state),
                    "by_category": Value::Object(u.by_category),
                });
            }
            Ok(body)
        })
    }
}
