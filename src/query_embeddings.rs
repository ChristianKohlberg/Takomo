//! Bounded, process-local query vectors. Never caches document results or errors.
use crate::{auth::debit_shared_window, store::search::EmbeddingConfig};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
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

type Completion = Option<(Instant, QueryEmbedding)>;
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
        // Hash the complete identity: no raw credential or query in map keys or diagnostics.
        let identity =
            serde_json::to_vec(&(config.fingerprint(), credential, token_id, query.trim()))
                .unwrap();
        let key = format!("{:x}", Sha256::digest(identity));
        let (generation, mut receiver, sender) = {
            let mut entries = self.entries.lock().unwrap();
            entries.items.retain(|_, entry| {
                entry
                    .result
                    .borrow()
                    .as_ref()
                    .is_none_or(|(at, _)| at.elapsed() < self.ttl)
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
            // The bounded operation owns completion even if its first HTTP client leaves.
            tokio::spawn(async move {
                let result = match cache.pending.clone().try_acquire_owned() {
                    Ok(_permit) => {
                        if debit_shared_window(&cache.rate, &token_id, QUERY_EMBEDDINGS_PER_MINUTE)
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
                let mut entries = cache.entries.lock().unwrap();
                if entries.generation == generation && !matches!(result, QueryEmbedding::Ready(_)) {
                    entries.items.remove(&key);
                }
                // Old generations only finish their existing waiters; never reinsert entries.
                sender.send_replace(Some((Instant::now(), result)));
            });
        }
        loop {
            if let Some((_, result)) = receiver.borrow_and_update().as_ref() {
                return (generation, result.clone());
            }
            if receiver.changed().await.is_err() {
                return (generation, QueryEmbedding::Unavailable);
            }
        }
    }
}
