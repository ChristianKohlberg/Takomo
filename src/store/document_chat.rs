//! A shared document thread with an explicit, immutable context for every turn.
use super::{
    agent_chat,
    helpers::{emit_event, ensure_project_writable},
    Store,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, sha256_hex, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    pub message: String,
    pub request_id: String,
    pub action: String,
    pub section_ids: Vec<String>,
    pub whole_document: bool,
}
impl Send {
    pub fn validate(&self) -> ApiResult<()> {
        agent_chat::bounded(&self.message, 8000, "message")?;
        agent_chat::bounded(&self.request_id, 120, "request_id")?;
        if !["discuss", "grill", "draft_tests", "draft_questions"].contains(&self.action.as_str()) {
            return Err(ApiError::validation(
                "validation.document_chat",
                "Unknown document action",
            ));
        }
        if self.whole_document != self.section_ids.is_empty() || self.section_ids.len() > 500 {
            return Err(ApiError::validation(
                "validation.document_chat",
                "Choose whole_document with no section IDs, or select 1–500 sections",
            ));
        }
        let mut unique = HashSet::new();
        for id in &self.section_ids {
            agent_chat::bounded(id, 120, "section ID")?;
            if !unique.insert(id) {
                return Err(ApiError::validation(
                    "validation.document_chat",
                    "Section IDs must be unique",
                ));
            }
        }
        Ok(())
    }
    fn intent(&self) -> String {
        let mut ids = self.section_ids.clone();
        ids.sort();
        json!({"message":self.message,"action":self.action,"section_ids":ids,"whole_document":self.whole_document}).to_string()
    }
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.agent_job", message)
}
fn view(conn: &Connection, map: &str) -> ApiResult<Value> {
    let mut value = agent_chat::view(conn, map, "")?;
    value["turn_limit"] = json!(agent_chat::MAX_TURNS);
    for job in value["jobs"].as_array_mut().unwrap() {
        let (action, ids, whole, sections): (String, String, bool, String) = conn.query_row(
            "SELECT d.action,d.section_ids,d.whole_document,json_extract(j.snapshot,'$.sections') FROM document_agent_jobs d JOIN agent_jobs j ON j.id=d.job WHERE d.job=?1",
            [job["id"].as_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        job["action"] = json!(action);
        job["section_ids"] =
            serde_json::from_str(&ids).map_err(|e| ApiError::internal(e.to_string()))?;
        job["whole_document"] = json!(whole);
        let sections: Vec<Value> =
            serde_json::from_str(&sections).map_err(|e| ApiError::internal(e.to_string()))?;
        job["section_count"] = json!(sections.len());
        job["sections"] = json!(sections
            .iter()
            .map(|s| json!({"id":s["id"],"title":s["title"]}))
            .collect::<Vec<_>>());
    }
    Ok(value)
}
fn retry(
    conn: &Connection,
    cid: &str,
    map: &str,
    ctx: &AuthCtx,
    req: &Send,
) -> ApiResult<Option<Value>> {
    let prior: Option<(String,String,String,bool)> = conn.query_row(
        "SELECT j.prompt,d.action,d.section_ids,d.whole_document FROM agent_jobs j JOIN document_agent_jobs d ON d.job=j.id WHERE j.conversation_id=?1 AND j.requested_by=?2 AND j.request_id=?3",
        params![cid,ctx.actor,req.request_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((message, action, ids, whole_document)) = prior else {
        return Ok(None);
    };
    let old = Send {
        message,
        action,
        section_ids: serde_json::from_str(&ids).map_err(|e| ApiError::internal(e.to_string()))?,
        whole_document,
        request_id: req.request_id.clone(),
    };
    if old.intent() != req.intent() {
        return Err(conflict(
            "request_id already names different instructions or context. Use a new request_id.",
        ));
    }
    Ok(Some(view(conn, map)?))
}

impl Store {
    pub fn retry_document_message(
        &self,
        ctx: &AuthCtx,
        map: &str,
        project: &str,
        req: &Send,
    ) -> ApiResult<Option<Value>> {
        self.with_conn(|conn| {
            ensure_project_writable(conn, project)?;
            let cid: Option<String> = conn
                .query_row(
                    "SELECT id FROM agent_conversations WHERE mindmap=?1 AND node=''",
                    [map],
                    |r| r.get(0),
                )
                .optional()?;
            match cid {
                Some(cid) => retry(conn, &cid, map, ctx, req),
                None => Ok(None),
            }
        })
    }
    pub fn document_conversation(&self, map: &str) -> ApiResult<Value> {
        self.with_conn(|conn| view(conn, map))
    }
    pub fn send_document_message(
        &self,
        ctx: &AuthCtx,
        map: &str,
        project: &str,
        snapshot: &str,
        req: &Send,
    ) -> ApiResult<Value> {
        req.validate()?;
        agent_chat::bounded(
            snapshot,
            100_000,
            "document snapshot; select fewer sections if this context is too large",
        )?;
        self.with_tx(|tx| {
            ensure_project_writable(tx, project)?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM mindmaps WHERE id=?1 AND project=?2)", params![map, project], |r| r.get(0))?;
            if !exists { return Err(ApiError::not_found("mindmap", map)); }
            agent_chat::expire(tx)?;
            let now = now_ms();
            let cid = format!("ac-{}", ticket_suffix(20));
            tx.execute("INSERT INTO agent_conversations(id,mindmap,node,project,created_at) VALUES(?1,?2,'',?3,?4) ON CONFLICT(mindmap,node) DO NOTHING", params![cid,map,project,now])?;
            let cid:String = tx.query_row("SELECT id FROM agent_conversations WHERE mindmap=?1 AND node=''", [map], |r|r.get(0))?;
            if let Some(previous) = retry(tx, &cid, map, ctx, req)? { return Ok(previous); }
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_jobs WHERE conversation_id=?1 AND status IN ('queued','running'))",[&cid],|r|r.get(0))?;
            if busy { return Err(conflict("This document already has a queued or running turn. Wait for its reply.")); }
            let turns:i64=tx.query_row("SELECT COUNT(*) FROM agent_jobs WHERE conversation_id=?1",[&cid],|r|r.get(0))?;
            if turns>=agent_chat::MAX_TURNS { return Err(conflict("This conversation reached its limit of 100 turns. Its history remains readable.")); }
            let jid=format!("aj-{}",ticket_suffix(20));
            tx.execute("INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",params![jid,cid,ctx.actor,req.request_id,req.message,snapshot,sha256_hex(snapshot.as_bytes()),now])?;
            tx.execute("INSERT INTO document_agent_jobs(job,action,section_ids,whole_document) VALUES(?1,?2,?3,?4)",params![jid,req.action,serde_json::to_string(&req.section_ids).unwrap(),req.whole_document])?;
            tx.execute("INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'user',?4,?5)",params![format!("am-{}",ticket_suffix(20)),cid,jid,req.message,now])?;
            emit_event(tx,None,Some(project),&ctx.actor,"agent_job.queued",json!({"job_id":jid,"conversation_id":cid}),now)?;
            view(tx,map)
        })
    }
}
