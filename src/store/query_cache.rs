//! Optional, rebuildable query-vector cache. Authoritative content never lives here.
use super::Store;
use crate::error::ApiResult;
use rusqlite::{params, OptionalExtension};

impl Store {
    pub fn query_cache_generation(&self) -> ApiResult<u64> {
        self.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT generation FROM query_cache_generation WHERE id=1",
                [],
                |r| r.get(0),
            )?)
        })
    }

    pub fn cached_query_vector(
        &self,
        key: &str,
        generation: u64,
        dimensions: usize,
        now: i64,
    ) -> ApiResult<Option<(i64, Vec<f32>)>> {
        let row: Option<(i64, String)> = self.with_conn(|conn| Ok(conn.query_row(
            "SELECT expires_at,vector FROM query_embedding_cache WHERE cache_key=?1 AND generation=?2 AND expires_at>?3 AND generation=(SELECT generation FROM query_cache_generation WHERE id=1)",
            params![key, generation, now], |r| Ok((r.get(0)?,r.get(1)?))).optional()?))?;
        let Some((expiry, raw)) = row else {
            return Ok(None);
        };
        let Ok(vector) = serde_json::from_str::<Vec<f32>>(&raw) else {
            return Ok(None);
        };
        if vector.len() != dimensions
            || vector.iter().any(|v| !v.is_finite())
            || !vector.iter().any(|v| *v != 0.0)
        {
            return Ok(None);
        }
        self.cache_transaction(|tx| {
            tx.execute(
                "UPDATE query_embedding_cache SET last_used=?2 WHERE cache_key=?1",
                params![key, now],
            )?;
            Ok(())
        })?;
        Ok(Some((expiry, vector)))
    }

    pub fn cache_query_vector(
        &self,
        key: &str,
        generation: u64,
        vector: &[f32],
        expires_at: i64,
        now: i64,
        capacity: usize,
    ) -> ApiResult<()> {
        let raw = serde_json::to_string(vector).expect("validated finite vector");
        self.cache_transaction(|tx| {
            tx.execute("DELETE FROM query_embedding_cache WHERE expires_at<=?1", [now])?;
            tx.execute("INSERT INTO query_embedding_cache(cache_key,generation,vector,expires_at,last_used) SELECT ?1,?2,?3,?4,?5 WHERE ?4>?5 AND ?2=(SELECT generation FROM query_cache_generation WHERE id=1) ON CONFLICT(cache_key) DO UPDATE SET generation=excluded.generation,vector=excluded.vector,expires_at=excluded.expires_at,last_used=excluded.last_used",params![key,generation,raw,expires_at,now])?;
            tx.execute("DELETE FROM query_embedding_cache WHERE cache_key IN (SELECT cache_key FROM query_embedding_cache ORDER BY last_used DESC,cache_key LIMIT -1 OFFSET ?1)",[capacity as i64])?;
            Ok(())
        })
    }
}
