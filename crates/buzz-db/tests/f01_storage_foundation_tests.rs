#![deny(unsafe_code)]
//! Integration tests for Feature 01: Storage Foundation.
//!
//! Validates:
//! - TC-F01-001: Fresh local store creation (zero DB server)
//! - TC-F01-002: Cross-store identity and provenance mapping
//! - TC-F01-003: Cascading delete propagation across SQLite, vectors, and graph
//! - TC-F01-004: Crash/restart recovery without external services
//! - Secret redaction prior to persistent storage
//! - Workspace isolation at retrieval time
//! - 10k-chunk smoke benchmark

use buzz_core::memory::{Chunk, Document, Entity, Relation, VectorFilter};
use buzz_db::memory::{EmbeddedMemoryStore, GraphStore, MetadataStore, VectorStore};
use std::time::Instant;
use uuid::Uuid;

#[tokio::test]
async fn test_tc_f01_001_fresh_local_store_creation() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_fresh_{}", Uuid::new_v4()));

    // Launch on clean directory without any external DB process
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("Store initialization must succeed offline");

    assert!(temp_dir.join("db").join("orbit.db").exists());
    assert!(temp_dir.join("vectors").exists());
    assert!(temp_dir.join("graph").exists());

    let stats = store.stats().await.expect("stats");
    assert_eq!(stats.total_documents, 0);
    assert_eq!(stats.total_chunks, 0);
    assert_eq!(stats.total_vectors, 0);
    assert_eq!(stats.total_entities, 0);
    assert_eq!(stats.total_relations, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_tc_f01_002_cross_store_identity_and_secret_redaction() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_identity_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open store");

    // 1. Create document
    let doc = Document::new(
        "file",
        "/src/api/auth.rs",
        "/workspace/primary",
        b"pub fn login() { let secret = \"sk-proj1234567890abcdef1234567890\"; }",
        1000,
        serde_json::json!({"language": "rust", "acl": {"read": ["dev"]}}),
    );
    let saved_doc = store.remember_document(&doc).await.expect("remember doc");
    assert_eq!(saved_doc.id, doc.id);

    // 2. Create chunk with secret to verify redaction before persistence
    let raw_chunk = Chunk::new(
        saved_doc.id,
        &saved_doc.workspace_path,
        0,
        "let secret = \"sk-proj1234567890abcdef1234567890\"; let pat = \"ghp_abcdefghijklmnopqrstuvwxyz123456\";",
        "project",
    );
    let saved_chunk = store
        .remember_chunk(&raw_chunk, Some(vec![0.5, 0.5, 0.0]))
        .await
        .expect("remember chunk");

    // Verify secret was sanitized
    assert!(!saved_chunk.content.contains("sk-proj1234567890"));
    assert!(!saved_chunk.content.contains("ghp_abcdefghijkl"));
    assert!(saved_chunk.content.contains("[REDACTED:OPENAI_KEY]"));
    assert!(saved_chunk.content.contains("[REDACTED:GITHUB_PAT]"));

    // Verify vector row maps directly to authoritative chunk_id
    let vector_hits = store
        .search_vectors(&[0.5, 0.5, 0.0], &VectorFilter::default(), 5)
        .await
        .expect("search vectors");
    assert_eq!(vector_hits.len(), 1);
    assert_eq!(vector_hits[0].chunk_id, saved_chunk.id);
    assert_eq!(vector_hits[0].document_id, saved_doc.id);

    // 3. Create graph node and edge referencing provenance chunk_id
    let entity = Entity::new(&saved_doc.workspace_path, "AuthModule", "Architecture", "Handles user auth");
    let mut relation = Relation::new(
        &saved_doc.workspace_path,
        entity.id,
        entity.id,
        "implements",
        0.99,
    );
    relation.provenance_chunk_id = Some(saved_chunk.id);

    store.remember_graph(&[entity.clone()], &[relation.clone()]).await.expect("remember graph");

    // Verify graph hits map to provenance
    let neighborhood = store.graph_neighborhood(&[entity.id], 1).await.expect("neighborhood");
    assert_eq!(neighborhood.len(), 1);
    assert_eq!(neighborhood[0].entity.id, entity.id);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_tc_f01_003_delete_propagation() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_delete_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open store");

    let doc = Document::new(
        "file",
        "/src/temp.rs",
        "/workspace/delete_test",
        b"temporary code",
        200,
        serde_json::json!({}),
    );
    let saved_doc = store.remember_document(&doc).await.expect("remember doc");

    let chunk = Chunk::new(saved_doc.id, &saved_doc.workspace_path, 0, "temporary code", "project");
    let saved_chunk = store
        .remember_chunk(&chunk, Some(vec![1.0, 0.0]))
        .await
        .expect("remember chunk");

    let ent = Entity::new(&saved_doc.workspace_path, "TempEnt", "Concept", "Temp");
    let mut rel = Relation::new(&saved_doc.workspace_path, ent.id, ent.id, "temp_rel", 1.0);
    rel.provenance_chunk_id = Some(saved_chunk.id);
    store.remember_graph(&[ent.clone()], &[rel.clone()]).await.expect("remember graph");

    assert_eq!(store.metadata().count_documents().await.unwrap(), 1);
    assert_eq!(store.metadata().count_chunks().await.unwrap(), 1);
    assert_eq!(store.vectors().count_vectors().await.unwrap(), 1);
    assert_eq!(store.graph().count_relations().await.unwrap(), 1);

    // Delete document -> verify propagation to chunks, vectors, and relations
    let deleted = store.forget_document(&saved_doc.id).await.expect("forget doc");
    assert!(deleted);

    assert_eq!(store.metadata().count_documents().await.unwrap(), 0);
    assert_eq!(store.metadata().count_chunks().await.unwrap(), 0);
    assert_eq!(store.vectors().count_vectors().await.unwrap(), 0);
    assert_eq!(store.graph().count_relations().await.unwrap(), 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_tc_f01_004_restart_recovery() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_recovery_{}", Uuid::new_v4()));
    let doc_id;
    let chunk_id;
    let ent_id;

    // 1. Initial write
    {
        let store = EmbeddedMemoryStore::open(&temp_dir).expect("open initial store");
        let doc = Document::new(
            "session",
            "session://123",
            "/workspace/recovery",
            b"User: what is orbit? Assistant: Orbit is memory.",
            400,
            serde_json::json!({}),
        );
        let saved_doc = store.remember_document(&doc).await.expect("remember doc");
        doc_id = saved_doc.id;

        let chunk = Chunk::new(doc_id, &saved_doc.workspace_path, 0, "Orbit is memory.", "project");
        let saved_chunk = store
            .remember_chunk(&chunk, Some(vec![0.1, 0.9]))
            .await
            .expect("remember chunk");
        chunk_id = saved_chunk.id;

        let ent = Entity::new(&saved_doc.workspace_path, "OrbitSystem", "Architecture", "Unified memory");
        ent_id = ent.id;
        store.remember_graph(&[ent], &[]).await.expect("remember graph");
    }

    // 2. Simulate restart / reopen store from same directory
    {
        let reopened_store = EmbeddedMemoryStore::open(&temp_dir).expect("reopen store");

        // Verify document recovered
        let fetched_doc = reopened_store.metadata().get_document(&doc_id).await.expect("get doc");
        assert!(fetched_doc.is_some());
        assert_eq!(fetched_doc.unwrap().source_type, "session");

        // Verify chunk recovered
        let fetched_chunk = reopened_store.metadata().get_chunk(&chunk_id).await.expect("get chunk");
        assert!(fetched_chunk.is_some());
        assert_eq!(fetched_chunk.unwrap().content, "Orbit is memory.");

        // Verify vectors recovered and searchable
        let vector_hits = reopened_store
            .search_vectors(&[0.1, 0.9], &VectorFilter::default(), 5)
            .await
            .expect("search vectors");
        assert_eq!(vector_hits.len(), 1);
        assert_eq!(vector_hits[0].chunk_id, chunk_id);

        // Verify graph recovered
        let nh = reopened_store.graph_neighborhood(&[ent_id], 1).await.expect("graph nh");
        assert_eq!(nh.len(), 1);
        assert_eq!(nh[0].entity.name, "OrbitSystem");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_workspace_scoping_isolation() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_scoping_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open store");

    let doc_a = Document::new("file", "/a.rs", "/workspace/A", b"code A", 10, serde_json::json!({}));
    let doc_b = Document::new("file", "/b.rs", "/workspace/B", b"code B", 10, serde_json::json!({}));

    store.remember_document(&doc_a).await.expect("doc a");
    store.remember_document(&doc_b).await.expect("doc b");

    let chunk_a = Chunk::new(doc_a.id, "/workspace/A", 0, "chunk A", "project");
    let chunk_b = Chunk::new(doc_b.id, "/workspace/B", 0, "chunk B", "project");

    store.remember_chunk(&chunk_a, Some(vec![1.0, 0.0])).await.expect("chunk a");
    store.remember_chunk(&chunk_b, Some(vec![1.0, 0.0])).await.expect("chunk b");

    // Search scoped to workspace A only
    let filter_a = VectorFilter {
        workspace_path: Some("/workspace/A".to_string()),
        document_id: None,
        scope: None,
    };
    let hits_a = store.search_vectors(&[1.0, 0.0], &filter_a, 10).await.expect("search a");
    assert_eq!(hits_a.len(), 1);
    assert_eq!(hits_a[0].workspace_path, "/workspace/A");

    // Search scoped to workspace B only
    let filter_b = VectorFilter {
        workspace_path: Some("/workspace/B".to_string()),
        document_id: None,
        scope: None,
    };
    let hits_b = store.search_vectors(&[1.0, 0.0], &filter_b, 10).await.expect("search b");
    assert_eq!(hits_b.len(), 1);
    assert_eq!(hits_b[0].workspace_path, "/workspace/B");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_10k_chunk_smoke_benchmark() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_10k_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open store");

    let doc = Document::new(
        "file",
        "/src/large_repo.rs",
        "/workspace/bench",
        b"benchmark file content",
        1000,
        serde_json::json!({}),
    );
    let saved_doc = store.remember_document(&doc).await.expect("remember doc");

    let count = 10_000;
    let mut chunks = Vec::with_capacity(count);
    let mut embeddings = Vec::with_capacity(count);

    for i in 0..count {
        let chunk_id = Uuid::new_v4();
        chunks.push(Chunk {
            id: chunk_id,
            document_id: saved_doc.id,
            workspace_path: saved_doc.workspace_path.clone(),
            chunk_index: i as i32,
            content: format!("Benchmark chunk line content index {i}"),
            token_count: 8,
            scope: "project".to_string(),
            agent_name: None,
            line_start: Some(i as i32),
            line_end: Some(i as i32 + 1),
            created_at: chrono::Utc::now(),
        });

        embeddings.push(buzz_core::memory::EmbeddingRecord {
            chunk_id,
            document_id: saved_doc.id,
            workspace_path: saved_doc.workspace_path.clone(),
            embedding: vec![(i % 100) as f32 / 100.0, ((i + 50) % 100) as f32 / 100.0, 0.5],
            content: format!("Benchmark chunk line content index {i}"),
            created_at: chrono::Utc::now(),
        });
    }

    let start_ingest = Instant::now();
    for chunk in chunks.chunks(1000) {
        for c in chunk {
            store.metadata().upsert_chunk(c).await.expect("upsert chunk");
        }
    }
    store.vectors().upsert_embeddings(&embeddings).await.expect("upsert embeddings");
    let ingest_duration = start_ingest.elapsed();

    // Verify 10k items present
    assert_eq!(store.metadata().count_chunks().await.unwrap(), count);
    assert_eq!(store.vectors().count_vectors().await.unwrap(), count);

    // Search latency benchmark
    let start_search = Instant::now();
    let hits = store
        .search_vectors(&[0.5, 0.5, 0.5], &VectorFilter::default(), 10)
        .await
        .expect("search benchmark");
    let search_duration = start_search.elapsed();

    assert_eq!(hits.len(), 10);
    println!("10k Ingest duration: {ingest_duration:?}, 10k Search latency: {search_duration:?}");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
