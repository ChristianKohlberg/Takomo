//! Rebuildable search projections. The CRDT log remains authoritative.
use super::{mindmapdoc, Store};
use crate::{
    error::{ApiError, ApiResult},
    ids::{now_ms, sha256_hex},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use yrs::{
    types::text::YChange, types::ToJson, updates::decoder::Decode, ReadTxn, Text, Transact,
    XmlFragment, XmlOut,
};
pub const MAX_ATTEMPTS: i64 = 3;
pub const RESULT_LIMIT: usize = 20;
pub const CANDIDATE_LIMIT: usize = 100;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingConfig {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub quiet_seconds: i64,
    pub max_wait_seconds: i64,
}
impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "voyage".into(),
            endpoint: "https://api.voyageai.com/v1/embeddings".into(),
            model: "voyage-4-lite".into(),
            dimensions: 1024,
            quiet_seconds: 60,
            max_wait_seconds: 300,
        }
    }
}
impl EmbeddingConfig {
    pub fn fingerprint(&self) -> String {
        sha256_hex(
            format!(
                "v1:{}:{}:{}:{}",
                self.provider, self.endpoint, self.model, self.dimensions
            )
            .as_bytes(),
        )
    }
    pub fn validate(&self) -> ApiResult<()> {
        let url = reqwest::Url::parse(&self.endpoint).map_err(|_| {
            ApiError::validation("embeddings.endpoint", "Invalid provider endpoint")
        })?;
        if !["voyage", "openai"].contains(&self.provider.as_str())
            || !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || self.model.trim().is_empty()
            || self.model.len() > 200
            || self.dimensions == 0
            || self.dimensions > 4096
            || !(1..=3600).contains(&self.quiet_seconds)
            || self.max_wait_seconds < self.quiet_seconds
            || self.max_wait_seconds > 86400
        {
            return Err(ApiError::validation("embeddings.config","Choose a supported provider, HTTP(S) endpoint without credentials/query, model, 1–4096 dimensions and a quiet delay from 1–3600 seconds with maximum wait no shorter and at most one day"));
        }
        Ok(())
    }
}
pub fn migrate(conn: &Connection) -> ApiResult<()> {
    let tx = conn.unchecked_transaction()?;
    let first_upgrade: bool = tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='search_dirty_maps')",
        [],
        |r| r.get(0),
    )?;
    tx.execute_batch(include_str!("search.sql"))?;
    if first_upgrade {
        tx.execute(
            "INSERT OR IGNORE INTO search_dirty_maps SELECT id,updated_at FROM mindmaps",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}
fn config(conn: &Connection) -> ApiResult<(EmbeddingConfig, String)> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT config,api_key FROM embedding_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((c, k)) => Ok((
            serde_json::from_str(&c)
                .map_err(|_| ApiError::internal("Invalid stored embedding configuration"))?,
            k,
        )),
        None => Ok((EmbeddingConfig::default(), String::new())),
    }
}
#[derive(Clone, Debug)]
pub struct EmbeddingJob {
    pub map_id: String,
    pub node_id: String,
    pub hash: String,
    pub fingerprint: String,
    pub lease: i64,
    pub chunks: Vec<(i64, String)>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub node_id: String,
    pub title: String,
    pub heading_path: Vec<String>,
    pub excerpt: String,
    pub passage: String,
    pub highlights: Vec<String>,
    pub match_kind: String,
}
#[derive(Clone, Debug, Default)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    pub used_vectors: bool,
    pub candidates: usize,
    pub configured: bool,
    pub projection_error: Option<String>,
}
/// Split only within a large paragraph as a last resort;
/// Normal sections remain one chunk.
pub fn chunks(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for paragraph in text.split('\n').filter(|p| !p.trim().is_empty()) {
        if current.chars().count() + paragraph.chars().count() + 1 > 2000 && !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
        if paragraph.chars().count() > 2000 {
            let chars: Vec<char> = paragraph.chars().collect();
            for part in chars.chunks(2000) {
                result.push(part.iter().collect());
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(paragraph);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}
fn plain<T: ReadTxn, F: XmlFragment>(txn: &T, frag: &F) -> String {
    frag.children(txn)
        .map(|child| match child {
            XmlOut::Text(text) => text
                .diff(txn, YChange::identity)
                .into_iter()
                .filter_map(|part| match part.insert.to_json(txn) {
                    yrs::Any::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect::<String>(),
            XmlOut::Element(el) => plain(txn, &el),
            XmlOut::Fragment(f) => plain(txn, &f),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn hydrate(conn: &Connection, map: &str) -> ApiResult<yrs::Doc> {
    let doc = yrs::Doc::new();
    {
        let mut txn = doc.transact_mut();
        let mut stmt =
            conn.prepare("SELECT blob FROM crdt_updates WHERE object_id=?1 ORDER BY seq")?;
        for row in stmt.query_map([map], |r| r.get::<_, Vec<u8>>(0))? {
            let update = yrs::Update::decode_v1(&row?)
                .map_err(|_| ApiError::internal("Cannot decode source document"))?;
            txn.apply_update(update)
                .map_err(|_| ApiError::internal("Cannot replay source document"))?;
        }
    }
    Ok(doc)
}
fn is_dirty(conn: &Connection, map: &str) -> ApiResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM search_dirty_maps WHERE map_id=?1)",
        [map],
        |row| row.get(0),
    )?)
}
fn projection_failure(conn: &Connection, map: &str) -> ApiResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT message FROM search_failures WHERE map_id=?1",
            [map],
            |r| r.get(0),
        )
        .optional()?)
}
pub const PROJECTION_ATTEMPTS: usize = 3;
pub const PROJECTION_DEFERRED: &str = "The document changed while it was being indexed, repeatedly; results reflect the last completed projection and the next read or worker pass retries";
struct NodeProjection {
    id: String,
    title: String,
    path_json: String,
    passage: String,
    hash: String,
}
/// What a map's log says, computed on a reader so the writer is held only to apply it.
pub struct Projection {
    seq: i64,
    nodes: Vec<NodeProjection>,
}
impl Projection {
    pub fn seq(&self) -> i64 {
        self.seq
    }
}
fn log_seq(conn: &Connection, map: &str) -> ApiResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq),0) FROM crdt_updates WHERE object_id=?1",
        [map],
        |r| r.get(0),
    )?)
}
fn manual_gate(conn: &Connection, map: &str) -> ApiResult<()> {
    let project: Option<String> = conn
        .query_row("SELECT project FROM mindmaps WHERE id=?1", [map], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(project) = project {
        super::helpers::ensure_project_writable(conn, &project)?;
    }
    Ok(())
}
fn compute(conn: &Connection, map: &str) -> ApiResult<Projection> {
    let seq = log_seq(conn, map)?;
    let doc = hydrate(conn, map)?;
    let (_, _, nodes) = mindmapdoc::snapshot(&doc, map);
    let by_id: HashMap<_, _> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut projected = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let mut path = vec![node.title.clone()];
        let mut parent = node.parent.as_deref();
        let mut seen = HashSet::new();
        seen.insert(node.id.as_str());
        while let Some(id) = parent {
            if !seen.insert(id) {
                break;
            }
            let Some(p) = by_id.get(id) else { break };
            path.push(p.title.clone());
            parent = p.parent.as_deref();
        }
        path.reverse();
        let passage = mindmapdoc::read_section_prose(&doc, &node.id)
            .map(|f| plain(&doc.transact(), &f))
            .unwrap_or_else(|| node.notes.clone());
        let path_json = serde_json::to_string(&path).unwrap();
        let hash = sha256_hex(format!("{path_json}\n{passage}").as_bytes());
        projected.push(NodeProjection {
            id: node.id.clone(),
            title: node.title.clone(),
            path_json,
            passage,
            hash,
        });
    }
    Ok(Projection {
        seq,
        nodes: projected,
    })
}
fn apply(
    conn: &Connection,
    map: &str,
    projection: &Projection,
    now: i64,
    manual: bool,
) -> ApiResult<bool> {
    if manual {
        manual_gate(conn, map)?;
    }
    if log_seq(conn, map)? != projection.seq {
        return Ok(false);
    }
    let (settings, _) = config(conn)?;
    let fingerprint = settings.fingerprint();
    let quiet = settings.quiet_seconds * 1000;
    let max_wait = settings.max_wait_seconds * 1000;
    let mut present = HashSet::new();
    for node in &projection.nodes {
        present.insert(node.id.as_str());
        let old: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM search_nodes WHERE map_id=?1 AND node_id=?2",
                params![map, node.id],
                |r| r.get(0),
            )
            .optional()?;
        if old.as_deref() != Some(&node.hash) {
            conn.execute(
                "INSERT INTO search_nodes(map_id,node_id,content_hash)VALUES(?1,?2,?3)ON
CONFLICT(map_id,node_id)DO UPDATE SET content_hash=excluded.content_hash",
                params![map, node.id, node.hash],
            )?;
            conn.execute(
                "DELETE FROM search_chunks WHERE map_id=?1 AND node_id=?2",
                params![map, node.id],
            )?;
            for (ordinal, chunk) in chunks(&node.passage).iter().enumerate() {
                conn.execute(
                    "INSERT INTO
search_chunks(map_id,node_id,ordinal,title,heading_path,passage,content_hash)VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![map, node.id, ordinal, node.title, node.path_json, chunk, node.hash],
                )?;
            }
            conn.execute(
                "INSERT INTO
embedding_jobs(map_id,node_id,content_hash,fingerprint,first_changed,due_at)VALUES(?1,?2,?3,?4,?5,?6)ON
CONFLICT(map_id,node_id)DO UPDATE SET
content_hash=excluded.content_hash,fingerprint=excluded.fingerprint,
first_changed=CASE WHEN embedding_jobs.first_changed+?7<=excluded.first_changed OR embedding_jobs.attempts>=?8 THEN excluded.first_changed ELSE embedding_jobs.first_changed END,
due_at=min(excluded.due_at,(CASE WHEN embedding_jobs.first_changed+?7<=excluded.first_changed OR embedding_jobs.attempts>=?8 THEN excluded.first_changed ELSE embedding_jobs.first_changed END)+?7),
lease_until=0,attempts=0,last_error=NULL",
                params![
                    map,
                    node.id,
                    node.hash,
                    fingerprint,
                    now,
                    if manual { now } else { now + quiet },
                    max_wait,
                    MAX_ATTEMPTS
                ],
            )?;
        } else {
            let missing: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM search_chunks WHERE map_id=?1 AND node_id=?2 AND (vector IS
NULL OR fingerprint<>?3))",
                params![map, node.id, fingerprint],
                |r| r.get(0),
            )?;
            if missing {
                conn.execute(
                    "INSERT OR IGNORE INTO
embedding_jobs(map_id,node_id,content_hash,fingerprint,first_changed,due_at)VALUES(?1,?2,?3,?4,?5,?6)",
                    params![map, node.id, node.hash, fingerprint, now, if manual { now } else { now + quiet }],
                )?;
            }
        }
    }
    let mut stmt = conn.prepare("SELECT node_id FROM search_nodes WHERE map_id=?1")?;
    let stored: Vec<String> = stmt
        .query_map([map], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    drop(stmt);
    for id in stored {
        if !present.contains(id.as_str()) {
            conn.execute(
                "DELETE FROM search_nodes WHERE map_id=?1 AND node_id=?2",
                params![map, id],
            )?;
        }
    }
    if manual {
        conn.execute(
            "UPDATE embedding_jobs SET due_at=?2,attempts=0,last_error=NULL WHERE map_id=?1",
            params![map, now],
        )?;
    }
    conn.execute("DELETE FROM search_dirty_maps WHERE map_id=?1", [map])?;
    conn.execute("DELETE FROM search_failures WHERE map_id=?1", [map])?;
    Ok(true)
}
fn record_failure(conn: &Connection, map: &str, now: i64, message: &str) -> ApiResult<()> {
    conn.execute("DELETE FROM search_dirty_maps WHERE map_id=?1", [map])?;
    conn.execute(
        "INSERT INTO search_failures(map_id,failed_at,message)VALUES(?1,?2,?3)ON
CONFLICT(map_id)DO UPDATE SET failed_at=excluded.failed_at,message=excluded.message",
        params![map, now, message],
    )?;
    Ok(())
}
impl Store {
    pub fn embedding_config(&self) -> ApiResult<(EmbeddingConfig, String)> {
        self.with_conn(config)
    }
    pub fn save_embedding_config(
        &self,
        next: EmbeddingConfig,
        key: Option<String>,
    ) -> ApiResult<Value> {
        next.validate()?;
        if key.as_ref().is_some_and(|s| s.len() > 4096) {
            return Err(ApiError::validation(
                "embeddings.key",
                "API key is too long",
            ));
        }
        self.with_tx(|tx| {
            let (old, old_key) = config(tx)?;
            let key = key.unwrap_or_else(|| if old.provider == next.provider && old.endpoint == next.endpoint { old_key } else { String::new() });
            let fingerprint = next.fingerprint();
            tx.execute(
                "INSERT INTO embedding_settings(id,config,api_key,fingerprint)VALUES(1,?1,?2,?3)ON
CONFLICT(id)DO UPDATE SET
config=excluded.config,api_key=excluded.api_key,fingerprint=excluded.fingerprint",
                params![serde_json::to_string(&next).unwrap(), key, fingerprint],
            )?;
            if old.fingerprint() != fingerprint {
                tx.execute(
                    "INSERT INTO
embedding_jobs(map_id,node_id,content_hash,fingerprint,first_changed,due_at)SELECT
map_id,node_id,content_hash,?1,?2,?2 FROM search_nodes WHERE 1 ON
CONFLICT(map_id,node_id)DO UPDATE SET
fingerprint=excluded.fingerprint,content_hash=excluded.content_hash,due_at=excluded.due_at,lease_until=0,attempts=0,last_error=NULL",
                    params![fingerprint, now_ms()],
                )?;
            }
            let mut value = serde_json::to_value(&next).unwrap();
            value["configured"] = json!(!key.is_empty());
            Ok(value)
        })
    }
    pub fn compute_projection(&self, map: &str) -> ApiResult<Projection> {
        self.with_conn(|conn| compute(conn, map))
    }
    pub fn apply_projection(
        &self,
        map: &str,
        projection: &Projection,
        now: i64,
        manual: bool,
    ) -> ApiResult<bool> {
        self.with_tx(|tx| apply(tx, map, projection, now, manual))
    }
    /// Project the map's log into the search tables. `Ok(false)` means the log
    /// kept growing under the projection for every bounded attempt; the map
    /// stays dirty and the next read or worker pass tries again.
    pub fn refresh_search(&self, map: &str, manual: bool, now: i64) -> ApiResult<bool> {
        if manual {
            self.with_conn(|conn| manual_gate(conn, map))?;
        } else if !self.with_conn(|conn| is_dirty(conn, map))? {
            return Ok(true);
        }
        for _ in 0..PROJECTION_ATTEMPTS {
            let step = self
                .compute_projection(map)
                .and_then(|projection| self.apply_projection(map, &projection, now, manual));
            match step {
                Ok(true) => return Ok(true),
                Ok(false) => continue,
                Err(error) => {
                    if error.status == axum::http::StatusCode::CONFLICT {
                        return Err(error);
                    }
                    self.with_tx(|tx| record_failure(tx, map, now, &error.body.message))?;
                    return Err(error);
                }
            }
        }
        Ok(false)
    }
    /// The read path: project what is dirty, and when the map's source cannot be
    /// projected, answer from what was last projected and say so rather than fail
    /// the read.
    pub fn project_search(&self, map: &str, now: i64) -> ApiResult<Option<String>> {
        match self.refresh_search(map, false, now) {
            Ok(true) => self.with_conn(|conn| projection_failure(conn, map)),
            Ok(false) => Ok(Some(PROJECTION_DEFERRED.into())),
            Err(error) => match self.with_conn(|conn| projection_failure(conn, map))? {
                None => Err(error),
                recorded => Ok(recorded),
            },
        }
    }
    pub fn refresh_dirty_search(&self, now: i64) -> ApiResult<()> {
        let maps: Vec<String> = self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT map_id FROM search_dirty_maps ORDER BY changed_at LIMIT 10")?;
            let maps = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            Ok(maps)
        })?;
        for map in maps {
            if let Err(error) = self.refresh_search(&map, false, now) {
                eprintln!("search projection failed for {map}: {}", error.body.message);
            }
        }
        Ok(())
    }
    pub fn search_status(&self, map: &str) -> ApiResult<Value> {
        self.with_conn(|conn| {
            let (c, k) = config(conn)?;
            let now = now_ms();
            let queued: i64 = conn.query_row("SELECT count(*) FROM embedding_jobs WHERE map_id=?1", [map], |r| r.get(0))?;
            let running: i64 = conn.query_row("SELECT count(*) FROM embedding_jobs WHERE map_id=?1 AND lease_until>?2", params![map, now], |r| r.get(0))?;
            let total: i64 = conn.query_row("SELECT count(*) FROM search_nodes WHERE map_id=?1", [map], |r| r.get(0))?;
            let indexed: i64 = conn.query_row(
                "SELECT count(*) FROM search_nodes n WHERE map_id=?1 AND NOT EXISTS(SELECT 1 FROM
search_chunks c WHERE c.map_id=n.map_id AND c.node_id=n.node_id AND (c.vector IS NULL OR
c.fingerprint<>?2))",
                params![map, c.fingerprint()],
                |r| r.get(0),
            )?;
            let failed: i64 = conn.query_row("SELECT count(*) FROM embedding_jobs WHERE map_id=?1 AND attempts>=?2", params![map, MAX_ATTEMPTS], |r| r.get(0))?;
            let pending: i64 = conn.query_row("SELECT count(*) FROM embedding_jobs WHERE map_id=?1 AND attempts<?2 AND lease_until<=?3", params![map, MAX_ATTEMPTS, now], |r| r.get(0))?;
            let (passages_total, passages_indexed): (i64, i64) = conn.query_row(
                "SELECT count(*),coalesce(sum(CASE WHEN vector IS NOT NULL AND fingerprint=?2 THEN 1 ELSE 0 END),0) FROM search_chunks WHERE map_id=?1",
                params![map, c.fingerprint()], |r| Ok((r.get(0)?,r.get(1)?)),
            )?;
            let last_synced_at: Option<i64> = conn.query_row(
                "SELECT last_synced_at FROM embedding_sync_history WHERE map_id=?1 AND fingerprint=?2",
                params![map, c.fingerprint()], |r| r.get(0),
            ).optional()?;
            let projection = projection_failure(conn, map)?;
            let stale = projection.is_some() || is_dirty(conn, map)?;
            let error = match &projection {
                Some(message) => Some(message.clone()),
                None => conn
                    .query_row("SELECT last_error FROM embedding_jobs WHERE map_id=?1 AND last_error IS NOT NULL ORDER BY attempts DESC LIMIT 1", [map], |r| r.get(0))
                    .optional()?,
            };
            Ok(json!({
            "configured":!k.is_empty(),"queued":queued,"running":running,"failed":failed,"indexed":indexed,"total":total,"last_error":error,"projection":if stale{"stale"}else{"current"},
            "pending":pending,"passages_total":passages_total,"passages_indexed":passages_indexed,"last_synced_at":last_synced_at}
            ))
        })
    }
    pub fn claim_embedding_job(&self, now: i64) -> ApiResult<Option<EmbeddingJob>> {
        let eligible = self.with_conn(|conn| {
            let (c, k) = config(conn)?;
            if k.is_empty() {
                return Ok(false);
            }
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM embedding_jobs WHERE due_at<=?1 AND lease_until<=?1 AND
fingerprint=?2 AND attempts<?3)",
                params![now, c.fingerprint(), MAX_ATTEMPTS],
                |r| r.get::<_, bool>(0),
            )?)
        })?;
        if !eligible {
            return Ok(None);
        }
        self.with_tx(|tx| {
            let (c, k) = config(tx)?;
            if k.is_empty() {
                return Ok(None);
            }
            let row: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT map_id,node_id,content_hash FROM embedding_jobs WHERE due_at<=?1 AND
lease_until<=?1 AND fingerprint=?2 AND attempts<?3 ORDER BY due_at LIMIT 1",
                    params![now, c.fingerprint(), MAX_ATTEMPTS],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((map, node, hash)) = row else { return Ok(None) };
            let lease = now + 60000;
            tx.execute("UPDATE embedding_jobs SET lease_until=?3 WHERE map_id=?1 AND node_id=?2", params![map, node, lease])?;
            let mut stmt = tx.prepare("SELECT id,heading_path,passage FROM search_chunks WHERE map_id=?1 AND node_id=?2 ORDER BY ordinal")?;
            let chunks = stmt
                .query_map(params![map, node], |r| {
                    let path: String = r.get(1)?;
                    let text: String = r.get(2)?;
                    Ok((r.get(0)?, format!("{path}\n{text}")))
                })?
                .collect::<Result<_, _>>()?;
            Ok(Some(EmbeddingJob {
                map_id: map,
                node_id: node,
                hash,
                fingerprint: c.fingerprint(),
                lease,
                chunks,
            }))
        })
    }
    pub fn finish_embedding_job(
        &self,
        job: &EmbeddingJob,
        vectors: Result<&[Vec<f32>], &str>,
    ) -> ApiResult<bool> {
        let now = now_ms();
        let projected = matches!(self.refresh_search(&job.map_id, false, now), Ok(true));
        self.with_tx(|tx| {
            let (c, _) = config(tx)?;
            if !projected || is_dirty(tx, &job.map_id)? {
                tx.execute(
                    "UPDATE embedding_jobs SET lease_until=0,due_at=?4 WHERE map_id=?1 AND node_id=?2 AND lease_until=?3",
                    params![job.map_id, job.node_id, job.lease, now + c.quiet_seconds * 1000],
                )?;
                return Ok(false);
            }
            if c.fingerprint() != job.fingerprint {
                return Ok(false);
            }
            let valid: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM embedding_jobs WHERE map_id=?1 AND node_id=?2 AND
content_hash=?3 AND fingerprint=?4 AND lease_until=?5)",
                params![job.map_id, job.node_id, job.hash, job.fingerprint, job.lease],
                |r| r.get(0),
            )?;
            if !valid {
                return Ok(false);
            }
            match vectors {
                Ok(vectors) => {
                    if vectors.len() != job.chunks.len() || vectors.iter().any(|v| v.len() != c.dimensions || v.iter().any(|n| !n.is_finite()) || v.iter().all(|n| *n == 0.0)) {
                        return Err(ApiError::internal("Invalid embedding completion"));
                    }
                    for ((id, _), vector) in job.chunks.iter().zip(vectors) {
                        tx.execute(
                            "UPDATE search_chunks SET vector=?2,fingerprint=?3 WHERE id=?1 AND content_hash=?4",
                            params![id, serde_json::to_string(vector).unwrap(), job.fingerprint, job.hash],
                        )?;
                    }
                    tx.execute("DELETE FROM embedding_jobs WHERE map_id=?1 AND node_id=?2", params![job.map_id, job.node_id])?;
                    // Only an accepted provider completion can establish a timestamp.
                    // In particular, legacy vectors and an unchanged manual sync do not.
                    tx.execute(
                        "INSERT INTO embedding_sync_history(map_id,fingerprint,last_synced_at)
SELECT ?1,?2,?3 WHERE
EXISTS(SELECT 1 FROM search_chunks WHERE map_id=?1) AND
NOT EXISTS(SELECT 1 FROM embedding_jobs WHERE map_id=?1) AND
NOT EXISTS(SELECT 1 FROM search_failures WHERE map_id=?1) AND
NOT EXISTS(SELECT 1 FROM search_chunks WHERE map_id=?1 AND (vector IS NULL OR fingerprint<>?2))
ON CONFLICT(map_id,fingerprint) DO UPDATE SET last_synced_at=excluded.last_synced_at",
                        params![job.map_id, job.fingerprint, now],
                    )?;
                }
                Err(error) => {
                    tx.execute(
                        "UPDATE embedding_jobs SET
lease_until=0,attempts=attempts+1,due_at=?3+min(300000,10000*(attempts+1)),last_error=?4
WHERE map_id=?1 AND node_id=?2",
                        params![job.map_id, job.node_id, now_ms(), error],
                    )?;
                }
            }
            Ok(true)
        })
    }
}
pub fn terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .take(16)
        .map(str::to_lowercase)
        .collect()
}
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return -1.0;
    }
    let mut dot = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    for (&a, &b) in a.iter().zip(b) {
        dot += (a as f64) * (b as f64);
        aa += (a as f64).powi(2);
        bb += (b as f64).powi(2);
    }
    if aa == 0.0 || bb == 0.0 {
        return -1.0;
    }
    dot / (aa * bb).sqrt()
}
fn excerpt_start(passage: &str, tokens: &[String]) -> usize {
    let mut folded = String::new();
    let mut origin = Vec::new();
    for (index, c) in passage.chars().enumerate() {
        for lower in c.to_lowercase() {
            folded.push(lower);
            origin.push(index);
        }
    }
    tokens
        .iter()
        .filter_map(|token| folded.find(token.as_str()))
        .min()
        .and_then(|byte| origin.get(folded[..byte].chars().count()).copied())
        .unwrap_or(0)
}
impl Store {
    pub fn search_document(
        &self,
        map: &str,
        query: &str,
        vector: Option<(&[f32], String)>,
    ) -> ApiResult<SearchOutcome> {
        let projection_error = self.project_search(map, now_ms())?;
        self.with_conn(|conn| {
            let (config, key) = config(conn)?;
            let fingerprint = config.fingerprint();
            let vector = vector.filter(|(_, generation)| generation == &fingerprint).map(|(v, _)| v);
            let tokens = terms(query);
            let outcome = SearchOutcome {
                configured: !key.is_empty(),
                projection_error,
                ..Default::default()
            };
            if tokens.is_empty() {
                return Ok(outcome);
            }
            let fts = tokens.iter().map(|s| format!("\"{}\"", s.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ");
            let mut lexical = conn.prepare(
                "SELECT c.id,c.node_id FROM search_fts JOIN search_chunks c ON c.id=search_fts.rowid WHERE search_fts
MATCH ?1 AND c.map_id=?2 ORDER BY bm25(search_fts,4,2,1) LIMIT ?3",
            )?;
            let lexical_hits: Vec<(i64, String)> = lexical
                .query_map(params![fts, map, CANDIDATE_LIMIT as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            let mut scores: HashMap<i64, (String, f64, bool, bool)> = lexical_hits
                .into_iter()
                .enumerate()
                .map(|(rank, (id, node))| (id, (node, 1.0 / (60.0 + rank as f64), true, false)))
                .collect();
            if let Some(q) = vector {
                let mut stmt = conn.prepare("SELECT id,node_id,vector FROM search_chunks WHERE map_id=?1 AND fingerprint=?2 AND vector IS NOT NULL")?;
                let mut ranked = Vec::new();
                for row in stmt.query_map(params![map, fingerprint], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
                    let (id, node, value) = row?;
                    if let Ok(v) = serde_json::from_str::<Vec<f32>>(&value) {
                        let similarity = cosine(q, &v);
                        if similarity > 0.2 {
                            ranked.push((id, node, similarity));
                        }
                    }
                }
                ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
                for (rank, (id, node, _)) in ranked.into_iter().take(CANDIDATE_LIMIT).enumerate() {
                    let s = scores.entry(id).or_insert((node, 0.0, false, false));
                    s.1 += 1.0 / (60.0 + rank as f64);
                    s.3 = true;
                }
            }
            let candidates = scores.values().map(|(node, ..)| node.as_str()).collect::<HashSet<_>>().len();
            let mut ranked: Vec<_> = scores.into_iter().collect();
            ranked.sort_by(|a, b| b.1 .1.total_cmp(&a.1 .1).then(a.0.cmp(&b.0)));
            let mut seen = HashSet::new();
            let mut hits = Vec::new();
            for (id, (node, _, keyword, semantic)) in ranked {
                if !seen.insert(node) {
                    continue;
                }
                let (node, title, path, passage): (String, String, String, String) = conn.query_row("SELECT node_id,title,heading_path,passage FROM search_chunks WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?;
                let first = if keyword { excerpt_start(&passage, &tokens) } else { 0 };
                let start = first.saturating_sub(60);
                let excerpt: String = passage.chars().skip(start).take(280).collect();
                let highlights = if keyword {
                    let folded = excerpt.to_lowercase();
                    tokens.iter().filter(|token| folded.contains(token.as_str())).cloned().collect()
                } else {
                    vec![]
                };
                hits.push(SearchHit {
                    node_id: node,
                    title,
                    heading_path: serde_json::from_str(&path).unwrap_or_default(),
                    excerpt,
                    passage,
                    highlights,
                    match_kind: if keyword && semantic {
                        "both"
                    } else if keyword {
                        "keyword"
                    } else {
                        "semantic"
                    }
                    .into(),
                });
                if hits.len() == RESULT_LIMIT {
                    break;
                }
            }
            Ok(SearchOutcome {
                hits,
                used_vectors: vector.is_some(),
                candidates,
                ..outcome
            })
        })
    }
}
impl AsRef<Store> for Store {
    fn as_ref(&self) -> &Store {
        self
    }
}
async fn off_runtime<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::internal(format!("search worker task failed: {e}")))?
}
/// One bounded worker pass. Provider work never holds a SQLite lock, and the
/// log replay never runs on an async runtime thread.
pub async fn process_jobs<S>(store: Arc<S>) -> ApiResult<()>
where
    S: AsRef<Store> + Send + Sync + 'static,
{
    let refresher = store.clone();
    off_runtime(move || AsRef::<Store>::as_ref(&*refresher).refresh_dirty_search(now_ms())).await?;
    for _ in 0..8 {
        let Some(job) = AsRef::<Store>::as_ref(&*store).claim_embedding_job(now_ms())? else {
            break;
        };
        let (config, key) = AsRef::<Store>::as_ref(&*store).embedding_config()?;
        let mut vectors = Vec::new();
        let mut failure = None;
        for batch in job.chunks.chunks(32) {
            let texts = batch
                .iter()
                .map(|(_, text)| text.clone())
                .collect::<Vec<_>>();
            match crate::embeddings::embed(&config, &key, &texts, false).await {
                Ok(v) => vectors.extend(v),
                Err(e) => {
                    failure = Some(e.body.message.clone());
                    break;
                }
            }
        }
        let finisher = store.clone();
        let (map, node) = (job.map_id.clone(), job.node_id.clone());
        if let Err(error) = off_runtime(move || {
            let store: &Store = (*finisher).as_ref();
            match failure {
                Some(message) => store.finish_embedding_job(&job, Err(&message)),
                None => store.finish_embedding_job(&job, Ok(&vectors)),
            }
        })
        .await
        {
            eprintln!(
                "search indexing could not finish {map}/{node}: {}",
                error.body.message
            );
        }
    }
    Ok(())
}
