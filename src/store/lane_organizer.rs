//! Human-reviewed organization proposals. Workers cannot mutate lane membership.
use super::{
    agent_chat::{bounded, SendMessage},
    helpers::{emit_event, ensure_project_writable, get_ticket_required},
    Store,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{iso, now_ms, sha256_hex, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;
use yrs::{updates::decoder::Decode, Doc, Transact, Update};
fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::validation("validation.lane_organizer", message)
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.lane_organizer", message)
}
fn parse(raw: &str) -> ApiResult<Value> {
    serde_json::from_str(raw).map_err(|e| ApiError::internal(e.to_string()))
}
fn human(ctx: &AuthCtx, project: &str) -> ApiResult<()> {
    ctx.require_scope("write")?;
    ctx.require_scope("human")?;
    ctx.require_project(project)
}
fn snapshot(c: &Connection, project: &str) -> ApiResult<Value> {
    let mut s=c.prepare("SELECT t.id FROM tickets t JOIN workflow_states w ON w.project=t.project AND w.state=t.state WHERE t.project=?1 AND t.archived_at IS NULL AND t.type<>'epic' AND w.terminal=0 AND NOT EXISTS(SELECT 1 FROM work_lane_tickets m JOIN work_lanes l ON l.id=m.lane WHERE m.ticket=t.id AND json_extract(l.data,'$.archived')=0) ORDER BY t.id LIMIT 201")?;
    let ids = s
        .query_map([project], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.len() > 200 {
        return Err(invalid(
            "More than 200 pending tickets. Organize some manually before requesting a proposal; no partial collection was sent.",
        ));
    }
    let mut tickets = Vec::new();
    for id in ids {
        let t = get_ticket_required(c, &id)?;
        tickets.push(json!({"id":t.id,"title":t.title,"body":t.body,"state":t.state,"state_category":t.state_category,"type":t.ty,"parent":t.parent,"priority":t.priority,"labels":t.labels,"tags":t.tags,"links":t.links,"metadata":t.metadata,"version":t.version}));
    }
    let mut s = c.prepare("SELECT data FROM work_lanes WHERE project=?1 AND json_extract(data,'$.archived')=0 ORDER BY id LIMIT 101")?;
    let raws = s
        .query_map([project], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if raws.len() > 100 {
        return Err(invalid(
            "More than 100 active lanes. Archive unused lanes before requesting a proposal; no partial collection was sent.",
        ));
    }
    let mut lanes = Vec::new();
    for raw in raws {
        let mut lane = parse(&raw)?;
        let mut s =
            c.prepare("SELECT ticket FROM work_lane_tickets WHERE lane=?1 ORDER BY ticket")?;
        lane["tickets"] = json!(s
            .query_map([lane["id"].as_str().unwrap()], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?);
        let mut members = Vec::new();
        for id in lane["tickets"].as_array().unwrap() {
            let t = get_ticket_required(c, id.as_str().unwrap())?;
            members.push(json!({"id":t.id,"title":t.title,"body":t.body,"state":t.state,"version":t.version}));
        }
        lane["existing_tickets"] = json!(members);
        lanes.push(lane);
    }
    // Read the persisted CRDT in the same transaction as tickets. Include its
    // sequence in the fingerprint so edits cannot be accepted against old prose.
    let mut s = c.prepare("SELECT id,title FROM mindmaps WHERE project=?1 ORDER BY id LIMIT 21")?;
    let maps = s
        .query_map([project], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if maps.len() > 20 {
        return Err(invalid("More than 20 specification documents. This organizer version cannot snapshot this project without omissions."));
    }
    let mut specifications = Vec::new();
    for (id, title) in maps {
        let size: i64 = c.query_row(
            "SELECT COALESCE(SUM(length(blob)),0) FROM crdt_updates WHERE object_id=?1",
            [&id],
            |r| r.get(0),
        )?;
        if size > 8_000_000 {
            return Err(invalid(
                "A specification history exceeds 8 MB. Compact the document before organizing; no partial specification was sent.",
            ));
        }
        let doc = Doc::new();
        let mut s = c.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
        for blob in s.query_map([&id], |r| r.get::<_, Vec<u8>>(0))? {
            let update =
                Update::decode_v1(&blob?).map_err(|e| ApiError::internal(e.to_string()))?;
            doc.transact_mut()
                .apply_update(update)
                .map_err(|e| ApiError::internal(e.to_string()))?;
        }
        let (_, _, nodes) = super::mindmapdoc::snapshot(&doc, &id);
        let sections = super::mindmapdoc::tree_order(&nodes)
            .into_iter()
            .map(|n| json!({"id":n.id,"title":n.title,"body":n.notes,"parent":n.parent}))
            .collect::<Vec<_>>();
        let seq: i64 = c.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM crdt_updates WHERE object_id=?1",
            [&id],
            |r| r.get(0),
        )?;
        specifications
            .push(json!({"id":id,"title":title,"sections":sections,"persisted_sequence":seq}));
    }
    let out = json!({"project":project,"tickets":tickets,"lanes":lanes,"specifications":specifications,"specification_source":"Persisted specification text; unsaved editor changes are not included.","omitted":false});
    if out.to_string().len() > 512_000 {
        return Err(invalid(
            "Project context exceeds 512000 bytes. Reduce pending work or specification scope before requesting a proposal; nothing was omitted silently.",
        ));
    }
    Ok(out)
}
fn view(c: &Connection, project: &str) -> ApiResult<Value> {
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
        [project],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(ApiError::not_found("project", project));
    }
    let cid: Option<String> = c
        .query_row(
            "SELECT conversation FROM lane_organizer_conversations WHERE project=?1",
            [project],
            |r| r.get(0),
        )
        .optional()?;
    let Some(cid) = cid else {
        return Ok(json!({"conversation_id":null,"messages":[],"jobs":[],"total":0,"limit":20}));
    };
    let total: i64 = c.query_row(
        "SELECT COUNT(*) FROM agent_jobs WHERE conversation_id=?1",
        [&cid],
        |r| r.get(0),
    )?;
    let mut s=c.prepare("SELECT j.id,CASE WHEN j.status='running' AND (j.lease_expires_at<=?2 OR j.deadline<=?2) THEN 'failed' ELSE j.status END,COALESCE(j.error,CASE WHEN j.status='running' AND (j.lease_expires_at<=?2 OR j.deadline<=?2) THEN 'The worker lease expired. Send a new organizer request.' END),j.created_at,j.snapshot,j.source_revision,o.proposal,o.accepted_at FROM agent_jobs j JOIN lane_organizer_jobs o ON o.job=j.id WHERE conversation_id=?1 ORDER BY j.rowid DESC LIMIT 20")?;
    let rows = s
        .query_map(params![cid, now_ms()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut jobs = Vec::new();
    for (id, status, error, created, snap, revision, proposal, accepted) in rows {
        jobs.push(json!({"id":id,"status":status,"error":error,"created_at":iso(created),"snapshot":parse(&snap)?,"source_revision":revision,"proposal":proposal.map(|p|parse(&p)).transpose()?,"accepted_at":accepted.map(iso)}));
    }
    let mut s = c.prepare("SELECT id,role,body,created_at FROM (SELECT rowid,id,role,body,created_at FROM agent_messages WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT 40) ORDER BY rowid")?;
    let messages = s
        .query_map([&cid], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"role":r.get::<_,String>(1)?,"body":r.get::<_,String>(2)?,"created_at":iso(r.get(3)?)}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out =
        json!({"conversation_id":cid,"messages":messages,"jobs":jobs,"total":total,"limit":20});
    if total > 20 {
        out["note"] = json!("Showing the latest 20 requests and 40 conversation messages. Older jobs remain available in the agent queue inspector.");
    }
    Ok(out)
}
fn text<'a>(v: &'a Value, key: &str, max: usize, empty: bool) -> ApiResult<&'a str> {
    let s = v[key]
        .as_str()
        .ok_or_else(|| invalid(format!("{key} must be a string.")))?;
    if s.len() > max || (!empty && s.trim().is_empty()) {
        return Err(invalid(format!(
            "{key} must fit {max} bytes and {}.",
            if empty {
                "may be empty"
            } else {
                "must contain text"
            }
        )));
    }
    Ok(s)
}
fn fields(v: &Value, allowed: &[&str]) -> ApiResult<()> {
    crate::api::reject_unknown(crate::api::body_object(v)?, allowed)
}
pub(super) fn validate_proposal(snap: &Value, p: &Value) -> ApiResult<()> {
    if p.to_string().len() > 256_000 {
        return Err(invalid("Proposal exceeds 256000 bytes."));
    }
    fields(p, &["groups", "unassigned"])?;
    let groups = p["groups"]
        .as_array()
        .ok_or_else(|| invalid("groups must be an array."))?;
    let unassigned = p["unassigned"]
        .as_array()
        .ok_or_else(|| invalid("unassigned must be an array."))?;
    let pending: HashSet<&str> = snap["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    let mut seen = HashSet::new();
    let mut lane_ids = HashSet::new();
    if groups.len() > 200 || unassigned.len() > 200 {
        return Err(invalid(
            "At most 200 groups or unassigned tickets are allowed.",
        ));
    }
    for group in groups {
        fields(
            group,
            &[
                "lane_id",
                "title",
                "purpose",
                "context",
                "readiness",
                "reason",
                "ticket_ids",
            ],
        )?;
        text(group, "title", 200, false)?;
        text(group, "purpose", 8000, true)?;
        text(group, "context", 64000, true)?;
        text(group, "reason", 4000, false)?;
        if ![Some("ready"), Some("needs_clarification")].contains(&group["readiness"].as_str()) {
            return Err(invalid(
                "readiness must be ready or needs_clarification (advisory only).",
            ));
        }
        match group.get("lane_id") {
            Some(Value::Null) => {}
            Some(Value::String(id)) => {
                if !lane_ids.insert(id) {
                    return Err(invalid("An existing lane may appear only once."));
                }
                let lane = snap["lanes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|l| l["id"] == *id)
                    .ok_or_else(|| invalid("Proposed lane is outside this project snapshot."))?;
                if lane["title"] != group["title"] || lane["purpose"] != group["purpose"] {
                    return Err(invalid(
                        "An existing lane's title and purpose must remain unchanged.",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    "lane_id must be null for a new lane or an existing snapshot lane ID.",
                ))
            }
        }
        let ids = group["ticket_ids"]
            .as_array()
            .ok_or_else(|| invalid("ticket_ids must be an array."))?;
        if ids.is_empty() || ids.len() > 200 {
            return Err(invalid("Each group must select 1–200 pending tickets."));
        }
        for id in ids {
            let id = id
                .as_str()
                .ok_or_else(|| invalid("Ticket IDs must be strings."))?;
            if !pending.contains(id) || !seen.insert(id) {
                return Err(invalid("Every selected ticket must occur exactly once and belong to the pending snapshot."));
            }
        }
    }
    for item in unassigned {
        fields(item, &["ticket_id", "reason"])?;
        let id = text(item, "ticket_id", 200, false)?;
        text(item, "reason", 4000, false)?;
        if !pending.contains(id) || !seen.insert(id) {
            return Err(invalid(
                "Unassigned tickets must be unique pending snapshot tickets.",
            ));
        }
    }
    if seen != pending {
        return Err(invalid(
            "Every pending ticket must appear once in a group or unassigned with a reason.",
        ));
    }
    Ok(())
}
pub(super) fn save_proposal(
    c: &Connection,
    jid: &str,
    proposal: Option<&Value>,
    completed: bool,
) -> ApiResult<()> {
    let organizer: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM lane_organizer_jobs WHERE job=?1)",
        [jid],
        |r| r.get(0),
    )?;
    if !organizer {
        if proposal.is_some() {
            return Err(invalid("Only lane organizer jobs accept a proposal."));
        }
        return Ok(());
    }
    if completed {
        let p = proposal
            .ok_or_else(|| invalid("Completed organizer jobs require a structured proposal."))?;
        let raw: String =
            c.query_row("SELECT snapshot FROM agent_jobs WHERE id=?1", [jid], |r| {
                r.get(0)
            })?;
        validate_proposal(&parse(&raw)?, p)?;
        c.execute(
            "UPDATE lane_organizer_jobs SET proposal=?2 WHERE job=?1",
            params![jid, p.to_string()],
        )?;
    } else if proposal.is_some() {
        return Err(invalid(
            "Failed jobs cannot publish an organization proposal.",
        ));
    }
    Ok(())
}
impl Store {
    pub fn lane_organizer_view(&self, ctx: &AuthCtx, project: &str) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        ctx.require_project(project)?;
        self.with_conn(|c| view(c, project))
    }
    pub fn lane_organizer_send(
        &self,
        ctx: &AuthCtx,
        project: &str,
        req: &SendMessage,
    ) -> ApiResult<Value> {
        human(ctx, project)?;
        bounded(&req.message, 8000, "message")?;
        bounded(&req.request_id, 120, "request_id")?;
        self.with_tx(|c| {
            ensure_project_writable(c, project)?;
            super::agent_chat::expire(c)?;
            let now = now_ms();
            let cid = format!("ac-{}", ticket_suffix(20));
            let existing: Option<String> = c
                .query_row("SELECT conversation FROM lane_organizer_conversations WHERE project=?1", [project], |r| r.get(0))
                .optional()?;
            let cid = if let Some(id) = existing {
                id
            } else {
                c.execute(
                    "INSERT INTO agent_conversations(id,mindmap,node,project,created_at) VALUES(?1,NULL,'lane_organizer',?2,?3)",
                    params![cid, project, now],
                )?;
                c.execute("INSERT INTO lane_organizer_conversations VALUES(?1,?2)", params![project, cid])?;
                cid
            };
            let prior: Option<String> = c
                .query_row(
                    "SELECT prompt FROM agent_jobs WHERE conversation_id=?1 AND requested_by=?2 AND request_id=?3",
                    params![cid, ctx.actor, req.request_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                if prior != req.message {
                    return Err(conflict("request_id already names different instructions. Use a new request_id."));
                }
                return view(c, project);
            }
            let busy: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM agent_jobs WHERE conversation_id=?1 AND status IN ('queued','running'))", [&cid], |r| {
                r.get(0)
            })?;
            if busy {
                return Err(conflict("An organizer request is already active. Wait for its result before sending another."));
            }
            let snap = snapshot(c, project)?;
            if snap["tickets"].as_array().unwrap().is_empty() {
                return Err(invalid("No pending tickets: all live nonterminal tickets already belong to lanes."));
            }
            let raw = snap.to_string();
            let revision = sha256_hex(raw.as_bytes());
            let jid = format!("aj-{}", ticket_suffix(20));
            c.execute(
                "INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",
                params![jid, cid, ctx.actor, req.request_id, req.message, raw, revision, now],
            )?;
            c.execute("INSERT INTO lane_organizer_jobs(job) VALUES(?1)", [&jid])?;
            c.execute(
                "INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'user',?4,?5)",
                params![format!("am-{}", ticket_suffix(20)), cid, jid, req.message, now],
            )?;
            emit_event(c, None, Some(project), &ctx.actor, "agent_job.queued", json!({"job_id":jid,"conversation_id":cid}), now)?;
            view(c, project)
        })
    }
    pub fn lane_organizer_accept(
        &self,
        ctx: &AuthCtx,
        project: &str,
        jid: &str,
    ) -> ApiResult<Value> {
        human(ctx, project)?;
        self.with_tx(|c| {
            ensure_project_writable(c, project)?;
            let row: Option<(String, String, Option<String>, Option<i64>)> = c.query_row(
                "SELECT j.status,j.source_revision,o.proposal,o.accepted_at
                 FROM lane_organizer_jobs o JOIN agent_jobs j ON j.id=o.job
                 JOIN agent_conversations a ON a.id=j.conversation_id
                 WHERE o.job=?1 AND a.project=?2",
                params![jid, project],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).optional()?;
            let (status, revision, proposal, accepted) = row
                .ok_or_else(|| ApiError::not_found("agent_job", jid))?;
            if accepted.is_some() {
                return view(c, project);
            }
            if status != "completed" {
                return Err(conflict("Only a completed organizer proposal can be accepted."));
            }
            let snap = snapshot(c, project)?;
            if sha256_hex(snap.to_string().as_bytes()) != revision {
                return Err(conflict(
                    "Tickets, lane context, or specifications changed since this proposal. Request a fresh proposal before accepting.",
                ));
            }
            let proposal = parse(&proposal.ok_or_else(|| conflict("This organizer job has no proposal."))?)?;
            validate_proposal(&snap, &proposal)?;
            let now = now_ms();
            let lane_count: i64 = c.query_row(
                "SELECT COUNT(*) FROM work_lanes WHERE project=?1", [project], |r| r.get(0),
            )?;
            let groups = proposal["groups"].as_array().unwrap();
            let additions = groups.iter().filter(|g| g["lane_id"].is_null()).count() as i64;
            if lane_count + additions > 500 {
                return Err(invalid("Accepted proposal would exceed the project limit of 500 lanes. Reuse existing lanes."));
            }
            for group in groups {
                let is_new = group["lane_id"].is_null();
                let id = group["lane_id"].as_str().map(String::from)
                    .unwrap_or_else(|| format!("wl-{}", ticket_suffix(12)));
                let mut lane = if is_new {
                    json!({
                        "id": id, "project": project, "title": group["title"],
                        "purpose": group["purpose"], "context": "", "conversation_ref": null,
                        "archived": false, "created_at": iso(now), "updated_at": iso(now)
                    })
                } else {
                    snap["lanes"].as_array().unwrap().iter()
                        .find(|l| l["id"] == id).unwrap().clone()
                };
                // Snapshot-only member details must never become stored lane context.
                lane.as_object_mut().unwrap().remove("tickets");
                lane.as_object_mut().unwrap().remove("existing_tickets");
                lane["context"] = group["context"].clone();
                lane["readiness"] = json!({
                    "status": group["readiness"], "reason": group["reason"], "organizer_job": jid
                });
                lane["updated_at"] = json!(iso(now));
                let count: i64 = c.query_row(
                    "SELECT COUNT(*) FROM work_lane_tickets WHERE lane=?1", [&id], |r| r.get(0),
                )?;
                if count + group["ticket_ids"].as_array().unwrap().len() as i64 > 200 {
                    return Err(invalid("Accepted group would exceed 200 tickets in a lane. Request smaller groups."));
                }
                c.execute(
                    "INSERT INTO work_lanes(id,project,data,updated_at) VALUES(?1,?2,?3,?4)
                     ON CONFLICT(id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at",
                    params![id, project, lane.to_string(), now],
                )?;
                for ticket in group["ticket_ids"].as_array().unwrap() {
                    c.execute("INSERT INTO work_lane_tickets(lane,ticket) VALUES(?1,?2)",
                        params![id, ticket.as_str()])?;
                }
                emit_event(c, None, Some(project), &ctx.actor,
                    if is_new { "lane.created" } else { "lane.updated" },
                    json!({"lane": id, "organizer_job": jid}), now)?;
            }
            c.execute("UPDATE lane_organizer_jobs SET accepted_at=?2,accepted_by=?3 WHERE job=?1",
                params![jid, now, ctx.actor])?;
            emit_event(c, None, Some(project), &ctx.actor, "lane.organization_accepted",
                json!({"job_id": jid, "groups": groups.len()}), now)?;
            view(c, project)
        })
    }
}
