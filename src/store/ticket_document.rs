//! Typed, reviewed document associations; classification never changes hierarchy.
use super::{
    helpers::{emit_event, ensure_project_writable, get_ticket_required},
    Store,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, sha256_hex, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;
use yrs::{updates::decoder::Decode, Doc, GetString, Transact, Update};
pub const KIND: &str = "ticket_document_classify";
fn invalid(s: impl Into<String>) -> ApiError {
    ApiError::validation("validation.ticket_document", s)
}
fn conflict(s: &str) -> ApiError {
    ApiError::conflict("conflict.ticket_document", s)
}
fn parse(s: &str) -> ApiResult<Value> {
    serde_json::from_str(s).map_err(|e| ApiError::internal(e.to_string()))
}
fn human(ctx: &AuthCtx, project: &str) -> ApiResult<()> {
    ctx.require_scope("human")?;
    ctx.require_scope("write")?;
    ctx.require_project(project)
}
pub fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn terminal(c: &Connection, project: &str, state: &str) -> ApiResult<bool> {
    Ok(c.query_row(
        "SELECT terminal FROM workflow_states WHERE project=?1 AND state=?2",
        params![project, state],
        |r| r.get(0),
    )?)
}
fn revision(t: &super::model::Ticket) -> String {
    sha256_hex(
        json!({"id":t.id,"project":t.project,"title":t.title,"body":t.body,"parent":t.parent})
            .to_string()
            .as_bytes(),
    )
}
pub fn document_from_doc(doc: &Doc, map: &str, title: &str) -> ApiResult<Value> {
    let (_, _, nodes) = super::mindmapdoc::snapshot(doc, map);
    let sections:Vec<_>=super::mindmapdoc::tree_order(&nodes).into_iter().map(|n|{
  let xml=super::mindmapdoc::read_section_prose(doc,&n.id).map(|f|f.get_string(&doc.transact())).unwrap_or_default();
  let mut s=json!({"id":n.id,"parent_id":n.parent,"title":n.title,"notes":n.notes,"prose_xml":xml});
  s["version"]=json!(sha256_hex(s.to_string().as_bytes()));s["promoted_kind"]=json!(n.promoted_kind);s["promoted_id"]=json!(n.promoted_id);s
 }).collect();
    let v = json!({"kind":"document_workspace","schema_version":2,"mindmap_id":map,"title":title,"action":"discuss","context":{"mode":"automatic","section_ids":[],"pinned_section_ids":[],"quote":null},"sections":sections});
    Ok(v)
}
fn document(c: &Connection, project: &str) -> ApiResult<Option<Value>> {
    let map: Option<(String, String)> = c
        .query_row(
            "SELECT id,title FROM mindmaps WHERE project=?1 ORDER BY id LIMIT 1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, title)) = map else {
        return Ok(None);
    };
    let doc = Doc::new();
    let mut stmt = c.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
    for b in stmt.query_map([&id], |r| r.get::<_, Vec<u8>>(0))? {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&b?).map_err(|e| ApiError::internal(e.to_string()))?)
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(Some(document_from_doc(&doc, &id, &title)?))
}
fn section<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    doc["sections"].as_array()?.iter().find(|s| s["id"] == id)
}
fn accepted(c: &Connection, ticket: &str, project: &str, doc: &Value) -> ApiResult<bool> {
    let mut s=c.prepare("SELECT section_id FROM ticket_document_links WHERE ticket=?1 AND project=?2 AND mindmap=?3 AND state='accepted'")?;
    let ids = s
        .query_map(
            params![ticket, project, doc["mindmap_id"].as_str().unwrap()],
            |r| r.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids.iter().any(|id| section(doc, id).is_some()))
}
fn save_state(
    c: &Connection,
    ticket: &str,
    rev: &str,
    status: &str,
    error: Option<&str>,
) -> ApiResult<()> {
    c.execute("INSERT INTO ticket_document_state(ticket,revision,status,error) VALUES(?1,?2,?3,?4) ON CONFLICT(ticket) DO UPDATE SET revision=excluded.revision,status=excluded.status,error=excluded.error",params![ticket,rev,status,error])?;
    Ok(())
}
struct NewLink<'a> {
    relation: &'a str,
    provenance: &'a str,
    state: &'a str,
    primary: bool,
    reason: &'a str,
    quote: &'a str,
    actor: &'a str,
    job: Option<&'a str>,
}
fn insert_link(
    c: &Connection,
    t: &super::model::Ticket,
    doc: &Value,
    s: &Value,
    link: NewLink<'_>,
) -> ApiResult<String> {
    let NewLink {
        relation,
        provenance,
        state,
        primary,
        reason,
        quote,
        actor,
        job,
    } = link;
    let id = format!("tdl-{}", ticket_suffix(16));
    let now = now_ms();
    if primary {
        c.execute(
            "UPDATE ticket_document_links SET is_primary=0 WHERE ticket=?1 AND is_primary=1",
            [&t.id],
        )?;
    }
    c.execute("INSERT INTO ticket_document_links(id,ticket,project,mindmap,section_id,title,section_version,relation,provenance,state,is_primary,reason,quote,ticket_revision,job,created_by,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?17)",params![id,t.id,t.project,doc["mindmap_id"].as_str().unwrap(),s["id"].as_str().unwrap(),s["title"].as_str().unwrap(),s["version"].as_str().unwrap(),relation,provenance,state,primary,reason,quote,revision(t),job,actor,now])?;
    if provenance == "manual" {
        c.execute(
            "UPDATE ticket_document_links SET reviewed_by=?2,reviewed_at=?3 WHERE id=?1",
            params![id, actor, now],
        )?;
    }
    emit_event(
        c,
        Some(&t.id),
        Some(&t.project),
        actor,
        "ticket.document_linked",
        json!({"link":id,"section_id":s["id"],"state":state,"provenance":provenance}),
        now,
    )?;
    Ok(id)
}
/// Only trusted creation paths may establish historical origin.
pub(super) fn direct(
    c: &Connection,
    ticket: &str,
    doc: &Value,
    node: &str,
    actor: &str,
) -> ApiResult<()> {
    let t = get_ticket_required(c, ticket)?;
    let same: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM mindmaps WHERE id=?1 AND project=?2)",
        params![doc["mindmap_id"].as_str().unwrap_or(""), t.project],
        |r| r.get(0),
    )?;
    if !same {
        return Err(invalid("Direct source must belong to the ticket project"));
    }
    let s = section(doc, node).ok_or_else(|| invalid("Direct source section missing"))?;
    insert_link(
        c,
        &t,
        doc,
        s,
        NewLink {
            relation: "source",
            provenance: "direct",
            state: "accepted",
            primary: true,
            reason: "Created directly from this document section",
            quote: "",
            actor,
            job: None,
        },
    )?;
    Ok(())
}
fn links(c: &Connection, t: &super::model::Ticket, doc: Option<&Value>) -> ApiResult<Vec<Value>> {
    let mut stmt=c.prepare("SELECT id,project,mindmap,section_id,title,section_version,relation,provenance,state,is_primary,reason,quote,created_by,created_at,updated_at,ticket_revision,reviewed_by,reviewed_at,(SELECT d.document_revision FROM ticket_document_jobs d WHERE d.job=ticket_document_links.job) FROM ticket_document_links WHERE ticket=?1 ORDER BY created_at,id")?;
    let mut rows=stmt.query_map([&t.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"ticket":t.id,"project":r.get::<_,String>(1)?,"mindmap":r.get::<_,String>(2)?,"section_id":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,"section_version":r.get::<_,String>(5)?,"relation":r.get::<_,String>(6)?,"provenance":r.get::<_,String>(7)?,"state":r.get::<_,String>(8)?,"primary":r.get::<_,bool>(9)?,"reason":r.get::<_,String>(10)?,"quote":r.get::<_,String>(11)?,"created_by":r.get::<_,String>(12)?,"created_at":r.get::<_,i64>(13)?,"updated_at":r.get::<_,i64>(14)?,"ticket_revision":r.get::<_,String>(15)?,"reviewed_by":r.get::<_,Option<String>>(16)?,"reviewed_at":r.get::<_,Option<i64>>(17)?,"classification_revision":r.get::<_,Option<String>>(18)?})))?.collect::<Result<Vec<_>,_>>()?;
    let doc_revision = doc.map(|d| sha256_hex(d.to_string().as_bytes()));
    for link in &mut rows {
        link["captured_title"] = link["title"].clone();
        let same = link["project"] == t.project;
        let source = doc
            .filter(|d| same && d["mindmap_id"] == link["mindmap"])
            .and_then(|d| section(d, link["section_id"].as_str().unwrap()));
        link["missing"] = json!(source.is_none());
        link["stale"] = json!(
            source.is_none_or(|s| s["version"] != link["section_version"])
                || link["ticket_revision"] != revision(t)
        );
        if let Some(captured) = link["classification_revision"].as_str() {
            if doc_revision.as_deref() != Some(captured) {
                link["stale"] = json!(true);
            }
        }
        link.as_object_mut()
            .unwrap()
            .remove("classification_revision");
        if let Some(s) = source {
            link["title"] = s["title"].clone();
        }
        if !same {
            link["title"] = json!("Unavailable section");
            link["captured_title"] = json!("Unavailable section");
            link["quote"] = json!("");
            link["reason"] = json!("This reference belongs to the ticket's previous project");
        }
    }
    Ok(rows)
}
fn view(c: &Connection, ticket: &str, doc: Option<&Value>) -> ApiResult<Value> {
    let t = get_ticket_required(c, ticket)?;
    let entries = links(c, &t, doc)?;
    let state: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT status,error FROM ticket_document_state WHERE ticket=?1 AND revision=?2",
            params![ticket, revision(&t)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    type JobStatus = (String, String, Option<String>, Option<String>, bool);
    let job:Option<JobStatus>=c.query_row("SELECT j.id,j.status,j.error,d.proposal,d.stale FROM ticket_document_jobs d JOIN agent_jobs j ON j.id=d.job WHERE d.ticket=?1 AND j.conversation_id IN (SELECT id FROM agent_conversations WHERE project=?2) ORDER BY j.rowid DESC LIMIT 1",params![ticket,t.project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let pending: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM ticket_document_pending WHERE ticket=?1)",
        [ticket],
        |r| r.get(0),
    )?;
    let valid_link = doc
        .map(|d| accepted(c, ticket, &t.project, d))
        .transpose()?
        .unwrap_or(false);
    let active = job
        .as_ref()
        .is_some_and(|(_, status, _, _, _)| status == "queued" || status == "running");
    let classification = if active {
        let (id, status, error, _, _) = job.as_ref().unwrap();
        json!({"job_id":id,"status":status,"error":error})
    } else if valid_link {
        json!({"status":"linked"})
    } else if pending {
        json!({"status":"queued"})
    } else if state
        .as_ref()
        .is_some_and(|(status, _)| status == "unavailable")
    {
        json!({"status":"unavailable","error":state.as_ref().unwrap().1})
    } else if let Some((id, status, error, proposal, stale)) = job {
        let p = proposal
            .map(|p| parse(&p))
            .transpose()?
            .unwrap_or(Value::Null);
        json!({"job_id":id,"status":if stale{"stale"}else if status=="completed"&&p["candidates"].as_array().is_some_and(Vec::is_empty){"no_match"}else{&status},"error":error,"no_match_reason":p["no_match_reason"],"ambiguity":p["ambiguity"]})
    } else if let Some((status, error)) = state {
        json!({"status":status,"error":error})
    } else {
        Value::Null
    };
    Ok(json!({"links":entries,"classification":classification}))
}

fn enqueue(
    c: &Connection,
    ticket: &str,
    actor: &str,
    request: Option<&str>,
    live_doc: Option<&Value>,
) -> ApiResult<Option<String>> {
    let t = get_ticket_required(c, ticket)?;
    let rev = revision(&t);
    if t.archived_at.is_some() || terminal(c, &t.project, &t.state)? {
        return Ok(None);
    }
    ensure_project_writable(c, &t.project)?;
    if let Some(request) = request {
        super::agent_chat::bounded(request, 120, "request_id")?;
        let old:Option<String>=c.query_row("SELECT j.id FROM agent_jobs j JOIN ticket_document_jobs d ON d.job=j.id WHERE d.ticket=?1 AND j.requested_by=?2 AND j.request_id=?3",params![ticket,actor,request],|r|r.get(0)).optional()?;
        if old.is_some() {
            return Ok(old);
        }
    }
    if live_doc.is_none() {
        let bytes:i64=c.query_row("SELECT COALESCE(SUM(length(u.blob)),0) FROM crdt_updates u JOIN mindmaps m ON m.id=u.object_id WHERE m.project=?1",[&t.project],|r|r.get(0))?;
        if bytes > 8_000_000 {
            save_state(
                c,
                ticket,
                &rev,
                "unavailable",
                Some("Document history exceeds 8 MB; compact it or attach references manually"),
            )?;
            return Ok(None);
        }
    }
    let doc = match live_doc {
        Some(d) => d.clone(),
        None => match document(c, &t.project) {
            Ok(Some(d)) => d,
            Ok(None) => {
                save_state(
                    c,
                    ticket,
                    &rev,
                    "unavailable",
                    Some("This project has no document to classify against"),
                )?;
                return Ok(None);
            }
            Err(e) => {
                save_state(c, ticket, &rev, "unavailable", Some(&e.body.message))?;
                return Ok(None);
            }
        },
    };
    if doc.to_string().len() > 8_000_000 || doc["sections"].as_array().is_none_or(|s| s.len() > 500)
    {
        save_state(c,ticket,&rev,"unavailable",Some("Classification supports documents up to 500 sections and 8 MB; manual references remain available"))?;
        return Ok(None);
    }
    // Existing CRDT promotion pointers prove only the exact root origin, never children by title or parent.
    if let Some(origin) = doc["sections"].as_array().and_then(|sections| {
        sections
            .iter()
            .find(|s| s["promoted_id"] == ticket && s["promoted_kind"] == "epic")
    }) {
        let proven:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE project=?1 AND kind='mindmap_promoted' AND json_extract(payload,'$.mindmap')=?2 AND json_extract(payload,'$.node')=?3 AND json_extract(payload,'$.id')=?4 AND json_extract(payload,'$.kind')='epic')",params![t.project,doc["mindmap_id"].as_str().unwrap(),origin["id"].as_str().unwrap(),ticket],|r|r.get(0))?;
        if proven && !accepted(c, ticket, &t.project, &doc)? {
            direct(
                c,
                ticket,
                &doc,
                origin["id"].as_str().unwrap(),
                "system:document-lineage",
            )?;
        }
    }
    if request.is_none() && accepted(c, ticket, &t.project, &doc)? {
        save_state(c, ticket, &rev, "linked", None)?;
        return Ok(None);
    }
    let active:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM ticket_document_jobs d JOIN agent_jobs j ON j.id=d.job WHERE d.ticket=?1 AND j.status='running')",[ticket],|r|r.get(0))?;
    if active {
        return Err(conflict(
            "Classification is running; the latest material edit will be classified afterward",
        ));
    }
    let docrev = sha256_hex(doc.to_string().as_bytes());
    if request.is_none() {
        let same:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM ticket_document_jobs WHERE ticket=?1 AND ticket_revision=?2 AND document_revision=?3)",params![ticket,rev,docrev],|r|r.get(0))?;
        if same {
            return Ok(None);
        }
    }
    c.execute("UPDATE agent_jobs SET status='failed',error='Superseded by a newer classification request',finished_at=?2 WHERE id IN (SELECT job FROM ticket_document_jobs WHERE ticket=?1) AND status='queued'",params![ticket,now_ms()])?;
    let cid = format!("ac-{}", ticket_suffix(20));
    let jid = format!("aj-{}", ticket_suffix(20));
    let now = now_ms();
    let snap = json!({"kind":KIND,"schema_version":1,"ticket":{"id":t.id,"project":t.project,"title":t.title,"body":t.body,"parent":t.parent,"revision":rev},"document":doc});
    let raw = snap.to_string();
    if raw.len() > 8_200_000 {
        save_state(
            c,
            ticket,
            &rev,
            "unavailable",
            Some("The combined ticket and document snapshot exceeds 8.2 MB"),
        )?;
        return Ok(None);
    }

    c.execute(
        "INSERT INTO agent_conversations(id,node,project,created_at) VALUES(?1,?2,?3,?4)",
        params![cid, format!("classification:{ticket}"), t.project, now],
    )?;
    c.execute("INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",params![jid,cid,actor,request.unwrap_or(&rev),"Find document sections directly relevant to this ticket. Return evidence-backed candidates or explain that no suitable section exists.",raw,sha256_hex(raw.as_bytes()),now])?;
    c.execute("INSERT INTO ticket_document_jobs(job,ticket,ticket_revision,document_revision,reconsider) VALUES(?1,?2,?3,?4,?5)",params![jid,ticket,rev,docrev,request.is_some()])?;
    save_state(c, ticket, &rev, "queued", None)?;
    emit_event(
        c,
        Some(ticket),
        Some(&t.project),
        actor,
        "agent_job.queued",
        json!({"job_id":jid,"kind":KIND}),
        now,
    )?;
    Ok(Some(jid))
}
impl Store {
    pub fn ticket_document_view(
        &self,
        ctx: &AuthCtx,
        ticket: &str,
        live: Option<&Value>,
    ) -> ApiResult<Value> {
        self.with_conn(|c| {
            let t = get_ticket_required(c, ticket)?;
            ctx.require_scope("read")?;
            ctx.require_project(&t.project)?;
            let d = match live {
                Some(d) => Some(d.clone()),
                None => document(c, &t.project)?,
            };
            view(c, ticket, d.as_ref())
        })
    }
    pub fn ticket_document_config(&self, ctx: &AuthCtx, project: &str) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        ctx.require_project(project)?;
        self.with_conn(|c| {
            super::helpers::get_workflow(c, project)?;
            let mode: Option<String> = c
                .query_row(
                    "SELECT mode FROM ticket_document_settings WHERE project=?1",
                    [project],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(json!({"mode":mode.unwrap_or_else(||"suggest".into())}))
        })
    }
    pub fn set_ticket_document_config(
        &self,
        ctx: &AuthCtx,
        project: &str,
        mode: &str,
    ) -> ApiResult<Value> {
        ctx.require_scope("admin")?;
        ctx.require_project(project)?;
        if !["suggest", "auto_apply_clear"].contains(&mode) {
            return Err(invalid("mode must be suggest or auto_apply_clear"));
        }
        self.with_tx(|c|{ensure_project_writable(c,project)?;c.execute("INSERT INTO ticket_document_settings(project,mode) VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET mode=excluded.mode",params![project,mode])?;emit_event(c,None,Some(project),&ctx.actor,"project.document_classification_configured",json!({"mode":mode}),now_ms())?;Ok(json!({"mode":mode}))})
    }
    pub fn request_ticket_classification(
        &self,
        ctx: &AuthCtx,
        ticket: &str,
        request: &str,
        live: Option<&Value>,
    ) -> ApiResult<Value> {
        self.with_tx(|c| {
            let t = get_ticket_required(c, ticket)?;
            human(ctx, &t.project)?;
            let job = enqueue(c, ticket, &ctx.actor, Some(request), live)?;
            c.execute(
                "DELETE FROM ticket_document_pending WHERE ticket=?1",
                [ticket],
            )?;
            Ok(json!({"job_id":job}))
        })
    }
    pub fn backfill_ticket_classification(
        &self,
        ctx: &AuthCtx,
        project: &str,
        request: &str,
    ) -> ApiResult<Value> {
        human(ctx, project)?;
        super::agent_chat::bounded(request, 120, "request_id")?;
        self.with_tx(|c|{ensure_project_writable(c,project)?;let linked=filtered_tickets(c,Some(project),None,None)?;let queued=c.execute("INSERT OR IGNORE INTO ticket_document_pending(ticket) SELECT t.id FROM tickets t JOIN workflow_states ws ON ws.project=t.project AND ws.state=t.state WHERE t.project=?1 AND t.archived_at IS NULL AND ws.terminal=0 AND t.id NOT IN (SELECT value FROM json_each(?2))",params![project,serde_json::to_string(&linked).unwrap()])?;Ok(json!({"scheduled":queued}))})
    }
    pub fn sweep_ticket_classification(&self) -> ApiResult<usize> {
        self.with_tx(|c|{
  let mut s=c.prepare("SELECT p.ticket FROM ticket_document_pending p JOIN tickets t ON t.id=p.ticket JOIN projects pr ON pr.id=t.project WHERE pr.archived_at IS NULL AND NOT EXISTS(SELECT 1 FROM ticket_document_jobs d JOIN agent_jobs j ON j.id=d.job WHERE d.ticket=p.ticket AND j.status='running') ORDER BY p.rowid LIMIT 20")?;
  let ids=s.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;let mut n=0;
  for id in ids {if enqueue(c,&id,"system:document-classifier",None,None)?.is_some(){n+=1;}c.execute("DELETE FROM ticket_document_pending WHERE ticket=?1",[id])?;}Ok(n)
 })
    }
    pub fn add_ticket_document_link(
        &self,
        ctx: &AuthCtx,
        ticket: &str,
        node: &str,
        primary: bool,
        reason: &str,
        doc: &Value,
    ) -> ApiResult<Value> {
        self.with_tx(|c|{
  let t=get_ticket_required(c,ticket)?;human(ctx,&t.project)?;ensure_project_writable(c,&t.project)?;
  let map_project:Option<String>=c.query_row("SELECT project FROM mindmaps WHERE id=?1",[doc["mindmap_id"].as_str().unwrap()],|r|r.get(0)).optional()?;if map_project.as_deref()!=Some(&t.project){return Err(invalid("Document must belong to this ticket's project"))}
  let s=section(doc,node).ok_or_else(||invalid("No such section in this project's document"))?;super::agent_chat::bounded(reason,4000,"reason")?;
  c.execute("UPDATE ticket_document_links SET state='removed',is_primary=0,updated_at=?3 WHERE ticket=?1 AND section_id=?2 AND relation='related' AND state<>'removed'",params![ticket,node,now_ms()])?;
  insert_link(c,&t,doc,s,NewLink { relation:"related", provenance:"manual", state:"accepted",primary,reason,quote:"",actor:&ctx.actor,job:None })?;view(c,ticket,Some(doc))
 })
    }
    pub fn change_ticket_document_link(
        &self,
        ctx: &AuthCtx,
        ticket: &str,
        id: &str,
        state: Option<&str>,
        primary: Option<bool>,
        doc: Option<&Value>,
    ) -> ApiResult<Value> {
        self.with_tx(|c|{
  let t=get_ticket_required(c,ticket)?;human(ctx,&t.project)?;ensure_project_writable(c,&t.project)?;
  let link=links(c,&t,doc)?.into_iter().find(|l|l["id"]==id).ok_or_else(||ApiError::not_found("ticket_document_link",id))?;
  let desired=state.unwrap_or(link["state"].as_str().unwrap());if !["accepted","removed"].contains(&desired){return Err(invalid("state must be accepted or removed"))}
  if desired=="accepted" {
   if link["missing"]==true{return Err(conflict("This document section no longer exists in the ticket's project"))}
   if link["state"]!="accepted"&&link["stale"]==true{return Err(conflict("This suggestion is stale. Classify the current ticket again or add its section manually"))}
  }
  let primary=desired=="accepted"&&primary.unwrap_or(link["primary"].as_bool().unwrap());
  if primary{c.execute("UPDATE ticket_document_links SET is_primary=0 WHERE ticket=?1",[ticket])?;}
  c.execute("UPDATE ticket_document_links SET state=?2,is_primary=?3,updated_at=?4,reviewed_by=?5,reviewed_at=?4 WHERE id=?1",params![id,desired,primary,now_ms(),ctx.actor])?;
  emit_event(c,Some(ticket),Some(&t.project),&ctx.actor,"ticket.document_reference_updated",json!({"link":id,"state":desired,"primary":primary}),now_ms())?;view(c,ticket,doc)
 })
    }
    pub fn project_document_links(
        &self,
        ctx: &AuthCtx,
        project: &str,
        node: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        ctx.require_project(project)?;
        self.with_conn(|c|{
            super::helpers::get_workflow(c,project)?;
            let Some(doc)=document(c,project)? else{return Ok(json!({"items":[],"total":0,"limit":limit,"offset":offset}))};
            let live_ids:Vec<_>=doc["sections"].as_array().unwrap().iter().map(|s|s["id"].clone()).collect();
            let ids=serde_json::to_string(&live_ids).unwrap();
            let filter=" FROM ticket_document_links l JOIN tickets t ON t.id=l.ticket WHERE t.project=?1 AND l.project=t.project AND l.mindmap=?2 AND l.state='accepted' AND l.section_id IN (SELECT value FROM json_each(?3)) AND (?4 IS NULL OR l.section_id=?4)";
            let total:i64=c.query_row(&format!("SELECT COUNT(*){filter}"),params![project,doc["mindmap_id"].as_str().unwrap(),ids,node],|r|r.get(0))?;
            let mut stmt=c.prepare(&format!("SELECT json_object('id',l.id,'ticket',l.ticket,'project',l.project,'mindmap',l.mindmap,'section_id',l.section_id,'title',l.title,'captured_title',l.title,'section_version',l.section_version,'relation',l.relation,'provenance',l.provenance,'state',l.state,'primary',json(CASE WHEN l.is_primary THEN 'true' ELSE 'false' END),'reason',l.reason,'quote',l.quote,'created_by',l.created_by,'created_at',l.created_at,'reviewed_by',l.reviewed_by,'reviewed_at',l.reviewed_at,'ticket_title',t.title,'ticket_state',t.state){filter} ORDER BY t.id,l.created_at,l.id LIMIT ?5 OFFSET ?6"))?;
            let rows=stmt.query_map(params![project,doc["mindmap_id"].as_str().unwrap(),ids,node,limit as i64,offset as i64],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
            let mut items=Vec::new();for raw in rows{let mut l=parse(&raw)?;let source=section(&doc,l["section_id"].as_str().unwrap()).unwrap();l["title"]=source["title"].clone();l["missing"]=json!(false);l["stale"]=json!(l["section_version"]!=source["version"]);items.push(l);}
            Ok(json!({"items":items,"total":total,"limit":limit,"offset":offset}))
        })
    }

    pub fn classification_job_project(
        &self,
        ctx: &AuthCtx,
        jid: &str,
    ) -> ApiResult<Option<String>> {
        self.with_conn(|c|{let project:Option<String>=c.query_row("SELECT ac.project FROM ticket_document_jobs d JOIN agent_jobs j ON j.id=d.job JOIN agent_conversations ac ON ac.id=j.conversation_id WHERE d.job=?1",[jid],|r|r.get(0)).optional()?;if let Some(p)=&project{ctx.require_project(p)?}Ok(project)})
    }
}

pub(super) fn save_proposal(
    c: &Connection,
    jid: &str,
    proposal: Option<&Value>,
    evidence: Option<&Value>,
    completed: bool,
    live_doc: Option<&Value>,
) -> ApiResult<()> {
    let row:Option<(String,String,String,String,bool)>=c.query_row("SELECT d.ticket,d.ticket_revision,d.document_revision,j.snapshot,d.reconsider FROM ticket_document_jobs d JOIN agent_jobs j ON j.id=d.job WHERE d.job=?1",[jid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((ticket, trev, drev, raw, reconsider)) = row else {
        return Ok(());
    };
    if !completed {
        return Ok(());
    }
    let p = proposal.ok_or_else(|| invalid("Classification completion requires a proposal"))?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Candidate {
        section_id: String,
        version: String,
        quote: String,
        rationale: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Proposal {
        candidates: Vec<Candidate>,
        ambiguity: Option<String>,
        no_match_reason: Option<String>,
    }
    let parsed: Proposal = serde_json::from_value(p.clone())
        .map_err(|e| invalid(format!("Invalid classification proposal: {e}")))?;
    if p.to_string().len() > 24_000 || parsed.candidates.len() > 3 {
        return Err(invalid(
            "Classification proposals allow at most three candidates and 24000 bytes",
        ));
    }
    if parsed.candidates.is_empty() {
        super::agent_chat::bounded(
            parsed.no_match_reason.as_deref().unwrap_or(""),
            4000,
            "no_match_reason",
        )?;
    } else if parsed.no_match_reason.is_some() {
        return Err(invalid("A candidate proposal cannot also report no match"));
    }
    if let Some(a) = &parsed.ambiguity {
        super::agent_chat::bounded(a, 4000, "ambiguity")?;
    }
    let snap = parse(&raw)?;
    let doc = &snap["document"];
    let sources = evidence
        .and_then(|e| e.get("document"))
        .and_then(|d| d.get("sources"))
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Classification requires actual document source evidence"))?;
    if sources.len() > 500 {
        return Err(invalid("Too many retrieved sources"));
    }
    let mut seen_sources = HashSet::new();
    for source in sources {
        let id = source["section_id"]
            .as_str()
            .ok_or_else(|| invalid("Invalid source ID"))?;
        let s = section(doc, id)
            .ok_or_else(|| invalid("Retrieved source outside captured document"))?;
        if !seen_sources.insert(id) || source["version"] != s["version"] {
            return Err(invalid("Source versions must match captured sections"));
        }
    }
    let mut seen = HashSet::new();
    for item in &parsed.candidates {
        let s = section(doc, &item.section_id)
            .ok_or_else(|| invalid("Candidate section is outside the captured document"))?;
        if !seen.insert(&item.section_id)
            || s["version"] != item.version
            || !seen_sources.contains(item.section_id.as_str())
        {
            return Err(invalid(
                "Candidates must be unique retrieved sections at their captured version",
            ));
        }
        super::agent_chat::bounded(&item.quote, 2000, "source quote")?;
        super::agent_chat::bounded(&item.rationale, 4000, "rationale")?;
        if !normalized(s["notes"].as_str().unwrap()).contains(&normalized(&item.quote)) {
            return Err(invalid(
                "Candidate quote does not occur in the captured section",
            ));
        }
    }
    let t = get_ticket_required(c, &ticket)?;
    let current = match live_doc {
        Some(d) => Some(d.clone()),
        None => document(c, &t.project)?,
    };
    let stale = revision(&t) != trev
        || current
            .as_ref()
            .is_none_or(|d| sha256_hex(d.to_string().as_bytes()) != drev)
        || t.archived_at.is_some()
        || terminal(c, &t.project, &t.state)?
        || ensure_project_writable(c, &t.project).is_err();
    let mode: Option<String> = c
        .query_row(
            "SELECT mode FROM ticket_document_settings WHERE project=?1",
            [&t.project],
            |r| r.get(0),
        )
        .optional()?;
    let any_accepted: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM ticket_document_links WHERE ticket=?1 AND state='accepted')",
        [&ticket],
        |r| r.get(0),
    )?;
    let auto = !stale
        && !any_accepted
        && mode.as_deref() == Some("auto_apply_clear")
        && parsed.candidates.len() == 1
        && parsed.ambiguity.is_none();
    if !stale {
        c.execute("UPDATE ticket_document_links SET state='removed',is_primary=0,updated_at=?2 WHERE ticket=?1 AND state='suggested' AND provenance='automatic'",params![ticket,now_ms()])?;
    }
    for item in parsed.candidates.iter().filter(|_| !stale) {
        let s = section(doc, &item.section_id).unwrap();
        let dismissed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM ticket_document_links WHERE ticket=?1 AND section_id=?2 AND section_version=?3 AND ticket_revision=?4 AND state='removed' AND reviewed_by IS NOT NULL)",params![ticket,item.section_id,item.version,trev],|r|r.get(0))?;
        if dismissed && !reconsider {
            continue;
        }
        let title = normalized(s["title"].as_str().unwrap()).to_lowercase();
        let unique = doc["sections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| normalized(s["title"].as_str().unwrap()).to_lowercase() == title)
            .count()
            == 1;
        let clear = auto && unique && title == normalized(&t.title).to_lowercase();
        // A suggestion never replaces a confirmed or manually attached reference.
        let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM ticket_document_links WHERE ticket=?1 AND mindmap=?2 AND section_id=?3 AND state='accepted')",params![ticket,doc["mindmap_id"].as_str().unwrap(),item.section_id],|r|r.get(0))?;
        if exists {
            continue;
        }
        c.execute("UPDATE ticket_document_links SET state='removed',is_primary=0 WHERE ticket=?1 AND section_id=?2 AND state='suggested'",params![ticket,item.section_id])?;
        let id = insert_link(
            c,
            &t,
            doc,
            s,
            NewLink {
                relation: "related",
                provenance: "automatic",
                state: if clear { "accepted" } else { "suggested" },
                primary: clear,
                reason: &item.rationale,
                quote: &item.quote,
                actor: "system:document-classifier",
                job: Some(jid),
            },
        )?;
        c.execute(
            "UPDATE ticket_document_links SET ticket_revision=?2 WHERE id=?1",
            params![id, trev],
        )?;
    }
    c.execute(
        "UPDATE ticket_document_jobs SET proposal=?2,stale=?3 WHERE job=?1",
        params![jid, p.to_string(), stale],
    )?;
    save_state(
        c,
        &ticket,
        &trev,
        if stale {
            "stale"
        } else if parsed.candidates.is_empty() {
            "no_match"
        } else {
            "completed"
        },
        None,
    )?;
    Ok(())
}

/// One relation query and one source decode per referenced project, not per ticket.
pub(super) fn hydrate_refs(c: &Connection, tickets: &mut [super::model::Ticket]) -> ApiResult<()> {
    let ids: Vec<_> = tickets.iter().map(|t| t.id.as_str()).collect();
    let mut stmt=c.prepare("SELECT l.id,l.ticket,l.project,l.mindmap,l.section_id,l.title,l.is_primary,l.provenance FROM ticket_document_links l WHERE l.state='accepted' AND l.ticket IN (SELECT value FROM json_each(?1)) ORDER BY l.is_primary DESC,l.created_at,l.id")?;
    let rows = stmt
        .query_map([serde_json::to_string(&ids).unwrap()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, bool>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut docs = std::collections::HashMap::new();
    for (id, ticket, project, map, node, captured, primary, provenance) in rows {
        let Some(t) = tickets.iter_mut().find(|t| t.id == ticket) else {
            continue;
        };
        if project != t.project {
            continue;
        }
        if !docs.contains_key(&project) {
            docs.insert(project.clone(), document(c, &project)?);
        }
        let source = docs[&project]
            .as_ref()
            .filter(|d| d["mindmap_id"] == map)
            .and_then(|d| section(d, &node));
        if source.is_none() {
            continue;
        }
        t.document_refs.push(json!({"id":id,"section_id":node,"title":source.map(|s|s["title"].clone()).unwrap_or(json!(captured)),"primary":primary,"provenance":provenance,"missing":source.is_none()}));
    }
    Ok(())
}
pub(super) fn filtered_tickets(
    c: &Connection,
    project: Option<&str>,
    allowed: Option<&[String]>,
    node: Option<&str>,
) -> ApiResult<Vec<String>> {
    let allowed = allowed.map(|v| serde_json::to_string(v).unwrap());
    let mut s=c.prepare("SELECT DISTINCT t.id FROM tickets t JOIN ticket_document_links l ON l.ticket=t.id WHERE l.state='accepted' AND l.project=t.project AND (?1 IS NULL OR t.project=?1) AND (?2 IS NULL OR t.project IN (SELECT value FROM json_each(?2))) AND (?3 IS NULL OR l.section_id=?3)")?;
    let ids = s
        .query_map(params![project, allowed, node], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = c.prepare(&format!(
        "SELECT {} FROM tickets t WHERE t.id IN (SELECT value FROM json_each(?1))",
        super::helpers::TICKET_COLS
    ))?;
    let mut tickets = rows
        .query_map(
            [serde_json::to_string(&ids).unwrap()],
            super::helpers::row_to_ticket,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    hydrate_refs(c, &mut tickets)?;
    Ok(tickets
        .into_iter()
        .filter(|t| {
            t.document_refs
                .iter()
                .any(|r| r["missing"] == false && node.is_none_or(|n| r["section_id"] == n))
        })
        .map(|t| t.id)
        .collect())
}
