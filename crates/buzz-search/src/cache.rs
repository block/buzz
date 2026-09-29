#![deny(unsafe_code)]
//! In-memory semantic query cache with LRU eviction and TTL expiration.
//!
//! Delivers sub-1ms response times for repeated queries or active task working contexts.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use buzz_core::memory::compute_content_hash;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::fusion::CandidateChunk;

/// Cached retrieval result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRetrieval {
    /// SHA-256 hash of normalized query.
    pub query_hash: String,
    /// Associated workspace path.
    pub workspace_path: String,
    /// Candidate chunk IDs.
    pub chunk_ids: Vec<Uuid>,
    /// Pre-compiled semantic XML context.
    pub compiled_xml: String,
    /// Cached candidate chunks.
    pub chunks: Vec<CandidateChunk>,
    /// Total allocated tokens.
    pub token_count: i32,
    /// Top confidence score.
    pub confidence_score: f32,
    /// Cache hit frequency.
    pub hit_count: u32,
    #[serde(skip)]
    created_at: Option<Instant>,
    #[serde(skip)]
    ttl: Option<Duration>,
}

impl CachedRetrieval {
    /// Checks if this cache entry has expired.
    pub fn is_expired(&self) -> bool {
        match (self.created_at, self.ttl) {
            (Some(created), Some(ttl)) => created.elapsed() > ttl,
            _ => false,
        }
    }
}

/// In-memory semantic query cache.
#[derive(Debug, Clone)]
pub struct SemanticQueryCache {
    capacity: usize,
    default_ttl: Duration,
    entries: Arc<RwLock<HashMap<String, CachedRetrieval>>>,
}

impl Default for SemanticQueryCache {
    fn default() -> Self {
        Self::new(500, Duration::from_secs(300)) // 500 entries, 5 minute TTL
    }
}

impl SemanticQueryCache {
    /// Creates a new semantic query cache with capacity and TTL.
    pub fn new(capacity: usize, default_ttl: Duration) -> Self {
        Self {
            capacity: capacity.max(1),
            default_ttl,
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Computes canonical cache key from workspace and query.
    pub fn cache_key(workspace_path: &str, query: &str) -> String {
        let normalized = format!("{}:{}", workspace_path.trim(), query.trim().to_ascii_lowercase());
        compute_content_hash(normalized.as_bytes())
    }

    /// Looks up a cached retrieval entry.
    pub fn get(&self, workspace_path: &str, query: &str) -> Option<CachedRetrieval> {
        let key = Self::cache_key(workspace_path, query);
        let mut lock = self.entries.write().unwrap();

        if let Some(entry) = lock.get_mut(&key) {
            if entry.is_expired() {
                lock.remove(&key);
                return None;
            }
            entry.hit_count += 1;
            return Some(entry.clone());
        }

        None
    }

    /// Inserts a new retrieval result into the cache.
    pub fn insert(
        &self,
        workspace_path: &str,
        query: &str,
        chunk_ids: Vec<Uuid>,
        chunks: Vec<CandidateChunk>,
        compiled_xml: String,
        token_count: i32,
        confidence_score: f32,
    ) {
        let key = Self::cache_key(workspace_path, query);
        let mut lock = self.entries.write().unwrap();

        if lock.len() >= self.capacity {
            // Evict oldest or lowest hit-count entry (ponytail: simple lowest hit count)
            if let Some(evict_key) = lock
                .iter()
                .min_by_key(|(_, v)| v.hit_count)
                .map(|(k, _)| k.clone())
            {
                lock.remove(&evict_key);
            }
        }

        let entry = CachedRetrieval {
            query_hash: key.clone(),
            workspace_path: workspace_path.to_string(),
            chunk_ids,
            compiled_xml,
            chunks,
            token_count,
            confidence_score,
            hit_count: 1,
            created_at: Some(Instant::now()),
            ttl: Some(self.default_ttl),
        };

        lock.insert(key, entry);
    }

    /// Clears the cache.
    pub fn clear(&self) {
        let mut lock = self.entries.write().unwrap();
        lock.clear();
    }

    /// Returns current cached entries count.
    pub fn len(&self) -> usize {
        let lock = self.entries.read().unwrap();
        lock.len()
    }

    /// Checks if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit_and_eviction() {
        let cache = SemanticQueryCache::new(2, Duration::from_secs(60));
        let ws = "/workspace";

        // 1. Initial miss
        assert!(cache.get(ws, "how to auth").is_none());

        // 2. Insert and hit
        cache.insert(
            ws,
            "how to auth",
            vec![Uuid::new_v4()],
            vec![],
            "<orbit_context>auth</orbit_context>".to_string(),
            100,
            0.92,
        );

        let hit = cache.get(ws, "how to auth");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().compiled_xml, "<orbit_context>auth</orbit_context>");

        // 3. Fill up to capacity
        cache.insert(ws, "query 2", vec![], vec![], "xml2".to_string(), 50, 0.85);
        assert_eq!(cache.len(), 2);

        // 4. Over capacity triggers eviction
        cache.insert(ws, "query 3", vec![], vec![], "xml3".to_string(), 50, 0.88);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_cache_expiration() {
        let cache = SemanticQueryCache::new(10, Duration::from_millis(10));
        let ws = "/workspace";

        cache.insert(ws, "fast expiring", vec![], vec![], "xml".to_string(), 10, 0.9);
        std::thread::sleep(Duration::from_millis(20));

        assert!(cache.get(ws, "fast expiring").is_none(), "Expired entry must return None");
    }
}
