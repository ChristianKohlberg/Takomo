//! Moving tickets between projects.
//!
//! A ticket id is minted as `<project>-<suffix>` and that prefix is never parsed
//! back out anywhere — not by the server, not by the clients, not by the SPA. So
//! a move keeps the id: it is quoted in commit messages, PR bodies and other
//! systems, and rewriting it would break every one of those references to buy
//! nothing. After a move the prefix reads as provenance ("where this was
//! originally filed"), and `project` is the answer to "where does it live now".
//!
//! What a move has to reconcile is everything that IS keyed on the project:
//!
//! - **State.** States belong to a project's workflow. A state the target
//!   workflow also defines is kept; anything else lands on the target's
//!   `initial`. That is a real loss of progress, so it is never silent: every
//!   moved ticket reports `from_state`/`to_state` and a `state_reset` flag, and
//!   the emitted event carries the same.
//! - **Parent.** Parent and child must share a project (enforced on create and
//!   on patch), so a moved ticket whose parent stays behind is unparented, and a
//!   child left behind by a moving parent is orphaned — which is exactly the
//!   `descendants: false` case the caller asked for.
//! - **Tags.** A tag reference is `kind:handle` into the *project's* registry,
//!   so the target project gets stubs for the moved ticket's tags, the same way
//!   a patch that names an unregistered tag does.
//! - **Questions and promotions** carry a denormalized `project` column, which
//!   is updated with the ticket. Their history is not rewritten — the event log
//!   keeps recording the project each event happened in.
//! - **Checklist lanes** are filed under a project and may point at an epic in
//!   it. An epic that leaves takes no lanes with it (they cover the old
//!   project's surface), so those lanes are detached — `epic` cleared — and any
//!   epic-level checklist policy override is dropped. Both are reported.
//!
//! Dependency edges are deliberately left alone: `deps` never required both ends
//! to share a project, so a cross-project blocker is a legal edge before the move
//! and stays one after it.
//!
//! A claimed ticket is not moved. A lease means someone is working the ticket
//! right now against a workflow that is about to change underneath them, and a
//! bulk reorganization is not the place to resolve that — the whole call is
//! refused, naming the leases, so the caller can release or wait.

use super::helpers::{emit_event, ensure_project_writable, get_ticket_opt, get_workflow};
use super::tags::ensure_tags_exist;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::now_ms;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Ceiling on how many tickets one move may touch, roots plus descendants.
///
/// A move holds the single write mutex every claim in the process queues behind,
/// so an unbounded epic subtree would stall the whole store. Reaching the ceiling
/// is a 422 rather than a truncation: a half-moved epic — some children in the
/// new project, some in the old, parents cleared in between — is worse than a
/// refusal the caller can split.
pub const MAX_MOVE_TICKETS: usize = 500;

/// What happened to one ticket in a move.
pub struct MovedTicket {
    pub id: String,
    pub from_project: String,
    pub from_state: String,
    pub to_state: String,
    /// The ticket's parent stayed behind, so the link was cut.
    pub parent_cleared: bool,
}

impl MovedTicket {
    fn to_json(&self, to_project: &str) -> Value {
        json!({
            "id": self.id,
            "from_project": self.from_project,
            "to_project": to_project,
            "from_state": self.from_state,
            "to_state": self.to_state,
            "state_reset": self.from_state != self.to_state,
            "parent_cleared": self.parent_cleared,
        })
    }
}

/// The result of one move, in the order the tickets were visited.
pub struct MoveOutcome {
    pub to_project: String,
    pub moved: Vec<MovedTicket>,
    /// Named or reached but already in the target project: nothing to do.
    pub unchanged: Vec<String>,
    /// Tickets left behind whose parent moved away, so their `parent` was
    /// cleared. Only ever non-empty with `descendants: false`.
    pub orphaned: Vec<String>,
    /// Checklist lanes whose epic left the project, so their `epic` was cleared.
    pub lanes_detached: Vec<String>,
    /// Epic-level checklist policy overrides dropped with the departing epic.
    pub policies_dropped: Vec<String>,
}

impl MoveOutcome {
    pub fn to_json(&self) -> Value {
        let state_resets = self
            .moved
            .iter()
            .filter(|m| m.from_state != m.to_state)
            .count();
        let mut note = Vec::new();
        if state_resets > 0 {
            note.push(format!(
                "{state_resets} ticket(s) had a state the '{}' workflow does not define and landed on its initial state; see `state_reset` per ticket.",
                self.to_project
            ));
        }
        if !self.orphaned.is_empty() {
            note.push(format!(
                "{} ticket(s) stayed behind and lost their parent, because a parent and child must share a project. Re-file them with PATCH /v1/tickets/{{id}} {{\"parent\": \"...\"}}.",
                self.orphaned.len()
            ));
        }
        if !self.lanes_detached.is_empty() {
            note.push(format!(
                "{} checklist lane(s) lost their epic, which left the project; the lanes themselves did not move.",
                self.lanes_detached.len()
            ));
        }
        let mut out = json!({
            "to_project": self.to_project,
            "moved": self.moved.iter().map(|m| m.to_json(&self.to_project)).collect::<Vec<_>>(),
            "total": self.moved.len(),
            "unchanged": self.unchanged,
            "orphaned": self.orphaned,
            "lanes_detached": self.lanes_detached,
            "policies_dropped": self.policies_dropped,
        });
        if !note.is_empty() {
            out["note"] = Value::String(note.join(" "));
        }
        out
    }
}

/// The store-side shape of a move.
pub struct MoveRequest {
    /// The tickets to move. An epic id here moves the epic; whether its subtree
    /// comes along is [`MoveRequest::descendants`].
    pub tickets: Vec<String>,
    pub to_project: String,
    /// Move each named ticket's full descendant subtree with it (the default).
    /// With `false`, only the named tickets move and their children are orphaned
    /// in place.
    pub descendants: bool,
}

/// One ticket's move-relevant columns.
struct Row {
    id: String,
    project: String,
    state: String,
    parent: Option<String>,
    tags: Vec<String>,
    claim_holder: Option<String>,
    claim_expires_at: Option<i64>,
}

fn load_row(conn: &Connection, id: &str) -> ApiResult<Option<Row>> {
    let row = conn
        .query_row(
            "SELECT id, project, state, parent, tags, claim_holder, claim_expires_at \
             FROM tickets WHERE id = ?1",
            params![id],
            |r| {
                let tags: String = r.get(4)?;
                Ok(Row {
                    id: r.get(0)?,
                    project: r.get(1)?,
                    state: r.get(2)?,
                    parent: r.get(3)?,
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    claim_holder: r.get(5)?,
                    claim_expires_at: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Every descendant of `root`, root excluded, in breadth order.
///
/// `UNION` (not `UNION ALL`) stops at an already-visited id, so a malformed
/// `parent` cycle terminates instead of hanging the write transaction.
fn descendants_of(conn: &Connection, root: &str) -> ApiResult<Vec<String>> {
    let mut stmt = conn.prepare(
        r#"
        WITH RECURSIVE sub(id) AS (
            SELECT id FROM tickets WHERE parent = ?1
            UNION
            SELECT t.id FROM tickets t JOIN sub ON t.parent = sub.id
        )
        SELECT id FROM sub
        "#,
    )?;
    let ids = stmt
        .query_map(params![root], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

impl Store {
    /// Move tickets into another project, optionally with their subtrees.
    ///
    /// One transaction: either every ticket lands in the target project with its
    /// parent links, tags, questions, promotions and lanes reconciled, or nothing
    /// moves. See the module docs for what each of those means.
    pub fn move_tickets(&self, req: &MoveRequest, actor: &str) -> ApiResult<MoveOutcome> {
        if req.tickets.is_empty() {
            return Err(ApiError::validation(
                "validation.tickets",
                "Name at least one ticket to move: {\"tickets\": [\"<id>\", ...], \"to_project\": \"<project>\"}.",
            ));
        }
        let now = now_ms();
        self.with_tx(|tx| {
            // The target workflow is also the project's existence check: an
            // unknown project is a 404 here rather than a foreign-key error
            // three statements later.
            let wf = get_workflow(tx, &req.to_project)?;
            // Both ends of the move, checked below once the set is resolved: an
            // archived project neither takes work nor gives it up.
            ensure_project_writable(tx, &req.to_project)?;
            let target_states: HashSet<&str> = wf.states.iter().map(|s| s.id.as_str()).collect();

            // Resolve the move set: the named roots, deduped in the order given,
            // then each root's descendants when they come along.
            let mut order: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for id in &req.tickets {
                if get_ticket_opt(tx, id)?.is_none() {
                    return Err(ApiError::not_found("ticket", id));
                }
                if seen.insert(id.clone()) {
                    order.push(id.clone());
                }
            }
            if req.descendants {
                for id in req.tickets.iter() {
                    for child in descendants_of(tx, id)? {
                        if seen.insert(child.clone()) {
                            order.push(child);
                        }
                    }
                }
            }
            if order.len() > MAX_MOVE_TICKETS {
                return Err(ApiError::validation(
                    "validation.move_too_large",
                    format!(
                        "This move covers {} tickets; the limit is {MAX_MOVE_TICKETS} per call, because a move holds the write lock every claim queues behind. Move the subtree in parts — name the child epics one at a time — or move the epic alone with \"descendants\": false and re-file its children after.",
                        order.len()
                    ),
                ));
            }

            let mut rows: HashMap<String, Row> = HashMap::new();
            for id in &order {
                let row = load_row(tx, id)?.ok_or_else(|| ApiError::not_found("ticket", id))?;
                rows.insert(id.clone(), row);
            }

            // The source side of the gate. A ticket may not be moved OUT of an
            // archived project either: the archive froze that project's work as
            // it stood, and emptying it a ticket at a time is exactly the
            // "moving things around" the gate exists to stop.
            for row in rows.values() {
                ensure_project_writable(tx, &row.project)?;
            }

            // A lease means someone is working the ticket against a workflow
            // that is about to change under them. Refuse the whole call rather
            // than move around the claimed ones and leave a split subtree.
            let held: Vec<&Row> = order
                .iter()
                .filter_map(|id| rows.get(id))
                .filter(|r| {
                    r.project != req.to_project
                        && matches!((&r.claim_holder, r.claim_expires_at), (Some(_), Some(exp)) if exp > now)
                })
                .collect();
            if let Some(first) = held.first() {
                let holders: Vec<Value> = held
                    .iter()
                    .map(|r| json!({ "ticket": r.id, "holder": r.claim_holder }))
                    .collect();
                let subject = if held.len() == 1 {
                    format!(
                        "Ticket '{}' is claimed by '{}'",
                        first.id,
                        first.claim_holder.as_deref().unwrap_or("")
                    )
                } else {
                    format!(
                        "{} tickets in this move are claimed (e.g. '{}' by '{}')",
                        held.len(),
                        first.id,
                        first.claim_holder.as_deref().unwrap_or("")
                    )
                };
                return Err(ApiError::conflict(
                    "claim.held",
                    format!(
                        "{subject}; a move changes the workflow a lease is held against, so nothing was moved. Release the lease (POST /v1/tickets/{{id}}/release), wait for it to expire, or leave those tickets out of the move."
                    ),
                )
                .details(json!({ "claimed": holders })));
            }

            let moving: HashSet<&str> = order
                .iter()
                .filter(|id| rows.get(*id).map(|r| r.project != req.to_project) == Some(true))
                .map(|id| id.as_str())
                .collect();

            let mut out = MoveOutcome {
                to_project: req.to_project.clone(),
                moved: Vec::new(),
                unchanged: Vec::new(),
                orphaned: Vec::new(),
                lanes_detached: Vec::new(),
                policies_dropped: Vec::new(),
            };

            for id in &order {
                let row = &rows[id];
                if !moving.contains(id.as_str()) {
                    out.unchanged.push(id.clone());
                    continue;
                }

                // A state the target workflow does not define cannot be kept:
                // nothing could transition out of it. Land on `initial`.
                let to_state = if target_states.contains(row.state.as_str()) {
                    row.state.clone()
                } else {
                    wf.initial.clone()
                };
                // The parent survives only if it ends up in the target project
                // too — either because it is moving as well, or because it was
                // already there (re-filing a child under a parent that moved in
                // an earlier call). Anything else would leave a cross-project
                // edge, which parent/child does not allow.
                let keep_parent = match row.parent.as_deref() {
                    None => false,
                    Some(p) if moving.contains(p) => true,
                    Some(p) => {
                        let project: Option<String> = tx
                            .query_row(
                                "SELECT project FROM tickets WHERE id = ?1",
                                params![p],
                                |r| r.get(0),
                            )
                            .optional()?;
                        project.as_deref() == Some(req.to_project.as_str())
                    }
                };
                let parent_cleared = row.parent.is_some() && !keep_parent;

                ensure_tags_exist(tx, &req.to_project, &row.tags, actor, now)?;

                tx.execute(
                    "UPDATE tickets SET project = ?2, state = ?3, parent = CASE WHEN ?4 THEN parent ELSE NULL END, \
                     version = version + 1, updated_at = ?5 WHERE id = ?1",
                    params![id, req.to_project, to_state, keep_parent, now],
                )?;
                // Denormalized project columns follow the ticket, so /inbox and
                // the board keep finding this ticket's questions and badges under
                // the project it now lives in.
                tx.execute(
                    "UPDATE questions SET project = ?2 WHERE ticket = ?1",
                    params![id, req.to_project],
                )?;
                tx.execute(
                    "UPDATE promotions SET project = ?2 WHERE ticket = ?1",
                    params![id, req.to_project],
                )?;
                // An answer grant is a credential scoped to one question, and its
                // `project` is what the grant reports and is filtered by; it has
                // to follow the question it belongs to.
                tx.execute(
                    "UPDATE answer_grants SET project = ?2 \
                     WHERE question IN (SELECT id FROM questions WHERE ticket = ?1)",
                    params![id, req.to_project],
                )?;
                // A subtree share rooted at this ticket still covers the same
                // tickets — the subtree walked from `ref` — so the share stays
                // valid and only its recorded project is stale.
                tx.execute(
                    "UPDATE shares SET project = ?2 WHERE kind = 'subtree' AND \"ref\" = ?1",
                    params![id, req.to_project],
                )?;

                // Lanes stay with the project whose surface they cover; only
                // their pointer at a departed epic goes.
                let mut stmt = tx.prepare("SELECT id FROM lanes WHERE epic = ?1")?;
                let lanes = stmt
                    .query_map(params![id], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                if !lanes.is_empty() {
                    tx.execute(
                        "UPDATE lanes SET epic = NULL, updated_at = ?2 WHERE epic = ?1",
                        params![id, now],
                    )?;
                    out.lanes_detached.extend(lanes);
                }
                let dropped = tx.execute(
                    "DELETE FROM checklist_policies WHERE epic = ?1",
                    params![id],
                )?;
                if dropped > 0 {
                    out.policies_dropped.push(id.clone());
                }

                emit_event(
                    tx,
                    Some(id),
                    Some(&req.to_project),
                    actor,
                    "ticket_moved",
                    json!({
                        "from_project": row.project,
                        "to_project": req.to_project,
                        "from_state": row.state,
                        "to_state": to_state,
                        "state_reset": row.state != to_state,
                        "parent_cleared": parent_cleared,
                    }),
                    now,
                )?;

                out.moved.push(MovedTicket {
                    id: id.clone(),
                    from_project: row.project.clone(),
                    from_state: row.state.clone(),
                    to_state,
                    parent_cleared,
                });
            }

            // Children left behind by a moving parent. With `descendants: true`
            // there are none — the subtree came along — so this is the
            // `descendants: false` case: the epic goes, its children stay and
            // become orphans, which is what that flag means.
            for id in moving.iter() {
                let mut stmt = tx.prepare("SELECT id FROM tickets WHERE parent = ?1")?;
                let children = stmt
                    .query_map(params![id], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for child in children {
                    if moving.contains(child.as_str()) {
                        continue;
                    }
                    tx.execute(
                        "UPDATE tickets SET parent = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                        params![child, now],
                    )?;
                    let child_project: String = tx.query_row(
                        "SELECT project FROM tickets WHERE id = ?1",
                        params![child],
                        |r| r.get(0),
                    )?;
                    emit_event(
                        tx,
                        Some(&child),
                        Some(&child_project),
                        actor,
                        "ticket_orphaned",
                        json!({ "former_parent": id, "reason": "parent_moved_project" }),
                        now,
                    )?;
                    out.orphaned.push(child);
                }
            }
            out.orphaned.sort();
            out.orphaned.dedup();

            Ok(out)
        })
    }
}
