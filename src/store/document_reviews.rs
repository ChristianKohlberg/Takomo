//! Reviews route existing collaborative threads; there is only one conversation.
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
use yrs::{types::ToJson, Any, Doc, Map, MapPrelim, Out, Transact};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftComment {
    pub id: String,
    pub section_id: String,
    pub anchor: Value,
    pub text: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendReview {
    pub request_id: String,
    pub title: String,
    #[serde(default = "review_kind")]
    pub kind: String,
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(default)]
    pub comments: Vec<DraftComment>,
    #[serde(default)]
    pub thread_ids: Vec<String>,
}
fn review_kind() -> String {
    "review".into()
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAction {
    pub request_id: String,
    pub version: i64,
    pub action: String,
    pub thread_id: Option<String>,
    pub text: Option<String>,
    pub recipients: Option<Vec<String>>,
}
pub fn invalid(message: &str) -> ApiError {
    ApiError::validation("validation.document_review", message)
        .remedy("Check the request fields and retry.")
}
fn conflict(message: &str) -> ApiError {
    ApiError::conflict("conflict.document_review", message)
        .remedy("Reload the review before retrying.")
}
pub fn identity(ctx: &AuthCtx) -> String {
    ctx.user
        .as_ref()
        .map(|id| format!("user:{id}"))
        .unwrap_or_else(|| format!("token:{}", ctx.token_id))
}
fn bounded(value: &str, max: usize) -> ApiResult<()> {
    if value.trim().is_empty() || value.chars().count() > max {
        return Err(invalid(
            "A required field is empty or exceeds its length limit.",
        ));
    }
    Ok(())
}
impl SendReview {
    pub fn validate(&self) -> ApiResult<()> {
        bounded(&self.request_id, 120)?;
        bounded(&self.title, 300)?;
        if !["review", "question", "change", "mention"].contains(&self.kind.as_str()) {
            return Err(invalid("Unknown review kind."));
        }
        if self.comments.len() + self.thread_ids.len() == 0
            || self.comments.len() + self.thread_ids.len() > 100
        {
            return Err(invalid("Send between 1 and 100 comments."));
        }
        if self.kind != "review" && self.comments.len() + self.thread_ids.len() != 1 {
            return Err(invalid(
                "A request or notification addresses one comment thread.",
            ));
        }
        if self.kind == "change" && self.recipients.len() != 1 {
            return Err(invalid(
                "Choose exactly one responsible person for a change request.",
            ));
        }
        if self.kind == "mention" && self.recipients.is_empty() {
            return Err(invalid("Choose someone to notify."));
        }
        if self.recipients.len() > 20 {
            return Err(invalid("Choose at most 20 people."));
        }
        let mut ids = std::collections::HashSet::new();
        for id in self
            .thread_ids
            .iter()
            .chain(self.comments.iter().map(|c| &c.id))
        {
            bounded(id, 120)?;
            if !ids.insert(id) {
                return Err(invalid("Comment IDs must be unique."));
            }
        }
        for c in &self.comments {
            bounded(&c.section_id, 120)?;
            bounded(&c.text, 5000)?;
            bounded(c.anchor["quote"].as_str().unwrap_or(""), 5000)?;
            if !c.anchor["start"].is_object()
                || !c.anchor["end"].is_object()
                || c.anchor.to_string().len() > 20000
            {
                return Err(invalid("Invalid comment anchor."));
            }
        }
        Ok(())
    }
}
fn recipients(
    conn: &Connection,
    ctx: &AuthCtx,
    project: &str,
    names: &[String],
) -> ApiResult<Vec<Value>> {
    if names.len() > 20 {
        return Err(invalid("Choose at most 20 people."));
    }
    let mut out = vec![];
    for name in names {
        let u = Store::assignable_user(conn, name, project)?;
        if !out.iter().any(|v: &Value| v["id"] == u.id) {
            out.push(json!({"id":u.id,"label":if u.name.is_empty(){&u.handle}else{&u.name}}));
        }
    }
    ctx.require_project(project)?;
    Ok(out)
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let parse = |i| -> rusqlite::Result<Value> {
        let s: String = r.get(i)?;
        serde_json::from_str(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    Ok(
        json!({"id":r.get::<_,String>(0)?,"project":r.get::<_,String>(1)?,"mindmap":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,"creator":r.get::<_,String>(5)?,"creator_key":r.get::<_,String>(6)?,"recipients":parse(7)?,"thread_ids":parse(8)?,"snapshot":parse(9)?,"status":r.get::<_,String>(10)?,"version":r.get::<_,i64>(11)?,"created_at":r.get::<_,i64>(12)?,"updated_at":r.get::<_,i64>(13)?}),
    )
}
const COLUMNS:&str="id,project,mindmap,kind,title,creator,creator_key,recipients,thread_ids,snapshot,status,version,created_at,updated_at";
fn get(conn: &Connection, id: &str) -> ApiResult<Value> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM document_reviews WHERE id=?1"),
        [id],
        row,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("review", id))
}
fn decorate(conn: &Connection, ctx: &AuthCtx, mut v: Value) -> ApiResult<Value> {
    let id = v["id"].as_str().unwrap();
    let key = identity(ctx);
    let responded: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM document_review_responses WHERE review=?1 AND identity=?2 AND completed=1)",
        params![id, key],
        |r| r.get(0),
    )?;
    let participated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM document_review_responses WHERE review=?1 AND identity=?2)",
        params![id, key],
        |r| r.get(0),
    )?;
    let mine = v["creator_key"] == key;
    let addressed = v["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| Some(p["id"].as_str().unwrap()) == ctx.user.as_deref());
    let shared = v["recipients"].as_array().unwrap().is_empty();
    v["can_work"] = json!(addressed || ctx.scopes.contains("admin"));
    v["is_creator"] = json!(mine);
    v["responded"] = json!(responded);
    v["needs_me"] = json!(
        v["status"] != "closed"
            && v["kind"] != "mention"
            && if v["status"] == "ready" {
                mine
            } else {
                (addressed || shared) && !responded
            }
    );
    v["following"] = json!(mine || addressed || participated);
    v.as_object_mut().unwrap().remove("creator_key");
    Ok(v)
}
impl Store {
    pub fn review(&self, ctx: &AuthCtx, id: &str) -> ApiResult<Value> {
        self.with_conn(|c| {
            let v = get(c, id)?;
            ctx.require_project(v["project"].as_str().unwrap())?;
            decorate(c, ctx, v)
        })
    }
    pub fn review_retry(
        &self,
        ctx: &AuthCtx,
        map: &str,
        req: &SendReview,
    ) -> ApiResult<Option<String>> {
        self.with_conn(|c|{let found:Option<(String,String)>=c.query_row("SELECT id,intent FROM document_reviews WHERE mindmap=?1 AND creator_key=?2 AND request_id=?3",params![map,identity(ctx),req.request_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            match found {Some((id,intent)) if intent==serde_json::to_string(req).unwrap()=>Ok(Some(id)),Some(_)=>Err(conflict("This request ID was already used for different review content.")),None=>Ok(None)}})
    }
    pub fn save_review(
        &self,
        ctx: &AuthCtx,
        map: &str,
        req: &SendReview,
        id: &str,
        threads: &Value,
        blob: &[u8],
    ) -> ApiResult<(i64, i64)> {
        req.validate()?;
        let now = now_ms();
        self.append_collab_update_with(map,blob,&ctx.actor,|c|{
            let project:String=c.query_row("SELECT project FROM mindmaps WHERE id=?1",[map],|r|r.get(0))?;
            ctx.require_project(&project)?;ensure_project_writable(c,&project)?;
            let people=recipients(c,ctx,&project,&req.recipients)?;
            let ids:Vec<&str>=threads.as_array().unwrap().iter().map(|t|t["id"].as_str().unwrap()).collect();
            let previous:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM document_reviews WHERE mindmap=?1 AND creator_key=?2 AND request_id=?3)",params![map,identity(ctx),req.request_id],|r|r.get(0))?;
            if previous{return Err(conflict("Review already sent. Retry the same request to read it."));}
            c.execute("INSERT INTO document_reviews(id,project,mindmap,kind,title,creator,creator_key,recipients,thread_ids,snapshot,request_id,intent,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",params![id,project,map,req.kind,req.title.trim(),ctx.actor,identity(ctx),json!(people).to_string(),json!(ids).to_string(),threads.to_string(),req.request_id,serde_json::to_string(req).unwrap(),now])?;
            emit_event(c,None,Some(&project),&ctx.actor,"document.review_sent",json!({"review":id,"mindmap":map}),now)?; Ok(())
        }).map(|(rows,seq,())|(rows,seq))
    }
    pub fn review_list(
        &self,
        ctx: &AuthCtx,
        project: Option<&str>,
        map: Option<&str>,
        queue: &str,
        limit: usize,
        offset: usize,
    ) -> ApiResult<Value> {
        if let Some(p) = project {
            ctx.require_project(p)?;
        }
        if !["all", "needs_me", "following", "mine", "shared"].contains(&queue) {
            return Err(invalid("Unknown review inbox queue."));
        }
        self.with_conn(|c|{
            // Scope and queue filters are applied before counting and pagination.
            let allowed=ctx.allowed_projects_vec(); let allowed_json=serde_json::to_string(&allowed.clone().unwrap_or_default()).unwrap();
            let filter="(?1 IS NULL OR project=?1) AND (?2 IS NULL OR mindmap=?2) AND (?3 OR project IN (SELECT value FROM json_each(?4))) AND CASE ?5
              WHEN 'mine' THEN creator_key=?6
              WHEN 'shared' THEN json_array_length(recipients)=0 AND status!='closed'
              WHEN 'following' THEN creator_key=?6 OR EXISTS(SELECT 1 FROM json_each(recipients) WHERE json_extract(value,'$.id')=?7) OR EXISTS(SELECT 1 FROM document_review_responses WHERE review=document_reviews.id AND identity=?6)
              WHEN 'needs_me' THEN status!='closed' AND kind!='mention' AND CASE WHEN status='ready' THEN creator_key=?6 ELSE (json_array_length(recipients)=0 OR EXISTS(SELECT 1 FROM json_each(recipients) WHERE json_extract(value,'$.id')=?7)) AND NOT EXISTS(SELECT 1 FROM document_review_responses WHERE review=document_reviews.id AND identity=?6 AND completed=1) END
              ELSE 1 END";
            let key=identity(ctx);
            let total:i64=c.query_row(&format!("SELECT count(*) FROM document_reviews WHERE {filter}"),params![project,map,allowed.is_none(),allowed_json,queue,key,ctx.user],|r|r.get(0))?;
            let mut stmt=c.prepare(&format!("SELECT {COLUMNS} FROM document_reviews WHERE {filter} ORDER BY updated_at DESC,id LIMIT ?8 OFFSET ?9"))?;
            let values=stmt.query_map(params![project,map,allowed.is_none(),allowed_json,queue,key,ctx.user,limit as i64,offset as i64],row)?.collect::<Result<Vec<_>,_>>()?;
            let items=values.into_iter().map(|v|decorate(c,ctx,v)).collect::<ApiResult<Vec<_>>>()?;
            Ok(json!({"items":items,"total":total,"limit":limit,"offset":offset,"truncated":((offset+limit) as i64)<total}))
        })
    }
    pub fn review_action_retry(
        &self,
        ctx: &AuthCtx,
        id: &str,
        req: &ReviewAction,
    ) -> ApiResult<bool> {
        self.with_conn(|c|{
        let intent:Option<String>=c.query_row("SELECT intent FROM document_review_actions WHERE review=?1 AND identity=?2 AND request_id=?3",params![id,identity(ctx),req.request_id],|r|r.get(0)).optional()?;
        match intent{Some(i) if i==serde_json::to_string(req).unwrap()=>Ok(true),Some(_)=>Err(conflict("This action ID was used with different content.")),None=>Ok(false)}
    })
    }
    pub fn save_review_action(
        &self,
        ctx: &AuthCtx,
        id: &str,
        req: &ReviewAction,
        threads: &Value,
        blob: &[u8],
    ) -> ApiResult<(i64, i64)> {
        let previous = self.review(ctx, id)?;
        let map = previous["mindmap"].as_str().unwrap();
        self.append_collab_update_with(map,blob,&ctx.actor,|c|{
            let v=get(c,id)?;let project=v["project"].as_str().unwrap();ctx.require_project(project)?;
            if v["version"]!=req.version{return Err(conflict("The review changed while you were responding."));}
            let creator=v["creator_key"]==identity(ctx);let admin=ctx.scopes.contains("admin");
            let owner=v["recipients"].as_array().unwrap().first().and_then(|p|p["id"].as_str());
            let responsible=owner==ctx.user.as_deref() && owner.is_some();
            let status=v["status"].as_str().unwrap(); let kind=v["kind"].as_str().unwrap();
            if status=="closed" && !["reopen","reply","reviewed"].contains(&req.action.as_str()){return Err(conflict("Reopen the review before changing it."));}
            let mut next=status;
            match req.action.as_str(){
                "reply"|"resolve"|"unresolve"|"reviewed"=>{},
                "start" if kind=="change" && (responsible||admin) && status=="open"=>next="in_progress",
                "ready" if kind=="change" && (responsible||admin) && status=="in_progress"=>next="ready",
                "close" if kind=="question"=>next="closed",
                "close" if (creator||admin) && (kind!="change"||status=="ready")=>{
                    if kind=="review" && threads.as_array().unwrap().iter().any(|t|t["resolved"]!=true){return Err(conflict("Resolve every comment before finishing this review."));} next="closed";
                },
                "reopen" if creator||admin=>{next="open";c.execute("DELETE FROM document_review_responses WHERE review=?1",[id])?;},
                "assign" if creator||admin=>{let names=req.recipients.as_deref().ok_or_else(||invalid("Choose recipients."))?;if (kind=="change"&&names.len()!=1)||(kind=="mention"&&names.is_empty()){return Err(invalid("A change request needs one owner."));}let people=recipients(c,ctx,project,names)?;c.execute("UPDATE document_reviews SET recipients=?2 WHERE id=?1",params![id,json!(people).to_string()])?;c.execute("DELETE FROM document_review_responses WHERE review=?1",[id])?;},
                _=>return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN,"auth.review_action","This action is not available to you in the review's current state.")),
            }
            if req.action=="close" && kind=="question" && req.text.as_deref().unwrap_or("").trim().is_empty(){return Err(invalid("Write the answer before closing this question."));}
            if ["reply","reviewed","close"].contains(&req.action.as_str()) {c.execute("INSERT INTO document_review_responses(review,identity,completed) VALUES (?1,?2,?3) ON CONFLICT(review,identity) DO UPDATE SET completed=MAX(completed,excluded.completed)",params![id,identity(ctx),req.action!="reply"])?;}
            c.execute("UPDATE document_reviews SET snapshot=?2,status=?3,version=version+1,updated_at=?4 WHERE id=?1",params![id,threads.to_string(),next,now_ms()])?;
            c.execute("INSERT INTO document_review_actions(review,identity,request_id,intent) VALUES (?1,?2,?3,?4)",params![id,identity(ctx),req.request_id,serde_json::to_string(req).unwrap()])?;
            emit_event(c,None,Some(project),&ctx.actor,"document.review_updated",json!({"review":id,"action":req.action}),now_ms())?;Ok(())
        }).map(|(rows,seq,())|(rows,seq))
    }
}

fn any(value: &Value) -> ApiResult<Any> {
    serde_json::from_value(value.clone()).map_err(|e| ApiError::internal(e.to_string()))
}
/// Return the same shape consumed by the existing comment panel.
pub fn threads(doc: &Doc, ids: &[String]) -> ApiResult<Value> {
    let map = doc.get_or_insert_map("documentComments");
    let tx = doc.transact();
    let mut out = vec![];
    for id in ids {
        let Some(Out::YMap(entry)) = map.get(&tx, id) else {
            return Err(conflict(
                "A comment was removed. Open its source and review the remaining comments.",
            ));
        };
        let mut v = serde_json::to_value(entry.to_json(&tx))
            .map_err(|e| ApiError::internal(e.to_string()))?;
        v["id"] = json!(id);
        let mut messages: Vec<Value> = v["messages"]
            .as_object()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        messages.sort_by_key(|m| m["created"].as_i64().unwrap_or(0));
        v["messages"] = json!(messages);
        out.push(v);
    }
    Ok(json!(out))
}
pub fn publish(doc: &Doc, ctx: &AuthCtx, req: &SendReview) -> ApiResult<Value> {
    req.validate()?;
    let nodes = doc.get_or_insert_map("nodes");
    let comments = doc.get_or_insert_map("documentComments");
    {
        let mut tx = doc.transact_mut();
        for c in &req.comments {
            if !nodes.contains_key(&tx, &c.section_id) {
                return Err(ApiError::not_found("section", &c.section_id));
            }
            if comments.contains_key(&tx, &c.id) {
                return Err(conflict("This comment ID already exists."));
            }
            let entry = comments.insert(&mut tx, c.id.as_str(), MapPrelim::default());
            entry.insert(&mut tx, "sectionId", c.section_id.as_str());
            entry.insert(&mut tx, "anchor", any(&c.anchor)?);
            entry.insert(&mut tx, "resolved", false);
            let messages = entry.insert(&mut tx, "messages", MapPrelim::default());
            let msg = format!("msg-{}", ticket_suffix(16));
            messages.insert(
                &mut tx,
                msg.as_str(),
                any(&json!({"id":msg,"author":ctx.actor,"text":c.text.trim(),"created":now_ms()}))?,
            );
        }
    }
    let ids: Vec<_> = req
        .thread_ids
        .iter()
        .cloned()
        .chain(req.comments.iter().map(|c| c.id.clone()))
        .collect();
    threads(doc, &ids)
}
pub fn apply_action(
    doc: &Doc,
    ctx: &AuthCtx,
    review: &Value,
    req: &ReviewAction,
) -> ApiResult<Value> {
    bounded(&req.request_id, 120)?;
    let ids: Vec<String> = serde_json::from_value(review["thread_ids"].clone())
        .map_err(|_| invalid("Invalid review threads."))?;
    if let Some(id) = &req.thread_id {
        if !ids.contains(id) {
            return Err(invalid("This comment does not belong to the review."));
        }
    }
    let target = req.thread_id.as_ref().or_else(|| ids.first());
    if ["reply", "resolve", "unresolve"].contains(&req.action.as_str()) && req.thread_id.is_none() {
        return Err(invalid("Choose a comment thread."));
    }
    if let Some(text) = &req.text {
        bounded(text, 5000)?;
    }
    if req.action == "reply" && req.text.is_none() {
        return Err(invalid("Write a reply."));
    }
    let comments = doc.get_or_insert_map("documentComments");
    {
        let mut tx = doc.transact_mut();
        if let Some(id) = target {
            let Some(Out::YMap(entry)) = comments.get(&tx, id) else {
                return Err(conflict("The source comment was removed."));
            };
            if let Some(text) = &req.text {
                let Some(Out::YMap(messages)) = entry.get(&tx, "messages") else {
                    return Err(invalid("Invalid comment messages."));
                };
                let msg = format!("msg-{}", ticket_suffix(16));
                messages.insert(
                    &mut tx,
                    msg.as_str(),
                    any(
                        &json!({"id":msg,"author":ctx.actor,"text":text.trim(),"created":now_ms()}),
                    )?,
                );
            }
            if ["resolve", "unresolve"].contains(&req.action.as_str()) {
                entry.insert(&mut tx, "resolved", req.action == "resolve");
            }
        }
    }
    threads(doc, &ids)
}
