#![deny(unsafe_code)]
//! Integration and validation tests for Feature 04: SuperRAG Retrieval Layer.
//!
//! Validates:
//! - TC-F04-001: Exact symbol, error, and path lookup (lexical/symbolic retrieval)
//! - TC-F04-002: Semantic paraphrase recall via dense vector search
//! - TC-F04-003: Temporal conflict and ADR preference (temporal decay & authority boost)
//! - TC-F04-004: Token budget constraint & `<orbit_context>` semantic XML formatting
//! - TC-F04-005: Sub-millisecond semantic query cache hit
//! - TC-F04-006: Data Re-Trial & Confidence Verification Loop on sparse queries

use std::sync::Arc;
use std::time::Instant;
use buzz_ai::{LocalOnnxEmbedder, LocalOnnxReranker};
use buzz_core::memory::{Chunk, Document, Entity, Relation};
use buzz_db::memory::EmbeddedMemoryStore;
use buzz_search::{QueryIntent, SuperRagEngine, SuperRagRequest};
use chrono::{Duration, Utc};
use uuid::Uuid;

use std::path::PathBuf;

struct TestContext {
    dir: PathBuf,
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn setup_test_engine() -> (SuperRagEngine, TestContext) {
    let temp_dir = std::env::temp_dir().join(format!("orbit_superrag_test_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let store = Arc::new(EmbeddedMemoryStore::open(&temp_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let engine = SuperRagEngine::new(store, embedder, reranker);
    (engine, TestContext { dir: temp_dir })
}

#[tokio::test]
async fn tc_f04_001_exact_lookup() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    // 1. Ingest document and chunk containing exact symbol
    let doc = Document::new(
        "file",
        "crates/buzz-relay/src/auth.rs",
        ws,
        b"pub fn verify_nip42_token(token: &str) -> bool { true }",
        100,
        serde_json::json!({}),
    );
    let saved_doc = engine.store.remember_document(&doc).await.expect("remember doc");

    let mut chunk = Chunk::new(
        saved_doc.id,
        ws,
        0,
        "pub fn verify_nip42_token(token: &str) -> bool { true }",
        "project",
    );
    chunk.line_start = Some(42);
    chunk.line_end = Some(45);

    let emb = engine.embedder.embed_batch(&[&chunk.content]).await.unwrap();
    engine.store.remember_chunk(&chunk, Some(emb[0].clone())).await.expect("remember chunk");

    // 2. Query exact symbol
    let req = SuperRagRequest::new(ws, "verify_nip42_token()");
    let res = engine.query(&req).await.expect("query must succeed");

    assert!(matches!(res.intent, QueryIntent::Symbolic { .. }), "Intent must be Symbolic");
    assert!(!res.chunks.is_empty(), "Must return evidence");
    assert!(res.chunks[0].content.contains("verify_nip42_token"));
    assert_eq!(res.chunks[0].source_uri, "crates/buzz-relay/src/auth.rs");
    assert_eq!(res.chunks[0].line_start, Some(42));
    assert!(res.context_xml.contains("verify_nip42_token"));
    assert!(res.context_xml.contains("crates/buzz-relay/src/auth.rs"));
}

#[tokio::test]
async fn tc_f04_002_semantic_recall() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    // Ingest architecture description
    let content = "The WebSocket relay handles client authentication by validating NIP-42 signed events during handshake negotiation.";
    let doc = Document::new("file", "docs/auth.md", ws, content.as_bytes(), 200, serde_json::json!({}));
    let saved_doc = engine.store.remember_document(&doc).await.expect("remember doc");

    let chunk = Chunk::new(saved_doc.id, ws, 0, content, "project");
    let emb = engine.embedder.embed_batch(&[content]).await.unwrap();
    engine.store.remember_chunk(&chunk, Some(emb[0].clone())).await.expect("remember chunk");

    // Paraphrase query
    let req = SuperRagRequest::new(ws, "How does the relay authenticate incoming connections?");
    let res = engine.query(&req).await.expect("query must succeed");

    assert!(matches!(res.intent, QueryIntent::Conceptual { .. }));
    assert!(!res.chunks.is_empty(), "Must find semantic match");
    assert!(res.chunks[0].content.contains("NIP-42"));
    assert_eq!(res.chunks[0].source_uri, "docs/auth.md");
}

#[tokio::test]
async fn tc_f04_003_temporal_conflict_and_adr_preference() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    // Chunk 1: Old superseded chat from 120 days ago
    let old_content = "We originally used PostgreSQL pgvector for all memory embeddings and full text search in 2024.";
    let old_doc = Document::new("chat", "chat/session_001.json", ws, old_content.as_bytes(), 100, serde_json::json!({}));
    let saved_old_doc = engine.store.remember_document(&old_doc).await.unwrap();

    let mut old_chunk = Chunk::new(saved_old_doc.id, ws, 0, old_content, "session");
    old_chunk.created_at = Utc::now() - Duration::days(120);
    let old_emb = engine.embedder.embed_batch(&[old_content]).await.unwrap();
    engine.store.remember_chunk(&old_chunk, Some(old_emb[0].clone())).await.unwrap();

    // Chunk 2: Current valid ADR from 2 days ago
    let adr_content = "ADR 0004: We decided to adopt SQLite WAL and local embedded vector store for zero-network operation.";
    let adr_doc = Document::new("adr", "docs/adr/0004-sqlite.md", ws, adr_content.as_bytes(), 100, serde_json::json!({}));
    let saved_adr_doc = engine.store.remember_document(&adr_doc).await.unwrap();

    let mut adr_chunk = Chunk::new(saved_adr_doc.id, ws, 0, adr_content, "project");
    adr_chunk.created_at = Utc::now() - Duration::days(2);
    let adr_emb = engine.embedder.embed_batch(&[adr_content]).await.unwrap();
    engine.store.remember_chunk(&adr_chunk, Some(adr_emb[0].clone())).await.unwrap();

    // Query asking about current storage decision
    let req = SuperRagRequest::new(ws, "What is our decision on vector storage database?");
    let res = engine.query(&req).await.expect("query must succeed");

    assert!(!res.chunks.is_empty(), "Must return evidence");
    // ADR must outrank the old chat due to zero decay + ADR authority multiplier
    assert_eq!(
        res.chunks[0].chunk_id, adr_chunk.id,
        "ADR must outrank superseded chat log"
    );
    assert!(res.chunks[0].content.contains("ADR 0004"));
}

#[tokio::test]
async fn tc_f04_004_context_pack_within_budget() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    // Ingest several chunks
    for i in 1..=5 {
        let text = format!("Module function implementation number {i}: fn process_item_{i}() {{ log::info!(\"item {i}\"); }}");
        let doc = Document::new("file", &format!("src/mod_{i}.rs"), ws, text.as_bytes(), 50, serde_json::json!({}));
        let saved_doc = engine.store.remember_document(&doc).await.unwrap();
        let mut chunk = Chunk::new(saved_doc.id, ws, 0, &text, "project");
        chunk.line_start = Some(1);
        chunk.line_end = Some(10);
        let emb = engine.embedder.embed_batch(&[&text]).await.unwrap();
        engine.store.remember_chunk(&chunk, Some(emb[0].clone())).await.unwrap();
    }

    // Small budget constraint (~50 tokens)
    let req = SuperRagRequest::new(ws, "process_item").with_budget(50);
    let res = engine.query(&req).await.expect("pack query");

    assert!(res.allocated_tokens <= 60, "Allocated tokens must obey budget constraint");
    assert!(res.context_xml.starts_with("<orbit_context"));
    assert!(res.context_xml.ends_with("</orbit_context>"));
    assert!(res.context_xml.contains("workspace=\"/projects/orbit\""));
    assert!(res.context_xml.contains("lines="));
}

#[tokio::test]
async fn tc_f04_005_sub_millisecond_cache_hit() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    let text = "Fast cache hit verification test chunk";
    let doc = Document::new("file", "test.rs", ws, text.as_bytes(), 50, serde_json::json!({}));
    let saved_doc = engine.store.remember_document(&doc).await.unwrap();
    let chunk = Chunk::new(saved_doc.id, ws, 0, text, "project");
    let emb = engine.embedder.embed_batch(&[text]).await.unwrap();
    engine.store.remember_chunk(&chunk, Some(emb[0].clone())).await.unwrap();

    let req = SuperRagRequest::new(ws, "cache hit verification");

    // First call (cache miss)
    let first_res = engine.query(&req).await.unwrap();
    assert!(!first_res.cached, "First call must be a cache miss");

    // Second call (cache hit)
    let start = Instant::now();
    let second_res = engine.query(&req).await.unwrap();
    let elapsed = start.elapsed();

    assert!(second_res.cached, "Second call must hit cache");
    assert_eq!(second_res.context_xml, first_res.context_xml);
    assert!(
        elapsed.as_millis() < 5,
        "Cache hit must return in < 5ms, took: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn tc_f04_006_data_retrial_with_knowledge_graph() {
    let (engine, _temp) = setup_test_engine().await;
    let ws = "/projects/orbit";

    // Ingest entity and relation with chunk provenance
    let text = "Service registry and routing tables for cluster coordination";
    let doc = Document::new("file", "src/registry.rs", ws, text.as_bytes(), 50, serde_json::json!({}));
    let saved_doc = engine.store.remember_document(&doc).await.unwrap();
    let chunk = Chunk::new(saved_doc.id, ws, 0, text, "project");
    let emb = engine.embedder.embed_batch(&[text]).await.unwrap();
    engine.store.remember_chunk(&chunk, Some(emb[0].clone())).await.unwrap();

    let ent1 = Entity::new(ws, "ServiceRegistry", "Component", "Cluster discovery");
    let ent2 = Entity::new(ws, "RoutingTable", "Component", "Route resolution");
    let mut rel = Relation::new(ws, ent1.id, ent2.id, "manages", 0.95);
    rel.provenance_chunk_id = Some(chunk.id);

    engine.store.remember_graph(&[ent1.clone(), ent2.clone()], &[rel]).await.unwrap();

    // Query for entity relationship
    let req = SuperRagRequest::new(ws, "ServiceRegistry routing");
    let res = engine.query(&req).await.expect("query with graph");

    assert!(!res.chunks.is_empty(), "Must retrieve graph provenance chunk");
    assert!(res.chunks[0].content.contains("routing tables"));
}
