//! Immutable execution attempts over collaboratively edited definitions.
use super::{
    checklist,
    helpers::{emit_event, ensure_project_writable},
    mindmapdoc, Store,
};
use crate::{
    error::{ApiError, ApiResult},
    ids::{iso, now_ms, ticket_suffix},
};
use rmcp::schemars;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use yrs::{updates::decoder::Decode, Doc, GetString, Transact, Update};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub check: String,
    pub definition_revision: String,
    pub specification_revision: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCreate {
    pub definitions: Vec<Selection>,
    pub environment: Option<String>,
    pub code_ref: String,
    pub idempotency_key: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultCreate {
    pub case: String,
    pub actor_kind: String,
    pub verdict: String,
    pub note: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub idempotency_key: String,
}
fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::validation("validation.test_run", message)
}
fn conflict(message: impl Into<String>) -> ApiError {
    ApiError::conflict("conflict.test_run", message)
}
fn hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn parse(raw: String) -> ApiResult<Value> {
    serde_json::from_str(&raw).map_err(|e| ApiError::internal(e.to_string()))
}
fn id(prefix: &str) -> String {
    format!("{prefix}-{}", ticket_suffix(16))
}
fn bounded(value: &str, max: usize, name: &str) -> ApiResult<()> {
    if value.trim().is_empty() || value.len() > max {
        return Err(invalid(format!("{name} must contain 1–{max} bytes.")));
    }
    Ok(())
}

/// Both CRDT log and SQL projection are read under the same SQLite transaction.
fn specification(conn: &Connection, project: &str, node: Option<&str>) -> ApiResult<Value> {
    let Some(node) = node else {
        return Ok(Value::Null);
    };
    let map: Option<String> = conn
        .query_row(
            "SELECT id FROM mindmaps WHERE project=?1 ORDER BY created_at,id LIMIT 1",
            [project],
            |r| r.get(0),
        )
        .optional()?;
    let Some(map) = map else {
        return Ok(json!({"node":node,"missing":true}));
    };
    let doc = Doc::new();
    {
        let mut stmt =
            conn.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
        let mut txn = doc.transact_mut();
        for blob in stmt.query_map([&map], |r| r.get::<_, Vec<u8>>(0))? {
            let update =
                Update::decode_v1(&blob?).map_err(|e| ApiError::internal(e.to_string()))?;
            txn.apply_update(update)
                .map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }
    let (root, _) = mindmapdoc::roots(&doc);
    let nodes = mindmapdoc::normalise(mindmapdoc::read_nodes(&doc.transact(), &root));
    let mut selected = nodes.iter().find(|n| n.id == node);
    if selected.is_none() {
        return Ok(json!({"mindmap":map,"node":node,"missing":true}));
    }
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    while let Some(section) = selected {
        if !seen.insert(&section.id) {
            break;
        }
        let prose = mindmapdoc::read_section_prose(&doc, &section.id)
            .map(|fragment| fragment.get_string(&doc.transact()));
        path.push(
            json!({"id":section.id,"title":section.title,"prose":prose,"notes":section.notes}),
        );
        selected = section
            .parent
            .as_deref()
            .and_then(|parent| nodes.iter().find(|n| n.id == parent));
    }
    path.reverse();
    Ok(json!({"mindmap":map,"node":node,"sections":path}))
}

pub(crate) fn definition(conn: &Connection, check_id: &str) -> ApiResult<Value> {
    let check = conn
        .query_row(
            &format!("SELECT {} FROM checks WHERE id=?1", checklist::CHECK_COLS),
            [check_id],
            checklist::row_to_check,
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("check", check_id))?;
    let policy = checklist::resolve_policy(conn, &check)?;
    let mut cases = conn.prepare("SELECT id,key,label,assignment,seeded FROM cases WHERE check_id=?1 AND retired_at IS NULL ORDER BY key")?;
    let cases = cases.query_map([check_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,bool>(4)?)))?
        .map(|row| { let (id,key,label,assignment,seeded)=row?; Ok(json!({"id":id,"key":key,"label":label,"assignment":parse(assignment)?,"seeded":seeded})) })
        .collect::<ApiResult<Vec<Value>>>()?;
    let mut environments = conn.prepare(
        "SELECT environment FROM check_environments WHERE check_id=?1 ORDER BY environment",
    )?;
    let environments = environments
        .query_map([check_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut globs = conn.prepare("SELECT glob FROM check_globs WHERE check_id=?1 ORDER BY glob")?;
    let globs = globs
        .query_map([check_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let snapshot = json!({"id":check.id,"project":check.project,"node":check.node,"title":check.title,
        "body":check.body,"precondition":check.precondition,"layer":check.layer,"severity":check.severity,
        "verification":policy.verification,"expiry_days":policy.expiry_days,"expiry_releases":policy.expiry_releases,
        "environments":environments,"globs":globs,"metadata":check.metadata,"cases":cases});
    let spec = specification(conn, &check.project, check.node.as_deref())?;
    Ok(
        json!({"id":check.id,"project":check.project,"archived":check.archived_at.is_some(),
        "definition_revision":format!("def-{}",hash(&snapshot)),
        "specification_revision":if spec.is_null(){None}else{Some(format!("spec-{}",hash(&spec)))},
        "definition":snapshot,"specification":spec}),
    )
}

/// Historical verdicts have no reliable original definition or start time.
pub(super) fn import_legacy(conn: &Connection, only: Option<&str>) -> ApiResult<()> {
    conn.execute("INSERT OR IGNORE INTO test_runs (id,project,kind,status,environment,code_ref,created_by,executor,created_at,finished_at)
        SELECT 'run-legacy-'||v.id,c.project,'legacy','completed',v.environment,v.release,v.actor,v.actor,v.at,v.at
        FROM case_verdicts v JOIN cases k ON k.id=v.case_id JOIN checks c ON c.id=k.check_id WHERE ?1 IS NULL OR v.id=?1",[only])?;
    conn.execute("INSERT OR IGNORE INTO test_run_cases (run_id,case_id,check_id)
        SELECT 'run-legacy-'||v.id,v.case_id,k.check_id FROM case_verdicts v JOIN cases k ON k.id=v.case_id WHERE ?1 IS NULL OR v.id=?1",[only])?;
    conn.execute("INSERT OR IGNORE INTO test_run_results
        (id,run_id,case_id,actor_kind,actor,user_id,verdict,note,recorded_at,idempotency_key,request_hash,legacy_verdict)
        SELECT 'result-legacy-'||v.id,'run-legacy-'||v.id,v.case_id,v.actor_kind,v.actor,v.user,v.verdict,v.note,v.at,v.id,'legacy',v.id
        FROM case_verdicts v WHERE ?1 IS NULL OR v.id=?1",[only])?;
    Ok(())
}

fn run_header(conn: &Connection, run: &str) -> ApiResult<Value> {
    let row=conn.query_row("SELECT id,project,kind,status,environment,environment_snapshot,code_ref,retry_of,created_by,executor,created_at,started_at,finished_at FROM test_runs WHERE id=?1",[run],|r| {
        Ok(json!({"id":r.get::<_,String>(0)?,"project":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,
            "status":r.get::<_,String>(3)?,"environment":r.get::<_,Option<String>>(4)?,
            "environment_snapshot":r.get::<_,Option<String>>(5)?,"code_ref":r.get::<_,Option<String>>(6)?,
            "retry_of":r.get::<_,Option<String>>(7)?,"created_by":r.get::<_,String>(8)?,"executor":r.get::<_,Option<String>>(9)?,
            "created_at":iso(r.get(10)?),"started_at":r.get::<_,Option<i64>>(11)?.map(iso),"finished_at":r.get::<_,Option<i64>>(12)?.map(iso)}))
    }).optional()?.ok_or_else(||ApiError::not_found("test_run",run))?;
    let mut out = row;
    if let Some(raw) = out["environment_snapshot"].as_str() {
        out["environment_snapshot"] = parse(raw.to_string())?;
    }
    Ok(out)
}
fn run_json(conn: &Connection, run: &str) -> ApiResult<Value> {
    let mut out = run_header(conn, run)?;
    let mut definitions = BTreeMap::new();
    let mut specifications = BTreeMap::new();
    let mut stmt=conn.prepare("SELECT case_id,check_id,definition_revision,specification_revision,case_snapshot FROM test_run_cases WHERE run_id=?1 ORDER BY check_id,case_id")?;
    let mut cases = Vec::new();
    let mut captured = serde_json::Map::new();
    for row in stmt.query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })? {
        let (case, check, revision, spec_revision, snapshot) = row?;
        if let Some(revision) = &revision {
            if !definitions.contains_key(revision) {
                let mut definition = parse(conn.query_row(
                    "SELECT snapshot FROM test_definition_revisions WHERE id=?1",
                    [revision],
                    |r| r.get(0),
                )?)?;
                definition.as_object_mut().unwrap().remove("cases");
                definitions.insert(revision.clone(), definition);
            }
        }
        if let Some(revision) = &spec_revision {
            if !specifications.contains_key(revision) {
                specifications.insert(
                    revision.clone(),
                    parse(conn.query_row(
                        "SELECT snapshot FROM test_specification_revisions WHERE id=?1",
                        [revision],
                        |r| r.get(0),
                    )?)?,
                );
            }
        }
        let mut results=conn.prepare("SELECT id,actor_kind,actor,user_id,verdict,note,evidence,recorded_at,legacy_verdict FROM test_run_results WHERE run_id=?1 AND case_id=?2 ORDER BY recorded_at,id")?;
        let results=results.query_map(params![run,case],|r|Ok(json!({"id":r.get::<_,String>(0)?,"actor_kind":r.get::<_,String>(1)?,
            "actor":r.get::<_,String>(2)?,"user":r.get::<_,Option<String>>(3)?,"verdict":r.get::<_,String>(4)?,
            "note":r.get::<_,Option<String>>(5)?,"evidence":r.get::<_,String>(6)?,"recorded_at":iso(r.get(7)?),"legacy_verdict":r.get::<_,Option<String>>(8)?})))?
            .map(|r|{let mut r=r?;r["evidence"]=parse(r["evidence"].as_str().unwrap().into())?;Ok(r)}).collect::<ApiResult<Vec<_>>>()?;
        captured.entry(check.clone()).or_insert_with(||json!({"definition_revision":revision,"specification_revision":spec_revision,
            "definition":revision.as_ref().and_then(|r|definitions.get(r)),"specification":spec_revision.as_ref().and_then(|r|specifications.get(r))}));
        cases.push(json!({"case":case,"check":check,"definition_revision":revision,"specification_revision":spec_revision,
            "snapshot":snapshot.map(parse).transpose()?,"results":results,"revision_known":revision.is_some()}));
    }
    out["cases"] = json!(cases);
    out["definitions"] = Value::Object(captured);
    Ok(out)
}
fn replay(
    conn: &Connection,
    project: &str,
    key: &str,
    fingerprint: &str,
) -> ApiResult<Option<Value>> {
    let previous: Option<(String, String)> = conn
        .query_row(
            "SELECT id,request_hash FROM test_runs WHERE project=?1 AND idempotency_key=?2",
            params![project, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match previous {
        Some((id, old)) if old == fingerprint => Ok(Some(run_json(conn, &id)?)),
        Some(_) => Err(conflict(
            "This idempotency key was already used for a different request.",
        )),
        None => Ok(None),
    }
}
impl Store {
    pub fn test_definition(&self, check: &str) -> ApiResult<Value> {
        self.with_conn(|c| definition(c, check))
    }
    pub fn list_test_definitions(
        &self,
        project: &str,
        offset: i64,
        limit: i64,
    ) -> ApiResult<Value> {
        self.with_conn(|conn|{
            let mut stmt=conn.prepare("SELECT id FROM checks WHERE project=?1 AND archived_at IS NULL ORDER BY created_at,id LIMIT ?2 OFFSET ?3")?;
            let ids=stmt.query_map(params![project,limit.clamp(1,100),offset.max(0)],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let items=ids.iter().map(|id| {
                let mut item=definition(conn,id)?;
                item["execution"]=current_execution(conn,&item,None,None)?;
                Ok(item)
            }).collect::<ApiResult<Vec<_>>>()?;
            let total:i64=conn.query_row("SELECT count(*) FROM checks WHERE project=?1 AND archived_at IS NULL",[project],|r|r.get(0))?;
            let mut page=crate::api::paged(items,total,limit.clamp(1,100),"Continue with next_offset.");
            page["next_offset"]=if offset.max(0)+(page["items"].as_array().unwrap().len() as i64)<total {json!(offset.max(0)+page["items"].as_array().unwrap().len() as i64)} else {Value::Null};
            page["project"]=json!(project); Ok(page)
        })
    }
    pub fn get_test_run(&self, id: &str) -> ApiResult<Value> {
        self.with_conn(|c| run_json(c, id))
    }
    pub fn list_test_runs(
        &self,
        project: &str,
        before: Option<&str>,
        limit: i64,
    ) -> ApiResult<Value> {
        self.with_conn(|conn|{
            let mut stmt=conn.prepare("SELECT id FROM test_runs WHERE project=?1 AND (?2 IS NULL OR (created_at,id)<(SELECT created_at,id FROM test_runs WHERE id=?2 AND project=?1)) ORDER BY created_at DESC,id DESC LIMIT ?3")?;
            let ids=stmt.query_map(params![project,before,limit.clamp(1,100)+1],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let more=ids.len()>limit.clamp(1,100) as usize;
            let items=ids.iter().take(limit.clamp(1,100) as usize).map(|id| {
                let mut run=run_header(conn,id)?;
                let count:i64=conn.query_row("SELECT count(*) FROM test_run_cases WHERE run_id=?1",[id],|r|r.get(0))?;
                let mut checks=conn.prepare("SELECT DISTINCT check_id FROM test_run_cases WHERE run_id=?1 ORDER BY check_id")?;
                let checks=checks.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                run["case_count"]=json!(count);run["checks"]=json!(checks);Ok(run)
            }).collect::<ApiResult<Vec<_>>>()?;
            let total:i64=conn.query_row("SELECT count(*) FROM test_runs WHERE project=?1",[project],|r|r.get(0))?;
            let cursor=if more {items.last().and_then(|v|v["id"].as_str()).map(String::from)}else{None};
            let mut page=crate::api::paged(items,total,limit.clamp(1,100),"Continue with next_cursor when present."); page["next_cursor"]=json!(cursor); Ok(page)
        })
    }
    pub fn create_test_run(&self, project: &str, req: &RunCreate, actor: &str) -> ApiResult<Value> {
        bounded(&req.idempotency_key, 128, "idempotency_key")?;
        bounded(&req.code_ref, 300, "code_ref")?;
        if req.definitions.is_empty() || req.definitions.len() > 100 {
            return Err(invalid("Choose between 1 and 100 definitions."));
        }
        let fingerprint = hash(&json!({"request":req,"actor":actor}));
        self.with_tx(|tx|{
            ensure_project_writable(tx,project)?;
            if let Some(previous)=replay(tx,project,&req.idempotency_key,&fingerprint)? {return Ok(previous)}
            let environment=req.environment.as_deref().map(|id|checklist::resolve_environment(tx,project,id)).transpose()?;
            let env_snapshot=environment.as_deref().map(|id|tx.query_row("SELECT slug,name,base_url,archived_at FROM environments WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<i64>>(3)?)))).transpose()?;
            if env_snapshot.as_ref().is_some_and(|v|v.3.is_some()){return Err(invalid("An archived environment cannot be used for a new run."))}
            let env_snapshot=env_snapshot.map(|(slug,name,base_url,_)|json!({"id":environment,"slug":slug,"name":name,"base_url":base_url}).to_string());
            let mut definitions=Vec::new();let mut seen=HashSet::new();let mut total=0;
            for requested in &req.definitions {
                if !seen.insert(&requested.check){return Err(invalid("A definition may only be selected once."))}
                let current=definition(tx,&requested.check)?;
                if current["project"]!=project || current["archived"]==true {return Err(invalid("Every definition must be active and belong to this project."))}
                if current["definition_revision"]!=requested.definition_revision || current["specification_revision"]!=json!(requested.specification_revision) {
                    return Err(ApiError::conflict("conflict.definition_changed","The definition or specification changed. Refresh the selection before creating a run."));
                }
                if current["specification"]["missing"]==true {return Err(invalid("Relink the definition to an existing specification section before running it."))}
                let declared=current["definition"]["environments"].as_array().unwrap();
                if !declared.is_empty() && !environment.as_ref().is_some_and(|id|declared.contains(&json!(id))) {
                    return Err(invalid("Choose an environment declared by every selected definition."))
                }
                let cases=current["definition"]["cases"].as_array().unwrap();
                if cases.is_empty(){return Err(invalid("Each selected definition needs at least one case."))}
                total+=cases.len();if total>5000{return Err(invalid("A run can contain at most 5000 cases."))}
                definitions.push(current);
            }
            let run=id("run");let now=now_ms();
            tx.execute("INSERT INTO test_runs(id,project,status,environment,environment_snapshot,code_ref,created_by,created_at,idempotency_key,request_hash) VALUES(?1,?2,'queued',?3,?4,?5,?6,?7,?8,?9)",
                params![run,project,environment,env_snapshot,req.code_ref,actor,now,req.idempotency_key,fingerprint])?;
            for current in definitions {
                let revision=current["definition_revision"].as_str().unwrap();
                tx.execute("INSERT OR IGNORE INTO test_definition_revisions(id,check_id,snapshot,created_at) VALUES(?1,?2,?3,?4)",
                    params![revision,current["id"].as_str(),current["definition"].to_string(),now])?;
                if let Some(spec)=current["specification_revision"].as_str() { tx.execute("INSERT OR IGNORE INTO test_specification_revisions(id,project,snapshot) VALUES(?1,?2,?3)",params![spec,project,current["specification"].to_string()])?; }
                for case in current["definition"]["cases"].as_array().unwrap(){
                    tx.execute("INSERT INTO test_run_cases(run_id,case_id,check_id,definition_revision,specification_revision,case_snapshot) VALUES(?1,?2,?3,?4,?5,?6)",
                        params![run,case["id"].as_str(),current["id"].as_str(),revision,current["specification_revision"].as_str(),case.to_string()])?;
                }
            }
            emit_event(tx,None,Some(project),actor,"test_run.created",json!({"run":run}),now)?;
            run_json(tx,&run)
        })
    }
    pub fn transition_test_run(&self, run: &str, action: &str, actor: &str) -> ApiResult<Value> {
        self.with_tx(|tx|{
            let current=run_json(tx,run)?;
            ensure_project_writable(tx,current["project"].as_str().unwrap())?;
            if current["kind"]=="legacy"{return Err(conflict("Legacy evidence has no execution lifecycle."))}
            let status=current["status"].as_str().unwrap();
            let now=now_ms();
            match action {
                "start" if status=="queued" => {tx.execute("UPDATE test_runs SET status='running',executor=?2,started_at=?3 WHERE id=?1",params![run,actor,now])?;}
                "start" if status=="running" && current["executor"]==actor => {}
                "complete" if status=="running" && current["executor"]==actor => {
                    for case in current["cases"].as_array().unwrap() {
                        let policy=current["definitions"][case["check"].as_str().unwrap()]["definition"]["verification"].as_str().unwrap_or("agent");
                        let needed=if policy=="human"{"human"}else{"agent"};
                        if !case["results"].as_array().unwrap().iter().any(|r|r["actor_kind"]==needed) {
                            return Err(conflict("Record an outcome for every case before completing the run."));
                        }
                    }
                    tx.execute("UPDATE test_runs SET status='completed',finished_at=?2 WHERE id=?1",params![run,now])?;
                }
                "complete" if status=="completed" && current["executor"]==actor => {}
                "cancel" if (status=="queued" && current["created_by"]==actor) || (status=="running" && current["executor"]==actor) => {
                    tx.execute("UPDATE test_runs SET status='cancelled',finished_at=?2 WHERE id=?1",params![run,now])?;
                }
                "cancel" if status=="cancelled" && (current["created_by"]==actor || current["executor"]==actor)=>{}
                _=>return Err(conflict("This transition is not available to this actor in the run's current state.")),
            }
            emit_event(tx,None,current["project"].as_str(),actor,"test_run.transition",json!({"run":run,"action":action}),now)?;
            run_json(tx,run)
        })
    }
    pub fn record_test_result(
        &self,
        run: &str,
        req: &ResultCreate,
        actor: &str,
        user: Option<&str>,
    ) -> ApiResult<Value> {
        bounded(&req.idempotency_key, 128, "idempotency_key")?;
        if !["agent", "human"].contains(&req.actor_kind.as_str())
            || !["pass", "fail", "blocked", "unreachable"].contains(&req.verdict.as_str())
        {
            return Err(invalid("Unknown result kind or verdict."));
        }
        if req.note.as_ref().is_some_and(|s| s.len() > 65536)
            || (req.verdict != "pass" && req.note.as_ref().is_none_or(|s| s.trim().is_empty()))
        {
            return Err(invalid(
                "A non-passing result needs a note; notes are limited to 65536 bytes.",
            ));
        }
        if req.evidence.len() > 20 || req.evidence.iter().any(|s| s.is_empty() || s.len() > 2048) {
            return Err(invalid(
                "Evidence accepts up to 20 references, each at most 2048 bytes.",
            ));
        }
        let fingerprint = hash(&json!({"request":req,"actor":actor,"user":user}));
        self.with_tx(|tx|{
            let current=run_json(tx,run)?;
            ensure_project_writable(tx,current["project"].as_str().unwrap())?;
            let old:Option<String>=tx.query_row("SELECT request_hash FROM test_run_results WHERE run_id=?1 AND idempotency_key=?2",params![run,req.idempotency_key],|r|r.get(0)).optional()?;
            if let Some(old)=old {return if old==fingerprint {Ok(current)} else {Err(conflict("This idempotency key already names another result."))}}
            if current["kind"]=="legacy" || current["status"]=="cancelled" || current["status"]=="queued" {return Err(conflict("Start an execution before recording results."))}
            if req.actor_kind=="agent" && (current["status"]!="running" || current["executor"]!=actor){return Err(conflict("Only the active executor can record execution results."))}
            let case=current["cases"].as_array().unwrap().iter().find(|c|c["case"]==req.case).ok_or_else(||invalid("The case is not part of this run."))?;
            let results=case["results"].as_array().unwrap();
            if results.iter().any(|r|r["actor_kind"]==req.actor_kind){return Err(conflict("Results are immutable. Create a retry for a new observation."))}
            if req.actor_kind=="human" && current["definitions"][case["check"].as_str().unwrap()]["definition"]["verification"]=="agent_then_human" && !results.iter().any(|r|r["actor_kind"]=="agent" && r["verdict"]=="pass") {return Err(conflict("Human approval requires a passing agent result from this same attempt."))}
            tx.execute("INSERT INTO test_run_results(id,run_id,case_id,actor_kind,actor,user_id,verdict,note,evidence,recorded_at,idempotency_key,request_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![id("result"),run,req.case,req.actor_kind,actor,user,req.verdict,req.note,json!(req.evidence).to_string(),now_ms(),req.idempotency_key,fingerprint])?;
            emit_event(tx,None,current["project"].as_str(),actor,"test_run.result",json!({"run":run,"case":req.case,"actor_kind":req.actor_kind,"verdict":req.verdict}),now_ms())?;
            run_json(tx,run)
        })
    }
    pub fn retry_test_run(&self, run: &str, key: &str, actor: &str) -> ApiResult<Value> {
        bounded(key, 128, "idempotency_key")?;
        self.with_tx(|tx|{
            let original=run_json(tx,run)?;
            let project=original["project"].as_str().unwrap();
            ensure_project_writable(tx,project)?;
            let fingerprint=hash(&json!({"retry_of":run,"actor":actor}));
            if let Some(previous)=replay(tx,project,key,&fingerprint)?{return Ok(previous)}
            if original["kind"]=="legacy" || !["completed","cancelled"].contains(&original["status"].as_str().unwrap()){return Err(conflict("Retry a finished execution; legacy evidence has no known revision to retry."))}
            let new=id("run");
            tx.execute("INSERT INTO test_runs(id,project,status,environment,environment_snapshot,code_ref,retry_of,created_by,created_at,idempotency_key,request_hash)
                SELECT ?2,project,'queued',environment,environment_snapshot,code_ref,id,?3,?4,?5,?6 FROM test_runs WHERE id=?1",
                params![run,new,actor,now_ms(),key,fingerprint])?;
            tx.execute("INSERT INTO test_run_cases SELECT ?2,case_id,check_id,definition_revision,specification_revision,case_snapshot FROM test_run_cases WHERE run_id=?1",params![run,new])?;
            emit_event(tx,None,Some(project),actor,"test_run.created",json!({"run":new,"retry_of":run}),now_ms())?;
            run_json(tx,&new)
        })
    }
}

/// A reading of a specific definition revision, never a mutable verdict on its draft.
fn current_execution(
    conn: &Connection,
    definition: &Value,
    environment: Option<&str>,
    code_ref: Option<&str>,
) -> ApiResult<Value> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.id FROM test_runs r JOIN test_run_cases c ON c.run_id=r.id
        WHERE c.check_id=?1 AND r.kind='execution' AND r.status!='cancelled'
        AND (?2 IS NULL OR r.environment=?2) AND (?3 IS NULL OR r.code_ref=?3)
        AND NOT EXISTS (SELECT 1 FROM test_runs newer JOIN test_run_cases nc ON nc.run_id=newer.id
            WHERE nc.check_id=c.check_id AND newer.kind='execution' AND newer.status!='cancelled'
            AND newer.environment IS r.environment AND (?3 IS NULL OR newer.code_ref=?3)
            AND newer.rowid>r.rowid)
        ORDER BY r.created_at DESC,r.id DESC",
    )?;
    let ids = stmt
        .query_map(
            params![definition["id"].as_str(), environment, code_ref],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut latest = BTreeMap::new();
    for id in ids {
        let run = run_json(conn, &id)?;
        let env = run["environment"].as_str().unwrap_or("").to_string();
        if latest.contains_key(&env) {
            continue;
        }
        let cases: Vec<&Value> = run["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["check"] == definition["id"])
            .collect();
        let current = cases.iter().all(|c| {
            c["definition_revision"] == definition["definition_revision"]
                && c["specification_revision"] == definition["specification_revision"]
        });
        let state = if !current || expired(conn, &run, &definition["definition"])? {
            "outdated"
        } else if run["status"] != "completed" {
            "in_progress"
        } else {
            let failed = cases.iter().any(|c| {
                c["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["verdict"] != "pass")
            });
            if failed {
                "failed"
            } else {
                let policy = definition["definition"]["verification"]
                    .as_str()
                    .unwrap_or("agent");
                let ready = cases.iter().all(|c| {
                    let results = c["results"].as_array().unwrap();
                    let has = |kind: &str| {
                        results
                            .iter()
                            .any(|r| r["actor_kind"] == kind && r["verdict"] == "pass")
                    };
                    match policy {
                        "human" => has("human"),
                        "agent_then_human" => has("agent") && has("human"),
                        _ => has("agent"),
                    }
                });
                if ready {
                    "verified"
                } else {
                    "needs_approval"
                }
            }
        };
        latest.insert(env,json!({"run":id,"environment":run["environment"],"code_ref":run["code_ref"],"state":state}));
    }
    let declared = definition["definition"]["environments"].as_array().unwrap();
    if !declared.is_empty() {
        latest.retain(|key, _| declared.contains(&json!(key)));
        for env in declared {
            let key = env.as_str().unwrap();
            if environment.is_none_or(|e| e == key) {
                latest
                    .entry(key.to_string())
                    .or_insert_with(|| json!({"environment":key,"state":"not_executed"}));
            }
        }
    }
    let states: Vec<Value> = latest.into_values().collect();
    let versions: HashSet<&str> = states
        .iter()
        .filter_map(|v| v["code_ref"].as_str())
        .collect();
    let state = if states.is_empty() {
        "not_executed"
    } else if states.iter().any(|v| v["state"] == "outdated") {
        "outdated"
    } else if states.iter().any(|v| v["state"] == "failed") {
        "failed"
    } else if states.iter().any(|v| v["state"] == "in_progress") {
        "in_progress"
    } else if states.iter().any(|v| v["state"] == "needs_approval") {
        "needs_approval"
    } else if states.iter().any(|v| v["state"] == "not_executed") {
        "not_executed"
    } else if versions.len() > 1 {
        "mixed_versions"
    } else {
        "verified"
    };
    Ok(json!({"state":state,"environments":states,"scope":"latest_attempt_per_environment"}))
}

fn expired(conn: &Connection, run: &Value, definition: &Value) -> ApiResult<bool> {
    let at: i64 = conn.query_row(
        "SELECT coalesce(started_at,created_at) FROM test_runs WHERE id=?1",
        [run["id"].as_str()],
        |r| r.get(0),
    )?;
    if definition["expiry_days"]
        .as_i64()
        .is_some_and(|days| now_ms().saturating_sub(at) >= days.saturating_mul(86_400_000))
    {
        return Ok(true);
    }
    if let Some(limit) = definition["expiry_releases"].as_i64() {
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM releases WHERE project=?1 AND created_at>?2",
            params![run["project"].as_str(), at],
            |r| r.get(0),
        )?;
        if count >= limit {
            return Ok(true);
        }
    }
    Ok(false)
}
