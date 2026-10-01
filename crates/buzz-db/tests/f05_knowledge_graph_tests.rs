#![deny(unsafe_code)]
//! Feature 05: Knowledge Graph Integration Tests
//!
//! Validates:
//! - TC-F05-001: Entity & relation creation with deterministic IDs and chunk provenance
//! - TC-F05-002: Bi-temporal invalidation and contradiction resolution (preserving history)
//! - TC-F05-003: Multi-hop graph-assisted recall and bounded neighborhood traversal

use buzz_core::memory::{
    Chunk, Document, Entity, EntityType, Relation, RelationType,
};
use buzz_db::memory::store::EmbeddedMemoryStore;
use chrono::{Duration, Utc};
use uuid::Uuid;

/// TC-F05-001: Entity and relation creation with deterministic IDs and provenance.
#[tokio::test]
async fn tc_f05_001_entity_relation_creation() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_tc_f05_001_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open embedded memory store");
    let ws = "/workspace/orbit";

    // 1. Ingest document and chunk to establish provenance ID
    let doc = Document::new(
        "file",
        "/workspace/orbit/src/database.rs",
        ws,
        b"pub struct DatabasePool;\npub fn connect() -> bool { true }",
        1000,
        serde_json::json!({}),
    );
    let doc = store.remember_document(&doc).await.expect("remember doc");

    let chunk = Chunk::new(
        doc.id,
        ws,
        0,
        "pub struct DatabasePool;\npub fn connect() -> bool { true }",
        "project",
    );
    let chunk = store.remember_chunk(&chunk, None).await.expect("remember chunk");

    // 2. Create deterministic entities
    let file_node = Entity::new_deterministic(
        ws,
        "/workspace/orbit/src/database.rs",
        EntityType::FILE,
        "Database abstraction module",
    );
    let pool_node = Entity::new_deterministic(
        ws,
        "DatabasePool",
        EntityType::SYMBOL,
        "Connection pool manager",
    );
    let connect_node = Entity::new_deterministic(
        ws,
        "connect",
        EntityType::SYMBOL,
        "Pool connection initializer",
    );

    // Verify deterministic ID generation
    let expected_file_id = Entity::deterministic_id(ws, EntityType::FILE, "/workspace/orbit/src/database.rs");
    assert_eq!(file_node.id, expected_file_id);

    // 3. Create relations with provenance chunk ID
    let mut rel_defines_pool = Relation::new(
        ws,
        file_node.id,
        pool_node.id,
        RelationType::DEFINES,
        1.0,
    );
    rel_defines_pool.provenance_chunk_id = Some(chunk.id);

    let mut rel_defines_connect = Relation::new(
        ws,
        file_node.id,
        connect_node.id,
        RelationType::DEFINES,
        1.0,
    );
    rel_defines_connect.provenance_chunk_id = Some(chunk.id);

    let mut rel_depends = Relation::new(
        ws,
        connect_node.id,
        pool_node.id,
        RelationType::DEPENDS_ON,
        0.95,
    );
    rel_depends.provenance_chunk_id = Some(chunk.id);

    store
        .remember_graph(
            &[file_node.clone(), pool_node.clone(), connect_node.clone()],
            &[rel_defines_pool.clone(), rel_defines_connect.clone(), rel_depends.clone()],
        )
        .await
        .expect("remember graph");

    // 4. Assert nodes and edges exist with exact provenance
    let stats = store.stats().await.expect("fetch stats");
    assert_eq!(stats.total_entities, 3);
    assert_eq!(stats.total_relations, 3);

    let fetched_pool = store.get_entity(&pool_node.id).await.unwrap().expect("pool exists");
    assert_eq!(fetched_pool.name, "DatabasePool");
    assert_eq!(fetched_pool.entity_type, EntityType::SYMBOL);

    let active_rels = store.list_relations(ws, true).await.unwrap();
    assert_eq!(active_rels.len(), 3);
    for r in &active_rels {
        assert_eq!(r.provenance_chunk_id, Some(chunk.id));
    }

    // 5. Test duplicate entity upsert with metadata/description merging
    let mut duplicate_pool = Entity::new_deterministic(
        ws,
        "DatabasePool",
        EntityType::SYMBOL,
        "Supports connection timeouts and retry backoff",
    );
    duplicate_pool.metadata = serde_json::json!({"max_conns": 20});

    store.remember_graph(&[duplicate_pool], &[]).await.unwrap();

    let merged_pool = store.get_entity(&pool_node.id).await.unwrap().expect("pool exists");
    // Entity count unchanged (merged)
    assert_eq!(store.stats().await.unwrap().total_entities, 3);
    assert!(merged_pool.description.contains("Connection pool manager"));
    assert!(merged_pool.description.contains("Supports connection timeouts"));
    assert_eq!(merged_pool.metadata["max_conns"], 20);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// TC-F05-002: Temporal invalidation & contradiction resolution without losing history.
#[tokio::test]
async fn tc_f05_002_temporal_invalidation() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_tc_f05_002_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open embedded memory store");
    let ws = "/workspace/orbit";

    // Scenario: System initially chose PostgreSQL as primary store (ADR-001)
    let adr1_node = Entity::new_deterministic(
        ws,
        "ADR-001: Centralized Postgres",
        EntityType::DECISION,
        "Use Postgres database server for all data",
    );
    let app_node = Entity::new_deterministic(
        ws,
        "OrbitBackend",
        EntityType::PROJECT,
        "Core application daemon",
    );
    let postgres_node = Entity::new_deterministic(
        ws,
        "PostgreSQL",
        EntityType::TECHNOLOGY,
        "External relational database",
    );

    let t0 = Utc::now() - Duration::days(30);
    let mut rel_initial = Relation::new(
        ws,
        app_node.id,
        postgres_node.id,
        "USES_DATABASE",
        1.0,
    );
    rel_initial.valid_at = t0;

    store
        .remember_graph(
            &[adr1_node.clone(), app_node.clone(), postgres_node.clone()],
            &[rel_initial.clone()],
        )
        .await
        .expect("remember initial graph");

    // Verify initial active state
    let active_at_t0 = store
        .graph_neighborhood_as_of(&[app_node.id], 1, Some(t0 + Duration::days(5)), None)
        .await
        .expect("query at t0");
    assert_eq!(active_at_t0.len(), 2);
    assert!(active_at_t0.iter().any(|h| h.entity.id == postgres_node.id));

    // Later, ADR-002 supersedes ADR-001 and replaces Postgres with embedded SQLite + LanceDB
    let adr2_node = Entity::new_deterministic(
        ws,
        "ADR-002: Embedded SQLite & LanceDB",
        EntityType::DECISION,
        "Migrate to embedded local database engine",
    );
    let sqlite_node = Entity::new_deterministic(
        ws,
        "EmbeddedSQLite",
        EntityType::TECHNOLOGY,
        "In-process zero-daemon database",
    );

    let t1 = Utc::now() - Duration::days(1);
    let mut rel_replacement = Relation::new(
        ws,
        app_node.id,
        sqlite_node.id,
        "USES_DATABASE",
        1.0,
    );
    rel_replacement.valid_at = t1;

    let mut rel_supersedes = Relation::new(
        ws,
        adr2_node.id,
        adr1_node.id,
        RelationType::SUPERSEDES,
        1.0,
    );
    rel_supersedes.valid_at = t1;

    store
        .remember_graph(
            &[adr2_node.clone(), sqlite_node.clone()],
            &[rel_supersedes.clone()],
        )
        .await
        .expect("remember adr2");

    // Execute contradiction resolution: USES_DATABASE between app_node and postgres_node is superseded
    let invalidated = store
        .resolve_graph_contradiction(
            ws,
            &app_node.id,
            &postgres_node.id,
            "USES_DATABASE",
            &rel_replacement,
        )
        .await
        .expect("resolve contradiction");

    assert_eq!(invalidated, 1);

    // 1. Current state: only SQLite is active for USES_DATABASE
    let current_active = store.list_relations(ws, true).await.unwrap();
    let current_db_rel = current_active.iter().find(|r| r.relation_type == "USES_DATABASE").unwrap();
    assert_eq!(current_db_rel.target_entity_id, sqlite_node.id);
    assert!(current_db_rel.is_active());

    // 2. Full audit history: the old Postgres relation is NOT deleted, but carries invalid_at timestamp
    let all_rels = store.list_relations(ws, false).await.unwrap();
    let old_rel = all_rels.iter().find(|r| r.target_entity_id == postgres_node.id).unwrap();
    assert!(!old_rel.is_active());
    assert!(old_rel.invalid_at.is_some());
    assert_eq!(old_rel.invalid_at.unwrap(), t1);

    // 3. Time-travel query at (t0 + 5 days) still historically recalls PostgreSQL!
    let historical_hits = store
        .graph_neighborhood_as_of(&[app_node.id], 1, Some(t0 + Duration::days(5)), None)
        .await
        .expect("historical time travel");
    assert!(historical_hits.iter().any(|h| h.entity.id == postgres_node.id));
    assert!(!historical_hits.iter().any(|h| h.entity.id == sqlite_node.id));

    // 4. Present query (as_of = None) sees EmbeddedSQLite and SUPERSEDES edge!
    let present_hits = store
        .graph_neighborhood_as_of(&[app_node.id], 1, None, None)
        .await
        .expect("present query");
    assert!(present_hits.iter().any(|h| h.entity.id == sqlite_node.id));
    assert!(!present_hits.iter().any(|h| h.entity.id == postgres_node.id));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// TC-F05-003: Graph-assisted recall over multi-hop dependency chains.
#[tokio::test]
async fn tc_f05_003_graph_assisted_recall() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_tc_f05_003_{}", Uuid::new_v4()));
    let store = EmbeddedMemoryStore::open(&temp_dir).expect("open embedded memory store");
    let ws = "/workspace/orbit";

    // Setup 4-tier chain:
    // UserInterface -> (DEPENDS_ON) -> SearchEngine -> (DEPENDS_ON) -> Indexer -> (DEPENDS_ON) -> StorageBackend
    let ui_node = Entity::new_deterministic(ws, "UserInterface", EntityType::SYMBOL, "UI Layer");
    let engine_node = Entity::new_deterministic(ws, "SearchEngine", EntityType::SYMBOL, "SuperRAG Engine");
    let indexer_node = Entity::new_deterministic(ws, "Indexer", EntityType::SYMBOL, "Token Indexer");
    let storage_node = Entity::new_deterministic(ws, "StorageBackend", EntityType::TECHNOLOGY, "Sqlite & LanceDB");

    let rel1 = Relation::new(ws, ui_node.id, engine_node.id, RelationType::DEPENDS_ON, 1.0);
    let rel2 = Relation::new(ws, engine_node.id, indexer_node.id, RelationType::DEPENDS_ON, 0.9);
    let rel3 = Relation::new(ws, indexer_node.id, storage_node.id, RelationType::DEPENDS_ON, 0.85);

    store
        .remember_graph(
            &[ui_node.clone(), engine_node.clone(), indexer_node.clone(), storage_node.clone()],
            &[rel1, rel2, rel3],
        )
        .await
        .expect("remember chain");

    // 1-Hop Traversal from UserInterface: finds SearchEngine
    let hits_1hop = store.graph_neighborhood(&[ui_node.id], 1).await.expect("1 hop");
    assert_eq!(hits_1hop.len(), 2); // UI + SearchEngine
    assert_eq!(hits_1hop[0].entity.id, ui_node.id);
    assert_eq!(hits_1hop[1].entity.id, engine_node.id);

    // 2-Hop Traversal (Reasoning Expansion): reaches Indexer
    let hits_2hop = store.graph_neighborhood(&[ui_node.id], 2).await.expect("2 hops");
    assert_eq!(hits_2hop.len(), 3); // UI + SearchEngine + Indexer
    assert_eq!(hits_2hop[2].entity.id, indexer_node.id);
    assert_eq!(hits_2hop[2].depth, 2);

    // 3-Hop Traversal (SuperRAG Re-Trial Expansion): reaches StorageBackend
    let hits_3hop = store.graph_neighborhood(&[ui_node.id], 3).await.expect("3 hops");
    assert_eq!(hits_3hop.len(), 4); // All 4 nodes
    assert_eq!(hits_3hop[3].entity.id, storage_node.id);
    assert_eq!(hits_3hop[3].depth, 3);

    // Hard Node Cap test: max_nodes = 2 should return at most 2 nodes
    let capped_hits = store
        .graph_neighborhood_as_of(&[ui_node.id], 3, None, Some(2))
        .await
        .expect("capped traversal");
    assert_eq!(capped_hits.len(), 2);

    // Verify Crash-Safe Persistence & Re-open
    drop(store);

    let reopened = EmbeddedMemoryStore::open(&temp_dir).expect("reopen store");
    assert_eq!(reopened.stats().await.unwrap().total_entities, 4);
    assert_eq!(reopened.stats().await.unwrap().total_relations, 3);

    let hits_after_reopen = reopened.graph_neighborhood(&[ui_node.id], 2).await.unwrap();
    assert_eq!(hits_after_reopen.len(), 3);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
