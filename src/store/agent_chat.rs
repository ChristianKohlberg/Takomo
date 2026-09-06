//! Durable section conversations and a leased, single-turn job queue.
use super::{
    helpers::{emit_event, ensure_project_writable},
    Store,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const LEASE_SECONDS: i64 = 60;
const MAX_RUN_MS: i64 = 15 * 60 * 1000;
const MAX_TURNS: i64 = 100;

pub fn bounded(value: &str, max: usize, name: &str) -> ApiResult<()> {
    if value.trim().is_empty() || value.len() > max {
        return Err(ApiError::validation(
            "validation.agent_chat",
            format!("{name} must contain 1–{max} bytes."),
        ));
    }
    Ok(())
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.agent_job", message)
}
fn id(prefix: &str) -> String {
    format!("{prefix}-{}", ticket_suffix(20))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    pub message: String,
    pub request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub service_id: String,
    #[serde(default)]
    pub wait_seconds: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub service_id: String,
    pub attempt_id: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultInput {
    pub service_id: String,
    pub attempt_id: String,
    pub status: String,
    pub message: Option<String>,
    pub error: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
}

fn view(conn: &Connection, map: &str, node: &str) -> ApiResult<Value> {
    let conversation = conn.query_row(
        "SELECT id, created_at FROM agent_conversations WHERE mindmap=?1 AND node=?2",
        params![map,node], |r| Ok(json!({"id":r.get::<_,String>(0)?,"mindmap":map,"node":node,"created_at":r.get::<_,i64>(1)?})),
    ).optional()?;
    let Some(conversation) = conversation else {
        return Ok(json!({"conversation":null,"messages":[],"jobs":[]}));
    };
    let cid = conversation["id"].as_str().unwrap();
    let mut stmt = conn.prepare("SELECT id,job_id,role,body,created_at FROM agent_messages WHERE conversation_id=?1 ORDER BY rowid LIMIT 200")?;
    let messages = stmt.query_map([cid], |r| Ok(json!({"id":r.get::<_,String>(0)?,"job_id":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"body":r.get::<_,String>(3)?,"created_at":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
    let mut stmt = conn.prepare("SELECT id,status,error,created_at,source_revision,finished_at FROM agent_jobs WHERE conversation_id=?1 ORDER BY rowid LIMIT 100")?;
    let jobs = stmt.query_map([cid], |r| Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"error":r.get::<_,Option<String>>(2)?,"created_at":r.get::<_,i64>(3)?,"source_revision":r.get::<_,String>(4)?,"finished_at":r.get::<_,Option<i64>>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"conversation":conversation,"messages":messages,"jobs":jobs,"turn_limit":MAX_TURNS}))
}

/// Expiry never retries a turn: Codex may have completed it before a disconnect.
fn expire(conn: &Connection) -> ApiResult<usize> {
    let now = now_ms();
    let mut stmt = conn.prepare("SELECT j.id,c.project FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id JOIN projects p ON p.id=c.project WHERE (j.status='running' AND (j.lease_expires_at<=?1 OR j.deadline<=?1)) OR (j.status IN ('queued','running') AND p.archived_at IS NOT NULL)")?;
    let expired = stmt
        .query_map([now], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (job, project) in &expired {
        conn.execute("UPDATE agent_jobs SET status='failed',error='The agent run was interrupted or the project was archived. No response was saved. Send a new message to continue.',finished_at=?2 WHERE id=?1", params![job,now])?;
        emit_event(
            conn,
            None,
            Some(project),
            "system:agent",
            "agent_job.failed",
            json!({"job_id":job}),
            now,
        )?;
    }
    Ok(expired.len())
}

struct ActiveJob {
    conversation: String,
    project: String,
    status: String,
    attempt: Option<String>,
    service: Option<String>,
    token: Option<String>,
    lease: Option<i64>,
    deadline: Option<i64>,
    thread: Option<String>,
    turn: Option<String>,
    result: Option<String>,
}
fn job(conn: &Connection, id: &str, ctx: &AuthCtx) -> ApiResult<ActiveJob> {
    let job = conn.query_row("SELECT j.conversation_id,c.project,j.status,j.attempt_id,j.service_id,j.token_id,j.lease_expires_at,j.deadline,j.thread_id,j.turn_id,j.result_json FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id WHERE j.id=?1",[id],|r| Ok(ActiveJob{conversation:r.get(0)?,project:r.get(1)?,status:r.get(2)?,attempt:r.get(3)?,service:r.get(4)?,token:r.get(5)?,lease:r.get(6)?,deadline:r.get(7)?,thread:r.get(8)?,turn:r.get(9)?,result:r.get(10)?})).optional()?.ok_or_else(||ApiError::not_found("agent_job",id))?;
    ctx.require_project(&job.project)?;
    Ok(job)
}
fn owner(job: &ActiveJob, ctx: &AuthCtx, service: &str, attempt: &str) -> ApiResult<()> {
    if job.attempt.as_deref() != Some(attempt)
        || job.service.as_deref() != Some(service)
        || job.token.as_deref() != Some(&ctx.token_id)
    {
        return Err(conflict(
            "This attempt belongs to another service or token. Do not submit its result.",
        ));
    }
    Ok(())
}
fn live(job: &ActiveJob) -> ApiResult<()> {
    let now = now_ms();
    if job.status != "running" || job.lease.unwrap_or(0) <= now || job.deadline.unwrap_or(0) <= now
    {
        return Err(conflict("This job is no longer running or its lease expired. Stop this attempt; late results cannot be saved."));
    }
    Ok(())
}
fn session(
    conn: &Connection,
    job: &ActiveJob,
    id: &str,
    thread: Option<&str>,
    turn: Option<&str>,
) -> ApiResult<()> {
    for (name, v) in [("thread_id", thread), ("turn_id", turn)] {
        if let Some(v) = v {
            bounded(v, 200, name)?;
        }
    }
    if job
        .thread
        .as_deref()
        .zip(thread)
        .is_some_and(|(a, b)| a != b)
        || job.turn.as_deref().zip(turn).is_some_and(|(a, b)| a != b)
    {
        return Err(conflict(
            "Codex thread and turn IDs cannot change during an attempt.",
        ));
    }
    if let Some(thread) = thread {
        let existing: Option<String> = conn.query_row(
            "SELECT thread_id FROM agent_conversations WHERE id=?1",
            [&job.conversation],
            |r| r.get(0),
        )?;
        if existing.as_deref().is_some_and(|s| s != thread) {
            return Err(conflict(
                "This conversation already belongs to another Codex thread.",
            ));
        }
        conn.execute(
            "UPDATE agent_conversations SET thread_id=?2 WHERE id=?1",
            params![job.conversation, thread],
        )?;
    }
    conn.execute("UPDATE agent_jobs SET thread_id=COALESCE(thread_id,?2),turn_id=COALESCE(turn_id,?3) WHERE id=?1",params![id,thread,turn])?;
    Ok(())
}

// Explicit projection: inspection must never serialize token ownership or the
// canonical result-delivery payload. Titles describe the submitted snapshot.
const INSPECT_COLUMNS: &str = "
    j.id,j.conversation_id,c.project,c.mindmap,c.node,
    substr(j.snapshot,3,instr(j.snapshot,char(10))-3),j.status,j.requested_by,
    j.source_revision,j.created_at,j.finished_at,j.lease_expires_at,j.deadline,
    j.service_id,c.service_id,j.attempt_id,j.thread_id,j.turn_id,j.error";
fn inspect_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id":r.get::<_,String>(0)?, "conversation_id":r.get::<_,String>(1)?,
        "project":r.get::<_,String>(2)?, "mindmap":r.get::<_,String>(3)?,
        "node":r.get::<_,String>(4)?, "section_title":r.get::<_,String>(5)?,
        "status":r.get::<_,String>(6)?, "requested_by":r.get::<_,String>(7)?,
        "source_revision":r.get::<_,String>(8)?, "created_at":r.get::<_,i64>(9)?,
        "finished_at":r.get::<_,Option<i64>>(10)?, "lease_expires_at":r.get::<_,Option<i64>>(11)?,
        "deadline":r.get::<_,Option<i64>>(12)?, "service_id":r.get::<_,Option<String>>(13)?,
        "conversation_service_id":r.get::<_,Option<String>>(14)?, "attempt_id":r.get::<_,Option<String>>(15)?,
        "thread_id":r.get::<_,Option<String>>(16)?, "turn_id":r.get::<_,Option<String>>(17)?,
        "error":r.get::<_,Option<String>>(18)?
    }))
}

impl Store {
    /// Bounded, project-authorized inspection. Reading does not claim or expire jobs.
    pub fn inspect_agent_jobs(
        &self,
        ctx: &AuthCtx,
        project: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> ApiResult<Value> {
        if let Some(project) = project {
            ctx.require_project(project)?;
        }
        if !(1..=100).contains(&limit) {
            return Err(ApiError::validation(
                "validation.agent_chat",
                "limit must be between 1 and 100.",
            ));
        }
        if status.is_some_and(|s| !matches!(s, "queued" | "running" | "completed" | "failed")) {
            return Err(ApiError::validation(
                "validation.agent_chat",
                "status must be queued, running, completed, or failed.",
            ));
        }
        self.with_conn(|conn| {
            let allowed = ctx.allowed_projects_vec().map(|p| serde_json::to_string(&p).unwrap());
            let scope = " FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id WHERE (?1 IS NULL OR c.project=?1) AND (?2 IS NULL OR c.project IN (SELECT value FROM json_each(?2)))";
            let mut counts = json!({"queued":0,"running":0,"completed":0,"failed":0});
            let mut stmt = conn.prepare(&format!("SELECT j.status,COUNT(*){scope} GROUP BY j.status"))?;
            for row in stmt.query_map(params![project,allowed], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))? {
                let (status,count) = row?;
                counts[status] = json!(count);
            }
            let total = match status {
                Some(status) => counts[status].as_i64().unwrap(),
                None => counts.as_object().unwrap().values().map(|v|v.as_i64().unwrap()).sum(),
            };
            let mut stmt = conn.prepare(&format!("SELECT {INSPECT_COLUMNS}{scope} AND (?3 IS NULL OR j.status=?3) ORDER BY j.created_at DESC,j.rowid DESC LIMIT ?4"))?;
            let items = stmt.query_map(params![project,allowed,status,limit], inspect_row)?.collect::<Result<Vec<_>,_>>()?;
            let mut result = crate::api::paged(items,total,limit,"Only the newest matching jobs are returned. Narrow project/status or raise limit to at most 100; inspect older jobs by ID.");
            result["counts"] = counts;
            Ok(result)
        })
    }
    pub fn inspect_agent_job(&self, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            let mut value = conn.query_row(&format!("SELECT {INSPECT_COLUMNS} FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id WHERE j.id=?1"),[id],inspect_row).optional()?.ok_or_else(||ApiError::not_found("agent_job",id))?;
            ctx.require_project(value["project"].as_str().unwrap())?;
            let (prompt,snapshot,response): (String,String,Option<String>) = conn.query_row("SELECT j.prompt,j.snapshot,(SELECT body FROM agent_messages WHERE job_id=j.id AND role='assistant') FROM agent_jobs j WHERE j.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            value["prompt"] = json!(prompt);
            value["snapshot"] = json!(snapshot);
            value["response"] = json!(response);
            let mut stmt = conn.prepare("SELECT id,job_id,role,body,created_at FROM agent_messages WHERE conversation_id=?1 ORDER BY rowid LIMIT 200")?;
            let messages = stmt.query_map([value["conversation_id"].as_str().unwrap()], |r| Ok(json!({"id":r.get::<_,String>(0)?,"job_id":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"body":r.get::<_,String>(3)?,"created_at":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
            Ok(json!({"job":value,"messages":messages}))
        })
    }
    pub fn agent_conversation(&self, map: &str, node: &str) -> ApiResult<Value> {
        self.with_conn(|conn| view(conn, map, node))
    }
    pub fn send_agent_message(
        &self,
        ctx: &AuthCtx,
        map: &str,
        node: &str,
        project: &str,
        snapshot: &str,
        req: &SendMessage,
    ) -> ApiResult<Value> {
        bounded(&req.message, 8000, "message")?;
        bounded(&req.request_id, 120, "request_id")?;
        bounded(snapshot, 100_000, "section snapshot")?;
        self.with_tx(|tx| {
            ensure_project_writable(tx,project)?;
            expire(tx)?;
            let now=now_ms();
            tx.execute("INSERT INTO agent_conversations(id,mindmap,node,project,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(mindmap,node) DO NOTHING",params![id("ac"),map,node,project,now])?;
            let cid:String=tx.query_row("SELECT id FROM agent_conversations WHERE mindmap=?1 AND node=?2",params![map,node],|r|r.get(0))?;
            let previous:Option<String>=tx.query_row("SELECT prompt FROM agent_jobs WHERE conversation_id=?1 AND requested_by=?2 AND request_id=?3",params![cid,ctx.actor,req.request_id],|r|r.get(0)).optional()?;
            if let Some(previous)=previous {
                if previous!=req.message { return Err(conflict("request_id was already used for another message. Use a new request_id.")); }
                return view(tx,map,node);
            }
            let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_jobs WHERE conversation_id=?1 AND status IN ('queued','running'))",[&cid],|r|r.get(0))?;
            if active { return Err(conflict("This section already has a queued or running turn. Wait for its response before sending another message.")); }
            let turns:i64=tx.query_row("SELECT COUNT(*) FROM agent_jobs WHERE conversation_id=?1",[&cid],|r|r.get(0))?;
            if turns>=MAX_TURNS { return Err(conflict("This conversation reached the MVP limit of 100 turns. Its complete history remains readable.")); }
            let jid=id("aj");
            let revision=format!("{:x}",Sha256::digest(snapshot.as_bytes()));
            tx.execute("INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",params![jid,cid,ctx.actor,req.request_id,req.message,snapshot,revision,now])?;
            tx.execute("INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'user',?4,?5)",params![id("am"),cid,jid,req.message,now])?;
            emit_event(tx,None,Some(project),&ctx.actor,"agent_job.queued",json!({"job_id":jid,"conversation_id":cid}),now)?;
            view(tx,map,node)
        })
    }
    pub fn claim_agent_job(&self, ctx: &AuthCtx, service: &str) -> ApiResult<Option<Value>> {
        bounded(service, 120, "service_id")?;
        self.with_tx(|tx| {
            expire(tx)?;
            // MVP: at most one active job per connected service, even if it has
            // accidentally been started twice. Conversations stay on that service.
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_jobs WHERE service_id=?1 AND status='running')",[service],|r|r.get(0))?;
            if busy { return Ok(None); }
            let allowed=ctx.allowed_projects_vec().map(|p|serde_json::to_string(&p).unwrap());
            let candidate:Option<(String,String,Option<String>)>=tx.query_row("SELECT j.id,c.id,c.thread_id FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id JOIN projects p ON p.id=c.project WHERE j.status='queued' AND p.archived_at IS NULL AND (c.service_id IS NULL OR c.service_id=?1) AND (?2 IS NULL OR c.project IN (SELECT value FROM json_each(?2))) ORDER BY j.created_at,j.rowid LIMIT 1",params![service,allowed],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((jid,cid,thread))=candidate else { return Ok(None); };
            let now=now_ms(); let attempt=id("attempt");
            tx.execute("UPDATE agent_jobs SET status='running',attempt_id=?2,service_id=?3,token_id=?4,thread_id=?5,lease_expires_at=?6,deadline=?7 WHERE id=?1",params![jid,attempt,service,ctx.token_id,thread,now+LEASE_SECONDS*1000,now+MAX_RUN_MS])?;
            tx.execute("UPDATE agent_conversations SET service_id=?2 WHERE id=?1",params![cid,service])?;
            let value=tx.query_row("SELECT prompt,snapshot,source_revision FROM agent_jobs WHERE id=?1",[&jid],|r|Ok(json!({"id":jid,"attempt_id":attempt,"conversation_id":cid,"prompt":r.get::<_,String>(0)?,"snapshot":r.get::<_,String>(1)?,"source_revision":r.get::<_,String>(2)?,"thread_id":thread,"lease_seconds":LEASE_SECONDS})))?;
            let project:String=tx.query_row("SELECT project FROM agent_conversations WHERE id=?1",[&cid],|r|r.get(0))?;
            emit_event(tx,None,Some(&project),&ctx.actor,"agent_job.running",json!({"job_id":jid}),now)?;
            Ok(Some(value))
        })
    }
    pub fn heartbeat_agent_job(
        &self,
        ctx: &AuthCtx,
        id: &str,
        req: &Heartbeat,
    ) -> ApiResult<Value> {
        self.with_tx(|tx| {
            let job = job(tx, id, ctx)?;
            owner(&job, ctx, &req.service_id, &req.attempt_id)?;
            live(&job)?;
            ensure_project_writable(tx, &job.project)?;
            session(
                tx,
                &job,
                id,
                req.thread_id.as_deref(),
                req.turn_id.as_deref(),
            )?;
            let lease = (now_ms() + LEASE_SECONDS * 1000).min(job.deadline.unwrap());
            tx.execute(
                "UPDATE agent_jobs SET lease_expires_at=?2 WHERE id=?1",
                params![id, lease],
            )?;
            Ok(json!({"lease_expires_at":lease}))
        })
    }
    pub fn finish_agent_job(
        &self,
        ctx: &AuthCtx,
        jid: &str,
        req: &ResultInput,
    ) -> ApiResult<Value> {
        if !matches!(req.status.as_str(), "completed" | "failed") {
            return Err(ApiError::validation(
                "validation.agent_chat",
                "status must be completed or failed.",
            ));
        }
        if req.status == "completed" {
            bounded(req.message.as_deref().unwrap_or(""), 64_000, "message")?;
            if req.error.is_some() {
                return Err(ApiError::validation(
                    "validation.agent_chat",
                    "A completed turn cannot carry an error.",
                ));
            }
            bounded(req.thread_id.as_deref().unwrap_or(""), 200, "thread_id")?;
            bounded(req.turn_id.as_deref().unwrap_or(""), 200, "turn_id")?;
        } else {
            bounded(req.error.as_deref().unwrap_or(""), 4000, "error")?;
            if req.message.is_some() {
                return Err(ApiError::validation(
                    "validation.agent_chat",
                    "A failed turn cannot publish an assistant message.",
                ));
            }
        }
        let canonical =
            serde_json::to_string(req).map_err(|e| ApiError::internal(e.to_string()))?;
        self.with_tx(|tx| {
            let job=job(tx,jid,ctx)?; owner(&job,ctx,&req.service_id,&req.attempt_id)?;
            if let Some(previous)=&job.result {
                if previous==&canonical { return Ok(json!({"ok":true,"status":job.status})); }
                return Err(conflict("A different result is already recorded for this attempt."));
            }
            live(&job)?; ensure_project_writable(tx,&job.project)?;
            session(tx,&job,jid,req.thread_id.as_deref(),req.turn_id.as_deref())?;
            let now=now_ms();
            if let Some(body)=&req.message {
                tx.execute("INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'assistant',?4,?5)",params![id("am"),job.conversation,jid,body,now])?;
            }
            tx.execute("UPDATE agent_jobs SET status=?2,error=?3,result_json=?4,finished_at=?5 WHERE id=?1",params![jid,req.status,req.error,canonical,now])?;
            emit_event(tx,None,Some(&job.project),&ctx.actor,&format!("agent_job.{}",req.status),json!({"job_id":jid}),now)?;
            Ok(json!({"ok":true,"status":req.status}))
        })
    }
    pub fn sweep_expired_agent_jobs(&self) -> ApiResult<usize> {
        self.with_tx(|tx| expire(tx))
    }
}
