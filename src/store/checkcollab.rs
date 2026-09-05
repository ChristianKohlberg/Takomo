//! Check definition text in the canonical CRDT log, with transactional SQL projections.
use super::Store;
use crate::error::{ApiError, ApiResult};
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Doc, GetString, Map, MapPrelim, MapRef, Out, ReadTxn, StateVector, Text, TextPrelim,
    Transact, Update,
};
type Fields = BTreeMap<String, Value>;
type Snapshot = BTreeMap<String, Fields>;
fn snapshot(conn: &Connection, id: &str) -> ApiResult<Snapshot> {
    let fields = conn.query_row(
        "SELECT title,body,precondition FROM checks WHERE id=?1",
        [id],
        |r| {
            Ok(BTreeMap::from([
                ("title".into(), json!(r.get::<_, String>(0)?)),
                ("body".into(), json!(r.get::<_, String>(1)?)),
                ("precondition".into(), json!(r.get::<_, String>(2)?)),
            ]))
        },
    )?;
    Ok(BTreeMap::from([("definition".into(), fields)]))
}
fn load(conn: &Connection, id: &str) -> ApiResult<Doc> {
    let doc = Doc::new();
    let mut stmt = conn.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
    let blobs = stmt
        .query_map([id], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut txn = doc.transact_mut();
    for blob in blobs {
        let update = Update::decode_v1(&blob)
            .map_err(|e| ApiError::internal(format!("Invalid stored CRDT: {e}")))?;
        txn.apply_update(update)
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    drop(txn);
    Ok(doc)
}

fn root(doc: &Doc) -> MapRef {
    doc.get_or_insert_map("nodes")
}

/// A minimal text splice retains the identities of unchanged characters.
fn splice(text: &yrs::TextRef, txn: &mut yrs::TransactionMut, value: &str) {
    let old = text.get_string(txn);
    if old == value {
        return;
    }
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = value.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    // Yrs uses byte offsets by default; browser Yjs uses UTF-16 offsets locally.
    let start = a[..prefix].iter().map(|c| c.len_utf8()).sum::<usize>() as u32;
    let removed = a[prefix..a.len() - suffix]
        .iter()
        .map(|c| c.len_utf8())
        .sum::<usize>() as u32;
    if removed > 0 {
        text.remove_range(txn, start, removed);
    }
    let insert: String = b[prefix..b.len() - suffix].iter().collect();
    if !insert.is_empty() {
        text.insert(txn, start, &insert);
    }
}

fn reflect(doc: &Doc, before: &Snapshot, after: &Snapshot) {
    let root = root(doc);
    let mut txn = doc.transact_mut();
    for key in before.keys() {
        if !after.contains_key(key) {
            root.remove(&mut txn, key);
        }
    }
    for (key, fields) in after {
        let entry = match root.get(&txn, key) {
            Some(Out::YMap(map)) => map,
            _ => root.insert(&mut txn, key.clone(), MapPrelim::default()),
        };
        for (field, value) in fields {
            if before.get(key).and_then(|f| f.get(field)) == Some(value) {
                continue;
            }
            if ["title", "body", "precondition"].contains(&field.as_str()) {
                let text = match entry.get(&txn, field) {
                    Some(Out::YText(text)) => text,
                    _ => entry.insert(&mut txn, field.clone(), TextPrelim::new("")),
                };
                splice(&text, &mut txn, value.as_str().unwrap_or(""));
            } else {
                let any = match value {
                    Value::String(s) => Any::String(s.clone().into()),
                    Value::Number(n) => Any::Number(n.as_f64().unwrap_or(0.0)),
                    Value::Bool(b) => Any::Bool(*b),
                    _ => Any::Null,
                };
                entry.insert(&mut txn, field.clone(), any);
            }
        }
    }
}

fn save(conn: &Connection, id: &str, doc: &Doc, actor: &str) -> ApiResult<Vec<u8>> {
    let kind = "check";
    let mut blobs = vec![doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default())];
    let mut stmt = conn.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1")?;
    blobs.extend(
        stmt.query_map([id], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    let state = yrs::merge_updates_v1(&blobs).map_err(|e| ApiError::internal(e.to_string()))?;
    if state.len() > super::crdt::MAX_OBJECT_BYTES as usize {
        return Err(ApiError::validation(
            "validation.document_too_large",
            "Shared state exceeds the document size limit.",
        ));
    }
    // These bounded structured objects keep one complete CRDT update, including
    // tombstones and client clocks. Compaction never resets character identities.
    conn.execute("DELETE FROM crdt_updates WHERE object_id=?1", [id])?;
    conn.execute("INSERT INTO crdt_updates(object_id,object_kind,blob,bytes,created_by,created_at) VALUES(?1,?2,?3,?4,?5,?6)", params![id,kind,state,state.len() as i64,actor,crate::ids::now_ms()])?;
    Ok(state)
}

fn read_snapshot(doc: &Doc) -> ApiResult<Snapshot> {
    use yrs::types::ToJson;
    let root = root(doc);
    let txn = doc.transact();
    for (_, value) in root.iter(&txn) {
        let Out::YMap(entry) = value else {
            return Err(ApiError::validation(
                "validation.collab_state",
                "Shared entries must be Y.Map objects.",
            ));
        };
        for field in ["title", "body", "precondition"] {
            if entry
                .get(&txn, field)
                .is_some_and(|v| !matches!(v, Out::YText(_)))
            {
                return Err(ApiError::validation(
                    "validation.collab_state",
                    "Shared text fields must use Y.Text.",
                ));
            }
        }
    }
    let mut encoded = String::new();
    root.to_json(&txn).to_json(&mut encoded);
    serde_json::from_str(&encoded).map_err(|_| {
        ApiError::validation(
            "validation.collab_state",
            "Shared nodes must be keyed objects containing text and scalar fields.",
        )
    })
}

pub(crate) fn seed_existing(conn: &Connection) -> ApiResult<()> {
    let mut stmt=conn.prepare("SELECT id FROM checks WHERE NOT EXISTS(SELECT 1 FROM crdt_updates WHERE object_id=checks.id)")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let doc = Doc::new();
            reflect(&doc, &Snapshot::new(), &snapshot(conn, &id)?);
            save(conn, &id, &doc, "migration").map(|_| ())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(e);
            }
        }
    }
    Ok(())
}
impl Store {
    pub(crate) fn with_check_tx<T>(
        &self,
        id: &str,
        actor: &str,
        f: impl FnOnce(&Transaction) -> ApiResult<T>,
    ) -> ApiResult<T> {
        let (out, update) = self.with_tx(|tx| {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM checks WHERE id=?1)",
                [id],
                |r| r.get(0),
            )?;
            let before = if exists {
                snapshot(tx, id)?
            } else {
                Snapshot::new()
            };
            let doc = load(tx, id)?;
            let vector = doc.transact().state_vector();
            let out = f(tx)?;
            reflect(&doc, &before, &snapshot(tx, id)?);
            save(tx, id, &doc, actor)?;
            let update = doc.transact().encode_state_as_update_v1(&vector);
            Ok((out, update))
        })?;
        let _ = self.check_updates.send((id.into(), update));
        Ok(out)
    }
    pub fn apply_check_update(&self, id: &str, blob: &[u8], actor: &str) -> ApiResult<()> {
        if blob.len() > super::crdt::MAX_UPDATE_BYTES {
            return Err(ApiError::validation(
                "validation.collab_state",
                "The shared update exceeds the frame limit.",
            ));
        }
        self.with_tx(|tx| {
   let object=super::crdt::resolve(tx,id)?;
   if object.kind!=super::crdt::CollabKind::Check {return Err(ApiError::validation("validation.collab_state","This is not a check definition."));}
   super::helpers::ensure_project_writable(tx,&object.project)?;
   let doc=load(tx,id)?;
   let update=Update::decode_v1(blob).map_err(|e|ApiError::validation("validation.collab_state",e.to_string()))?;
   doc.transact_mut().apply_update(update).map_err(|e|ApiError::validation("validation.collab_state",e.to_string()))?;
   let data=read_snapshot(&doc)?;
   let fields=data.get("definition").ok_or_else(||ApiError::validation("validation.collab_state","A check needs its definition."))?;
   let text=|field:&str|->ApiResult<&str> { fields.get(field).and_then(Value::as_str).ok_or_else(||ApiError::validation("validation.collab_state","Check fields must be shared text.")) };
   let (title,body,precondition)=(text("title")?,text("body")?,text("precondition")?);
   if title.len()>65536 || body.len()>262144 || precondition.len()>262144 {return Err(ApiError::validation("validation.collab_state","Check text exceeds its size limit."));}
   tx.execute("UPDATE checks SET title=?2,body=?3,precondition=?4,updated_at=?5,version=version+1 WHERE id=?1 AND (title!=?2 OR body!=?3 OR precondition!=?4)",params![id,title,body,precondition,crate::ids::now_ms()])?;
   // Keep causal updates even when their dependencies have not arrived yet.
   tx.execute("INSERT INTO crdt_updates(object_id,object_kind,blob,bytes,created_by,created_at) VALUES(?1,'check',?2,?3,?4,?5)",params![id,blob,blob.len() as i64,actor,crate::ids::now_ms()])?;
   save(tx,id,&doc,actor)?; Ok(())
  })?;
        let _ = self.check_updates.send((id.into(), blob.to_vec()));
        Ok(())
    }
}
