//! Immutable saved CRDT versions. Live-log compaction never discards this archive.
//! `recorded_by` is the flusher, NOT an attribution of all merged edits.
use super::{helpers::ensure_project_writable, mindmapdoc, Store};
use crate::{
    error::{ApiError, ApiResult},
    ids::{iso, now_ms},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use yrs::{
    updates::{decoder::Decode, encoder::Encode},
    Doc, GetString, IdSet, ReadTxn, StateVector, Transact, Update,
};

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(e.to_string())
}
fn apply(doc: &Doc, blob: &[u8]) -> ApiResult<()> {
    doc.transact_mut()
        .apply_update(Update::decode_v1(blob).map_err(internal)?)
        .map_err(internal)
}
fn head(conn: &Connection, map: &str) -> ApiResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(version),0) FROM specification_versions WHERE mindmap=?1",
        [map],
        |r| r.get(0),
    )?)
}
fn remember(conn: &Connection, map: &str, insertions: &IdSet, deletions: &IdSet) -> ApiResult<()> {
    conn.execute("INSERT INTO specification_history_heads(mindmap,insertions,deletions) VALUES(?1,?2,?3) ON CONFLICT(mindmap) DO UPDATE SET insertions=excluded.insertions,deletions=excluded.deletions",params![map,insertions.encode_v1(),deletions.encode_v1()])?;
    Ok(())
}
/// Establish the oldest state we can truthfully recover, without inventing past edits.
pub(crate) fn baseline(conn: &Connection, map: &str) -> ApiResult<()> {
    if head(conn, map)? != 0 {
        return Ok(());
    }
    let doc = Doc::new();
    let mut stmt = conn.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
    let mut found = false;
    for blob in stmt.query_map([map], |r| r.get::<_, Vec<u8>>(0))? {
        apply(&doc, &blob?)?;
        found = true;
    }
    if found {
        let state = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let update = Update::decode_v1(&state).map_err(internal)?;
        if update.is_empty() {
            return Ok(());
        }
        conn.execute("INSERT INTO specification_versions(mindmap,version,blob,state,kind,recorded_at,recorded_by) VALUES(?1,1,?2,?2,'baseline',?3,NULL)",params![map,state,now_ms()])?;
        remember(conn, map, &update.insertions(true), update.delete_set())?;
    }
    Ok(())
}
/// Called inside the SAME transaction as the canonical append, before its insertion.
pub(crate) fn record(
    conn: &Connection,
    map: &str,
    blob: &[u8],
    actor: &str,
    at: i64,
) -> ApiResult<()> {
    baseline(conn, map)?;
    let previous = head(conn, map)?;
    let update = Update::decode_v1(blob).map_err(internal)?;
    let mut inserted = update.insertions(true);
    let mut deleted = update.delete_set().clone();
    if previous != 0 {
        let (old_i, old_d): (Vec<u8>, Vec<u8>) = conn.query_row(
            "SELECT insertions,deletions FROM specification_history_heads WHERE mindmap=?1",
            [map],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let old_i = IdSet::decode_v1(&old_i).map_err(internal)?;
        let old_d = IdSet::decode_v1(&old_d).map_err(internal)?;
        // Sync retries include the entire delete set. Track actual ID ranges, not
        // just a state vector: out-of-order inserts can have gaps we must retain.
        if inserted.diff(&old_i).is_empty() && deleted.diff(&old_d).is_empty() {
            return Ok(());
        }
        inserted.merge_with(old_i);
        deleted.merge_with(old_d);
    } else if inserted.is_empty() && deleted.is_empty() {
        return Ok(());
    }
    remember(conn, map, &inserted, &deleted)?;
    let version = previous + 1;
    // Materialize every 64 saves so reads replay at most 63 deltas. Keep all earlier
    // deltas and materializations: they represent real historic states, not a cache.
    let state = if previous == 0 || version % 64 == 0 {
        let doc = if previous == 0 {
            Doc::new()
        } else {
            document(conn, map, previous)?
        };
        apply(&doc, blob)?;
        let bytes = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        Some(bytes)
    } else {
        None
    };
    conn.execute("INSERT INTO specification_versions(mindmap,version,blob,state,kind,recorded_at,recorded_by) VALUES(?1,?2,?3,?4,'save',?5,?6)",params![map,version,blob,state,at,actor])?;
    Ok(())
}
fn document(conn: &Connection, map: &str, version: i64) -> ApiResult<Doc> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM specification_versions WHERE mindmap=?1 AND version=?2)",
        params![map, version],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(ApiError::not_found(
            "specification_version",
            &version.to_string(),
        ));
    }
    let (base,blob):(i64,Vec<u8>)=conn.query_row("SELECT version,state FROM specification_versions WHERE mindmap=?1 AND version<=?2 AND state IS NOT NULL ORDER BY version DESC LIMIT 1",params![map,version],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let doc = Doc::new();
    apply(&doc, &blob)?;
    let mut stmt=conn.prepare("SELECT blob FROM specification_versions WHERE mindmap=?1 AND version>?2 AND version<=?3 ORDER BY version")?;
    for blob in stmt.query_map(params![map, base, version], |r| r.get::<_, Vec<u8>>(0))? {
        apply(&doc, &blob?)?;
    }
    Ok(doc)
}
fn summary(conn: &Connection, map: &str, version: i64) -> ApiResult<Value> {
    let mut value=conn.query_row("SELECT kind,recorded_at,recorded_by FROM specification_versions WHERE mindmap=?1 AND version=?2",params![map,version],|r|Ok(json!({"version":version,"kind":r.get::<_,String>(0)?,"recorded_at":iso(r.get(1)?),"recorded_by":r.get::<_,Option<String>>(2)?}))).optional()?.ok_or_else(||ApiError::not_found("specification_version",&version.to_string()))?;
    let mut stmt=conn.prepare("SELECT name,actor,user,created_at FROM specification_checkpoints WHERE mindmap=?1 AND version=?2 ORDER BY id")?;
    value["checkpoints"]=Value::Array(stmt.query_map(params![map,version],|r|Ok(json!({"name":r.get::<_,String>(0)?,"actor":r.get::<_,String>(1)?,"user":r.get::<_,Option<String>>(2)?,"created_at":iso(r.get(3)?)})))?.collect::<Result<Vec<_>,_>>()?);
    Ok(value)
}
impl Store {
    pub fn specification_history(
        &self,
        map: &str,
        before: Option<i64>,
        limit: i64,
        checkpoints: bool,
    ) -> ApiResult<Value> {
        let limit = limit.clamp(1, 100);
        self.with_conn(|conn| {
            let predicate="mindmap=?1 AND (?2=0 OR EXISTS(SELECT 1 FROM specification_checkpoints c WHERE c.mindmap=specification_versions.mindmap AND c.version=specification_versions.version))";
            let total:i64=conn.query_row(&format!("SELECT COUNT(*) FROM specification_versions WHERE {predicate}"),params![map,checkpoints],|r|r.get(0))?;
            let mut stmt=conn.prepare(&format!("SELECT version FROM specification_versions WHERE {predicate} AND (?3 IS NULL OR version<?3) ORDER BY version DESC LIMIT ?4"))?;
            let versions=stmt.query_map(params![map,checkpoints,before,limit+1],|r|r.get::<_,i64>(0))?.collect::<Result<Vec<_>,_>>()?;
            let more=versions.len()>limit as usize;
            let items=versions.iter().take(limit as usize).map(|v|summary(conn,map,*v)).collect::<ApiResult<Vec<_>>>()?;
            Ok(json!({"items":items,"total":total,"limit":limit,"head":head(conn,map)?,"next_cursor":if more { versions.get(limit as usize-1).copied() } else { None },"note":"Versions start when history was enabled. Saved batches may contain several collaborators' changes; recorded_by identifies the flusher, not every author."}))
        })
    }
    pub fn specification_version(&self, map: &str, version: i64) -> ApiResult<Value> {
        self.with_conn(|conn| {
            let doc = document(conn, map, version)?;
            let mut info = summary(conn, map, version)?;
            let (mut nodes, rels, _) = mindmapdoc::snapshot(&doc, map);
            for node in &mut nodes {
                if let Some(prose) =
                    mindmapdoc::read_section_prose(&doc, node["id"].as_str().unwrap_or_default())
                {
                    node["prose_xml"] = json!(prose.get_string(&doc.transact()));
                }
            }
            info["nodes"] = json!(nodes);
            info["relationships"] = json!(rels);
            Ok(info)
        })
    }
    pub fn specification_version_state(&self, map: &str, version: i64) -> ApiResult<Vec<u8>> {
        self.with_conn(|conn| {
            Ok(document(conn, map, version)?
                .transact()
                .encode_state_as_update_v1(&StateVector::default()))
        })
    }
    pub fn checkpoint_specification(
        &self,
        map: &str,
        expected: i64,
        name: &str,
        actor: &str,
        user: Option<&str>,
    ) -> ApiResult<Value> {
        if name.trim().is_empty() || name.len() > 200 {
            return Err(ApiError::validation(
                "validation.specification_history",
                "Checkpoint name must contain 1–200 bytes.",
            ));
        }
        self.with_tx(|tx| {
            let object=super::crdt::resolve(tx,map)?; ensure_project_writable(tx,&object.project)?;
            let current=head(tx,map)?;
            if current!=expected { return Err(ApiError::conflict("conflict.specification_history","The saved specification changed. Refresh history and review the latest version before naming it.")); }
            // A legacy map needs a baseline even when nobody has edited it yet.
            baseline(tx,map)?;
            let current=head(tx,map)?;
            if current==0 { return Err(ApiError::conflict("conflict.specification_history","Save some specification content before creating a checkpoint.")); }
            // Exact retries are harmless. Names cannot be moved to another version.
            let existing:Option<i64>=tx.query_row("SELECT version FROM specification_checkpoints WHERE mindmap=?1 AND name=?2",params![map,name.trim()],|r|r.get(0)).optional()?;
            if let Some(existing)=existing {
                if existing!=current { return Err(ApiError::conflict("conflict.specification_history","This checkpoint name already belongs to another version. Choose another name.")); }
            } else {
                let names: i64 = tx.query_row("SELECT COUNT(*) FROM specification_checkpoints WHERE mindmap=?1 AND version=?2", params![map,current], |r|r.get(0))?;
                if names >= 16 { return Err(ApiError::validation("validation.specification_history","A saved version may have at most 16 checkpoint names. Reuse an existing name.")); }
                tx.execute("INSERT INTO specification_checkpoints(mindmap,version,name,actor,user,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![map,current,name.trim(),actor,user,now_ms()])?;
            }
            summary(tx,map,current)
        })
    }
}
