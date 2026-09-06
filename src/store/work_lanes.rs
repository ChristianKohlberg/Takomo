//! Persistent work context and immutable, explicitly dispatched assignments.
use super::{
    helpers::{emit_event, ensure_project_writable, get_ticket_required},
    Store,
};
use crate::{
    error::{ApiError, ApiResult},
    ids::{iso, now_ms, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.handoff", message)
}
fn record(c: &Connection, table: &str, id: &str) -> ApiResult<Value> {
    let raw: String = c
        .query_row(
            &format!("SELECT data FROM {table} WHERE id=?1"),
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            if table == "work_lanes" {
                ApiError::not_found("lane", id)
            } else {
                ApiError::not_found("handoff", id)
            }
        })?;
    serde_json::from_str(&raw)
        .map_err(|e| ApiError::internal(format!("Stored lane JSON is invalid: {e}")))
}
fn save_lane(c: &Connection, lane: &mut Value, actor: &str) -> ApiResult<()> {
    let now = now_ms();
    lane["updated_at"] = json!(iso(now));
    c.execute(
        "UPDATE work_lanes SET data=?1,updated_at=?2 WHERE id=?3",
        params![lane.to_string(), now, lane["id"].as_str()],
    )?;
    emit_event(
        c,
        None,
        lane["project"].as_str(),
        actor,
        "lane.updated",
        json!({
        "lane":lane["id"]}
        ),
        now,
    )?;
    Ok(())
}
fn writable(c: &Connection, lane: &Value) -> ApiResult<()> {
    ensure_project_writable(c, lane["project"].as_str().unwrap())?;
    if lane["archived"] == true {
        return Err(conflict(
            "This lane is archived. Unarchive it before changing its work.",
        ));
    }
    Ok(())
}
fn tickets(c: &Connection, id: &str) -> ApiResult<Vec<Value>> {
    let mut stmt =
        c.prepare("SELECT ticket FROM work_lane_tickets WHERE lane=?1 ORDER BY ticket")?;
    let ids = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| {
            let t = get_ticket_required(c, id)?;
            Ok(json!({
            "id":t.id,
            "title":t.title,
            "state":t.state,
            "body":t.body,
            "links":t.links,
            "metadata":t.metadata}
            ))
        })
        .collect()
}
fn detail(c: &Connection, id: &str) -> ApiResult<Value> {
    let mut v = record(c, "work_lanes", id)?;
    v["tickets"] = json!(tickets(c, id)?);
    v["handoff_count"] = json!(c.query_row(
        "SELECT COUNT(*) FROM work_handoffs WHERE lane=?1",
        [id],
        |r| r.get::<_, i64>(0)
    )?);
    Ok(v)
}
fn text_field(body: &Value, key: &str, required: bool, max: usize) -> ApiResult<()> {
    match body.get(key) {
        Some(Value::String(v)) if v.len() <= max && (!required || !v.trim().is_empty()) => Ok(()),
        None if !required => Ok(()),
        _ => Err(ApiError::bad_request(
            "validation.field_type",
            format!(
                "{key} must be {}a string of at most {max} bytes.",
                if required { "a nonempty " } else { "" }
            ),
        )),
    }
}
fn fields(body: &Value, allowed: &[&str]) -> ApiResult<()> {
    crate::api::reject_unknown(crate::api::body_object(body)?, allowed)
}
fn validate_draft(body: &Value) -> ApiResult<()> {
    fields(
        body,
        &[
            "kind",
            "provider",
            "instructions",
            "ticket_ids",
            "target_revision",
            "parent_handoff",
        ],
    )?;
    for (key, values) in [
        ("kind", &["preparation", "implementation", "review"][..]),
        ("provider", &["codex", "claude"][..]),
    ] {
        if !body[key].as_str().is_some_and(|v| values.contains(&v)) {
            return Err(ApiError::bad_request(
                "validation.field_type",
                format!("Choose {key} from {}.", values.join(", ")),
            ));
        }
    }
    text_field(body, "instructions", true, 32000)?;
    let ids = body["ticket_ids"].as_array().ok_or_else(|| {
        ApiError::bad_request(
            "validation.field_type",
            "ticket_ids must be an array of ticket IDs.",
        )
    })?;
    if ids.len() > 200
        || (ids.is_empty() && body["kind"] != "preparation")
        || ids
            .iter()
            .any(|i| i.as_str().is_none_or(|s| s.is_empty() || s.len() > 200))
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        return Err(ApiError::bad_request(
            "validation.field_type",
            "Select 1–200 unique ticket IDs; preparation may have no tickets.",
        ));
    }
    let review = body["kind"] == "review";
    text_field(body, "target_revision", review, 200)?;
    text_field(body, "parent_handoff", review, 200)?;
    if !review && (body.get("target_revision").is_some() || body.get("parent_handoff").is_some()) {
        return Err(ApiError::bad_request(
            "validation.field_type",
            "Only review handoffs accept target_revision and parent_handoff.",
        ));
    }
    Ok(())
}
impl Store {
    pub fn work_lane_create(
        &self,
        project: &str,
        title: &str,
        purpose: &str,
        context: &str,
        actor: &str,
    ) -> ApiResult<Value> {
        text_field(
            &json!({
            "title":title}
            ),
            "title",
            true,
            200,
        )?;
        text_field(
            &json!({
            "purpose":purpose}
            ),
            "purpose",
            false,
            8000,
        )?;
        text_field(
            &json!({
            "context":context}
            ),
            "context",
            false,
            64000,
        )?;
        self.with_tx(|c| {
            ensure_project_writable(c, project)?;
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM work_lanes WHERE project=?1",
                [project],
                |r| r.get(0),
            )?;
            if count >= 500 {
                return Err(conflict("Project has 500 lanes. Reuse an existing lane."));
            }
            let id = format!("wl-{}", ticket_suffix(12));
            let now = now_ms();
            let v = json!({
            "id":id,
            "project":project,
            "title":title,
            "purpose":purpose,
            "context":context,
            "conversation_ref":null,
            "archived":false,
            "created_at":iso(now),
            "updated_at":iso(now)}
            );
            c.execute(
                "INSERT INTO work_lanes VALUES(?1,?2,?3,?4)",
                params![id, project, v.to_string(), now],
            )?;
            emit_event(
                c,
                None,
                Some(project),
                actor,
                "lane.created",
                json!({
                "lane":id}
                ),
                now,
            )?;
            detail(c, &id)
        })
    }
    pub fn work_lane_get(&self, id: &str) -> ApiResult<Value> {
        self.with_conn(|c| detail(c, id))
    }
    pub fn work_lane_list(
        &self,
        project: &str,
        limit: i64,
        offset: i64,
    ) -> ApiResult<(Vec<Value>, i64)> {
        self.with_conn(|c| {
            let limit = limit.clamp(1, 200);
            let offset = offset.max(0);
            let total = c.query_row("SELECT COUNT(*) FROM work_lanes WHERE project=?1", [project], |r| r.get(0))?;
            let mut s = c.prepare("SELECT id FROM work_lanes WHERE project=?1 ORDER BY updated_at DESC,id LIMIT ?2 OFFSET ?3")?;
            let ids = s.query_map(params![project, limit, offset], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok((ids.iter().map(|id| detail(c, id)).collect::<ApiResult<Vec<_>>>()?, total))
        })
    }
    pub fn work_lane_patch(&self, id: &str, patch: &Value, actor: &str) -> ApiResult<Value> {
        fields(patch, &["title", "purpose", "context", "archived"])?;
        for (key, max) in [("title", 200), ("purpose", 8000), ("context", 64000)] {
            if patch.get(key).is_some() {
                text_field(patch, key, key == "title", max)?;
            }
        }
        if patch.get("archived").is_some_and(|v| !v.is_boolean()) {
            return Err(ApiError::bad_request(
                "validation.field_type",
                "archived must be a boolean.",
            ));
        }
        self.with_tx(|c| {
            let mut v = record(c, "work_lanes", id)?;
            ensure_project_writable(c, v["project"].as_str().unwrap())?;
            for key in ["title", "purpose", "context", "archived"] {
                if let Some(x) = patch.get(key) {
                    v[key] = x.clone();
                }
            }
            save_lane(c, &mut v, actor)?;
            detail(c, id)
        })
    }
    pub fn work_lane_ticket(
        &self,
        id: &str,
        ticket: &str,
        remove: bool,
        actor: &str,
    ) -> ApiResult<Value> {
        self.with_tx(|c| {
            let mut v = record(c, "work_lanes", id)?;
            writable(c, &v)?;
            let t = get_ticket_required(c, ticket)?;
            if v["project"] != t.project {
                return Err(conflict("Ticket and lane must belong to the same project."));
            }
            if remove {
                c.execute("DELETE FROM work_lane_tickets WHERE lane=?1 AND ticket=?2", params![id, ticket])?;
            } else {
                let count: i64 = c.query_row("SELECT COUNT(*) FROM work_lane_tickets WHERE lane=?1", [id], |r| r.get(0))?;
                if count >= 200
                    && !c.query_row("SELECT EXISTS(SELECT 1 FROM work_lane_tickets WHERE lane=?1 AND ticket=?2)", params![id, ticket], |r| {
                        r.get::<_, bool>(0)
                    })?
                {
                    return Err(conflict("Lane has 200 tickets. Remove completed work before adding more."));
                }
                c.execute("INSERT OR IGNORE INTO work_lane_tickets VALUES(?1,?2)", params![id, ticket])?;
            }
            save_lane(c, &mut v, actor)?;
            detail(c, id)
        })
    }
    pub fn work_handoff_get(&self, id: &str) -> ApiResult<Value> {
        self.with_conn(|c| record(c, "work_handoffs", id))
    }
    pub fn work_handoff_list(
        &self,
        project: &str,
        lane: Option<&str>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> ApiResult<(Vec<Value>, i64)> {
        self.with_conn(|c| {
            let limit = limit.clamp(1, 200);
            let offset = offset.max(0);
            let predicate = "project=?1 AND (?2 IS NULL OR lane=?2) AND (?3 IS NULL OR status=?3 OR (?3='ready' AND (status='queued' OR (status='running' AND lease_until<=?4))))";
            let total = c.query_row(&format!("SELECT COUNT(*) FROM work_handoffs WHERE {predicate}"), params![project, lane, status, now_ms()], |r| r.get(0))?;
            let order = if lane.is_some() { "created_at DESC,id DESC" } else { "created_at,id" };
            let mut s = c.prepare(&format!("SELECT data FROM work_handoffs WHERE {predicate} ORDER BY {order} LIMIT ?5 OFFSET ?6"))?;
            let raws = s
                .query_map(params![project, lane, status, now_ms(), limit, offset], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                raws.iter()
                    .map(|r| serde_json::from_str(r).map_err(|e| ApiError::internal(format!("Stored handoff JSON is invalid: {e}"))))
                    .collect::<ApiResult<Vec<_>>>()?,
                total,
            ))
        })
    }
    pub fn work_handoff_create(
        &self,
        lane_id: &str,
        body: &Value,
        actor: &str,
    ) -> ApiResult<Value> {
        validate_draft(body)?;
        self.with_tx(|c| {
            let mut lane = record(c, "work_lanes", lane_id)?;
            writable(c, &lane)?;
            let selected = body["ticket_ids"].as_array().unwrap();
            let all = tickets(c, lane_id)?;
            let mut scope = Vec::new();
            for id in selected {
                let t = all
                    .iter()
                    .find(|t| t["id"] == *id)
                    .ok_or_else(|| conflict("Every selected ticket must belong to this lane. Attach it before creating the handoff."))?;
                scope.push(t.clone());
            }
            if body["kind"] == "review" {
                let parent = record(c, "work_handoffs", body["parent_handoff"].as_str().unwrap())?;
                if parent["lane"] != lane_id || parent["kind"] != "implementation" || parent["status"] != "completed" || parent["revision"] != body["target_revision"] {
                    return Err(conflict("Review must reference a completed implementation in this lane and its exact result revision."));
                }
            }
            let id = format!("ho-{}", ticket_suffix(12));
            let now = now_ms();
            let v = json!({
            "id":id,
            "project":lane["project"],
            "lane":lane_id,
            "kind":body["kind"],
            "provider":body["provider"],
            "instructions":body["instructions"],
            "ticket_ids":selected,
            "snapshot":{
            "lane":lane,
            "tickets":scope}
            ,
            "target_revision":body.get("target_revision"),
            "parent_handoff":body.get("parent_handoff"),
            "status":"draft",
            "attempt":0,
            "lease_expires_at":null,
            "result":null,
            "revision":null,
            "conversation_ref":null,
            "created_at":iso(now),
            "updated_at":iso(now)}
            );
            c.execute(
                "INSERT INTO work_handoffs(id,project,lane,status,data,created_at) VALUES(?1,?2,?3,'draft',?4,?5)",
                params![id, v["project"].as_str(), lane_id, v.to_string(), now],
            )?;
            emit_event(
                c,
                None,
                v["project"].as_str(),
                actor,
                "handoff.created",
                json!({
                "handoff":id,
                "lane":lane_id}
                ),
                now,
            )?;
            save_lane(c, &mut lane, actor)?;
            Ok(v)
        })
    }
    pub fn work_handoff_action(
        &self,
        id: &str,
        action: &str,
        body: &Value,
        actor: &str,
        token: &str,
    ) -> ApiResult<Value> {
        if action == "heartbeat" || action == "result" {
            fields(
                body,
                if action == "heartbeat" {
                    &["attempt"][..]
                } else {
                    &[
                        "attempt",
                        "status",
                        "result",
                        "revision",
                        "conversation_ref",
                    ][..]
                },
            )?;
            if body["attempt"].as_i64().is_none_or(|n| n < 1) {
                return Err(ApiError::bad_request(
                    "validation.field_type",
                    "attempt must be the positive integer returned by claim.",
                ));
            }
            if action == "result" {
                if ![Some("completed"), Some("failed")].contains(&body["status"].as_str()) {
                    return Err(ApiError::bad_request(
                        "validation.field_type",
                        "Result status must be completed or failed.",
                    ));
                }
                text_field(body, "result", true, 64000)?;
                text_field(body, "revision", false, 200)?;
                text_field(body, "conversation_ref", false, 2000)?;
            }
        }
        self.with_tx(|c| {
            let mut v = record(c, "work_handoffs", id)?;
            let mut lane = record(c, "work_lanes", v["lane"].as_str().unwrap())?;
            ensure_project_writable(c, v["project"].as_str().unwrap())?;
            let now = now_ms();
            let (lease, owner, owner_actor): (Option<i64>, Option<String>, Option<String>) = c.query_row("SELECT lease_until,owner_token,owner_actor FROM work_handoffs WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            let status = v["status"].as_str().unwrap().to_string();
            let mut new_lease = None;
            match action {
                "dispatch" => {
                    writable(c, &lane)?;
                    if status != "draft" {
                        return Err(conflict("Only a draft handoff can be dispatched. Create a new handoff for another attempt."));
                    }
                    v["status"] = json!("queued");
                }
                "cancel" => {
                    if !["draft", "queued", "running"].contains(&status.as_str()) {
                        return Err(conflict("This handoff is already finished."));
                    }
                    v["status"] = json!("cancelled");
                }
                "claim" => {
                    writable(c, &lane)?;
                    if status != "queued" && !(status == "running" && lease.is_some_and(|l| l <= now)) {
                        return Err(conflict("Handoff is not available. Claim a queued assignment or one whose worker lease expired."));
                    }
                    v["status"] = json!("running");
                    v["attempt"] = json!(v["attempt"].as_i64().unwrap() + 1);
                    new_lease = Some(now + 120_000);
                }
                "heartbeat" | "result" => {
                    if status != "running" || lease.is_none_or(|l| l <= now) || owner.as_deref() != Some(token) || owner_actor.as_deref() != Some(actor) || body["attempt"] != v["attempt"] {
                        return Err(conflict("Worker lease or attempt is no longer current. Stop this execution; do not submit stale results."));
                    }
                    if action == "heartbeat" {
                        new_lease = Some(now + 120_000);
                    } else {
                        v["status"] = body["status"].clone();
                        v["result"] = body["result"].clone();
                        v["revision"] = body.get("revision").cloned().unwrap_or(Value::Null);
                        v["conversation_ref"] = body.get("conversation_ref").cloned().unwrap_or(Value::Null);
                        if v["status"] == "completed" && v["kind"] == "preparation" {
                            let unchanged = ["context", "title", "purpose"].iter().all(|k| lane[k] == v["snapshot"]["lane"][k]);
                            v["context_applied"] = json!(unchanged);
                            if unchanged {
                                lane["context"] = v["result"].clone();
                                lane["conversation_ref"] = v["conversation_ref"].clone();
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
            v["updated_at"] = json!(iso(now));
            v["lease_expires_at"] = json!(new_lease.map(iso));
            c.execute(
                "UPDATE work_handoffs SET status=?1,lease_until=?2,owner_token=?3,owner_actor=?4,data=?5 WHERE id=?6",
                params![
                    v["status"].as_str(),
                    new_lease,
                    if new_lease.is_some() { Some(token) } else { None },
                    if new_lease.is_some() { Some(actor) } else { None },
                    v.to_string(),
                    id
                ],
            )?;
            if action != "heartbeat" {
                emit_event(
                    c,
                    None,
                    v["project"].as_str(),
                    actor,
                    "handoff.updated",
                    json!({
                    "handoff":id,
                    "lane":v["lane"],
                    "status":v["status"]}
                    ),
                    now,
                )?;
                save_lane(c, &mut lane, actor)?;
            }
            Ok(v)
        })
    }
}
