use super::Store;
use crate::{
    api::codex_connection::{Command, Poll},
    auth::AuthCtx,
    error::{ApiError, ApiResult},
    ids::{now_ms, ticket_suffix},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
fn invalid() -> ApiError {
    ApiError::validation(
        "validation.codex_connection",
        "Choose a supported action and a request ID of 1–120 characters.",
    )
}
fn expire(c: &rusqlite::Connection) -> ApiResult<()> {
    c.execute("UPDATE codex_connections SET action=NULL,report=json_object('status','error','error','Connection request expired. Start a new request.') WHERE action IS NOT NULL AND expires_at<=?1",[now_ms()])?;
    Ok(())
}
fn view(c: &rusqlite::Connection, id: &str) -> ApiResult<Value> {
    let raw:String=c.query_row("SELECT json_object('id',id,'service_id',service_id,'projects',json(projects),'seen_at',seen_at,'busy',json(CASE busy WHEN 1 THEN 'true' ELSE 'false' END),'report',json(report),'requested_by',requested_by,'command_id',command_id,'action',action,'expires_at',expires_at) FROM codex_connections WHERE id=?1",[id],|r|r.get(0)).optional()?.ok_or_else(||ApiError::not_found("codex_connection",id))?;
    serde_json::from_str(&raw).map_err(|e| ApiError::internal(e.to_string()))
}
impl Store {
    pub fn codex_connections(&self) -> ApiResult<Value> {
        self.account_transaction(|c| {
            expire(c)?;
            let mut q =
                c.prepare("SELECT id FROM codex_connections ORDER BY seen_at DESC LIMIT 100")?;
            let ids = q
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let items = ids
                .iter()
                .map(|id| view(c, id))
                .collect::<ApiResult<Vec<_>>>()?;
            Ok(json!({"items":items}))
        })
    }
    pub fn codex_command(&self, id: &str, req: &Command, actor: &str) -> ApiResult<Value> {
        if !matches!(
            req.action.as_str(),
            "login" | "refresh" | "logout" | "cancel"
        ) || req.request_id.is_empty()
            || req.request_id.len() > 120
        {
            return Err(invalid());
        }
        self.account_transaction(|c|{expire(c)?;let current=view(c,id)?;
 if current["command_id"]==req.request_id {return Ok(current);}
 if !current["action"].is_null() && req.action!="cancel" {return Err(ApiError::conflict("conflict.codex_connection","A connection request is already pending. Cancel it or wait for completion."));}
 c.execute("UPDATE codex_connections SET command_id=?2,action=?3,expires_at=?4,requested_by=?5,report=json_remove(report,'$.device','$.error') WHERE id=?1",params![id,req.request_id,req.action,now_ms()+600_000,actor])?;
 view(c,id)
 })
    }
    pub fn codex_poll(&self, ctx: &AuthCtx, req: &Poll) -> ApiResult<Value> {
        if req.service_id.is_empty()
            || req.service_id.len() > 120
            || req.command_id.as_ref().is_some_and(|v| v.len() > 120)
        {
            return Err(invalid());
        }
        self.account_transaction(|c|{expire(c)?;
 let old:Option<String>=c.query_row("SELECT id FROM codex_connections WHERE token_id=?1 AND service_id=?2",params![ctx.token_id,req.service_id],|r|r.get(0)).optional()?;
 if old.is_none() && c.query_row("SELECT count(*) FROM codex_connections",[],|r|r.get::<_,i64>(0))? >= 100 {return Err(ApiError::validation("validation.codex_connection","This instance supports at most 100 registered Codex workers."));}
 let id=old.unwrap_or_else(||format!("cc-{}",ticket_suffix(20)));
 let projects=ctx.allowed_projects_vec().map_or(json!("*"),|p|json!(p));
 c.execute("INSERT INTO codex_connections(id,token_id,service_id,projects,seen_at,busy) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(token_id,service_id) DO UPDATE SET projects=excluded.projects,seen_at=excluded.seen_at,busy=excluded.busy",params![id,ctx.token_id,req.service_id,projects.to_string(),now_ms(),req.busy])?;
 if let Some(report)=&req.report {
  let current=view(c,&id)?;
  let matches=current["command_id"].as_str()==req.command_id.as_deref();
  if (matches && !current["action"].is_null()) || (req.command_id.is_none() && current["action"].is_null()) {
   let pending=report.status=="login_pending";
   if !pending || current["action"]=="login" {
    c.execute("UPDATE codex_connections SET report=?2,action=CASE WHEN ?3 THEN action ELSE NULL END WHERE id=?1",params![id,serde_json::to_string(report).unwrap(),pending])?;
   }
  }
 }
 view(c,&id)
 })
    }
}
