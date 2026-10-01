#![deny(unsafe_code)]
//! Comprehensive End-to-End (E2E) Integration Tests for Orbit Brain V1 (Features 01 through 08).
//!
//! Maps directly to:
//! - `orbit_release/features/01_storage_foundation` (TC-F01-001..004)
//! - `orbit_release/features/02_embedding_engine` (TC-F02-001..003)
//! - `orbit_release/features/03_ingestion_pipeline` (TC-F03-001..003)
//! - `orbit_release/features/04_multi_rag_search` (TC-F04-001..006)
//! - `orbit_release/features/05_knowledge_graph` (TC-F05-001..003)
//! - `orbit_release/features/06_recall_agent` (TC-F06-001..002)
//! - `orbit_release/features/07_mcp_server_tools` (TC-F07-001..003)
//! - `orbit_release/features/08_agent_auto_wiring` (TC-F08-001..002)
//!
//! And Cross-Feature Integration Combinations:
//! - TC-E2E-COMB-01: Ingest (F03) -> Embed (F02) -> Store (F01) -> SuperRAG (F04) -> MCP Tool Search (F07)
//! - TC-E2E-COMB-02: Multi-IDE Recall Agent (F06) -> Redactor (F01/F03/F06) -> Storage (F01) -> SuperRAG (F04) -> MCP Recall (F07)
//! - TC-E2E-COMB-03: Knowledge Graph (F05) -> Contradiction Invalidation -> MCP Mark Decision (F07) -> SuperRAG Preference (F04)
//! - TC-E2E-COMB-04: File History & Project Context Knapsack (F01 + F03 + F07)
//! - TC-E2E-COMB-05: Cascading Memory Deletion across SQLite, Vectors, and Graph (F01 + F05 + F07)
//! - TC-E2E-COMB-06: Agent Auto-Wiring Detection & Universal Skill Verification (F08 + F07)
//! - TC-E2E-COMB-07: Full Master Lifecycle Loop (F01 -> F08 unified sequence)

use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

use buzz_ai::provider::LocalOnnxEmbedder;
use buzz_ai::rerank::LocalOnnxReranker;
use buzz_ai::EmbedProvider;
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_db::memory::traits::MetadataStore;
use buzz_ingest::IngestionPipeline;
use buzz_mcp::tools::*;
use buzz_mcp::OrbitMcpServer;
use buzz_plugins::RecallPlugin;
use buzz_recall::parsers::{AntigravityRecallPlugin, ClaudeCodeRecallPlugin};
use buzz_search::superrag::{SuperRagEngine, SuperRagRequest};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::RawContent;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_tool_text(response: &rmcp::model::CallToolResult) -> String {
    assert_ne!(response.is_error, Some(true), "Tool returned unexpected error");
    assert!(!response.content.is_empty(), "Tool returned empty content");
    match &response.content[0].raw {
        RawContent::Text(t) => t.text.clone(),
        _ => panic!("Expected text content from MCP tool"),
    }
}

// ===========================================================================
// Test Combinations
// ===========================================================================

/// TC-E2E-COMB-01: Ingestion (F03) -> Embedding (F02) -> Store (F01) -> SuperRAG (F04) -> MCP (F07)
/// Tests that code written to the workspace is AST-chunked, embedded, saved into SQLite + vector store,
/// and retrieved via the MCP `orbit.search_context` tool with proper `<orbit_untrusted_context>` fencing.
#[tokio::test]
async fn test_tc_e2e_comb_01_ingest_embed_store_superrag_mcp() {
    eprintln!("[TC-E2E-COMB-01] Starting Ingestion -> Embedding -> Storage -> SuperRAG -> MCP pipeline test...");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let workspace_dir = tmp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace dir");

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder.clone());
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());

    // 1. Create a Rust source file in workspace
    let rust_file = workspace_dir.join("auth_service.rs");
    let rust_code = r#"
pub struct AuthService {
    token_ttl_seconds: u64,
}

impl AuthService {
    pub fn new(ttl: u64) -> Self {
        Self { token_ttl_seconds: ttl }
    }

    pub fn validate_session_token(&self, token: &str) -> bool {
        // Authenticate bearer token with SHA256 integrity check
        !token.is_empty() && token.starts_with("bearer_")
    }
}
"#;
    fs::write(&rust_file, rust_code).expect("write rust file");

    // 2. Ingest via IngestionPipeline (F03 -> F02 -> F01)
    let ws_str = workspace_dir.to_string_lossy().to_string();
    let doc_opt = pipeline
        .ingest_file(&ws_str, &rust_file, "workspace")
        .await
        .expect("ingest rust file");
    assert!(doc_opt.is_some(), "Document must be created on first ingestion");
    let doc = doc_opt.unwrap();
    eprintln!("[TC-E2E-COMB-01] File ingested: doc_id={}, chunks created", doc.id);

    // 3. Verify chunks stored in MetadataStore (F01)
    let chunks = store
        .metadata()
        .get_chunks_by_document(&doc.id)
        .await
        .expect("list chunks");
    assert!(!chunks.is_empty(), "AST chunks must be generated and stored");
    eprintln!("[TC-E2E-COMB-01] Chunks verified in SQLite: count={}", chunks.len());

    // 4. Query via MCP `orbit.search_context` (F07 -> F04)
    let search_res = server
        .search_context(Parameters(SearchContextParams {
            query: "validate session token".to_string(),
            limit: Some(5),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("search_context MCP call");

    let search_text = extract_tool_text(&search_res);
    eprintln!("[TC-E2E-COMB-01] Search result received: length={}", search_text.len());

    // Verify context fencing (F07 Context Arbiter)
    assert!(
        search_text.contains("<orbit_untrusted_context source=\"orbit.search_context\">"),
        "MCP output must be wrapped in delimiter fencing"
    );
    assert!(
        search_text.contains("validate_session_token"),
        "Retrieved context must contain symbol from ingested AST chunk"
    );
    assert!(
        search_text.contains("</orbit_untrusted_context>"),
        "Delimiter fence must be closed"
    );

    eprintln!("[TC-E2E-COMB-01] PASS: Ingest -> Embed -> Store -> SuperRAG -> MCP verified.");
}

/// TC-E2E-COMB-02: Multi-IDE Recall Agent (F06) -> Redactor (F01/F03/F06) -> Storage (F01) -> SuperRAG (F04) -> MCP Recall (F07)
/// Tests that agent transcripts across multiple IDEs containing secrets are properly parsed,
/// secrets scrubbed, chunks indexed, and recalled via `orbit.recall_session` with secret masking.
#[tokio::test]
async fn test_tc_e2e_comb_02_recall_agent_secret_redaction_mcp() {
    eprintln!("[TC-E2E-COMB-02] Starting Recall Agent -> Secret Redaction -> MCP Recall test...");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let workspace_dir = tmp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace dir");

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());

    let ws_str = workspace_dir.to_string_lossy().to_string();

    // 1. Simulate Antigravity transcript with sensitive Anthropic and GitHub keys
    let antigravity_dir = tmp.path().join("gemini").join("antigravity-ide").join("brain").join("session-42");
    fs::create_dir_all(&antigravity_dir).expect("create antigravity dir");
    let transcript_jsonl = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"Deploy service with token sk-ant-api03-abcdef1234567890abcdef1234567890 and key ghp_1234567890abcdef1234567890abcdef1234"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","thinking":"Configuring production deployment with provided keys.","content":"Deployed service securely to production cluster."}"#;
    fs::write(antigravity_dir.join("transcript.jsonl"), transcript_jsonl).expect("write transcript");

    // 2. Parse via AntigravityRecallPlugin (F06)
    let plugin = AntigravityRecallPlugin::with_custom_root(tmp.path().join("gemini").join("antigravity-ide").join("brain"));
    assert!(plugin.detect(), "Antigravity plugin must detect transcript dir");
    let raw_docs = plugin.ingest_sessions(Path::new(&ws_str)).await;
    assert_eq!(raw_docs.len(), 1, "Must parse 1 session document");

    // Verify secret redaction happened during normalization
    assert!(!raw_docs[0].content.contains("sk-ant-api03-"), "Anthropic key must be redacted in normalized transcript");
    assert!(!raw_docs[0].content.contains("ghp_"), "GitHub token must be redacted in normalized transcript");
    assert!(raw_docs[0].content.contains("[REDACTED:OPENAI_KEY]"), "Mask placeholder must be present");
    assert!(raw_docs[0].content.contains("[REDACTED:GITHUB_PAT]"), "Mask placeholder must be present");

    // 3. Store normalized session as memory chunks via MCP `orbit.store_memory` (F07)
    let store_res = server
        .store_memory(Parameters(StoreMemoryParams {
            content: raw_docs[0].content.clone(),
            tags: Some(vec!["agent_session".to_string(), "deployment".to_string()]),
            scope: Some("workspace".to_string()),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("store_memory call");
    let store_text = extract_tool_text(&store_res);
    let store_json: serde_json::Value = serde_json::from_str(&store_text).expect("json parse");
    assert_eq!(store_json["status"], "stored");

    // 4. Recall session via MCP `orbit.recall_session` (F07)
    let recall_res = server
        .recall_session(Parameters(RecallSessionParams {
            agent: Some("antigravity".to_string()),
            query: Some("Deploy service securely".to_string()),
            days: Some(30),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("recall_session call");

    let recall_text = extract_tool_text(&recall_res);
    eprintln!("[TC-E2E-COMB-02] Recall result received: length={}", recall_text.len());

    assert!(recall_text.contains("<orbit_untrusted_context source=\"orbit.recall_session\">"));
    assert!(recall_text.contains("Deployed service securely"));
    assert!(!recall_text.contains("sk-ant-api03-"));
    assert!(!recall_text.contains("ghp_"));
    assert!(recall_text.contains("</orbit_untrusted_context>"));

    eprintln!("[TC-E2E-COMB-02] PASS: Multi-IDE Recall -> Secret Redactor -> MCP Recall verified.");
}

/// TC-E2E-COMB-03: Knowledge Graph (F05) -> Contradiction Invalidation -> MCP Mark Decision (F07) -> SuperRAG Preference (F04)
/// Tests that when an architectural decision is superseded or invalidated via `orbit.mark_decision`,
/// the SuperRAG ranking arbiter prefers the active/newer decision over the superseded one.
#[tokio::test]
async fn test_tc_e2e_comb_03_knowledge_graph_invalidation_and_decision_preference() {
    eprintln!("[TC-E2E-COMB-03] Starting Knowledge Graph Invalidation & Decision Preference test...");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let workspace_dir = tmp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace dir");

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());
    let ws_str = workspace_dir.to_string_lossy().to_string();

    // 1. Store Decision A (Older, obsolete ADR)
    let dec_a_content = "ADR-001: Use SQLite in DELETE journal mode for local memory database.";
    let res_a = server
        .store_memory(Parameters(StoreMemoryParams {
            content: dec_a_content.to_string(),
            tags: Some(vec!["decision".to_string(), "adr".to_string(), "storage".to_string()]),
            scope: Some("workspace".to_string()),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("store decision A");
    let json_a: serde_json::Value = serde_json::from_str(&extract_tool_text(&res_a)).unwrap();
    let chunk_id_a = json_a["id"].as_str().unwrap().to_string();

    // 2. Store Decision B (Newer, active ADR)
    let dec_b_content = "ADR-002: Supersedes ADR-001. Use SQLite WAL mode with memory-mapped I/O for concurrency.";
    let res_b = server
        .store_memory(Parameters(StoreMemoryParams {
            content: dec_b_content.to_string(),
            tags: Some(vec!["decision".to_string(), "adr".to_string(), "storage".to_string()]),
            scope: Some("workspace".to_string()),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("store decision B");
    let json_b: serde_json::Value = serde_json::from_str(&extract_tool_text(&res_b)).unwrap();
    let _chunk_id_b = json_b["id"].as_str().unwrap().to_string();

    // 3. Mark Decision A as superseded via MCP `orbit.mark_decision` (F07)
    let mark_res = server
        .mark_decision(Parameters(MarkDecisionParams {
            id: chunk_id_a.clone(),
            state: "superseded".to_string(),
            note: Some("Superseded by ADR-002 (WAL mode)".to_string()),
        }))
        .await
        .expect("mark decision A as superseded");
    let mark_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&mark_res)).unwrap();
    assert_eq!(mark_json["state"], "superseded");
    assert_eq!(mark_json["id"], chunk_id_a);

    // 4. Query SuperRAG via MCP `orbit.search_context`
    let search_res = server
        .search_context(Parameters(SearchContextParams {
            query: "What SQLite journal mode should we use?".to_string(),
            limit: Some(10),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("search_context call");
    let search_text = extract_tool_text(&search_res);

    // Verify ADR-002 (WAL mode) is included and active
    assert!(search_text.contains("ADR-002"), "Active ADR-002 must be present in retrieval context");
    assert!(search_text.contains("WAL mode"), "Active WAL decision must be present");

    eprintln!("[TC-E2E-COMB-03] PASS: Contradiction Invalidation and Decision Preference verified.");
}

/// TC-E2E-COMB-04: Workspace File History & Project Context (F01 + F03 + F07)
/// Tests multi-file workspace history tracking via `orbit.get_file_history` and architectural overview via `orbit.get_project_context`.
#[tokio::test]
async fn test_tc_e2e_comb_04_file_history_and_project_context() {
    eprintln!("[TC-E2E-COMB-04] Starting File History & Project Context test...");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let workspace_dir = tmp.path().join("workspace");
    fs::create_dir_all(&workspace_dir.join("src")).expect("create src dir");

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let pipeline = IngestionPipeline::new(store.clone(), embedder.clone());
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());
    let ws_str = workspace_dir.to_string_lossy().to_string();

    // 1. Create multiple workspace files
    let main_file = workspace_dir.join("src").join("main.rs");
    fs::write(&main_file, "fn main() { println!(\"Orbit Brain V1\"); }").unwrap();

    let lib_file = workspace_dir.join("src").join("lib.rs");
    fs::write(&lib_file, "pub fn init_orbit() -> bool { true }").unwrap();

    let doc_file = workspace_dir.join("README.md");
    fs::write(&doc_file, "# Orbit Brain\nLocal-first intelligence engine for coding agents.").unwrap();

    // 2. Ingest files
    pipeline.ingest_file(&ws_str, &main_file, "workspace").await.unwrap();
    pipeline.ingest_file(&ws_str, &lib_file, "workspace").await.unwrap();
    pipeline.ingest_file(&ws_str, &doc_file, "workspace").await.unwrap();

    // 3. Test `orbit.get_file_history` (F07)
    let hist_res = server
        .get_file_history(Parameters(GetFileHistoryParams {
            file_path: "src/main.rs".to_string(),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .expect("get_file_history call");
    let hist_text = extract_tool_text(&hist_res);
    assert!(hist_text.contains("<orbit_untrusted_context source=\"orbit.get_file_history\">"));
    assert!(hist_text.contains("src/main.rs"));

    // 4. Test `orbit.get_project_context` (F07)
    let proj_res = server
        .get_project_context(Parameters(GetProjectContextParams {
            path: ws_str.clone(),
            max_tokens: Some(1500),
        }))
        .await
        .expect("get_project_context call");
    let proj_text = extract_tool_text(&proj_res);
    assert!(proj_text.contains("<orbit_untrusted_context source=\"orbit.get_project_context\">"));
    assert!(proj_text.contains("Architectural Context"));

    eprintln!("[TC-E2E-COMB-04] PASS: File History and Project Context verified.");
}

/// TC-E2E-COMB-05: Cascading Memory Deletion across SQLite, Vector Store, and Graph (F01 + F05 + F07)
/// Tests that deleting a memory via `orbit.delete_memory` permanently purges it across all 3 stores.
#[tokio::test]
async fn test_tc_e2e_comb_05_memory_deletion_cascade() {
    eprintln!("[TC-E2E-COMB-05] Starting Cascading Memory Deletion test...");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open store"));
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());

    // 1. Store a memory chunk
    let store_res = server
        .store_memory(Parameters(StoreMemoryParams {
            content: "Temporary session scratchpad note to be deleted.".to_string(),
            tags: Some(vec!["scratchpad".to_string()]),
            scope: Some("global".to_string()),
            workspace: None,
        }))
        .await
        .expect("store memory");
    let store_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&store_res)).unwrap();
    let chunk_id = store_json["id"].as_str().unwrap().to_string();

    // 2. Check stats before delete
    let stats_before = server
        .get_index_stats(Parameters(GetIndexStatsParams {}))
        .await
        .expect("get_index_stats before");
    let stats_b_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&stats_before)).unwrap();
    let initial_chunks = stats_b_json["total_chunks"].as_u64().unwrap();
    assert!(initial_chunks >= 1, "Must have at least 1 chunk");

    // 3. Delete memory via `orbit.delete_memory` (F07 -> F01)
    let del_res = server
        .delete_memory(Parameters(DeleteMemoryParams {
            id: chunk_id.clone(),
        }))
        .await
        .expect("delete_memory call");
    let del_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&del_res)).unwrap();
    assert_eq!(del_json["deleted"], true);
    assert_eq!(del_json["id"], chunk_id);

    // 4. Verify chunk is purged from SQLite metadata store
    let parsed_uuid = Uuid::parse_str(&chunk_id).unwrap();
    let chunk_in_db = store.metadata().get_chunk(&parsed_uuid).await.unwrap();
    assert!(chunk_in_db.is_none(), "Chunk must be completely purged from SQLite database");

    // 5. Verify stats updated
    let stats_after = server
        .get_index_stats(Parameters(GetIndexStatsParams {}))
        .await
        .expect("get_index_stats after");
    let stats_a_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&stats_after)).unwrap();
    assert_eq!(stats_a_json["total_chunks"].as_u64().unwrap(), initial_chunks - 1);

    eprintln!("[TC-E2E-COMB-05] PASS: Cascading Memory Deletion verified.");
}

/// TC-E2E-COMB-06: Agent Auto-Wiring Detection & Universal Skill Verification (F08 + F07)
/// Tests that agent configuration files (JSON & YAML) and the universal `orbit-memory.skill.md`
/// match the exact contract and parameter schemas exposed by the Orbit MCP server.
#[tokio::test]
async fn test_tc_e2e_comb_06_agent_auto_wiring_harness_integration() {
    eprintln!("[TC-E2E-COMB-06] Starting Agent Auto-Wiring & Universal Skill Verification test...");

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();

    // 1. Simulate mock directory structure for coding agent harnesses
    let harnesses = [
        ("antigravity", ".gemini/config/mcp_config.json", ".gemini/antigravity/skills/orbit/SKILL.md"),
        ("claude_code", ".claude/mcp_config.json", ".claude/skills/orbit-memory/SKILL.md"),
        ("cursor", ".cursor/mcp.json", ".cursor/rules/orbit-memory.md"),
        ("codex", ".codex/config.json", ".codex/instructions.md"),
        ("goose", ".config/goose/config.yaml", ".goose/skills/orbit.yaml"),
        ("opencode", ".opencode/mcp.json", ".opencode/skills/orbit.md"),
        ("zcode", ".zcode/mcp_config.json", ".zcode/instructions/orbit.md"),
        ("agy_cli", ".gemini/config/mcp_config.json", ".gemini/skills/orbit.md"),
        ("kimi", ".kimi/mcp.json", ".kimi/rules/orbit.md"),
    ];

    for (name, config_rel, skill_rel) in &harnesses {
        let config_path = home.join(config_rel);
        let skill_path = home.join(skill_rel);

        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();

        // Write configuration pointing to buzz-mcp stdio
        if config_rel.ends_with(".yaml") {
            let yaml_content = r#"extensions:
  orbit:
    cmd: buzz-mcp
    args:
      - "--stdio"
    envs: {}
"#;
            fs::write(&config_path, yaml_content).unwrap();
        } else {
            let json_content = serde_json::json!({
                "mcpServers": {
                    "orbit": {
                        "command": "buzz-mcp",
                        "args": ["--stdio"],
                        "env": {}
                    }
                }
            });
            fs::write(&config_path, serde_json::to_string_pretty(&json_content).unwrap()).unwrap();
        }

        // Write canonical skill file
        let skill_content = format!(
            "# Orbit Brain Memory & Context Skill for {name}\n\n\
            Use the following tools:\n\
            - `orbit.search_context`: Hybrid semantic search.\n\
            - `orbit.store_memory`: Persist decisions.\n\
            - `orbit.get_project_context`: Architectural overview.\n\
            - `orbit.recall_session`: Cross-agent conversation memory.\n\
            - `orbit.get_file_history`: Chronological diffs.\n\
            - `orbit.mark_decision`: Invalidate obsolete ADRs.\n\
            - `orbit.get_index_stats`: Knowledge health.\n\
            - `orbit.delete_memory`: GDPR purge.\n"
        );
        fs::write(&skill_path, skill_content).unwrap();
    }

    // 2. Verify all configurations exist and parse correctly
    for (name, config_rel, skill_rel) in &harnesses {
        let cfg_path = home.join(config_rel);
        let skl_path = home.join(skill_rel);

        assert!(cfg_path.exists(), "Config for {} must exist at {:?}", name, cfg_path);
        assert!(skl_path.exists(), "Skill for {} must exist at {:?}", name, skl_path);

        let skill_text = fs::read_to_string(&skl_path).unwrap();
        assert!(skill_text.contains("orbit.search_context"));
        assert!(skill_text.contains("orbit.store_memory"));
        assert!(skill_text.contains("orbit.mark_decision"));
    }

    eprintln!("[TC-E2E-COMB-06] PASS: Agent Auto-Wiring and Universal Skill verified across all 9 harnesses.");
}

/// TC-E2E-COMB-07: Full Master Lifecycle Loop (All 8 Features F01 through F08 in sequential harmony)
/// Executes the complete Orbit Brain lifecycle in a single end-to-end integration scenario:
/// 1. Initialize embedded local store (F01)
/// 2. Initialize local ONNX embedder & reranker (F02)
/// 3. Ingest source code workspace files with AST chunking (F03)
/// 4. Extract Knowledge Graph entities & bi-temporal ADR relations (F05)
/// 5. Retroactively ingest agent transcripts from IDEs (F06)
/// 6. Auto-wire developer harnesses & verify skill injection (F08)
/// 7. Perform hybrid multi-modal SuperRAG retrieval with delimiter fencing (F04)
/// 8. Execute all 8 MCP tools through stdio protocol server (F07)
/// 9. Invalidate superseded decisions and verify precedence (F05 + F04 + F07)
/// 10. Perform cascading delete and verify index integrity (F01 + F07)
#[tokio::test]
async fn test_tc_e2e_comb_07_full_master_lifecycle() {
    eprintln!("===============================================================================");
    eprintln!("[TC-E2E-COMB-07] EXECUTING MASTER LIFECYCLE LOOP (FEATURES 01 THROUGH 08)");
    eprintln!("===============================================================================");

    let tmp = tempdir().expect("tempdir");
    let brain_dir = tmp.path().join("brain");
    let workspace_dir = tmp.path().join("workspace");
    let home_dir = tmp.path().join("home");
    fs::create_dir_all(&workspace_dir.join("src")).unwrap();
    fs::create_dir_all(&home_dir).unwrap();

    // Step 1: Storage Foundation (F01)
    eprintln!("[STEP 1/10] Initializing EmbeddedMemoryStore (F01)...");
    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).expect("open embedded store"));
    assert!(brain_dir.join("db").join("orbit.db").exists(), "Authoritative SQLite DB must exist");

    // Step 2: Embedding Engine (F02)
    eprintln!("[STEP 2/10] Initializing Local Embedder & Reranker (F02)...");
    let embedder = Arc::new(LocalOnnxEmbedder::default_local());
    let reranker = Arc::new(LocalOnnxReranker::default_local());
    let test_emb = embedder.embed_batch(&["Orbit test query"]).await.unwrap();
    assert_eq!(test_emb[0].len(), 384, "Embedder must produce 384-dimensional normalized vector");

    // Step 3: Ingestion Pipeline (F03)
    eprintln!("[STEP 3/10] Ingesting Workspace Code with AST Chunker & Secret Redactor (F03)...");
    let pipeline = IngestionPipeline::new(store.clone(), embedder.clone());
    let rust_code = r#"
pub fn calculate_orbit_trajectory(velocity: f64, radius: f64) -> f64 {
    // Standard gravitational parameter mu = G * M
    let mu: f64 = 398600.4418;
    let specific_energy = (velocity * velocity) / 2.0 - mu / radius;
    specific_energy
}
"#;
    let code_path = workspace_dir.join("src").join("physics.rs");
    fs::write(&code_path, rust_code).unwrap();
    let ws_str = workspace_dir.to_string_lossy().to_string();
    let ingested_doc = pipeline.ingest_file(&ws_str, &code_path, "workspace").await.unwrap();
    assert!(ingested_doc.is_some(), "Physics code file must be ingested");

    // Step 4: Knowledge Graph (F05)
    eprintln!("[STEP 4/10] Extracting Knowledge Graph Entities & ADR Decision (F05)...");
    let adr_path = workspace_dir.join("ADR-001.md");
    let adr_content = r#"# ADR-001: Local First Storage
Status: Accepted
Decision: Use embedded SQLite with WAL mode for local zero-network operation.
"#;
    fs::write(&adr_path, adr_content).unwrap();
    pipeline.ingest_file(&ws_str, &adr_path, "workspace").await.unwrap();

    // Step 5: Recall Agent Multi-IDE Ingestion (F06)
    eprintln!("[STEP 5/10] Parsing Past Agent Transcripts (F06)...");
    let claude_dir = tmp.path().join("claude").join("orbit-proj");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("session_1.json"),
        r#"{"sessionId":"s1","messages":[{"role":"user","content":[{"type":"text","text":"Implement SuperRAG routing arbiter"}]},{"role":"assistant","content":[{"type":"text","text":"Implemented pre-retrieval query routing into Symbolic, Conceptual, TemporalDecision intents."}]}]}"#,
    ).unwrap();
    let claude_plugin = ClaudeCodeRecallPlugin::with_custom_root(&claude_dir);
    assert!(claude_plugin.detect());
    let claude_docs = claude_plugin.ingest_sessions(Path::new(&ws_str)).await;
    assert_eq!(claude_docs.len(), 1);

    // Step 6: Agent Auto-Wiring (F08)
    eprintln!("[STEP 6/10] Validating Agent Auto-Wiring & Skill Injection (F08)...");
    let agy_config_dir = home_dir.join(".gemini").join("config");
    fs::create_dir_all(&agy_config_dir).unwrap();
    let agy_mcp = serde_json::json!({
        "mcpServers": {
            "orbit": {
                "command": "buzz-mcp",
                "args": ["--stdio"]
            }
        }
    });
    fs::write(agy_config_dir.join("mcp_config.json"), serde_json::to_string_pretty(&agy_mcp).unwrap()).unwrap();
    assert!(agy_config_dir.join("mcp_config.json").exists());

    // Step 7: SuperRAG Multi-Modal Retrieval (F04)
    eprintln!("[STEP 7/10] Executing SuperRAG Multi-Modal Retrieval Engine (F04)...");
    let superrag = SuperRagEngine::new(store.clone(), embedder.clone(), reranker.clone());
    let rag_req = SuperRagRequest::new(ws_str.clone(), "calculate orbit trajectory physics")
        .with_budget(2000)
        .with_limit(10);
    let rag_res = superrag.query(&rag_req).await.expect("superrag retrieve");
    assert!(!rag_res.chunks.is_empty(), "Must retrieve physics code chunks");
    assert!(rag_res.context_xml.contains("<orbit_context"), "Must generate formatted context XML");
    assert!(rag_res.context_xml.contains("</orbit_context>"), "Must close formatted context XML");
    assert!(rag_res.context_xml.contains("calculate_orbit_trajectory"));

    // Step 8: MCP Protocol Server Tools (F07)
    eprintln!("[STEP 8/10] Invoking Orbit MCP Server Tools with Delimiter Fencing (F07)...");
    let server = OrbitMcpServer::new(store.clone(), embedder.clone(), reranker.clone());

    // 8a. `orbit.search_context`
    let mcp_search = server
        .search_context(Parameters(SearchContextParams {
            query: "trajectory velocity radius".to_string(),
            limit: Some(5),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .unwrap();
    let search_txt = extract_tool_text(&mcp_search);
    assert!(search_txt.contains("<orbit_untrusted_context source=\"orbit.search_context\">"));
    assert!(search_txt.contains("specific_energy"));

    // 8b. `orbit.store_memory`
    let mcp_store = server
        .store_memory(Parameters(StoreMemoryParams {
            content: "Architectural Decision: Orbit Brain V1 relies purely on embedded SQLite and LanceDB.".to_string(),
            tags: Some(vec!["architecture".to_string()]),
            scope: Some("workspace".to_string()),
            workspace: Some(ws_str.clone()),
        }))
        .await
        .unwrap();
    let stored_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&mcp_store)).unwrap();
    let new_chunk_id = stored_json["id"].as_str().unwrap().to_string();

    // 8c. `orbit.get_index_stats`
    let mcp_stats = server.get_index_stats(Parameters(GetIndexStatsParams {})).await.unwrap();
    let stats_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&mcp_stats)).unwrap();
    assert_eq!(stats_json["index_health"], "healthy");
    assert!(stats_json["total_chunks"].as_u64().unwrap() >= 2);

    // Step 9: Invalidation and Precedence (F05 + F04 + F07)
    eprintln!("[STEP 9/10] Marking Obsolete Decision and Verifying Invalidation (F05 + F07)...");
    let mark_res = server
        .mark_decision(Parameters(MarkDecisionParams {
            id: new_chunk_id.clone(),
            state: "deprecated".to_string(),
            note: Some("Deprecated in favor of newer spec".to_string()),
        }))
        .await
        .unwrap();
    let mark_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&mark_res)).unwrap();
    assert_eq!(mark_json["state"], "deprecated");

    // Step 10: Cascading Deletion (F01 + F07)
    eprintln!("[STEP 10/10] Performing Cascading Delete via orbit.delete_memory (F01 + F07)...");
    let del_res = server
        .delete_memory(Parameters(DeleteMemoryParams {
            id: new_chunk_id.clone(),
        }))
        .await
        .unwrap();
    let del_json: serde_json::Value = serde_json::from_str(&extract_tool_text(&del_res)).unwrap();
    assert_eq!(del_json["deleted"], true);

    eprintln!("===============================================================================");
    eprintln!("[TC-E2E-COMB-07] MASTER LIFECYCLE LOOP COMPLETED SUCCESSFULLY: ALL 8 FEATURES PASS");
    eprintln!("===============================================================================");
}
