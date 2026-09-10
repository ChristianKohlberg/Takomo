//! Bounded query vectors with optional restart-persistent storage. Never caches document results or errors.
use crate::{
    auth::debit_shared_window, ids::now_ms, server::AppState, store::search::EmbeddingConfig,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{watch, Semaphore};

pub const QUERY_EMBEDDINGS_PER_MINUTE: i64 = 60;
const CAPACITY: usize = 256;
const TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub enum QueryEmbedding {
    Ready(Arc<Vec<f32>>),
    Throttled,
    Unavailable,
}

type Completion = Option<(i64, QueryEmbedding)>;
struct Entry {
    result: watch::Receiver<Completion>,
    touched: u64,
}
#[derive(Default)]
struct Entries {
    generation: u64,
    clock: u64,
    items: HashMap<String, Entry>,
}
#[derive(Clone)]
pub struct QueryCache {
    entries: Arc<Mutex<Entries>>,
    /// token id -> unix-ms timestamps of outbound provider calls. Its own map
    /// rather than the token's `rate`: a search is a read, so it must not spend
    /// the write budget, yet each uncached query is a paid provider call. Hits
    /// and coalesced waiters never reach it.
    rate: Arc<Mutex<HashMap<String, VecDeque<i64>>>>,
    pending: Arc<Semaphore>,
    capacity: usize,
    ttl: Duration,
}
impl Default for QueryCache {
    fn default() -> Self {
        Self::new(CAPACITY, TTL)
    }
}
/// Hash the complete identity: no raw credential or query in map keys or diagnostics.
fn cache_key(config: &EmbeddingConfig, credential: &str, token_id: &str, query: &str) -> String {
    let identity =
        serde_json::to_vec(&(config.fingerprint(), credential, token_id, query.trim())).unwrap();
    format!("{:x}", Sha256::digest(identity))
}

impl QueryCache {
    /// Explicit bounds also permit short, deterministic expiration tests.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        assert!(capacity > 0);
        Self {
            entries: Arc::default(),
            rate: Arc::default(),
            pending: Arc::new(Semaphore::new(capacity)),
            capacity,
            ttl,
        }
    }
    pub fn invalidate(&self) {
        let mut entries = self.entries.lock().unwrap();
        entries.generation = entries.generation.wrapping_add(1);
        entries.items.clear();
    }
    pub fn synchronize_generation(&self, generation: u64) {
        let mut entries = self.entries.lock().unwrap();
        if generation > entries.generation {
            entries.generation = generation;
            entries.items.clear();
        }
    }
    pub async fn get_persistent(
        &self,
        state: Arc<AppState>,
        config: &EmbeddingConfig,
        credential: &str,
        token_id: &str,
        query: &str,
    ) -> (u64, QueryEmbedding) {
        let reader = state.clone();
        let generation =
            match crate::api::blocking_read(move || reader.store.query_cache_generation()).await {
                Ok(generation) => generation,
                Err(_) => return (0, QueryEmbedding::Unavailable),
            };
        self.synchronize_generation(generation);
        self.get_inner(config, credential, token_id, query, Some(state))
            .await
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.entries.lock().unwrap().generation == generation
    }
    pub async fn get(
        &self,
        config: &EmbeddingConfig,
        credential: &str,
        token_id: &str,
        query: &str,
    ) -> (u64, QueryEmbedding) {
        self.get_inner(config, credential, token_id, query, None)
            .await
    }
    async fn get_inner(
        &self,
        config: &EmbeddingConfig,
        credential: &str,
        token_id: &str,
        query: &str,
        persistent: Option<Arc<AppState>>,
    ) -> (u64, QueryEmbedding) {
        let key = cache_key(config, credential, token_id, query);
        let (generation, mut receiver, sender) = {
            let mut entries = self.entries.lock().unwrap();
            entries.items.retain(|_, entry| {
                let alive = entry.result.has_changed().is_ok();
                match entry.result.borrow().as_ref() {
                    Some((expiry, _)) => *expiry > now_ms(),
                    None => alive,
                }
            });
            entries.clock = entries.clock.wrapping_add(1);
            let clock = entries.clock;
            let generation = entries.generation;
            if let Some(entry) = entries.items.get_mut(&key) {
                entry.touched = clock;
                (generation, entry.result.clone(), None)
            } else {
                if entries.items.len() >= self.capacity {
                    let oldest = entries
                        .items
                        .iter()
                        .filter(|(_, entry)| entry.result.borrow().is_some())
                        .min_by_key(|(_, entry)| entry.touched)
                        .map(|(key, _)| key.clone());
                    if let Some(oldest) = oldest {
                        entries.items.remove(&oldest);
                    } else {
                        return (generation, QueryEmbedding::Throttled);
                    }
                }
                let (sender, receiver) = watch::channel(None);
                entries.items.insert(
                    key.clone(),
                    Entry {
                        result: receiver.clone(),
                        touched: clock,
                    },
                );
                (generation, receiver, Some(sender))
            }
        };
        if let Some(sender) = sender {
            let cache = self.clone();
            let config = config.clone();
            let credential = credential.to_owned();
            let token_id = token_id.to_owned();
            let query = query.trim().to_owned();
            let key = key.clone();
            // The bounded operation owns completion even if its first HTTP client leaves.
            tokio::spawn(async move {
                let stored = if let Some(state) = persistent.clone() {
                    let stored_key = key.clone();
                    crate::api::blocking_read(move || {
                        state.store.cached_query_vector(
                            &stored_key,
                            generation,
                            config.dimensions,
                            now_ms(),
                        )
                    })
                    .await
                    .ok()
                    .flatten()
                } else {
                    None
                };
                let (expiry, result) = if let Some((expiry, vector)) = stored {
                    (expiry, QueryEmbedding::Ready(Arc::new(vector)))
                } else {
                    let result = match cache.pending.clone().try_acquire_owned() {
                        Ok(_permit) => {
                            if debit_shared_window(
                                &cache.rate,
                                &token_id,
                                QUERY_EMBEDDINGS_PER_MINUTE,
                            )
                            .is_err()
                            {
                                QueryEmbedding::Throttled
                            } else {
                                match tokio::time::timeout(
                                    Duration::from_secs(3),
                                    crate::embeddings::embed(&config, &credential, &[query], true),
                                )
                                .await
                                {
                                    Ok(Ok(mut vectors)) => vectors
                                        .pop()
                                        .map(|v| QueryEmbedding::Ready(Arc::new(v)))
                                        .unwrap_or(QueryEmbedding::Unavailable),
                                    _ => QueryEmbedding::Unavailable,
                                }
                            }
                        }
                        Err(_) => QueryEmbedding::Throttled,
                    };
                    let expiry =
                        now_ms().saturating_add(cache.ttl.as_millis().min(i64::MAX as u128) as i64);
                    if let (Some(state), QueryEmbedding::Ready(vector)) = (persistent, &result) {
                        let stored_key = key.clone();
                        let vector = vector.clone();
                        let capacity = cache.capacity;
                        // Best-effort persistence: cache failure never blocks keyword fallback.
                        let _ = crate::api::blocking_read(move || {
                            state.store.cache_query_vector(
                                &stored_key,
                                generation,
                                &vector,
                                expiry,
                                now_ms(),
                                capacity,
                            )
                        })
                        .await;
                    }
                    (expiry, result)
                };
                let mut entries = cache.entries.lock().unwrap();
                if entries.generation == generation && !matches!(result, QueryEmbedding::Ready(_)) {
                    entries.items.remove(&key);
                }
                // Old generations only finish their existing waiters; never reinsert entries.
                sender.send_replace(Some((expiry, result)));
            });
        }
        loop {
            if let Some((_, result)) = receiver.borrow_and_update().as_ref() {
                return (generation, result.clone());
            }
            if receiver.changed().await.is_err() {
                let mut entries = self.entries.lock().unwrap();
                let dead = entries.items.get(&key).is_some_and(|entry| {
                    entry.result.same_channel(&receiver) && entry.result.borrow().is_none()
                });
                if dead {
                    entries.items.remove(&key);
                }
                return (generation, QueryEmbedding::Unavailable);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn mock_provider() -> (EmbeddingConfig, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let router = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({"data":[{"index":0,"embedding":[1.0,0.0,0.0]}]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = EmbeddingConfig {
            provider: "openai".into(),
            endpoint: format!("http://{}/embeddings", listener.local_addr().unwrap()),
            model: "fixture-v1".into(),
            dimensions: 3,
            ..Default::default()
        };
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (config, calls)
    }

    fn insert_inflight(cache: &QueryCache, key: &str) -> watch::Sender<Completion> {
        let (sender, receiver) = watch::channel(None);
        let mut entries = cache.entries.lock().unwrap();
        entries.items.insert(
            key.to_owned(),
            Entry {
                result: receiver,
                touched: 0,
            },
        );
        sender
    }

    #[tokio::test]
    async fn dead_inflight_entries_release_capacity_and_retry() {
        let (config, calls) = mock_provider().await;
        let cache = QueryCache::new(1, TTL);
        let key = cache_key(&config, "secret", "alice", "query");

        let (owned, cfg) = (cache.clone(), config.clone());
        let waiter = tokio::spawn(async move { owned.get(&cfg, "secret", "alice", "query").await });
        let sender = insert_inflight(&cache, &key);
        tokio::task::yield_now().await;
        drop(sender);
        assert!(matches!(
            waiter.await.unwrap().1,
            QueryEmbedding::Unavailable
        ));
        assert!(
            cache.entries.lock().unwrap().items.is_empty(),
            "a waiter whose sender vanished frees the slot at once"
        );

        drop(insert_inflight(&cache, &key));
        assert!(matches!(
            cache.get(&config, "secret", "alice", "other").await.1,
            QueryEmbedding::Ready(_)
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a dead slot is not capacity"
        );

        drop(insert_inflight(&cache, &key));
        assert!(matches!(
            cache.get(&config, "secret", "alice", "query").await.1,
            QueryEmbedding::Ready(_)
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "the dead identity is recomputed"
        );
    }
}
