//! Ticket CRUD: create (idempotent, with similar-title hints), get, list,
//! patch (commutative field sets; CAS for body), comments, dependency edges.

use super::helpers::{
    check_fence_for_write, clear_expired_claim, emit_event, get_ticket_opt, get_ticket_required,
    get_workflow, load_blocked_by, row_to_ticket, touch_ticket, TICKET_COLS,
};
use super::model::{
    Comment, Ticket, MAX_BODY, MAX_COMMENT, MAX_METADATA, MAX_TITLE, PRIORITIES, TICKET_TYPES,
};
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{comment_id, now_ms, sha256_hex, ticket_suffix};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default)]
pub struct TicketCreate {
    pub project: String,
    pub ty: Option<String>,
    pub parent: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    pub metadata: Option<Value>,
    pub blocked_by: Vec<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TicketPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    pub labels: Option<Vec<String>>,
    pub labels_add: Vec<String>,
    pub labels_remove: Vec<String>,
    /// None = absent; Some(None) = clear parent; Some(Some(id)) = set parent.
    pub parent: Option<Option<String>>,
    pub links: Option<Value>,
    pub metadata_merge: Option<Value>,
    pub fence: Option<i64>,
}

impl TicketPatch {
    /// True when the patch touches nothing but metadata_merge.
    fn only_metadata_merge(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.priority.is_none()
            && self.labels.is_none()
            && self.labels_add.is_empty()
            && self.labels_remove.is_empty()
            && self.parent.is_none()
            && self.links.is_none()
            && self.metadata_merge.is_some()
    }
}

/// Direction to walk the dependency graph in `dep_graph`.
#[derive(Debug, Clone, Copy)]
pub enum DepDirection {
    /// Tickets that block the node (follow `blocked_by` edges forward).
    BlockedBy,
    /// Tickets the node blocks (follow the inverse edges).
    Blocks,
    /// Both directions.
    Both,
}

impl DepDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            DepDirection::BlockedBy => "blocked_by",
            DepDirection::Blocks => "blocks",
            DepDirection::Both => "both",
        }
    }

    /// Parse the `direction` query value; None for an unrecognized value.
    pub fn parse(raw: &str) -> Option<DepDirection> {
        match raw {
            "blocked_by" => Some(DepDirection::BlockedBy),
            "blocks" => Some(DepDirection::Blocks),
            "both" => Some(DepDirection::Both),
            _ => None,
        }
    }
}

/// Whether a list query includes archived tickets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArchivedFilter {
    /// Default: only active (non-archived) tickets.
    #[default]
    Exclude,
    /// Only archived tickets.
    Only,
    /// Both archived and active.
    Include,
}

#[derive(Debug, Clone, Default)]
pub struct TicketListFilter {
    pub project: Option<String>,
    /// State id or category name.
    pub state: Option<String>,
    pub ty: Option<String>,
    /// AND semantics.
    pub labels: Vec<String>,
    pub parent: Option<String>,
    pub q: Option<String>,
    pub claimed_by: Option<String>,
    /// Token project scoping. None = unrestricted.
    pub allowed_projects: Option<Vec<String>>,
    /// Archived-ticket visibility (default: active only).
    pub archived: ArchivedFilter,
}

/// RFC 7386 merge-patch: objects merge recursively, null deletes, everything
/// else replaces.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(patch_map) => {
            if !target.is_object() {
                *target = Value::Object(serde_json::Map::new());
            }
            let target_map = target.as_object_mut().unwrap();
            for (k, v) in patch_map {
                if v.is_null() {
                    target_map.remove(k);
                } else if v.is_object() {
                    let entry = target_map.entry(k.clone()).or_insert(Value::Null);
                    merge_patch(entry, v);
                } else {
                    target_map.insert(k.clone(), v.clone());
                }
            }
        }
        other => *target = other.clone(),
    }
}

/// Lowercased alphanumeric title keywords of length >= 3, minus stopwords.
fn title_keywords(title: &str) -> Vec<String> {
    const STOP: [&str; 24] = [
        "the", "and", "for", "with", "from", "that", "this", "into", "when", "not", "are", "was",
        "has", "have", "can", "should", "add", "fix", "make", "use", "new", "all", "our", "its",
    ];
    let mut out: Vec<String> = Vec::new();
    for word in title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
    {
        if word.len() >= 3 && !STOP.contains(&word) && !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
    }
    out
}

fn validate_priority(p: &str) -> ApiResult<()> {
    if PRIORITIES.contains(&p) {
        return Ok(());
    }
    Err(ApiError::validation(
        "validation.priority",
        format!(
            "Unknown priority '{p}'. Use one of: {}.",
            PRIORITIES.join(", ")
        ),
    ))
}

fn validate_labels(labels: &[String]) -> ApiResult<()> {
    for l in labels {
        if l.is_empty() || l.len() > 100 {
            return Err(ApiError::validation(
                "validation.label",
                "Labels must be 1-100 characters.",
            ));
        }
    }
    Ok(())
}

fn validate_metadata_size(metadata: &Value) -> ApiResult<()> {
    let size = serde_json::to_string(metadata)
        .map(|s| s.len())
        .unwrap_or(0);
    if size > MAX_METADATA {
        return Err(ApiError::validation(
            "validation.metadata_size",
            format!(
                "metadata is {size} bytes serialized; the cap is {MAX_METADATA}. Trim old keys (set them to null via metadata_merge) or move bulk content into the ticket body or a comment."
            ),
        ));
    }
    Ok(())
}

/// Would setting `child`'s parent to `new_parent` create a cycle in the tree?
fn parent_cycle(conn: &Connection, child: &str, new_parent: &str) -> ApiResult<bool> {
    let mut cursor = Some(new_parent.to_string());
    let mut hops = 0;
    while let Some(node) = cursor {
        if node == child {
            return Ok(true);
        }
        hops += 1;
        if hops > 10_000 {
            return Err(ApiError::internal("parent chain too deep"));
        }
        cursor = conn
            .query_row(
                "SELECT parent FROM tickets WHERE id = ?1",
                params![node],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }
    Ok(false)
}

/// Would adding edge `ticket blocked_by target` create a dependency cycle?
/// True when `target` transitively depends on (is blocked by) `ticket`.
fn dep_cycle(conn: &Connection, ticket: &str, target: &str) -> ApiResult<bool> {
    if ticket == target {
        return Ok(true);
    }
    let hit: Option<i64> = conn
        .query_row(
            r#"
            WITH RECURSIVE reach(id) AS (
                SELECT ?1
                UNION
                SELECT d.blocked_by FROM deps d JOIN reach r ON d.ticket = r.id
            )
            SELECT 1 FROM reach WHERE id = ?2 LIMIT 1
            "#,
            params![target, ticket],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.is_some())
}

/// Minimum blended similarity score for a ticket to surface in `similar[]`.
/// Tuned so a genuine near-duplicate clears it while incidental one-word
/// overlap (the old cry-wolf behaviour) does not.
const SIMILAR_THRESHOLD: f64 = 0.30;

/// Score two title keyword sets: Jaccard overlap of the terms, plus a small
/// bonus when the ticket types match (a bug and a task that share words are
/// less likely dupes than two bugs). Returns (score, matched_terms).
fn similarity_score(
    kws: &[String],
    ty: &str,
    other_kws: &[String],
    other_ty: &str,
) -> (f64, Vec<String>) {
    let matched: Vec<String> = kws
        .iter()
        .filter(|k| other_kws.contains(k))
        .cloned()
        .collect();
    if matched.is_empty() {
        return (0.0, matched);
    }
    let union = kws.len() + other_kws.len() - matched.len();
    let jaccard = matched.len() as f64 / union as f64;
    // Same-type dupes are likelier; scale the overlap up by 15% (capped at 1.0)
    // rather than adding a flat bonus, so a single incidental shared word still
    // stays under the threshold instead of crossing it on type alone.
    let score = if ty == other_ty {
        (jaccard * 1.15).min(1.0)
    } else {
        jaccard
    };
    (score, matched)
}

/// Open (non-terminal) tickets in the project whose title keywords overlap the
/// new ticket's, scored by blended Jaccard + type match and thresholded so the
/// hint is trustworthy rather than a cry-wolf keyword match. Each entry carries
/// its `score` (0..1, 2 d.p.) and the `matched_terms` that drove it.
fn open_similar(
    conn: &Connection,
    project: &str,
    title: &str,
    ty: &str,
    exclude: &str,
) -> ApiResult<Vec<Value>> {
    let kws = title_keywords(title);
    if kws.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        r#"
        SELECT t.id, t.title, t.state, t.type FROM tickets t
        JOIN workflow_states ws ON ws.project = t.project AND ws.state = t.state
        WHERE t.project = ?1 AND ws.terminal = 0 AND t.id != ?2
        "#,
    )?;
    let rows = stmt
        .query_map(params![project, exclude], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut scored: Vec<(f64, Value)> = rows
        .into_iter()
        .filter_map(|(id, other_title, state, other_ty)| {
            let other_kws = title_keywords(&other_title);
            let (score, matched) = similarity_score(&kws, ty, &other_kws, &other_ty);
            if score >= SIMILAR_THRESHOLD {
                Some((
                    score,
                    json!({
                        "id": id,
                        "title": other_title,
                        "state": state,
                        "type": other_ty,
                        "score": (score * 100.0).round() / 100.0,
                        "matched_terms": matched,
                    }),
                ))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored.into_iter().take(5).map(|(_, v)| v).collect())
}

fn generate_ticket_id(conn: &Connection, project: &str) -> ApiResult<String> {
    for len in [4usize, 4, 4, 5, 5, 6, 8] {
        let candidate = format!("{project}-{}", ticket_suffix(len));
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM tickets WHERE id = ?1",
                params![candidate],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(candidate);
        }
    }
    Err(ApiError::internal("could not generate a unique ticket id"))
}

impl Store {
    /// Create a ticket. Returns (ticket, similar, replayed).
    pub fn create_ticket(
        &self,
        req: &TicketCreate,
        actor: &str,
        idempotency_key: Option<&str>,
    ) -> ApiResult<(Ticket, Vec<Value>, bool)> {
        if req.title.is_empty() || req.title.len() > MAX_TITLE {
            return Err(ApiError::validation(
                "validation.title",
                format!("title must be 1-{MAX_TITLE} characters."),
            ));
        }
        let ty = req.ty.clone().unwrap_or_else(|| "task".to_string());
        if !TICKET_TYPES.contains(&ty.as_str()) {
            return Err(ApiError::validation(
                "validation.type",
                format!(
                    "Unknown ticket type '{ty}'. Use one of: {}.",
                    TICKET_TYPES.join(", ")
                ),
            ));
        }
        let body = req.body.clone().unwrap_or_default();
        if body.len() > MAX_BODY {
            return Err(ApiError::validation(
                "validation.body",
                format!("body must be at most {MAX_BODY} bytes."),
            ));
        }
        let priority = req.priority.clone().unwrap_or_else(|| "normal".to_string());
        validate_priority(&priority)?;
        validate_labels(&req.labels)?;
        let metadata = req.metadata.clone().unwrap_or_else(|| json!({}));
        if !metadata.is_object() {
            return Err(ApiError::validation(
                "validation.metadata",
                "metadata must be a JSON object with namespaced keys like \"myagent.run_id\".",
            ));
        }
        validate_metadata_size(&metadata)?;

        let now = now_ms();
        self.with_tx(|tx| {
            let wf = get_workflow(tx, &req.project).map_err(|_| {
                ApiError::validation(
                    "validation.project",
                    format!(
                        "Unknown project '{}'. GET /v1/projects lists existing projects; an admin can create one with POST /v1/projects.",
                        req.project
                    ),
                )
            })?;

            // Idempotent replay?
            if let Some(key) = idempotency_key {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT ticket FROM idempotency WHERE actor = ?1 AND key = ?2",
                        params![actor, key],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(tid) = existing {
                    let ticket = get_ticket_required(tx, &tid)?;
                    let similar =
                        open_similar(tx, &ticket.project, &ticket.title, &ticket.ty, &ticket.id)?;
                    return Ok((ticket, similar, true));
                }
            }

            let state = match &req.state {
                None => wf.initial.clone(),
                Some(s) => {
                    if *s != wf.initial {
                        return Err(ApiError::validation(
                            "validation.state",
                            format!(
                                "Tickets are created in the workflow's initial state '{}'; '{s}' is not a legal initial state. Create the ticket, then move it with POST /v1/tickets/{{id}}/transition.",
                                wf.initial
                            ),
                        ));
                    }
                    s.clone()
                }
            };

            if let Some(parent) = &req.parent {
                let p = get_ticket_opt(tx, parent)?.ok_or_else(|| {
                    ApiError::validation(
                        "validation.parent",
                        format!("Parent ticket '{parent}' does not exist."),
                    )
                })?;
                if p.project != req.project {
                    return Err(ApiError::validation(
                        "validation.parent",
                        format!(
                            "Parent '{parent}' belongs to project '{}', not '{}'. Parent and child must share a project.",
                            p.project, req.project
                        ),
                    ));
                }
            }

            for dep in &req.blocked_by {
                if get_ticket_opt(tx, dep)?.is_none() {
                    return Err(ApiError::validation(
                        "validation.blocked_by",
                        format!("blocked_by references unknown ticket '{dep}'."),
                    ));
                }
            }

            let id = generate_ticket_id(tx, &req.project)?;
            tx.execute(
                "INSERT INTO tickets (id, project, type, parent, title, body, state, priority, labels, metadata, links, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}', ?11, ?12, ?12)",
                params![
                    id,
                    req.project,
                    ty,
                    req.parent,
                    req.title,
                    body,
                    state,
                    priority,
                    serde_json::to_string(&req.labels).unwrap(),
                    metadata.to_string(),
                    actor,
                    now
                ],
            )?;
            for dep in &req.blocked_by {
                tx.execute(
                    "INSERT OR IGNORE INTO deps (ticket, blocked_by) VALUES (?1, ?2)",
                    params![id, dep],
                )?;
            }
            if let Some(key) = idempotency_key {
                tx.execute(
                    "INSERT INTO idempotency (actor, key, ticket, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![actor, key, id, now],
                )?;
            }
            emit_event(
                tx,
                Some(&id),
                Some(&req.project),
                actor,
                "created",
                json!({ "title": req.title, "type": ty, "state": state }),
                now,
            )?;
            let ticket = get_ticket_required(tx, &id)?;
            let similar = open_similar(tx, &req.project, &req.title, &ty, &id)?;
            Ok((ticket, similar, false))
        })
    }

    pub fn get_ticket(&self, id: &str) -> ApiResult<Option<Ticket>> {
        self.with_conn(|conn| get_ticket_opt(conn, id))
    }

    /// Every ticket (with blocked_by) plus its comments, for JSONL export.
    /// Ordered by creation so parents/blockers generally precede dependents and
    /// a re-import replays deterministically. `project` narrows to one project;
    /// `allowed_projects` (token scoping) narrows to a permitted set (None =
    /// unrestricted). Comments come back oldest-first per ticket.
    pub fn export_tickets(
        &self,
        project: Option<&str>,
        allowed_projects: Option<&[String]>,
    ) -> ApiResult<Vec<(Ticket, Vec<Comment>)>> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {TICKET_COLS} FROM tickets t WHERE 1=1");
            let mut params_vec: Vec<SqlValue> = Vec::new();
            if let Some(p) = project {
                sql.push_str(" AND t.project = ?");
                params_vec.push(SqlValue::Text(p.to_string()));
            }
            if let Some(allowed) = allowed_projects {
                sql.push_str(" AND t.project IN (");
                for (i, p) in allowed.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    params_vec.push(SqlValue::Text(p.clone()));
                }
                sql.push(')');
            }
            sql.push_str(" ORDER BY t.created_at ASC, t.rowid ASC");

            let mut stmt = conn.prepare(&sql)?;
            let mut tickets = stmt
                .query_map(rusqlite::params_from_iter(params_vec), row_to_ticket)?
                .collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::with_capacity(tickets.len());
            for t in &mut tickets {
                load_blocked_by(conn, t)?;
                let mut cstmt = conn.prepare(
                    "SELECT id, ticket, author, body, created_at FROM comments WHERE ticket = ?1 ORDER BY created_at, id",
                )?;
                let comments = cstmt
                    .query_map(params![t.id], |r| {
                        Ok(Comment {
                            id: r.get(0)?,
                            ticket: r.get(1)?,
                            author: r.get(2)?,
                            body: r.get(3)?,
                            created_at: r.get(4)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                out.push((t.clone(), comments));
            }
            Ok(out)
        })
    }

    /// List/search with cursor pagination. Returns (tickets, next_cursor).
    pub fn list_tickets(
        &self,
        filter: &TicketListFilter,
        cursor: Option<i64>,
        limit: i64,
    ) -> ApiResult<(Vec<Ticket>, Option<String>)> {
        self.with_conn(|conn| {
            let mut sql = format!("SELECT {TICKET_COLS}, t.rowid AS rid FROM tickets t WHERE 1=1");
            let mut params_vec: Vec<SqlValue> = Vec::new();
            match filter.archived {
                ArchivedFilter::Exclude => sql.push_str(" AND t.archived_at IS NULL"),
                ArchivedFilter::Only => sql.push_str(" AND t.archived_at IS NOT NULL"),
                ArchivedFilter::Include => {}
            }
            if let Some(c) = cursor {
                sql.push_str(" AND t.rowid < ?");
                params_vec.push(SqlValue::Integer(c));
            }
            if let Some(p) = &filter.project {
                sql.push_str(" AND t.project = ?");
                params_vec.push(SqlValue::Text(p.clone()));
            }
            if let Some(allowed) = &filter.allowed_projects {
                sql.push_str(" AND t.project IN (");
                for (i, p) in allowed.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                    params_vec.push(SqlValue::Text(p.clone()));
                }
                sql.push(')');
            }
            if let Some(s) = &filter.state {
                sql.push_str(
                    " AND (t.state = ? OR EXISTS (SELECT 1 FROM workflow_states ws WHERE ws.project = t.project AND ws.state = t.state AND ws.category = ?))",
                );
                params_vec.push(SqlValue::Text(s.clone()));
                params_vec.push(SqlValue::Text(s.clone()));
            }
            if let Some(ty) = &filter.ty {
                sql.push_str(" AND t.type = ?");
                params_vec.push(SqlValue::Text(ty.clone()));
            }
            for label in &filter.labels {
                sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = ?)");
                params_vec.push(SqlValue::Text(label.clone()));
            }
            if let Some(p) = &filter.parent {
                sql.push_str(" AND t.parent = ?");
                params_vec.push(SqlValue::Text(p.clone()));
            }
            if let Some(q) = &filter.q {
                // Tokenized full-text: every whitespace-separated term must
                // match (AND), each against title OR body (case-insensitive).
                // "auth token" finds a ticket titled "token" with "auth" in the
                // body, but not one that mentions only "auth".
                for term in q.split_whitespace() {
                    sql.push_str(" AND (LOWER(t.title) LIKE ? OR LOWER(t.body) LIKE ?)");
                    let needle = format!("%{}%", term.to_lowercase());
                    params_vec.push(SqlValue::Text(needle.clone()));
                    params_vec.push(SqlValue::Text(needle));
                }
            }
            if let Some(holder) = &filter.claimed_by {
                sql.push_str(" AND t.claim_holder = ? AND t.claim_expires_at > ?");
                params_vec.push(SqlValue::Text(holder.clone()));
                params_vec.push(SqlValue::Integer(now_ms()));
            }
            sql.push_str(" ORDER BY t.rowid DESC LIMIT ?");
            params_vec.push(SqlValue::Integer(limit + 1));

            let mut stmt = conn.prepare(&sql)?;
            let mut rows_with_rid: Vec<(Ticket, i64)> = Vec::new();
            let mapped = stmt.query_map(rusqlite::params_from_iter(params_vec), |r| {
                let rid: i64 = r.get("rid")?;
                Ok((row_to_ticket(r)?, rid))
            })?;
            for row in mapped {
                rows_with_rid.push(row?);
            }
            let has_more = rows_with_rid.len() as i64 > limit;
            rows_with_rid.truncate(limit as usize);
            let next_cursor = if has_more {
                rows_with_rid.last().map(|(_, rid)| rid.to_string())
            } else {
                None
            };
            let mut tickets = Vec::with_capacity(rows_with_rid.len());
            for (mut t, _) in rows_with_rid {
                load_blocked_by(conn, &mut t)?;
                tickets.push(t);
            }
            Ok((tickets, next_cursor))
        })
    }

    /// PATCH semantics per the contract; see openapi.yaml TicketPatch.
    pub fn patch_ticket(
        &self,
        id: &str,
        patch: &TicketPatch,
        actor: &str,
        if_match: Option<i64>,
    ) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            let mut t = get_ticket_required(tx, id)?;
            if clear_expired_claim(tx, &t, now)? {
                t.claim_holder = None;
                t.claim_expires_at = None;
            }

            // Claimed-ticket write restrictions.
            match t.active_claim(now) {
                Some((holder, expires)) if holder != actor => {
                    let ns = format!("{actor}.");
                    let own_namespace_only = patch
                        .metadata_merge
                        .as_ref()
                        .and_then(|m| m.as_object())
                        .map(|m| !m.is_empty() && m.keys().all(|k| k.starts_with(&ns)))
                        .unwrap_or(false);
                    if !(patch.only_metadata_merge() && own_namespace_only) {
                        return Err(ApiError::conflict(
                            "claim.held",
                            format!(
                                "Ticket '{id}' is claimed by '{holder}' until {}. While claimed, non-holders may only merge non-empty metadata under their own namespace ('{ns}<key>') or add comments (POST /v1/tickets/{id}/comments). For other changes, wait for the lease to expire or coordinate with the holder.",
                                crate::ids::iso(expires)
                            ),
                        )
                        .details(json!({ "holder": holder, "expires_at": crate::ids::iso(expires) })));
                    }
                }
                // Holder must echo a current fence; an unclaimed ticket still
                // bounces a stale echoed fence (zombie writer protection).
                _ => check_fence_for_write(&t, actor, patch.fence, now, "update fields")?,
            }

            let mut changed: Vec<&str> = Vec::new();

            if let Some(title) = &patch.title {
                if title.is_empty() || title.len() > MAX_TITLE {
                    return Err(ApiError::validation(
                        "validation.title",
                        format!("title must be 1-{MAX_TITLE} characters."),
                    ));
                }
                tx.execute(
                    "UPDATE tickets SET title = ?2 WHERE id = ?1",
                    params![id, title],
                )?;
                changed.push("title");
            }

            if let Some(priority) = &patch.priority {
                validate_priority(priority)?;
                tx.execute(
                    "UPDATE tickets SET priority = ?2 WHERE id = ?1",
                    params![id, priority],
                )?;
                changed.push("priority");
            }

            if patch.labels.is_some()
                || !patch.labels_add.is_empty()
                || !patch.labels_remove.is_empty()
            {
                let mut labels = match &patch.labels {
                    Some(set) => set.clone(),
                    None => t.labels.clone(),
                };
                for l in &patch.labels_add {
                    if !labels.contains(l) {
                        labels.push(l.clone());
                    }
                }
                labels.retain(|l| !patch.labels_remove.contains(l));
                let mut seen = std::collections::HashSet::new();
                labels.retain(|l| seen.insert(l.clone()));
                validate_labels(&labels)?;
                tx.execute(
                    "UPDATE tickets SET labels = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(&labels).unwrap()],
                )?;
                changed.push("labels");
            }

            if let Some(parent_change) = &patch.parent {
                match parent_change {
                    None => {
                        tx.execute(
                            "UPDATE tickets SET parent = NULL WHERE id = ?1",
                            params![id],
                        )?;
                    }
                    Some(new_parent) => {
                        if new_parent == id {
                            return Err(ApiError::validation(
                                "validation.parent",
                                "A ticket cannot be its own parent.",
                            ));
                        }
                        let p = get_ticket_opt(tx, new_parent)?.ok_or_else(|| {
                            ApiError::validation(
                                "validation.parent",
                                format!("Parent ticket '{new_parent}' does not exist."),
                            )
                        })?;
                        if p.project != t.project {
                            return Err(ApiError::validation(
                                "validation.parent",
                                format!(
                                    "Parent '{new_parent}' belongs to project '{}', not '{}'.",
                                    p.project, t.project
                                ),
                            ));
                        }
                        if parent_cycle(tx, id, new_parent)? {
                            return Err(ApiError::validation(
                                "validation.parent_cycle",
                                format!(
                                    "Setting parent '{new_parent}' would create a cycle in the ticket tree."
                                ),
                            ));
                        }
                        tx.execute(
                            "UPDATE tickets SET parent = ?2 WHERE id = ?1",
                            params![id, new_parent],
                        )?;
                    }
                }
                changed.push("parent");
            }

            if let Some(links) = &patch.links {
                // links MERGES per key (not whole-object replace): each key in
                // the patch is set (string) or deleted (null); untouched keys
                // survive. So a worker attaching a `pr` link never clobbers a
                // `branch` link another writer set.
                let patch_obj = links.as_object().ok_or_else(|| {
                    ApiError::validation(
                        "validation.links",
                        "links must be a JSON object of string values (null to delete a key), e.g. {\"pr\": \"https://...\", \"branch\": null}.",
                    )
                })?;
                let mut merged = match &t.links {
                    Value::Object(m) => m.clone(),
                    _ => serde_json::Map::new(),
                };
                for (k, v) in patch_obj {
                    if v.is_null() {
                        merged.remove(k);
                    } else if v.is_string() {
                        merged.insert(k.clone(), v.clone());
                    } else {
                        return Err(ApiError::validation(
                            "validation.links",
                            format!("links.{k} must be a string (or null to delete it)."),
                        ));
                    }
                }
                let merged = Value::Object(merged);
                tx.execute(
                    "UPDATE tickets SET links = ?2 WHERE id = ?1",
                    params![id, merged.to_string()],
                )?;
                changed.push("links");
            }

            if let Some(mp) = &patch.metadata_merge {
                if !mp.is_object() {
                    return Err(ApiError::validation(
                        "validation.metadata_merge",
                        "metadata_merge must be a JSON object (RFC 7386 merge-patch; null values delete keys).",
                    ));
                }
                let mut metadata = t.metadata.clone();
                if !metadata.is_object() {
                    metadata = json!({});
                }
                merge_patch(&mut metadata, mp);
                validate_metadata_size(&metadata)?;
                tx.execute(
                    "UPDATE tickets SET metadata = ?2 WHERE id = ?1",
                    params![id, metadata.to_string()],
                )?;
                changed.push("metadata");
            }

            if let Some(body) = &patch.body {
                if body.len() > MAX_BODY {
                    return Err(ApiError::validation(
                        "validation.body",
                        format!("body must be at most {MAX_BODY} bytes."),
                    ));
                }
                match if_match {
                    None => {
                        return Err(ApiError::conflict(
                            "conflict.if_match_required",
                            format!(
                                "Replacing 'body' requires optimistic concurrency: send If-Match: \"{}\" (the current version, from GET /v1/tickets/{id}'s ETag or 'version' field). Field sets and metadata_merge never need If-Match.",
                                t.version
                            ),
                        )
                        .current_version(t.version)
                        .remedy(format!("GET /v1/tickets/{id}, then retry PATCH with If-Match")))
                    }
                    Some(v) if v != t.version => {
                        return Err(ApiError::conflict(
                            "conflict.version",
                            format!(
                                "If-Match version {v} does not match current version {}. The body changed since you read it. Re-read the ticket, re-apply your edit to the fresh body, and retry with If-Match: \"{}\".",
                                t.version, t.version
                            ),
                        )
                        .current_version(t.version)
                        .details(json!({ "body_sha256": sha256_hex(t.body.as_bytes()) }))
                        .remedy(format!("GET /v1/tickets/{id}, then retry PATCH with the fresh If-Match")))
                    }
                    Some(_) => {}
                }
                tx.execute(
                    "UPDATE tickets SET body = ?2 WHERE id = ?1",
                    params![id, body],
                )?;
                changed.push("body");
            }

            if changed.is_empty() {
                return Err(ApiError::validation(
                    "validation.empty_patch",
                    "The patch contains no changes. Provide at least one of: title, body, priority, labels, labels_add, labels_remove, parent, links, metadata_merge.",
                ));
            }

            touch_ticket(tx, id, now)?;
            emit_event(
                tx,
                Some(id),
                Some(&t.project),
                actor,
                "updated",
                json!({ "changed": changed }),
                now,
            )?;
            get_ticket_required(tx, id)
        })
    }

    pub fn add_comment(&self, ticket_id: &str, actor: &str, body: &str) -> ApiResult<Comment> {
        if body.is_empty() || body.len() > MAX_COMMENT {
            return Err(ApiError::validation(
                "validation.comment",
                format!("comment body must be 1-{MAX_COMMENT} bytes."),
            ));
        }
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, ticket_id)?;
            let comment = Comment {
                id: comment_id(),
                ticket: t.id.clone(),
                author: actor.to_string(),
                body: body.to_string(),
                created_at: now,
            };
            tx.execute(
                "INSERT INTO comments (id, ticket, author, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![comment.id, comment.ticket, comment.author, comment.body, now],
            )?;
            emit_event(
                tx,
                Some(&t.id),
                Some(&t.project),
                actor,
                "commented",
                json!({ "comment": comment.id }),
                now,
            )?;
            Ok(comment)
        })
    }

    /// Archive a ticket: set `archived_at` so it drops out of the default
    /// list/ready/board/metrics views. Idempotent — re-archiving keeps the
    /// original timestamp. Any state may be archived (terminal done/cancelled is
    /// the typical case). Returns the fresh ticket.
    pub fn archive_ticket(&self, id: &str, actor: &str) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            if t.archived_at.is_none() {
                let stamp = crate::ids::iso(now);
                tx.execute(
                    "UPDATE tickets SET archived_at = ?2, version = version + 1, updated_at = ?3 WHERE id = ?1",
                    params![id, stamp, now],
                )?;
                emit_event(
                    tx,
                    Some(id),
                    Some(&t.project),
                    actor,
                    "archived",
                    json!({ "archived_at": stamp }),
                    now,
                )?;
            }
            get_ticket_required(tx, id)
        })
    }

    /// Unarchive a ticket: clear `archived_at` so it returns to the default
    /// views. Idempotent — unarchiving an active ticket is a no-op.
    pub fn unarchive_ticket(&self, id: &str, actor: &str) -> ApiResult<Ticket> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, id)?;
            if t.archived_at.is_some() {
                tx.execute(
                    "UPDATE tickets SET archived_at = NULL, version = version + 1, updated_at = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
                emit_event(
                    tx,
                    Some(id),
                    Some(&t.project),
                    actor,
                    "unarchived",
                    json!({}),
                    now,
                )?;
            }
            get_ticket_required(tx, id)
        })
    }

    pub fn comments_for(&self, ticket_id: &str) -> ApiResult<Vec<Comment>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, ticket, author, body, created_at FROM comments WHERE ticket = ?1 ORDER BY created_at, id",
            )?;
            let rows = stmt
                .query_map(params![ticket_id], |r| {
                    Ok(Comment {
                        id: r.get(0)?,
                        ticket: r.get(1)?,
                        author: r.get(2)?,
                        body: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn children_of(&self, ticket_id: &str) -> ApiResult<Vec<Ticket>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {TICKET_COLS} FROM tickets t WHERE t.parent = ?1 ORDER BY t.created_at, t.id"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt
                .query_map(params![ticket_id], row_to_ticket)?
                .collect::<Result<Vec<_>, _>>()?;
            for t in &mut rows {
                load_blocked_by(conn, t)?;
            }
            Ok(rows)
        })
    }

    /// Tickets this ticket blocks (reverse edges).
    pub fn blocks_of(&self, ticket_id: &str) -> ApiResult<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT ticket FROM deps WHERE blocked_by = ?1 ORDER BY ticket")?;
            let rows = stmt
                .query_map(params![ticket_id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Build the dependency graph reachable from `root` in the requested
    /// direction(s). `blocked_by` follows edges to the tickets that block a
    /// node; `blocks` follows the inverse (tickets a node blocks); `both`
    /// follows both. With `transitive`, the walk chases the chain to its ends;
    /// otherwise it returns only `root`'s direct neighbours. The walk is
    /// cycle-safe: a `visited` set bounds it even when A blocks B blocks A.
    ///
    /// Edges are canonical `{ticket, blocked_by}` pairs (ticket is blocked by
    /// blocked_by) regardless of traversal direction, so the same edge is never
    /// duplicated when `both` reaches it from either side.
    pub fn dep_graph(
        &self,
        root: &str,
        direction: DepDirection,
        transitive: bool,
    ) -> ApiResult<Value> {
        use std::collections::{HashSet, VecDeque};
        self.with_conn(|conn| {
            // 404 if the root ticket does not exist.
            get_ticket_required(conn, root)?;

            let want_blocked_by =
                matches!(direction, DepDirection::BlockedBy | DepDirection::Both);
            let want_blocks = matches!(direction, DepDirection::Blocks | DepDirection::Both);

            let mut nodes: HashSet<String> = HashSet::new();
            let mut expanded: HashSet<String> = HashSet::new();
            let mut edge_seen: HashSet<(String, String)> = HashSet::new();
            let mut edges: Vec<(String, String)> = Vec::new();
            let mut queue: VecDeque<String> = VecDeque::new();

            nodes.insert(root.to_string());
            queue.push_back(root.to_string());

            while let Some(node) = queue.pop_front() {
                if !expanded.insert(node.clone()) {
                    continue; // already walked — cycle guard
                }
                // Collect (edge_ticket, edge_blocked_by, next_node) neighbours.
                let mut neigh: Vec<(String, String, String)> = Vec::new();
                if want_blocked_by {
                    let mut stmt =
                        conn.prepare("SELECT blocked_by FROM deps WHERE ticket = ?1")?;
                    let bs = stmt
                        .query_map(params![node], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    for b in bs {
                        neigh.push((node.clone(), b.clone(), b));
                    }
                }
                if want_blocks {
                    let mut stmt =
                        conn.prepare("SELECT ticket FROM deps WHERE blocked_by = ?1")?;
                    let ts = stmt
                        .query_map(params![node], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    for t in ts {
                        neigh.push((t.clone(), node.clone(), t));
                    }
                }
                for (et, eb, next) in neigh {
                    let key = (et.clone(), eb.clone());
                    if edge_seen.insert(key) {
                        edges.push((et, eb));
                    }
                    nodes.insert(next.clone());
                    if transitive {
                        queue.push_back(next);
                    }
                }
            }

            // Fetch node details (id, title, state, category, type), sorted for
            // determinism. A node may belong to another project (deps can cross
            // projects); it is still shown by id and details.
            let mut ids: Vec<String> = nodes.into_iter().collect();
            ids.sort();
            let mut node_json = Vec::with_capacity(ids.len());
            for id in &ids {
                let row = conn
                    .query_row(
                        "SELECT t.id, t.title, t.state, \
                         COALESCE((SELECT ws.category FROM workflow_states ws WHERE ws.project = t.project AND ws.state = t.state), '') AS category, \
                         t.type FROM tickets t WHERE t.id = ?1",
                        params![id],
                        |r| {
                            Ok(json!({
                                "id": r.get::<_, String>(0)?,
                                "title": r.get::<_, String>(1)?,
                                "state": r.get::<_, String>(2)?,
                                "state_category": r.get::<_, String>(3)?,
                                "type": r.get::<_, String>(4)?,
                            }))
                        },
                    )
                    .optional()?;
                if let Some(v) = row {
                    node_json.push(v);
                }
            }

            edges.sort();
            let edge_json: Vec<Value> = edges
                .into_iter()
                .map(|(t, b)| json!({ "ticket": t, "blocked_by": b }))
                .collect();

            Ok(json!({
                "root": root,
                "direction": direction.as_str(),
                "transitive": transitive,
                "nodes": node_json,
                "edges": edge_json,
            }))
        })
    }

    /// Add a blocked_by edge. Returns true if newly added (false = existed).
    /// Dependency edges drive readiness and the no_open_blockers guard, so
    /// they follow the same claim/fence rules as any other ticket mutation.
    pub fn add_dep(
        &self,
        ticket_id: &str,
        blocked_by: &str,
        actor: &str,
        fence: Option<i64>,
    ) -> ApiResult<bool> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, ticket_id)?;
            check_fence_for_write(&t, actor, fence, now, "modify dependencies")?;
            if get_ticket_opt(tx, blocked_by)?.is_none() {
                return Err(ApiError::validation(
                    "validation.blocked_by",
                    format!("blocked_by references unknown ticket '{blocked_by}'."),
                ));
            }
            if dep_cycle(tx, ticket_id, blocked_by)? {
                return Err(ApiError::validation(
                    "validation.dep_cycle",
                    format!(
                        "Adding blocked_by '{blocked_by}' to '{ticket_id}' would create a dependency cycle; '{blocked_by}' already depends on '{ticket_id}' (directly or transitively)."
                    ),
                ));
            }
            let n = tx.execute(
                "INSERT OR IGNORE INTO deps (ticket, blocked_by) VALUES (?1, ?2)",
                params![ticket_id, blocked_by],
            )?;
            if n > 0 {
                touch_ticket(tx, ticket_id, now)?;
                emit_event(
                    tx,
                    Some(ticket_id),
                    Some(&t.project),
                    actor,
                    "dep_added",
                    json!({ "blocked_by": blocked_by }),
                    now,
                )?;
            }
            Ok(n > 0)
        })
    }

    pub fn remove_dep(
        &self,
        ticket_id: &str,
        blocked_by: &str,
        actor: &str,
        fence: Option<i64>,
    ) -> ApiResult<bool> {
        let now = now_ms();
        self.with_tx(|tx| {
            let t = get_ticket_required(tx, ticket_id)?;
            check_fence_for_write(&t, actor, fence, now, "modify dependencies")?;
            let n = tx.execute(
                "DELETE FROM deps WHERE ticket = ?1 AND blocked_by = ?2",
                params![ticket_id, blocked_by],
            )?;
            if n > 0 {
                touch_ticket(tx, ticket_id, now)?;
                emit_event(
                    tx,
                    Some(ticket_id),
                    Some(&t.project),
                    actor,
                    "dep_removed",
                    json!({ "blocked_by": blocked_by }),
                    now,
                )?;
            }
            Ok(n > 0)
        })
    }
}
