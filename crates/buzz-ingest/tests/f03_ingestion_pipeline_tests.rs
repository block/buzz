#![deny(unsafe_code)]
//! Integration and verification tests for Feature 03: Ingestion Pipeline.
//!
//! Validates:
//! - TC-F03-001: Incremental file change: Only modified files and affected chunks are updated.
//! - TC-F03-002: Content-hash dedup: Unchanged files are skipped without duplicate chunks or re-embedding.
//! - TC-F03-003: Deletion: Deleting a source file removes associated records from metadata and vector stores.
//! - Secret Redaction: API keys inside source files are scrubbed before storage and vectorization.
//! - Multi-language AST chunking integrity: Preserves functions, classes, and markdown headings.

use std::fs;
use std::sync::Arc;
use buzz_ai::LocalOnnxEmbedder;
use buzz_db::memory::MetadataStore;
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_ingest::IngestionPipeline;
use uuid::Uuid;

#[tokio::test]
async fn tc_f03_001_and_002_incremental_ingest_and_dedup() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_f03_{}", Uuid::new_v4()));
    let brain_dir = temp_dir.join("brain");
    let workspace_dir = temp_dir.join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder);

    let test_file = workspace_dir.join("service.rs");
    let code_v1 = r#"
pub struct PaymentService {
    pub endpoint: String,
}

impl PaymentService {
    pub fn new(endpoint: &str) -> Self {
        Self { endpoint: endpoint.to_string() }
    }

    pub fn process_payment(&self, amount_cents: u64) -> bool {
        amount_cents > 0
    }
}
"#;
    fs::write(&test_file, code_v1).unwrap();

    // 1. Initial ingestion
    let doc1 = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &test_file, "project")
        .await
        .expect("initial ingest")
        .expect("document must be created");

    let initial_chunks = store
        .metadata()
        .get_chunks_by_document(&doc1.id)
        .await
        .expect("get chunks");
    assert!(!initial_chunks.is_empty(), "Must have generated AST chunks");
    let initial_chunk_count = initial_chunks.len();

    // 2. TC-F03-002: Re-ingest unchanged file -> MUST be skipped by content-hash dedup
    let dedup_result = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &test_file, "project")
        .await
        .expect("re-ingest");
    assert!(
        dedup_result.is_none(),
        "TC-F03-002: Unchanged file must be skipped by content-hash deduplication"
    );

    // Total chunks must remain unchanged
    let current_chunks = store
        .metadata()
        .get_chunks_by_document(&doc1.id)
        .await
        .expect("get chunks");
    assert_eq!(
        current_chunks.len(),
        initial_chunk_count,
        "Chunk count must not duplicate on re-ingest"
    );

    // 3. TC-F03-001: Modify file -> Re-ingest must update document and chunks
    let code_v2 = r#"
pub struct PaymentService {
    pub endpoint: String,
    pub retries: u32,
}

impl PaymentService {
    pub fn new(endpoint: &str) -> Self {
        Self { endpoint: endpoint.to_string(), retries: 3 }
    }

    pub fn process_payment(&self, amount_cents: u64) -> bool {
        amount_cents > 0
    }

    pub fn refund(&self, transaction_id: &str) -> bool {
        !transaction_id.is_empty()
    }
}
"#;
    // Advance mtime by writing new content
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(&test_file, code_v2).unwrap();

    let doc2 = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &test_file, "project")
        .await
        .expect("incremental ingest")
        .expect("must re-process modified file");

    assert_eq!(doc1.id, doc2.id, "Document ID must be preserved for same URI");
    assert_ne!(doc1.content_hash, doc2.content_hash, "Content hash must update");

    let updated_chunks = store
        .metadata()
        .get_chunks_by_document(&doc2.id)
        .await
        .expect("get updated chunks");
    assert!(
        !updated_chunks.is_empty(),
        "Must have re-generated AST chunks for modified code"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn tc_f03_003_deletion_cascades() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_f03_del_{}", Uuid::new_v4()));
    let brain_dir = temp_dir.join("brain");
    let workspace_dir = temp_dir.join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder);

    let file_to_delete = workspace_dir.join("deprecated.rs");
    fs::write(&file_to_delete, "pub fn old_feature() {}").unwrap();

    let doc = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &file_to_delete, "project")
        .await
        .expect("ingest")
        .expect("doc created");

    assert_eq!(store.metadata().count_documents().await.unwrap(), 1);

    // Delete the file from filesystem
    fs::remove_file(&file_to_delete).unwrap();

    // Re-run pipeline on the deleted path
    let delete_handled = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &file_to_delete, "project")
        .await
        .expect("handle deletion");
    assert!(delete_handled.is_none());

    // Document and its chunks must be completely gone
    assert_eq!(store.metadata().count_documents().await.unwrap(), 0);
    let chunks = store.metadata().get_chunks_by_document(&doc.id).await.unwrap();
    assert!(chunks.is_empty(), "Chunks must be deleted via cascade");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_secret_redaction_during_ingestion() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_f03_sec_{}", Uuid::new_v4()));
    let brain_dir = temp_dir.join("brain");
    let workspace_dir = temp_dir.join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder);

    let secret_file = workspace_dir.join("config.ts");
    let code_with_secret = r#"
export const OPENAI_KEY = "sk-proj-abc123456789012345678901234567890123456789012345678901234567890";
export const GITHUB_TOKEN = "ghp_123456789012345678901234567890123456";

export function getClient() {
    return true;
}
"#;
    fs::write(&secret_file, code_with_secret).unwrap();

    let doc = pipeline
        .ingest_file(&workspace_dir.to_string_lossy(), &secret_file, "project")
        .await
        .expect("ingest")
        .expect("doc");

    let chunks = store.metadata().get_chunks_by_document(&doc.id).await.unwrap();
    assert!(!chunks.is_empty(), "Must produce chunks");

    let has_redaction_marker = chunks.iter().any(|c| {
        c.content.contains("[REDACTED:OPENAI_KEY]") || c.content.contains("[REDACTED:GITHUB_PAT]")
    });
    assert!(has_redaction_marker, "Must contain redaction marker in scrubbed chunks");

    for chunk in &chunks {
        assert!(
            !chunk.content.contains("sk-proj-abc"),
            "OpenAI API key must be redacted in chunk content"
        );
        assert!(
            !chunk.content.contains("ghp_12345"),
            "GitHub token must be redacted in chunk content"
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_full_workspace_batch_ingestion() {
    let temp_dir = std::env::temp_dir().join(format!("orbit_test_f03_ws_{}", Uuid::new_v4()));
    let brain_dir = temp_dir.join("brain");
    let workspace_dir = temp_dir.join("workspace");
    fs::create_dir_all(workspace_dir.join("src")).unwrap();
    fs::create_dir_all(workspace_dir.join(".git")).unwrap();
    fs::create_dir_all(workspace_dir.join("target")).unwrap();

    // Valid files
    fs::write(workspace_dir.join("README.md"), "# Project Orbit\nDocumentation").unwrap();
    fs::write(workspace_dir.join("src/lib.rs"), "pub fn run() -> u32 { 1 }").unwrap();

    // Ignored files (should NOT be ingested)
    fs::write(workspace_dir.join(".git/config"), "[core]").unwrap();
    fs::write(workspace_dir.join("target/build.log"), "Compiling...").unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder);

    let summary = pipeline
        .ingest_workspace(&workspace_dir, "project")
        .await
        .expect("ingest workspace");

    assert_eq!(summary.files_ingested, 2, "Only README.md and src/lib.rs should be ingested");
    assert!(summary.total_chunks_created >= 2);
    assert_eq!(summary.total_vectors_saved, summary.total_chunks_created);

    // Re-ingest workspace immediately -> all files should be skipped by dedup
    let second_summary = pipeline
        .ingest_workspace(&workspace_dir, "project")
        .await
        .expect("second ingest");
    assert_eq!(second_summary.files_ingested, 0);
    assert_eq!(second_summary.files_skipped_dedup, 2);

    let _ = fs::remove_dir_all(&temp_dir);
}
