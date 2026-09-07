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
    #[serde(default)]
    pub section_ids: Vec<String>,
    #[serde(default)]
    pub whole_document: bool,
    pub context: Option<Context>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Quote {
    pub section_id: String,
    pub text: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    #[serde(default = "automatic")]
    pub mode: String,
    #[serde(default)]
    pub section_ids: Vec<String>,
    #[serde(default)]
    pub pinned_section_ids: Vec<String>,
    pub quote: Option<Quote>,
}
fn automatic() -> String {
    "automatic".into()
}
impl Default for Context {
    fn default() -> Self {
        Self {
            mode: automatic(),
            section_ids: vec![],
            pinned_section_ids: vec![],
            quote: None,
        }
    }
}
fn validate_ids(ids: &[String]) -> ApiResult<()> {
    let mut unique = HashSet::new();
    if ids.len() > 500 {
        return Err(invalid("At most 500 section IDs are allowed"));
    }
    for id in ids {
        agent_chat::bounded(id, 120, "section ID")?;
        if !unique.insert(id) {
            return Err(invalid("Section IDs must be unique"));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> ApiError {
    ApiError::validation("validation.document_chat", message)
}
impl Context {
    fn validate(&self) -> ApiResult<()> {
        validate_ids(&self.section_ids)?;
        validate_ids(&self.pinned_section_ids)?;
        if !["automatic", "selected", "whole_document"].contains(&self.mode.as_str()) {
            return Err(invalid("Unknown document context mode"));
        }
        if self.mode == "selected" {
            if self.section_ids.is_empty() && self.pinned_section_ids.is_empty() {
                return Err(invalid(
                    "Selected context needs at least one selected or pinned section",
                ));
            }
        } else if !self.section_ids.is_empty() || self.quote.is_some() {
            return Err(invalid(
                "Section selection and quotes require selected context",
            ));
        }
        if let Some(q) = &self.quote {
            agent_chat::bounded(&q.text, 12_000, "selected quote")?;
            if !self.section_ids.contains(&q.section_id)
                && !self.pinned_section_ids.contains(&q.section_id)
            {
                return Err(invalid(
                    "The quote must belong to a selected or pinned section",
                ));
            }
        }
        Ok(())
    }
    pub fn allows(&self, id: &str) -> bool {
        self.mode != "selected"
            || self
                .section_ids
                .iter()
                .chain(&self.pinned_section_ids)
                .any(|i| i == id)
    }
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
        if let Some(context) = &self.context {
            return context.validate();
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
        if let Some(context) = &self.context {
            let mut context = context.clone();
            context.section_ids.sort();
            context.pinned_section_ids.sort();
            return json!({"message":self.message,"action":self.action,"context":context})
                .to_string();
        }
        json!({"message":self.message,"action":self.action,"section_ids":ids,"whole_document":self.whole_document}).to_string()
    }
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.agent_job", message)
}
fn view(conn: &Connection, map: &str) -> ApiResult<Value> {
    let mut value = agent_chat::view(conn, map, "")?;
    let pins: Option<String> = conn
        .query_row(
            "SELECT pinned_section_ids FROM document_agent_settings WHERE mindmap=?1",
            [map],
            |r| r.get(0),
        )
        .optional()?;
    value["pinned_section_ids"] = pins
        .map(|p| serde_json::from_str::<Value>(&p))
        .transpose()
        .map_err(|e| ApiError::internal(e.to_string()))?
        .unwrap_or(json!([]));
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
        let workspace: Option<String> = conn
            .query_row(
                "SELECT context FROM document_workspace_jobs WHERE job=?1",
                [job["id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(context) = workspace {
            let context: Context =
                serde_json::from_str(&context).map_err(|e| ApiError::internal(e.to_string()))?;
            job["context"] = json!(context);
            let migration: Option<String> = conn
                .query_row(
                    "SELECT metadata FROM document_thread_migrations WHERE job=?1",
                    [job["id"].as_str().unwrap()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(m) = migration {
                job["migration"] =
                    serde_json::from_str(&m).map_err(|e| ApiError::internal(e.to_string()))?;
            }
            let allowed: Vec<_> = sections
                .iter()
                .filter(|s| context.allows(s["id"].as_str().unwrap()))
                .collect();
            job["section_count"] = json!(allowed.len());
            let evidence: Option<String>=conn.query_row("SELECT json_extract(result_json,'$.evidence.document') FROM agent_jobs WHERE id=?1",[job["id"].as_str().unwrap()],|r|r.get(0))?;
            if let Some(evidence) = evidence {
                let evidence: Value = serde_json::from_str(&evidence)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                job["coverage"] = evidence["coverage"].clone();
                job["sources"]=json!(evidence["sources"].as_array().unwrap().iter().filter_map(|source|sections.iter().find(|s|s["id"]==source["section_id"]).map(|s|json!({"section_id":s["id"],"title":s["title"],"version":s["version"]}))).collect::<Vec<_>>());
            }
        }
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
        context: conn.query_row("SELECT context FROM document_workspace_jobs WHERE job=(SELECT id FROM agent_jobs WHERE conversation_id=?1 AND requested_by=?2 AND request_id=?3)",params![cid,ctx.actor,req.request_id],|r|r.get::<_,String>(0)).optional()?.map(|v|serde_json::from_str(&v)).transpose().map_err(|e|ApiError::internal(e.to_string()))?,
    };
    if old.intent() != req.intent() {
        return Err(conflict(
            "request_id already names different instructions or context. Use a new request_id.",
        ));
    }
    Ok(Some(view(conn, map)?))
}

impl Store {
    pub fn set_document_pins(
        &self,
        ctx: &AuthCtx,
        map: &str,
        project: &str,
        pins: &[String],
    ) -> ApiResult<()> {
        validate_ids(pins)?;
        self.with_tx(|tx| {
            ensure_project_writable(tx,project)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mindmaps WHERE id=?1 AND project=?2)",params![map,project],|r|r.get(0))?;
            if !exists { return Err(ApiError::not_found("mindmap",map)); }
            tx.execute("INSERT INTO document_agent_settings(mindmap,pinned_section_ids) VALUES(?1,?2) ON CONFLICT(mindmap) DO UPDATE SET pinned_section_ids=excluded.pinned_section_ids",params![map,serde_json::to_string(pins).unwrap()])?;
            emit_event(tx,None,Some(project),&ctx.actor,"document_agent.pins_updated",json!({"mindmap":map,"pinned_section_ids":pins}),now_ms())?;
            Ok(())
        })
    }

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
            if req.context.is_some() {
                8_000_000
            } else {
                100_000
            },
            "document snapshot exceeds the supported size; split the document to continue",
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
            if let Some(context)=&req.context {
                tx.execute("INSERT INTO document_workspace_jobs(job,context) VALUES(?1,?2)",params![jid,serde_json::to_string(context).unwrap()])?;
            }
            tx.execute("INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'user',?4,?5)",params![format!("am-{}",ticket_suffix(20)),cid,jid,req.message,now])?;
            emit_event(tx,None,Some(project),&ctx.actor,"agent_job.queued",json!({"job_id":jid,"conversation_id":cid}),now)?;
            view(tx,map)
        })
    }
}

pub(super) fn validate_evidence(
    conn: &Connection,
    jid: &str,
    evidence: Option<&Value>,
    completed: bool,
) -> ApiResult<()> {
    let snapshot:Option<String>=conn.query_row("SELECT j.snapshot FROM agent_jobs j JOIN document_workspace_jobs w ON w.job=j.id WHERE j.id=?1",[jid],|r|r.get(0)).optional()?;
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let Some(document) = evidence.and_then(|e| e.get("document")) else {
        return if completed {
            Err(invalid(
                "Workspace completion requires document source evidence",
            ))
        } else {
            Ok(())
        };
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Source {
        section_id: String,
        version: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Coverage {
        read_section_ids: Vec<String>,
        total_sections: usize,
        complete: bool,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Evidence {
        sources: Vec<Source>,
        coverage: Coverage,
    }
    let evidence: Evidence = serde_json::from_value(document.clone())
        .map_err(|_| invalid("Invalid document source evidence"))?;
    let snapshot: Value =
        serde_json::from_str(&snapshot).map_err(|e| ApiError::internal(e.to_string()))?;
    let context: Context = serde_json::from_value(snapshot["context"].clone())
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let sections: Vec<_> = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| context.allows(s["id"].as_str().unwrap()))
        .collect();
    if evidence.sources.len() > 500 {
        return Err(invalid("Too many document sources"));
    }
    let mut seen = HashSet::new();
    for source in &evidence.sources {
        if !seen.insert(source.section_id.as_str())
            || !sections
                .iter()
                .any(|s| s["id"] == source.section_id && s["version"] == source.version)
        {
            return Err(invalid(
                "Source evidence must reference unique accessible sections at the captured version",
            ));
        }
    }
    validate_ids(&evidence.coverage.read_section_ids)?;
    if evidence.coverage.total_sections != sections.len()
        || evidence
            .coverage
            .read_section_ids
            .iter()
            .any(|id| !seen.contains(id.as_str()))
        || evidence.coverage.complete
            != (evidence.coverage.read_section_ids.len() == sections.len())
    {
        return Err(invalid(
            "Document coverage does not match its captured scope and sources",
        ));
    }
    Ok(())
}

pub(super) fn needs_migration(conn: &Connection, cid: &str, thread: &str) -> ApiResult<bool> {
    let workspace:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM document_thread_profiles WHERE conversation_id=?1 AND thread_id=?2)",params![cid,thread],|r|r.get(0))?;
    Ok(!workspace)
}
pub(super) fn record_migration(
    conn: &Connection,
    jid: &str,
    evidence: Option<&Value>,
    thread: Option<&str>,
) -> ApiResult<()> {
    let Some(metadata) = evidence.and_then(|e| e.get("document_migration")) else {
        return Ok(());
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Migration {
        previous_thread_id: String,
        new_thread_id: String,
        retained_turns: usize,
        omitted_turns: usize,
    }
    let m: Migration = serde_json::from_value(metadata.clone())
        .map_err(|_| invalid("Invalid document thread migration"))?;
    agent_chat::bounded(&m.previous_thread_id, 200, "previous thread")?;
    agent_chat::bounded(&m.new_thread_id, 200, "new thread")?;
    if thread != Some(m.new_thread_id.as_str())
        || m.previous_thread_id == m.new_thread_id
        || m.retained_turns > 1000
        || m.omitted_turns > 1000
    {
        return Err(invalid(
            "Document migration must identify this attempt's new thread and bounded history counts",
        ));
    }
    let (cid,current):(String,Option<String>)=conn.query_row("SELECT c.id,c.thread_id FROM agent_conversations c JOIN agent_jobs j ON j.conversation_id=c.id JOIN document_workspace_jobs w ON w.job=j.id WHERE j.id=?1",[jid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or_else(||invalid("Only workspace jobs can migrate a thread"))?;
    let previous: Option<String> = conn
        .query_row(
            "SELECT metadata FROM document_thread_migrations WHERE job=?1",
            [jid],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        if serde_json::from_str::<Value>(&previous).unwrap() != *metadata {
            return Err(conflict("Thread migration metadata cannot change"));
        }
        return Ok(());
    }
    if current.as_deref() != Some(&m.previous_thread_id)
        || !needs_migration(conn, &cid, &m.previous_thread_id)?
    {
        return Err(conflict("This document thread cannot be migrated again"));
    }
    conn.execute(
        "INSERT INTO document_thread_migrations(job,metadata) VALUES(?1,?2)",
        params![jid, metadata.to_string()],
    )?;
    Ok(())
}
pub(super) fn allows_migration(
    conn: &Connection,
    jid: &str,
    old: &str,
    new: &str,
) -> ApiResult<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM document_thread_migrations WHERE job=?1 AND json_extract(metadata,'$.previous_thread_id')=?2 AND json_extract(metadata,'$.new_thread_id')=?3)",params![jid,old,new],|r|r.get(0))?)
}
