use super::{helpers::ensure_project_writable, Store};
use crate::{
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, ticket_suffix},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
fn conflict() -> ApiError {
    ApiError::conflict("conflict.codebase_import","The extraction changed or its lease expired. Refresh its status; do not automatically start another model run.")
}
impl Store {
    pub fn codebase_telemetry(
        &self,
        ctx: &AuthCtx,
        id: &str,
        service: &str,
        attempt: &str,
        telemetry: Option<&Value>,
    ) -> ApiResult<()> {
        self.with_tx(|conn| {
        let project:String=conn.query_row("SELECT project FROM codebase_import_jobs WHERE id=?1 AND service_id=?2 AND attempt_id=?3 AND status='running' AND lease_expires_at>?4 AND deadline>?4",params![id,service,attempt,now_ms()],|r|r.get(0)).optional()?.ok_or_else(conflict)?;
        ctx.require_project(&project)?;
        ensure_project_writable(conn,&project)?;
        super::agent_usage::save(conn, id, telemetry)
        })
    }
    pub fn codebase_project_writable(&self, project: &str) -> ApiResult<()> {
        ensure_project_writable(&self.conn.lock().unwrap(), project)
    }
    pub fn github_connections(&self) -> ApiResult<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT installation,account,management_url FROM github_connections ORDER BY account LIMIT 100",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({"id":r.get::<_,u64>(0)?,"account":r.get::<_,String>(1)?,"management_url":r.get::<_,String>(2)?}))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn github_connect(&self, id: u64, account: &str, management_url: &str) -> ApiResult<()> {
        let conn = self.conn.lock().unwrap();
        let full:bool=conn.query_row("SELECT (SELECT count(*) FROM github_connections)>=100 AND NOT EXISTS(SELECT 1 FROM github_connections WHERE installation=?1)",[id],|r|r.get(0))?;
        if full {
            return Err(ApiError::validation("validation.github","This deployment supports at most 100 connected installations. Disconnect an unused account before adding another."));
        }
        conn.execute("INSERT INTO github_connections(installation,account,updated_at,management_url) VALUES(?1,?2,?3,?4) ON CONFLICT(installation) DO UPDATE SET account=excluded.account,updated_at=excluded.updated_at,management_url=excluded.management_url",params![id,account,now_ms(),management_url])?;
        Ok(())
    }
    pub fn github_disconnect(&self, id: u64) -> ApiResult<()> {
        self.with_tx(|tx| {
        tx.execute("UPDATE codebase_import_jobs SET status='failed',error='GitHub connection removed. Reconnect before explicitly starting a new extraction.' WHERE status IN ('queued','running') AND json_extract(source,'$.installation')=?1",[id])?;
        tx.execute("DELETE FROM github_connections WHERE installation=?1", [id])?;

        Ok(())
            })
    }
    pub fn project_repository(&self, project: &str) -> ApiResult<Option<Value>> {
        let conn = self.conn.lock().unwrap();
        let row=conn.query_row("SELECT installation,repository,full_name,scope FROM project_repositories WHERE project=?1",[project],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
        Ok(row.map(|(installation,repository,full_name,scope)|json!({"installation":installation,"repository":repository,"full_name":full_name,"scope":serde_json::from_str::<Value>(&scope).unwrap_or(Value::Null)})))
    }
    pub fn set_project_repository(&self, project: &str, body: &Value) -> ApiResult<()> {
        let conn = self.conn.lock().unwrap();
        ensure_project_writable(&conn, project)?;
        conn.execute("INSERT INTO project_repositories VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project) DO UPDATE SET installation=excluded.installation,repository=excluded.repository,full_name=excluded.full_name,scope=excluded.scope",params![project,body["installation"].as_u64(),body["repository"].as_u64(),body["full_name"].as_str(),body["scope"].to_string()])?;
        Ok(())
    }
    pub fn codebase_imports(&self, project: &str) -> ApiResult<Value> {
        let conn = self.conn.lock().unwrap();
        expire(&conn)?;
        let total: i64 = conn.query_row(
            "SELECT count(*) FROM codebase_import_jobs WHERE project=?1",
            [project],
            |r| r.get(0),
        )?;
        let mut stmt=conn.prepare("SELECT id,status,mindmap,error,created_at FROM codebase_import_jobs WHERE project=?1 ORDER BY created_at DESC LIMIT 20")?;
        let items=stmt.query_map([project],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"mindmap":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?,"created_at":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
        Ok(
            json!({"items":items,"total":total,"limit":20,"note":"Newest 20 extraction runs. A failed run is never retried automatically."}),
        )
    }
    pub fn existing_codebase_import(
        &self,
        ctx: &AuthCtx,
        project: &str,
        map: &str,
        request: &str,
    ) -> ApiResult<Option<Value>> {
        let conn = self.conn.lock().unwrap();
        let old=conn.query_row("SELECT id,mindmap,actor FROM codebase_import_jobs WHERE project=?1 AND request_id=?2",params![project,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        match old {
            Some((id, oldmap, actor)) if oldmap == map && actor == ctx.actor => {
                Ok(Some(json!({"id":id})))
            }
            Some(_) => Err(conflict()),
            None => Ok(None),
        }
    }
    pub fn enqueue_codebase_import(
        &self,
        ctx: &AuthCtx,
        project: &str,
        map: &str,
        request: &str,
        source: &Value,
    ) -> ApiResult<Value> {
        self.with_tx(|tx| {
        ensure_project_writable(tx, project)?;
        expire(tx)?;
        let old=tx.query_row("SELECT id,mindmap,actor FROM codebase_import_jobs WHERE project=?1 AND request_id=?2",params![project,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((id, oldmap, actor)) = old {
            if oldmap != map || actor != ctx.actor {
                return Err(conflict());
            }
            return Ok(json!({"id":id}));
        }
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM codebase_import_jobs WHERE project=?1 AND status IN ('queued','running'))",[project],|r|r.get(0))?;
        if active {
            return Err(conflict());
        }
        let id = format!("ci-{}", ticket_suffix(20));
        tx.execute("INSERT INTO codebase_import_jobs(id,project,mindmap,actor,request_id,status,source,created_at) VALUES(?1,?2,?3,?4,?5,'queued',?6,?7)",params![id,project,map,ctx.actor,request,source.to_string(),now_ms()])?;

        Ok(json!({"id":id}))
            })
    }
    pub fn claim_codebase_import(&self, ctx: &AuthCtx, service: &str) -> ApiResult<Value> {
        self.with_tx(|tx| {
        expire(tx)?;
        let allowed = ctx
            .projects
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap());
        let mut stmt=tx.prepare("SELECT id,project,mindmap,source FROM codebase_import_jobs WHERE status='queued' AND (?1 IS NULL OR project IN (SELECT value FROM json_each(?1))) ORDER BY created_at LIMIT 1")?;
        let candidates = stmt
            .query_map([allowed], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for (id, project, map, source) in candidates {
            if !ctx.can_project(&project) {
                continue;
            }
            ensure_project_writable(tx, &project)?;
            let attempt = ticket_suffix(32);
            tx.execute("UPDATE codebase_import_jobs SET status='running',service_id=?2,attempt_id=?3,lease_expires_at=?4,deadline=?5 WHERE id=?1",params![id,service,attempt,now_ms()+60_000,now_ms()+300_000])?;
            tx.execute("INSERT INTO agent_run_usage(job,telemetry,updated_at,started_at) VALUES(?1,'{}',?2,?2)",params![id,now_ms()])?;

            return Ok(
                json!({"job":{"id":id,"project":project,"mindmap":map,"source":serde_json::from_str::<Value>(&source).unwrap_or(Value::Null),"attempt_id":attempt}}),
            );
        }

        Ok(json!({"job":null}))
            })
    }
    pub fn codebase_lease(
        &self,
        ctx: &AuthCtx,
        id: &str,
        service: &str,
        attempt: &str,
        renew: bool,
    ) -> ApiResult<Value> {
        let conn = self.conn.lock().unwrap();
        expire(&conn)?;
        let row=conn.query_row("SELECT project,mindmap,actor,source,status,result FROM codebase_import_jobs WHERE id=?1 AND service_id=?2 AND attempt_id=?3",params![id,service,attempt],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?))).optional()?.ok_or_else(conflict)?;
        ctx.require_project(&row.0)?;
        ensure_project_writable(&conn, &row.0)?;
        if row.4 != "running" && row.4 != "completed" {
            return Err(conflict());
        }
        if renew && row.4 == "running" {
            conn.execute(
                "UPDATE codebase_import_jobs SET lease_expires_at=?2 WHERE id=?1",
                params![id, now_ms() + 60_000],
            )?;
        }
        Ok(
            json!({"project":row.0,"mindmap":row.1,"actor":row.2,"source":serde_json::from_str::<Value>(&row.3).unwrap_or(Value::Null),"status":row.4,"result":row.5.and_then(|s|serde_json::from_str::<Value>(&s).ok())}),
        )
    }
    pub fn finish_codebase_import(
        &self,
        id: &str,
        attempt: &str,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> ApiResult<()> {
        self.with_tx(|tx| {
        let n=tx.execute("UPDATE codebase_import_jobs SET status=?3,result=?4,error=?5 WHERE id=?1 AND attempt_id=?2 AND status='running'",params![id,attempt,if result.is_some(){"completed"}else{"failed"},result.map(Value::to_string),error])?;
        if n == 0 {
            return Err(conflict());
        }

        Ok(())
            })
    }
}
fn expire(conn: &rusqlite::Connection) -> ApiResult<()> {
    conn.execute("UPDATE codebase_import_jobs SET status='failed',error='Extraction interrupted or project archived. Review the document before explicitly starting another run.' WHERE (status='running' AND (lease_expires_at<=?1 OR deadline<=?1)) OR (status IN ('queued','running') AND project IN (SELECT id FROM projects WHERE archived_at IS NOT NULL))",[now_ms()])?;
    Ok(())
}

pub(super) fn inspect(conn: &rusqlite::Connection, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
    let raw:String=conn.query_row("SELECT json_object('id',id,'project',project,'kind','codebase_import','conversation_id','','conversation_service_id',NULL,'mindmap',mindmap,'node','','section_title','Repository extraction','status',status,'requested_by',actor,'created_at',created_at,'finished_at',NULL,'lease_expires_at',lease_expires_at,'deadline',deadline,'service_id',service_id,'attempt_id',attempt_id,'thread_id',NULL,'turn_id',NULL,'error',error,'source_revision',json_extract(source,'$.revision'),'source',json(source),'prompt','Recover implemented behavior from the selected repository scope.','snapshot',source,'response',result) FROM codebase_import_jobs WHERE id=?1",[id],|r|r.get(0)).optional()?.ok_or_else(||ApiError::not_found("agent job",id))?;
    let value: Value = serde_json::from_str(&raw).map_err(|e| ApiError::internal(e.to_string()))?;
    ctx.require_project(value["project"].as_str().unwrap())?;
    Ok(value)
}
