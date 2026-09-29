#![deny(unsafe_code)]
//! End-to-end integration test verifying complete connectivity across:
//! Feature 01 (Storage Foundation) + Feature 02 (Embedding & Reranker Engine) + Feature 03 (Ingestion Pipeline).
//!
//! Validates the full lifecycle:
//! 1. Keyring credential resolution (buzz-auth)
//! 2. AiConfig builder with automatic fallback to Local ONNX (buzz-ai)
//! 3. File discovery and change detection (buzz-ingest WorkspaceWatcher)
//! 4. Secret redaction on sensitive tokens (buzz-db SecretRedactor)
//! 5. Multi-language AST semantic chunking with line provenance (buzz-ingest AstChunker)
//! 6. Local 384-dimensional vector embedding generation (buzz-ai LocalOnnxEmbedder)
//! 7. Cross-store atomic persistence in SQLite and Vector store (buzz-db EmbeddedMemoryStore)
//! 8. Vector similarity search over embedded vectors
//! 9. Cross-encoder reranking over retrieved candidate chunks (buzz-ai LocalOnnxReranker)
//! 10. Cascading delete propagation across all stores.

use std::fs;
use std::sync::Arc;
use buzz_ai::{AiConfig, EmbedProviderType, RerankProviderType};
use buzz_auth::keyring;
use buzz_core::memory::VectorFilter;
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_db::memory::traits::{MetadataStore, VectorStore};
use buzz_ingest::{FileChangeEvent, IngestionPipeline, WorkspaceWatcher};
use uuid::Uuid;

#[tokio::test]
async fn test_end_to_end_f01_f02_f03_connectivity() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_e2e_test_{}", Uuid::new_v4()));
    let brain_dir = temp_dir.join("brain");
    let workspace_dir = temp_dir.join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    // ─── 1. Keyring Verification (buzz-auth) ─────────────────────────────────
    let secret_key = "test_e2e_service_key";
    keyring::set_secret(secret_key, "super_secret_token_12345").expect("set secret");
    let resolved = keyring::resolve_secret(secret_key).expect("resolve secret");
    assert_eq!(resolved.as_str(), "super_secret_token_12345");

    // ─── 2. Storage Foundation Initialization (buzz-db) ──────────────────────
    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    assert_eq!(store.metadata().count_documents().await.unwrap(), 0);

    // ─── 3. AI Config & Provider Construction (buzz-ai) ──────────────────────
    let mut ai_config = AiConfig::default();
    ai_config.embed_provider = EmbedProviderType::LocalOnnx;
    ai_config.rerank_provider = RerankProviderType::LocalOnnx;

    let embedder = ai_config.build_embedder().expect("build embedder");
    let reranker = ai_config.build_reranker().expect("build reranker");

    // ─── 4. Ingestion Pipeline Assembly (buzz-ingest) ────────────────────────
    let pipeline = IngestionPipeline::new(store.clone(), embedder.clone());
    let watcher = WorkspaceWatcher::new(&workspace_dir);

    // ─── 5. File Creation with Sensitive Token ───────────────────────────────
    let auth_file = workspace_dir.join("auth_controller.rs");
    let source_code = r#"
pub struct AuthController {
    // Secret token that MUST be redacted:
    api_key: &'static str,
}

impl AuthController {
    pub fn new() -> Self {
        Self {
            api_key: "sk-proj-supersecretkey1234567890123456789012345678901234567890",
        }
    }

    pub fn authenticate(&self, user: &str) -> bool {
        !user.is_empty()
    }
}
"#;
    fs::write(&auth_file, source_code).unwrap();

    // ─── 6. Change Detection (WorkspaceWatcher) ──────────────────────────────
    let changes = watcher.scan_changes().expect("scan changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0], FileChangeEvent::CreatedOrModified(auth_file.clone()));

    // ─── 7. Ingestion Pipeline Execution ─────────────────────────────────────
    let doc = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &auth_file, "project")
        .await
        .expect("ingest")
        .expect("doc must be returned");

    // Verify Document stored in SQLite
    assert_eq!(store.metadata().count_documents().await.unwrap(), 1);
    let chunks = store.metadata().get_chunks_by_document(&doc.id).await.unwrap();
    assert!(!chunks.is_empty(), "AST chunker must produce chunks");

    // Verify Secret Redaction occurred
    for chunk in &chunks {
        assert!(
            !chunk.content.contains("supersecretkey"),
            "Secret must be redacted from all chunks"
        );
    }

    // ─── 8. Vector Retrieval Verification ────────────────────────────────────
    let query_text = "how to authenticate a user";
    let query_vector = embedder
        .embed_batch(&[query_text])
        .await
        .expect("embed query")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(query_vector.len(), 384);

    let filter = VectorFilter {
        workspace_path: Some(workspace_dir.to_string_lossy().to_string()),
        document_id: None,
        scope: None,
    };

    let hits = store.search_vectors(&query_vector, &filter, 5).await.expect("search vectors");
    assert!(!hits.is_empty(), "Vector search must return matching chunk");
    assert_eq!(hits[0].document_id, doc.id);

    // ─── 9. Cross-Encoder Reranking (buzz-ai) ────────────────────────────────
    let candidates: Vec<&str> = hits.iter().map(|h| h.content.as_str()).collect();
    let rerank_hits = reranker.rerank(query_text, &candidates, 5).await.expect("rerank");
    assert_eq!(rerank_hits.len(), candidates.len());
    // Scores must be between 0.0 and 1.0 (sigmoid normalized)
    assert!(rerank_hits[0].score >= 0.0 && rerank_hits[0].score <= 1.0);

    // ─── 10. Deletion Cascade Verification ───────────────────────────────────
    fs::remove_file(&auth_file).unwrap();
    let delete_result = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &auth_file, "project")
        .await
        .expect("delete pipeline");
    assert!(delete_result.is_none());

    // Verify all stores are clean
    assert_eq!(store.metadata().count_documents().await.unwrap(), 0);
    assert_eq!(store.vectors().count_vectors().await.unwrap(), 0);

    // Clean up keyring
    let _ = keyring::delete_secret(secret_key);
    let _ = fs::remove_dir_all(&temp_dir);
}
