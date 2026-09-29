#![deny(unsafe_code)]
//! Embedded vector store implementation for Orbit memory.
//!
//! Stores dense embeddings and retrieval payloads in `~/.orbit/brain/vectors/`
//! with fast cosine similarity search and crash-safe file persistence.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use buzz_core::memory::{EmbeddingRecord, VectorFilter, VectorHit};
use uuid::Uuid;
use crate::error::{DbError, Result};
use crate::memory::traits::VectorStore;

/// Embedded persistent vector store with cosine similarity retrieval.
#[derive(Debug, Clone)]
pub struct EmbeddedVectorStore {
    data_dir: PathBuf,
    // ponytail: in-memory RwLock cache with binary journal persistence, upgrade to LanceDB arrow tables when vectors > 100k
    records: Arc<RwLock<Vec<EmbeddingRecord>>>,
}

impl EmbeddedVectorStore {
    /// Opens or creates the vector store at the given directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| DbError::Internal(format!("Failed to create vector store dir {}: {e}", data_dir.display())))?;

        let store = Self {
            data_dir,
            records: Arc::new(RwLock::new(Vec::new())),
        };

        store.load_from_disk()?;
        Ok(store)
    }

    /// Opens the default vector store at `~/.orbit/brain/vectors/`.
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let default_path = PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("vectors");
        Self::open(default_path)
    }

    fn journal_file(&self) -> PathBuf {
        self.data_dir.join("vectors.bin")
    }

    fn load_from_disk(&self) -> Result<()> {
        let path = self.journal_file();
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(&path)
            .map_err(|e| DbError::Internal(format!("Failed to open vector journal {}: {e}", path.display())))?;
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|e| DbError::Internal(format!("Failed to read vector journal: {e}")))?;

        if bytes.is_empty() {
            return Ok(());
        }

        let records: Vec<EmbeddingRecord> = serde_json::from_slice(&bytes)
            .map_err(|e| DbError::Internal(format!("Failed to deserialize vector records: {e}")))?;

        let mut lock = self.records.write().unwrap();
        *lock = records;
        Ok(())
    }

    fn save_to_disk(&self) -> Result<()> {
        let path = self.journal_file();
        let temp_path = self.data_dir.join("vectors.bin.tmp");

        let bytes = {
            let lock = self.records.read().unwrap();
            serde_json::to_vec(&*lock)
                .map_err(|e| DbError::Internal(format!("Failed to serialize vector records: {e}")))?
        };

        {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_path)
                .map_err(|e| DbError::Internal(format!("Failed to create temp vector journal: {e}")))?;
            let mut writer = BufWriter::new(file);
            writer
                .write_all(&bytes)
                .map_err(|e| DbError::Internal(format!("Failed to write vector journal: {e}")))?;
            writer
                .flush()
                .map_err(|e| DbError::Internal(format!("Failed to flush vector journal: {e}")))?;
        }

        std::fs::rename(&temp_path, &path)
            .map_err(|e| DbError::Internal(format!("Failed to atomically replace vector journal: {e}")))?;

        Ok(())
    }

    /// Computes cosine similarity between two float vectors.
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let mut dot = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;

        for (x, y) in a.iter().zip(b.iter()) {
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }

        if norm_a <= 0.0 || norm_b <= 0.0 {
            0.0
        } else {
            dot / (norm_a.sqrt() * norm_b.sqrt())
        }
    }
}

impl VectorStore for EmbeddedVectorStore {
    async fn upsert_embeddings(&self, items: &[EmbeddingRecord]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }

        {
            let mut lock = self.records.write().unwrap();
            for item in items {
                // Upsert: replace if existing chunk_id exists, else push
                if let Some(pos) = lock.iter().position(|r| r.chunk_id == item.chunk_id) {
                    lock[pos] = item.clone();
                } else {
                    lock.push(item.clone());
                }
            }
        }

        self.save_to_disk()?;
        Ok(())
    }

    async fn search(&self, query: &[f32], filter: &VectorFilter, k: usize) -> Result<Vec<VectorHit>> {
        let lock = self.records.read().unwrap();
        let mut scored: Vec<VectorHit> = Vec::new();

        for record in lock.iter() {
            // Apply workspace filter
            if let Some(ws) = &filter.workspace_path {
                if &record.workspace_path != ws {
                    continue;
                }
            }

            // Apply document filter
            if let Some(doc_id) = &filter.document_id {
                if &record.document_id != doc_id {
                    continue;
                }
            }

            let score = Self::cosine_similarity(query, &record.embedding);
            scored.push(VectorHit {
                chunk_id: record.chunk_id,
                document_id: record.document_id,
                score,
                content: record.content.clone(),
                workspace_path: record.workspace_path.clone(),
            });
        }

        // Sort descending by score
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        Ok(scored)
    }

    async fn delete_by_chunk_id(&self, chunk_id: &Uuid) -> Result<bool> {
        let modified;
        {
            let mut lock = self.records.write().unwrap();
            let before = lock.len();
            lock.retain(|r| r.chunk_id != *chunk_id);
            modified = lock.len() < before;
        }

        if modified {
            self.save_to_disk()?;
        }
        Ok(modified)
    }

    async fn delete_by_document_id(&self, document_id: &Uuid) -> Result<usize> {
        let count;
        {
            let mut lock = self.records.write().unwrap();
            let before = lock.len();
            lock.retain(|r| r.document_id != *document_id);
            count = before - lock.len();
        }

        if count > 0 {
            self.save_to_disk()?;
        }
        Ok(count)
    }

    async fn count_vectors(&self) -> Result<usize> {
        let lock = self.records.read().unwrap();
        Ok(lock.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_vector_store_search_and_reopen() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_vector_test_{}", Uuid::new_v4()));
        let doc_id = Uuid::new_v4();
        let chunk1_id = Uuid::new_v4();
        let chunk2_id = Uuid::new_v4();

        // 1. Insert embeddings
        {
            let store = EmbeddedVectorStore::open(&temp_dir).expect("open vector store");

            let rec1 = EmbeddingRecord {
                chunk_id: chunk1_id,
                document_id: doc_id,
                workspace_path: "/workspace".to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                content: "First chunk".to_string(),
                created_at: Utc::now(),
            };

            let rec2 = EmbeddingRecord {
                chunk_id: chunk2_id,
                document_id: doc_id,
                workspace_path: "/workspace".to_string(),
                embedding: vec![0.0, 1.0, 0.0],
                content: "Second chunk".to_string(),
                created_at: Utc::now(),
            };

            store.upsert_embeddings(&[rec1, rec2]).await.expect("upsert embeddings");
            assert_eq!(store.count_vectors().await.unwrap(), 2);

            // Search with query pointing close to rec1
            let hits = store
                .search(&[0.9, 0.1, 0.0], &VectorFilter::default(), 2)
                .await
                .expect("search");

            assert_eq!(hits.len(), 2);
            assert_eq!(hits[0].chunk_id, chunk1_id);
            assert!(hits[0].score > 0.9);
        }

        // 2. Reopen and verify persistence
        {
            let store = EmbeddedVectorStore::open(&temp_dir).expect("reopen vector store");
            assert_eq!(store.count_vectors().await.unwrap(), 2);

            let deleted = store.delete_by_chunk_id(&chunk1_id).await.expect("delete chunk");
            assert!(deleted);
            assert_eq!(store.count_vectors().await.unwrap(), 1);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
