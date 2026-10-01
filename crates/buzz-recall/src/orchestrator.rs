#![deny(unsafe_code)]
//! Background sync orchestrator and multi-agent transcript ingestion engine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use buzz_ai::EmbedProvider;
use buzz_core::memory::{Document, EmbeddingRecord};
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_db::memory::traits::{MetadataStore, VectorStore};
use buzz_plugins::{RawDocument, RecallPlugin};
use crate::chunker::SessionChunker;
use crate::error::Result;
use crate::normalizer::extract_session_decisions;
use crate::parsers::{
    AgyCliRecallPlugin, AntigravityRecallPlugin, ClaudeCodeRecallPlugin, CodexRecallPlugin,
    CursorRecallPlugin, GooseRecallPlugin, KimiRecallPlugin, OpenCodeRecallPlugin,
    ZCodeRecallPlugin,
};

/// Summary report returned after a recall ingestion run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RecallIngestSummary {
    /// Number of distinct agent harnesses scanned.
    pub total_detected_agents: usize,
    /// Number of new or modified sessions parsed and ingested.
    pub sessions_ingested: usize,
    /// Number of unchanged sessions skipped via content-hash deduplication.
    pub sessions_skipped_dedup: usize,
    /// Total context chunks generated.
    pub chunks_created: usize,
    /// Total dense vector embeddings persisted.
    pub vectors_saved: usize,
    /// Total architectural decisions or task resolutions extracted.
    pub decisions_extracted: usize,
}

/// Orchestrates multi-agent transcript discovery, redaction, chunking, and vector persistence.
#[derive(Clone)]
pub struct RecallOrchestrator {
    store: Arc<EmbeddedMemoryStore>,
    embedder: Arc<dyn EmbedProvider>,
    plugins: Vec<Arc<dyn RecallPlugin>>,
    chunker: SessionChunker,
    transcript_cache_dir: Option<PathBuf>,
}

impl RecallOrchestrator {
    /// Creates a new orchestrator with the specified store and embedder.
    pub fn new(store: Arc<EmbeddedMemoryStore>, embedder: Arc<dyn EmbedProvider>) -> Self {
        Self {
            store,
            embedder,
            plugins: Vec::new(),
            chunker: SessionChunker::default(),
            transcript_cache_dir: None,
        }
    }

    /// Creates an orchestrator pre-registered with all 9 standard IDE transcript parsers.
    pub fn with_all_default_plugins(store: Arc<EmbeddedMemoryStore>, embedder: Arc<dyn EmbedProvider>) -> Self {
        let mut orch = Self::new(store, embedder);
        orch.register_plugin(Arc::new(AntigravityRecallPlugin::new()));
        orch.register_plugin(Arc::new(ClaudeCodeRecallPlugin::new()));
        orch.register_plugin(Arc::new(CodexRecallPlugin::new()));
        orch.register_plugin(Arc::new(CursorRecallPlugin::new()));
        orch.register_plugin(Arc::new(GooseRecallPlugin::new()));
        orch.register_plugin(Arc::new(OpenCodeRecallPlugin::new()));
        orch.register_plugin(Arc::new(ZCodeRecallPlugin::new()));
        orch.register_plugin(Arc::new(AgyCliRecallPlugin::new()));
        orch.register_plugin(Arc::new(KimiRecallPlugin::new()));
        orch
    }

    /// Sets custom transcript cache directory (default is `<store.base_dir>/transcripts`).
    pub fn with_transcript_cache_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.transcript_cache_dir = Some(path.as_ref().to_path_buf());
        self
    }

    /// Registers a recall parser plugin.
    pub fn register_plugin(&mut self, plugin: Arc<dyn RecallPlugin>) {
        self.plugins.push(plugin);
    }

    /// Access the underlying memory store.
    pub fn store(&self) -> &Arc<EmbeddedMemoryStore> {
        &self.store
    }

    /// Returns list of agent harnesses that are currently detected on the machine.
    pub fn detect_available_agents(&self) -> Vec<&'static str> {
        self.plugins
            .iter()
            .filter(|p| p.detect())
            .map(|p| p.name())
            .collect()
    }

    /// Resolves the transcript caching directory on disk (`orbit_brain/transcripts/` or `<base_dir>/transcripts/`).
    pub fn cache_dir(&self) -> PathBuf {
        if let Some(ref custom) = self.transcript_cache_dir {
            custom.clone()
        } else {
            self.store.base_dir().join("transcripts")
        }
    }

    /// Caches a raw transcript copy safely in the local transcript cache.
    pub fn cache_raw_transcript(&self, raw: &RawDocument, agent_name: &str) -> Result<PathBuf> {
        let target_dir = self.cache_dir().join(agent_name);
        fs::create_dir_all(&target_dir)?;

        let session_id = raw
            .metadata
            .get("session_id")
            .and_then(|s| s.as_str())
            .unwrap_or("session");
        let safe_name = session_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let target_path = target_dir.join(format!("{safe_name}.jsonl"));

        let payload = serde_json::json!({
            "source_uri": raw.source_uri,
            "workspace_path": raw.workspace_path,
            "content_hash": raw.content_hash,
            "mtime_ns": raw.mtime_ns,
            "metadata": raw.metadata,
            "turns": raw.turns,
            "normalized_content": raw.content,
        });

        fs::write(&target_path, serde_json::to_string_pretty(&payload)?)?;
        Ok(target_path)
    }

    /// Ingests a single session document with content-hash deduplication, chunking, and embedding.
    pub async fn ingest_raw_session(
        &self,
        raw: &RawDocument,
        agent_name: &str,
    ) -> Result<Option<Document>> {
        let workspace_str = &raw.workspace_path;

        // 1. Content Hash Deduplication & Watermark Check
        if let Some(existing) = self
            .store
            .metadata()
            .get_document_by_hash(workspace_str, &raw.content_hash)
            .await?
        {
            if existing.mtime_ns == raw.mtime_ns {
                // Completely unchanged session; skip re-embedding
                return Ok(None);
            }
        }

        // 2. Cache transcript locally in orbit_brain/transcripts/
        if let Err(e) = self.cache_raw_transcript(raw, agent_name) {
            warn!("Failed to cache transcript for agent {agent_name}: {e}");
        }

        // 3. Upsert Document Record
        let mut doc = raw.to_document();
        if let Some(existing) = self
            .store
            .metadata()
            .get_document_by_uri(workspace_str, &raw.source_uri)?
        {
            doc.id = existing.id;
            doc.created_at = existing.created_at;
        }
        self.store.metadata().upsert_document(&doc).await?;

        // 4. Semantic Chunking
        let chunks = self.chunker.chunk_session(
            doc.id,
            workspace_str,
            &raw.content,
            agent_name,
            "session",
        );

        if chunks.is_empty() {
            return Ok(Some(doc));
        }

        // 5. Clean prior chunks for this document
        self.store.metadata().delete_chunks_by_document(&doc.id).await?;
        self.store.vectors().delete_by_document_id(&doc.id).await?;

        // 6. Batch Embedding Generation
        let chunk_texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = self.embedder.embed_batch(&chunk_texts).await?;

        // 7. Atomic Persistence of Chunks and Embeddings
        let mut embedding_records = Vec::with_capacity(chunks.len());
        for (chunk, embedding) in chunks.iter().zip(embeddings.into_iter()) {
            self.store.metadata().upsert_chunk(chunk).await?;

            embedding_records.push(EmbeddingRecord {
                chunk_id: chunk.id,
                document_id: doc.id,
                workspace_path: workspace_str.clone(),
                embedding,
                content: chunk.content.clone(),
                created_at: chunk.created_at,
            });
        }
        self.store.vectors().upsert_embeddings(&embedding_records).await?;

        Ok(Some(doc))
    }

    /// Discovers and ingests all sessions from all registered plugins for a workspace.
    pub async fn ingest_all(&self, workspace_path: &Path) -> Result<RecallIngestSummary> {
        let mut summary = RecallIngestSummary::default();
        let workspace_str = workspace_path.to_string_lossy().to_string();

        for plugin in &self.plugins {
            let name = plugin.name();
            if !plugin.detect() {
                continue;
            }

            summary.total_detected_agents += 1;
            let sessions = plugin.ingest_sessions(workspace_path).await;

            for mut raw in sessions {
                // Ensure workspace path matches
                if raw.workspace_path.is_empty() {
                    raw.workspace_path = workspace_str.clone();
                }

                let decisions = extract_session_decisions(&raw.turns);
                summary.decisions_extracted += decisions.len();

                match self.ingest_raw_session(&raw, name).await? {
                    Some(doc) => {
                        summary.sessions_ingested += 1;
                        let count = self
                            .store
                            .metadata()
                            .get_chunks_by_document(&doc.id)
                            .await?
                            .len();
                        summary.chunks_created += count;
                        summary.vectors_saved += count;
                    }
                    None => {
                        summary.sessions_skipped_dedup += 1;
                    }
                }
            }
        }

        Ok(summary)
    }

    /// Starts a background synchronization loop polling transcripts at regular intervals.
    /// Default interval is 300 seconds (5 minutes) or configured via `BUZZ_RECALL_SYNC_INTERVAL_SEC`.
    pub fn start_background_sync(
        self: Arc<Self>,
        workspace_path: PathBuf,
        interval_secs: Option<u64>,
    ) -> tokio::task::JoinHandle<()> {
        let interval = interval_secs
            .or_else(|| {
                std::env::var("BUZZ_RECALL_SYNC_INTERVAL_SEC")
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(300);

        tokio::spawn(async move {
            info!(
                "Starting Recall background sync loop (every {interval}s) for workspace: {:?}",
                workspace_path
            );
            loop {
                match self.ingest_all(&workspace_path).await {
                    Ok(summary) => {
                        if summary.sessions_ingested > 0 {
                            info!(
                                "Recall sync completed: {} sessions ingested, {} chunks created",
                                summary.sessions_ingested, summary.chunks_created
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Recall background sync encountered error: {e}");
                    }
                }
                sleep(Duration::from_secs(interval)).await;
            }
        })
    }
}
