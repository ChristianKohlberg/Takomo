//! Ticket-backed bug triage and explicit research, sharing the durable agent queue.
use super::{
    agent_chat::bounded,
    helpers::{emit_event, ensure_project_writable, get_ticket_required},
    Store,
};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, ticket_suffix},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub fn migrate(conn: &Connection) -> ApiResult<()> {
    let has_ticket: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('agent_conversations') WHERE name='ticket')",
        [],
        |r| r.get(0),
    )?;
    if !has_ticket {
        // SQLite's documented table rebuild: preserve child foreign-key names and data.
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        let result = conn.execute_batch(
            "BEGIN IMMEDIATE; CREATE TABLE agent_conversations_new (id TEXT PRIMARY KEY,mindmap TEXT REFERENCES
                mindmaps(id) ON DELETE CASCADE,node TEXT NOT NULL,ticket TEXT UNIQUE REFERENCES tickets(id) ON DELETE
                CASCADE,project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,service_id TEXT,thread_id
                TEXT,created_at INTEGER NOT NULL,UNIQUE(mindmap,node)); INSERT INTO
                agent_conversations_new(id,mindmap,node,project,service_id,thread_id,created_at) SELECT
                id,mindmap,node,project,service_id,thread_id,created_at FROM agent_conversations; DROP TABLE
                agent_conversations; ALTER TABLE agent_conversations_new RENAME TO agent_conversations; COMMIT;",
        );
        if result.is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        conn.pragma_update(None, "foreign_keys", "ON")?;
        result?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bug_triage(ticket TEXT PRIMARY KEY REFERENCES tickets(id) ON DELETE
                CASCADE,triage TEXT NOT NULL DEFAULT 'needs_triage',severity TEXT NOT NULL DEFAULT 'unknown',duplicate_of
                TEXT REFERENCES tickets(id),note TEXT,updated_by TEXT,updated_at INTEGER); CREATE TABLE IF NOT EXISTS
                bug_research_config(project TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,repository TEXT
                NOT NULL,revision TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS
                bug_research_jobs(job TEXT PRIMARY KEY REFERENCES agent_jobs(id) ON DELETE CASCADE,ticket TEXT NOT NULL
                REFERENCES tickets(id) ON DELETE CASCADE,repository_ref TEXT NOT NULL,cancelled INTEGER NOT NULL DEFAULT
                0,repository_revision TEXT,evidence TEXT); CREATE TABLE IF NOT EXISTS agent_steering(id INTEGER PRIMARY
                KEY AUTOINCREMENT,job TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,actor TEXT NOT
                NULL,request_id TEXT NOT NULL,message TEXT NOT NULL,created_at INTEGER NOT
                NULL,UNIQUE(job,actor,request_id));",
    )?;
    for column in ["repository_revision", "evidence"] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('bug_research_jobs') WHERE name=?1)",
            [column],
            |r| r.get(0),
        )?;
        if !exists {
            conn.execute(
                &format!("ALTER TABLE bug_research_jobs ADD COLUMN {column} TEXT"),
                [],
            )?;
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BugPatch {
    pub triage: Option<String>,
    pub severity: Option<String>,
    pub duplicate_of: Option<String>,
    pub note: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchConfig {
    pub repository: String,
    pub revision: String,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStart {
    pub request_id: String,
    pub message: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Steering {
    pub request_id: String,
    pub message: String,
}
fn conflict(s: &str) -> ApiError {
    ApiError::conflict("conflict.bug_research", s)
}
fn ticket(conn: &Connection, ctx: &AuthCtx, id: &str, write: bool) -> ApiResult<super::Ticket> {
    let t = get_ticket_required(conn, id)?;
    ctx.require_project(&t.project)?;
    if t.ty != "bug" {
        return Err(ApiError::validation(
            "validation.bug",
            "This ticket is not a bug. Create a ticket with type bug.",
        ));
    }
    if write {
        ensure_project_writable(conn, &t.project)?;
        if t.archived_at.is_some() {
            return Err(conflict(
                "Restore this archived ticket before changing triage or running research.",
            ));
        }
    }
    Ok(t)
}
fn detail(conn: &Connection, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
    let t = ticket(conn, ctx, id, false)?;
    let mut v=conn.query_row("SELECT triage,severity,duplicate_of,note,updated_by,updated_at FROM bug_triage WHERE ticket=?1",[id],|r|Ok(json!({"triage":r.get::<_,String>(0)?,"severity":r.get::<_,String>(1)?,"duplicate_of":r.get::<_,Option<String>>(2)?,"note":r.get::<_,Option<String>>(3)?,"updated_by":r.get::<_,Option<String>>(4)?,"updated_at":r.get::<_,Option<i64>>(5)?}))).optional()?.unwrap_or(json!({"triage":"needs_triage","severity":"unknown","duplicate_of":null,"note":null}));
    v["ticket"] = t.to_json(now_ms());
    let jid: Option<String> = conn
        .query_row(
            "SELECT j.id FROM agent_jobs j JOIN bug_research_jobs b ON b.job=j.id WHERE b.ticket=?1 ORDER BY j.rowid DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    v["latest_job"] = match jid {
        Some(j) => super::agent_chat::inspect_one(conn, ctx, &j)?,
        None => Value::Null,
    };
    Ok(v)
}
fn config(conn: &Connection, project: &str) -> ApiResult<Value> {
    Ok(conn
        .query_row("SELECT repository,revision,enabled FROM bug_research_config WHERE project=?1", [project], |r| {
            Ok(json!({"repository":r.get::<_,String>(0)?,"revision":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?}))
        })
        .optional()?
        .unwrap_or(json!({"repository":project,"revision":"HEAD","enabled":false})))
}
impl Store {
    pub fn list_bugs(
        &self,
        ctx: &AuthCtx,
        project: Option<&str>,
        triage: Option<&str>,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> ApiResult<Value> {
        self.list_bugs_filtered(
            ctx, project, triage, severity, limit, offset, "open", None, None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn list_bugs_filtered(
        &self,
        ctx: &AuthCtx,
        project: Option<&str>,
        triage: Option<&str>,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
        view: &str,
        q: Option<&str>,
        state: Option<&str>,
    ) -> ApiResult<Value> {
        self.list_bugs_advanced(
            ctx, project, triage, severity, limit, offset, view, q, state, None, None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn list_bugs_advanced(
        &self,
        ctx: &AuthCtx,
        project: Option<&str>,
        triage: Option<&str>,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
        view: &str,
        q: Option<&str>,
        state: Option<&str>,
        assignee: Option<&str>,
        research_status: Option<&str>,
    ) -> ApiResult<Value> {
        if research_status.is_some_and(|s| {
            !matches!(
                s,
                "none" | "queued" | "running" | "completed" | "failed" | "cancelled"
            )
        }) {
            return Err(ApiError::validation(
                "validation.bug",
                "Unknown research status.",
            ));
        }
        if !matches!(
            view,
            "open" | "all" | "in_progress" | "needs_triage" | "ready_for_review"
        ) {
            return Err(ApiError::validation("validation.bug", "Unknown bug view."));
        }
        let triage = match view {
            "needs_triage" => Some("needs_triage"),
            "ready_for_review" => Some("ready_for_review"),
            _ => triage,
        };
        ctx.require_scope("read")?;
        if let Some(p) = project {
            ctx.require_project(p)?;
        }
        if !(1..=100).contains(&limit) || offset < 0 {
            return Err(ApiError::validation(
                "validation.bug",
                "Use limit 1–100 and offset >= 0.",
            ));
        }
        self.with_conn(|c| {
            let allowed = ctx.allowed_projects_vec().map(|x| json!(x).to_string());
            let scope = " FROM tickets t LEFT JOIN bug_triage b ON b.ticket=t.id JOIN workflow_states ws ON ws.project=t.project
                AND ws.state=t.state WHERE t.type='bug' AND (?7='all' OR (t.archived_at IS NULL AND ws.terminal=0)) AND
                (?7!='in_progress' OR ws.category='active') AND (?8 IS NULL OR instr(lower(t.title||' '||t.id||'
                '||t.body),lower(?8))>0) AND (?9 IS NULL OR t.state=?9 OR ws.category=?9) AND (?10 IS NULL OR
                (t.claim_holder=?10 AND (t.claim_expires_at IS NULL OR t.claim_expires_at>CAST(strftime('%s','now') AS
                INTEGER)*1000))) AND (?11 IS NULL OR COALESCE((SELECT CASE WHEN br.cancelled=1 THEN 'cancelled' ELSE
                aj.status END FROM bug_research_jobs br JOIN agent_jobs aj ON aj.id=br.job WHERE br.ticket=t.id ORDER BY
                aj.rowid DESC LIMIT 1),'none')=?11) AND (?1 IS NULL OR t.project=?1) AND (?2 IS NULL OR t.project IN
                (SELECT value FROM json_each(?2))) AND (?3 IS NULL OR COALESCE(b.triage,'needs_triage')=?3) AND (?4 IS
                NULL OR COALESCE(b.severity,'unknown')=?4)";
            let total: i64 = c.query_row(
                &format!("SELECT COUNT(*){scope}"),
                params![project, allowed, triage, severity, limit, offset, view, q, state, assignee, research_status],
                |r| r.get(0),
            )?;
            let mut st = c.prepare(&format!(
                "SELECT t.id{scope} ORDER BY CASE COALESCE(b.severity,'unknown') WHEN 'critical' THEN 0 WHEN 'high' THEN 1
                WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END, CASE COALESCE(b.triage,'needs_triage') WHEN
                'needs_triage' THEN 0 ELSE 1 END,t.created_at,t.id LIMIT ?5 OFFSET ?6"
            ))?;
            let ids = st
                .query_map(params![project, allowed, triage, severity, limit, offset, view, q, state, assignee, research_status], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let items = ids.iter().map(|id| detail(c, ctx, id)).collect::<ApiResult<Vec<_>>>()?;
            let mut v = crate::api::paged(items, total, limit, "Use offset to inspect further bug tickets.");
            v["offset"] = json!(offset);
            Ok(v)
        })
    }
    pub fn get_bug(&self, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        self.with_conn(|c| detail(c, ctx, id))
    }
    pub fn patch_bug(&self, ctx: &AuthCtx, id: &str, r: &BugPatch) -> ApiResult<Value> {
        ctx.require_scope("write")?;
        if r.triage.as_deref().is_some_and(|s| {
            !matches!(
                s,
                "needs_triage"
                    | "ready_for_review"
                    | "confirmed"
                    | "needs_information"
                    | "duplicate"
                    | "not_a_bug"
            )
        }) {
            return Err(ApiError::validation(
                "validation.bug",
                "Unknown triage disposition.",
            ));
        }
        if r.severity
            .as_deref()
            .is_some_and(|s| !matches!(s, "unknown" | "critical" | "high" | "medium" | "low"))
        {
            return Err(ApiError::validation("validation.bug", "Unknown severity."));
        }
        if let Some(s) = &r.note {
            bounded(s, 8000, "note")?;
        }
        self.with_tx(|c|{let t=ticket(c,ctx,id,true)?;
if let Some(d)=&r.duplicate_of{let other=ticket(c,ctx,d,false)?;
if d==id||other.project!=t.project{return Err(conflict("Choose another bug in the same project as the duplicate target."));
}}let prior=detail(c,ctx,id)?;
let triage=r.triage.as_deref().unwrap_or(prior["triage"].as_str().unwrap());
let duplicate=if triage=="duplicate"{r.duplicate_of.as_deref().or(prior["duplicate_of"].as_str())}else{None};
if triage=="duplicate"&&duplicate.is_none(){return Err(conflict("A duplicate disposition requires duplicate_of."));
}c.execute("INSERT INTO bug_triage(ticket,triage,severity,duplicate_of,note,updated_by,updated_at)
                VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(ticket) DO UPDATE SET
                triage=excluded.triage,severity=excluded.severity,duplicate_of=excluded.duplicate_of,note=COALESCE(excluded.note,bug_triage.note),updated_by=excluded.updated_by,updated_at=excluded.updated_at",params![id,triage,r.severity.as_deref().unwrap_or(prior["severity"].as_str().unwrap()),duplicate,r.note,ctx.actor,now_ms()])?;
emit_event(c,Some(id),Some(&t.project),&ctx.actor,"bug.triaged",json!({"triage":triage}),now_ms())?;
detail(c,ctx,id)})
    }
    pub fn bug_research_config(&self, ctx: &AuthCtx, project: &str) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        ctx.require_project(project)?;
        self.with_conn(|c| config(c, project))
    }
    pub fn set_bug_research_config(
        &self,
        ctx: &AuthCtx,
        project: &str,
        r: &ResearchConfig,
    ) -> ApiResult<Value> {
        ctx.require_scope("admin")?;
        ctx.require_project(project)?;
        bounded(&r.repository, 120, "repository")?;
        bounded(&r.revision, 200, "revision")?;
        if !r
            .repository
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            return Err(ApiError::validation(
                "validation.bug",
                "repository must be a worker-configured key, not a filesystem path.",
            ));
        }
        self.with_tx(|c| {
            ensure_project_writable(c, project)?;
            c.execute(
                "INSERT INTO bug_research_config(project,repository,revision,enabled) VALUES(?1,?2,?3,?4) ON
                CONFLICT(project) DO UPDATE SET
                repository=excluded.repository,revision=excluded.revision,enabled=excluded.enabled",
                params![project, r.repository, r.revision, r.enabled],
            )?;
            emit_event(c, None, Some(project), &ctx.actor, "bug.research_configured", json!({"enabled":r.enabled}), now_ms())?;
            config(c, project)
        })
    }
    pub fn bug_research(&self, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
        ctx.require_scope("read")?;
        self.with_conn(|c| {
            ticket(c, ctx, id, false)?;
            research(c, ctx, id)
        })
    }
    pub fn start_bug_research(
        &self,
        ctx: &AuthCtx,
        id: &str,
        r: &ResearchStart,
    ) -> ApiResult<Value> {
        ctx.require_scope("write")?;
        bounded(&r.request_id, 120, "request_id")?;
        if let Some(s) = &r.message {
            bounded(s, 8000, "message")?;
        }
        self.with_tx(|c| {
            let t = ticket(c, ctx, id, true)?;
            super::agent_chat::expire(c)?;
            let cfg = config(c, &t.project)?;
            let prompt = r
                .message
                .as_deref()
                .unwrap_or("Research this bug against the configured codebase. Record evidence, uncertainty, reproduction gaps, and recommended triage. Do not change code or ticket workflow.");
            let prior: Option<String> = c
                .query_row(
                    "SELECT j.prompt FROM agent_jobs j JOIN bug_research_jobs b ON b.job=j.id WHERE b.ticket=?1 AND
                j.requested_by=?2 AND j.request_id=?3",
                    params![id, ctx.actor, r.request_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(p) = prior {
                if p != prompt {
                    return Err(conflict("request_id was used with different input. Use a new request_id."));
                }
                return research(c, ctx, id);
            }
            if cfg["enabled"] != true {
                return Err(conflict("Research is disabled. An admin must configure a worker repository and enable bug research for this project."));
            }
            let active: i64 = c.query_row(
                "SELECT COUNT(*) FROM agent_jobs j JOIN agent_conversations a ON a.id=j.conversation_id JOIN
                bug_research_jobs b ON b.job=j.id WHERE a.project=?1 AND j.status IN ('queued','running')",
                [&t.project],
                |r| r.get(0),
            )?;
            if active >= 100 {
                return Err(conflict("This project has reached 100 queued or running bug research jobs. Wait or cancel one."));
            }
            let busy: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM bug_research_jobs b JOIN agent_jobs j ON j.id=b.job WHERE b.ticket=?1 AND
                j.status IN ('queued','running'))",
                [id],
                |r| r.get(0),
            )?;
            if busy {
                return research(c, ctx, id);
            }
            let turns: i64 = c.query_row("SELECT COUNT(*) FROM bug_research_jobs WHERE ticket=?1", [id], |r| r.get(0))?;
            if turns >= 100 {
                return Err(conflict("This bug has reached its research history limit of 100 runs."));
            }
            let now = now_ms();
            c.execute(
                "INSERT INTO agent_conversations(id,mindmap,node,ticket,project,created_at) VALUES(?1,NULL,?2,?2,?3,?4) ON
                CONFLICT(ticket) DO NOTHING",
                params![format!("ac-{}", ticket_suffix(20)), id, t.project, now],
            )?;
            let cid: String = c.query_row("SELECT id FROM agent_conversations WHERE ticket=?1", [id], |r| r.get(0))?;
            c.execute("UPDATE agent_conversations SET thread_id=NULL,service_id=NULL WHERE id=?1", [&cid])?;
            let jid = format!("aj-{}", ticket_suffix(20));
            let snapshot = t.to_json(now).to_string();
            let revision = format!("{:x}", Sha256::digest(snapshot.as_bytes()));
            c.execute(
                "INSERT INTO
                agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at)
                VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",
                params![jid, cid, ctx.actor, r.request_id, prompt, snapshot, revision, now],
            )?;
            c.execute(
                "INSERT INTO bug_research_jobs(job,ticket,repository_ref) VALUES(?1,?2,?3)",
                params![jid, id, json!({"repository":cfg["repository"],"revision":cfg["revision"]}).to_string()],
            )?;
            c.execute(
                "INSERT INTO agent_messages(id,conversation_id,job_id,role,body,created_at) VALUES(?1,?2,?3,'user',?4,?5)",
                params![format!("am-{}", ticket_suffix(20)), cid, jid, prompt, now],
            )?;
            emit_event(c, Some(id), Some(&t.project), &ctx.actor, "agent_job.queued", json!({"job_id":jid,"kind":"bug_research"}), now)?;
            research(c, ctx, id)
        })
    }
    pub fn steer_agent_job(&self, ctx: &AuthCtx, jid: &str, r: &Steering) -> ApiResult<Value> {
        ctx.require_scope("write")?;
        bounded(&r.message, 8000, "message")?;
        bounded(&r.request_id, 120, "request_id")?;
        self.with_tx(|c| {
            let (t, status) = bug_job(c, ctx, jid)?;
            if !matches!(status.as_str(), "queued" | "running") {
                return Err(conflict("Only active research can be steered. Start a new run."));
            }
            let previous: Option<String> = c
                .query_row(
                    "SELECT message FROM agent_steering WHERE job=?1 AND actor=?2 AND request_id=?3",
                    params![jid, ctx.actor, r.request_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if previous != r.message {
                    return Err(conflict("request_id already names different steering."));
                }
                return Ok(json!({"ok":true}));
            }
            let count: i64 = c.query_row("SELECT COUNT(*) FROM agent_steering WHERE job=?1", [jid], |r| r.get(0))?;
            if count >= 50 {
                return Err(conflict("This run has reached 50 steering messages."));
            }
            c.execute(
                "INSERT INTO agent_steering(job,actor,request_id,message,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![jid, ctx.actor, r.request_id, r.message, now_ms()],
            )?;
            emit_event(c, Some(&t.id), Some(&t.project), &ctx.actor, "agent_job.steered", json!({"job_id":jid}), now_ms())?;
            Ok(json!({"ok":true}))
        })
    }
    pub fn cancel_agent_job(&self, ctx: &AuthCtx, jid: &str) -> ApiResult<Value> {
        ctx.require_scope("write")?;
        self.with_tx(|c| {
            let (t, status) = bug_job(c, ctx, jid)?;
            if !matches!(status.as_str(), "queued" | "running") {
                let cancelled: bool = c.query_row("SELECT cancelled FROM bug_research_jobs WHERE job=?1", [jid], |r| r.get(0))?;
                if cancelled {
                    return Ok(json!({"ok":true,"status":"cancelled"}));
                }
                return Err(conflict("This run already finished."));
            }
            c.execute("UPDATE bug_research_jobs SET cancelled=1 WHERE job=?1", [jid])?;
            c.execute("UPDATE agent_jobs SET status='failed',error='Cancelled by request',finished_at=?2 WHERE id=?1", params![jid, now_ms()])?;
            emit_event(c, Some(&t.id), Some(&t.project), &ctx.actor, "agent_job.cancelled", json!({"job_id":jid}), now_ms())?;
            Ok(json!({"ok":true,"status":"cancelled"}))
        })
    }
}
fn bug_job(c: &Connection, ctx: &AuthCtx, jid: &str) -> ApiResult<(super::Ticket, String)> {
    let (id, status): (String, String) = c
        .query_row("SELECT b.ticket,j.status FROM bug_research_jobs b JOIN agent_jobs j ON j.id=b.job WHERE b.job=?1", [jid], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .ok_or_else(|| ApiError::not_found("bug_research", jid))?;
    Ok((ticket(c, ctx, &id, true)?, status))
}
fn research(c: &Connection, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
    let mut stmt = c.prepare(
        "SELECT job FROM bug_research_jobs WHERE ticket=?1 ORDER BY rowid DESC LIMIT 100",
    )?;
    let ids = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let jobs = ids
        .iter()
        .map(|id| super::agent_chat::inspect_one(c, ctx, id))
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(json!({"total":jobs.len(),"limit":100,"jobs":jobs}))
}
