#![deny(unsafe_code)]
//! Orchestrated ingestion pipeline for Orbit memory.
//!
//! Executes the full end-to-end processing pipeline:
//! Content Hash Deduplication → Secret Redaction → AST Chunking →
//! Batch Embedding Generation → Atomic Persistence into `orbit_documents` and `orbit_chunks`.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use buzz_ai::EmbedProvider;
use buzz_core::memory::{compute_content_hash, Document, EmbeddingRecord};
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_db::memory::traits::{MetadataStore, VectorStore};
use buzz_db::redactor::SecretRedactor;
use serde::{Deserialize, Serialize};
use crate::chunker::AstChunker;
use crate::error::Result;
use crate::git::GitMetadataExtractor;
use crate::graph_extractor::KnowledgeGraphExtractor;
use crate::watcher::WorkspaceWatcher;

/// Summary report returned after a batch workspace ingestion run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestSummary {
    /// Files successfully processed and indexed.
    pub files_ingested: usize,
    /// Files skipped due to content hash match (zero re-embedding cost).
    pub files_skipped_dedup: usize,
    /// Total AST chunks generated.
    pub total_chunks_created: usize,
    /// Total dense vector embeddings saved.
    pub total_vectors_saved: usize,
    /// Total knowledge graph entities created or updated.
    pub total_entities_created: usize,
    /// Total knowledge graph relations created.
    pub total_relations_created: usize,
}

/// Orchestrates file ingestion across Redactor, Chunker, Embedder, Knowledge Graph, and Storage.
#[derive(Clone)]
pub struct IngestionPipeline {
    store: Arc<EmbeddedMemoryStore>,
    embedder: Arc<dyn EmbedProvider>,
    chunker: AstChunker,
    graph_extractor: KnowledgeGraphExtractor,
}

impl IngestionPipeline {
    /// Creates a new ingestion pipeline.
    pub fn new(store: Arc<EmbeddedMemoryStore>, embedder: Arc<dyn EmbedProvider>) -> Self {
        Self {
            store,
            embedder,
            chunker: AstChunker::default(),
            graph_extractor: KnowledgeGraphExtractor::new(),
        }
    }

    /// Creates a new ingestion pipeline with a custom chunker configuration.
    pub fn with_chunker(
        store: Arc<EmbeddedMemoryStore>,
        embedder: Arc<dyn EmbedProvider>,
        chunker: AstChunker,
    ) -> Self {
        Self {
            store,
            embedder,
            chunker,
            graph_extractor: KnowledgeGraphExtractor::new(),
        }
    }

    /// Access the underlying memory store.
    pub fn store(&self) -> &Arc<EmbeddedMemoryStore> {
        &self.store
    }

    /// Ingests a single file.
    ///
    /// Returns `Ok(Some(Document))` if newly ingested or modified,
    /// or `Ok(None)` if skipped due to content-hash deduplication.
    pub async fn ingest_file(
        &self,
        workspace_path: &str,
        file_path: impl AsRef<Path>,
        scope: &str,
    ) -> Result<Option<Document>> {
        let path = file_path.as_ref();
        if !path.exists() {
            // File was deleted; perform cascade deletion
            self.delete_file(workspace_path, path).await?;
            return Ok(None);
        }

        let raw_bytes = fs::read(path)?;
        let content_hash = compute_content_hash(&raw_bytes);
        let mtime_ns = WorkspaceWatcher::get_file_mtime_ns(path).unwrap_or(0);

        // 1. Content Hash Deduplication Check
        if let Some(existing) = self
            .store
            .metadata()
            .get_document_by_hash(workspace_path, &content_hash)
            .await?
        {
            if existing.mtime_ns == mtime_ns {
                // Completely unchanged; skip reprocessing
                return Ok(None);
            }
        }

        // 2. Secret Redaction
        let raw_text = String::from_utf8_lossy(&raw_bytes);
        let clean_content = SecretRedactor::redact(&raw_text);

        // 3. Git Metadata Enrichment
        let git_meta = GitMetadataExtractor::extract(workspace_path);
        let metadata_json = serde_json::json!({
            "git": git_meta,
            "file_name": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        });

        // 4. Create or Update Document Record
        let source_uri = path.to_string_lossy().to_string();
        let mut doc = Document::new(
            "file",
            &source_uri,
            workspace_path,
            clean_content.as_bytes(),
            mtime_ns,
            metadata_json,
        );
        if let Some(existing) = self.store.metadata().get_document_by_uri(workspace_path, &source_uri)? {
            doc.id = existing.id;
            doc.created_at = existing.created_at;
        }
        self.store.metadata().upsert_document(&doc).await?;

        // 5. AST-Aware Semantic Chunking
        let chunks = self.chunker.chunk(doc.id, workspace_path, path, &clean_content, scope);
        if chunks.is_empty() {
            return Ok(Some(doc));
        }

        // Clean previous chunks for this document
        self.store.metadata().delete_chunks_by_document(&doc.id).await?;
        self.store.vectors().delete_by_document_id(&doc.id).await?;

        // 6. Batch Embedding Generation
        let chunk_texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = self.embedder.embed_batch(&chunk_texts).await?;

        // 7. Atomic Persistence into Metadata & Vector Stores
        let mut embedding_records = Vec::with_capacity(chunks.len());
        for (chunk, embedding) in chunks.iter().zip(embeddings.into_iter()) {
            self.store.metadata().upsert_chunk(chunk).await?;

            embedding_records.push(EmbeddingRecord {
                chunk_id: chunk.id,
                document_id: doc.id,
                workspace_path: workspace_path.to_string(),
                embedding,
                content: chunk.content.clone(),
                created_at: chunk.created_at,
            });
        }

        self.store.vectors().upsert_embeddings(&embedding_records).await?;

        // 8. Knowledge Graph Extraction & Ingestion
        let extracted_graph = self.graph_extractor.extract_from_file_chunks(
            workspace_path,
            &source_uri,
            &chunks,
        );
        if !extracted_graph.entities.is_empty() || !extracted_graph.relations.is_empty() {
            self.store.remember_graph(&extracted_graph.entities, &extracted_graph.relations).await?;
        }

        Ok(Some(doc))
    }

    /// Deletes a file and cascades deletion across metadata and vectors.
    pub async fn delete_file(&self, workspace_path: &str, file_path: impl AsRef<Path>) -> Result<bool> {
        let uri = file_path.as_ref().to_string_lossy().to_string();
        let doc_opt = self.store.metadata().get_document_by_uri(workspace_path, &uri)?;
        if let Some(doc) = doc_opt {
            self.store.forget_document(&doc.id).await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Ingests all non-ignored files in a workspace.
    pub async fn ingest_workspace(
        &self,
        workspace_path: impl AsRef<Path>,
        scope: &str,
    ) -> Result<IngestSummary> {
        let root = workspace_path.as_ref();
        let watcher = WorkspaceWatcher::new(root);
        let files = watcher.discover_files()?;

        let mut summary = IngestSummary::default();
        let ws_str = root.to_string_lossy().to_string();

        for file in files {
            match self.ingest_file(&ws_str, &file, scope).await? {
                Some(doc) => {
                    summary.files_ingested += 1;
                    let chunks = self.store.metadata().get_chunks_by_document(&doc.id).await?;
                    summary.total_chunks_created += chunks.len();
                    summary.total_vectors_saved += chunks.len();
                }
                None => {
                    summary.files_skipped_dedup += 1;
                }
            }
        }

        // Tally total graph entities and relations
        let stats = self.store.stats().await?;
        summary.total_entities_created = stats.total_entities;
        summary.total_relations_created = stats.total_relations;

        Ok(summary)
    }
}
