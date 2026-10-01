#![deny(unsafe_code)]
//! Integration and feature tests for Feature 06: Recall Agent (9 IDE Parsers).
//!
//! Verifies:
//! - TC-F06-001: Run each supported transcript parser on a fixture -> events are normalized into shared model.
//! - TC-F06-002: Process same transcript twice -> no duplicate durable events/chunks are created (idempotency).
//! - Secret redaction across past agent transcripts.
//! - Transcript caching in `orbit_brain/transcripts/`.
//! - Multi-device policy context filtering.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

use buzz_ai::{AiConfig, EmbedProviderType};
use buzz_core::memory::VectorFilter;
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_db::memory::traits::{MetadataStore, VectorStore};
use buzz_plugins::RecallPlugin;
use buzz_recall::{
    filter_policy_eligible_chunks,
    AgyCliRecallPlugin, AntigravityRecallPlugin, ClaudeCodeRecallPlugin, CodexRecallPlugin,
    CursorRecallPlugin, GooseRecallPlugin, KimiRecallPlugin, OpenCodeRecallPlugin,
    PolicyContextQuery, RecallOrchestrator, ZCodeRecallPlugin,
};

#[tokio::test]
async fn tc_f06_001_all_9_parsers_fixtures() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let workspace = Path::new("/test/workspace");

    // 1. Antigravity fixture
    let antigravity_dir = root.join("antigravity").join("brain").join("session-alpha");
    fs::create_dir_all(&antigravity_dir).unwrap();
    fs::write(
        antigravity_dir.join("transcript.jsonl"),
        r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"Add SQLite index to orbit_chunks"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","thinking":"Adding index on document_id for faster lookups.","content":"Added CREATE INDEX idx_orbit_chunks_doc ON orbit_chunks(document_id);","tool_calls":[{"name":"sql_exec"}]}"#,
    )
    .unwrap();

    let p1 = AntigravityRecallPlugin::with_custom_root(root.join("antigravity").join("brain"));
    assert!(p1.detect());
    let docs1 = p1.ingest_sessions(workspace).await;
    assert_eq!(docs1.len(), 1);
    assert_eq!(docs1[0].metadata["agent_name"], "antigravity");
    assert!(docs1[0].content.contains("Add SQLite index"));
    assert!(docs1[0].content.contains("Adding index on document_id"));

    // 2. Claude Code fixture
    let claude_dir = root.join("claude").join("proj_alpha");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("session_1.json"),
        r#"{"sessionId":"claude_01","messages":[{"role":"user","content":[{"type":"text","text":"Implement bi-temporal knowledge graph"}]},{"role":"assistant","content":[{"type":"text","text":"Implemented invalid_at and valid_at timestamps for relations."}]}]}"#,
    )
    .unwrap();

    let p2 = ClaudeCodeRecallPlugin::with_custom_root(&claude_dir);
    assert!(p2.detect());
    let docs2 = p2.ingest_sessions(workspace).await;
    assert_eq!(docs2.len(), 1);
    assert_eq!(docs2[0].metadata["agent_name"], "claude_code");
    assert!(docs2[0].content.contains("Implement bi-temporal knowledge graph"));

    // 3. Codex fixture
    let codex_dir = root.join("codex");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::write(
        codex_dir.join("conv_1.json"),
        r#"{"id":"codex_conv_01","messages":[{"role":"user","content":"Optimize RRF fusion weights"},{"role":"assistant","content":"Set 0.7 dense and 0.3 BM25 lexical."}]}"#,
    )
    .unwrap();

    let p3 = CodexRecallPlugin::with_custom_root(&codex_dir);
    assert!(p3.detect());
    let docs3 = p3.ingest_sessions(workspace).await;
    assert_eq!(docs3.len(), 1);
    assert_eq!(docs3[0].metadata["agent_name"], "codex");
    assert!(docs3[0].content.contains("Optimize RRF fusion weights"));

    // 4. Cursor fixture
    let cursor_dir = root.join("cursor").join("ws_storage_1");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("composer.json"),
        r#"{"composerId":"cursor_comp_01","conversation":[{"type":1,"text":"Fix Catppuccin color theme"},{"type":2,"text":"Switched palette to Mocha with Mauve accents."}]}"#,
    )
    .unwrap();

    let p4 = CursorRecallPlugin::with_custom_root(root.join("cursor"));
    assert!(p4.detect());
    let docs4 = p4.ingest_sessions(workspace).await;
    assert_eq!(docs4.len(), 1);
    assert_eq!(docs4[0].metadata["agent_name"], "cursor");
    assert!(docs4[0].content.contains("Fix Catppuccin color theme"));

    // 5. Goose fixture
    let goose_dir = root.join("goose");
    fs::create_dir_all(&goose_dir).unwrap();
    fs::write(
        goose_dir.join("session_goose.json"),
        r#"{"id":"goose_s1","messages":[{"role":"user","content":[{"type":"text","text":"Run cargo clippy on all crates"}]},{"role":"assistant","content":[{"type":"text","text":"Clippy completed with 0 warnings."}]}]}"#,
    )
    .unwrap();

    let p5 = GooseRecallPlugin::with_custom_root(&goose_dir);
    assert!(p5.detect());
    let docs5 = p5.ingest_sessions(workspace).await;
    assert_eq!(docs5.len(), 1);
    assert_eq!(docs5[0].metadata["agent_name"], "goose");
    assert!(docs5[0].content.contains("Run cargo clippy"));

    // 6. OpenCode fixture
    let opencode_dir = root.join("opencode");
    fs::create_dir_all(&opencode_dir).unwrap();
    fs::write(
        opencode_dir.join("session_oc.md"),
        "# User\nDesign local-first SQLite schema\n\n# Assistant\nCreated orbit_documents and orbit_chunks tables.",
    )
    .unwrap();

    let p6 = OpenCodeRecallPlugin::with_custom_root(&opencode_dir);
    assert!(p6.detect());
    let docs6 = p6.ingest_sessions(workspace).await;
    assert_eq!(docs6.len(), 1);
    assert_eq!(docs6[0].metadata["agent_name"], "opencode");
    assert!(docs6[0].content.contains("Design local-first SQLite schema"));

    // 7. ZCode fixture
    let zcode_dir = root.join("zcode");
    fs::create_dir_all(&zcode_dir).unwrap();
    fs::write(
        zcode_dir.join("zcode_session.json"),
        r#"{"conversation_id":"zc_99","history":[{"speaker":"human","utterance":"Add D3 canvas force simulation"},{"speaker":"agent","utterance":"Implemented velocity decay and repulsive charge force."}]}"#,
    )
    .unwrap();

    let p7 = ZCodeRecallPlugin::with_custom_root(&zcode_dir);
    assert!(p7.detect());
    let docs7 = p7.ingest_sessions(workspace).await;
    assert_eq!(docs7.len(), 1);
    assert_eq!(docs7[0].metadata["agent_name"], "zcode");
    assert!(docs7[0].content.contains("Add D3 canvas force simulation"));

    // 8. AGY CLI fixture
    let agy_dir = root.join("agy_transcripts");
    fs::create_dir_all(&agy_dir).unwrap();
    fs::write(
        agy_dir.join("agy_run.json"),
        r#"[{"command":"agy init","prompt":"Initialize orbit repo","response":"Orbit initialized with local brain directory."}]"#,
    )
    .unwrap();

    let p8 = AgyCliRecallPlugin::with_custom_root(&agy_dir);
    assert!(p8.detect());
    let docs8 = p8.ingest_sessions(workspace).await;
    assert_eq!(docs8.len(), 1);
    assert_eq!(docs8[0].metadata["agent_name"], "agy_cli");
    assert!(docs8[0].content.contains("Initialize orbit repo"));

    // 9. Kimi fixture
    let kimi_dir = root.join("kimi");
    fs::create_dir_all(&kimi_dir).unwrap();
    fs::write(
        kimi_dir.join("kimi_01.json"),
        r#"{"chat_id":"kimi_chat_1","messages":[{"role":"user","content":"How does MMR diversity reranking work?"},{"role":"assistant","content":"MMR balances relevance with novelty to reduce redundancy."}]}"#,
    )
    .unwrap();

    let p9 = KimiRecallPlugin::with_custom_root(&kimi_dir);
    assert!(p9.detect());
    let docs9 = p9.ingest_sessions(workspace).await;
    assert_eq!(docs9.len(), 1);
    assert_eq!(docs9[0].metadata["agent_name"], "kimi");
    assert!(docs9[0].content.contains("How does MMR diversity reranking work?"));
}

#[tokio::test]
async fn tc_f06_002_incremental_replay_and_idempotency() {
    let temp = tempdir().unwrap();
    let brain_dir = temp.path().join("brain");
    let workspace_dir = temp.path().join("workspace");
    let cache_dir = temp.path().join("transcripts");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).unwrap());
    let mut ai_config = AiConfig::default();
    ai_config.embed_provider = EmbedProviderType::LocalOnnx;
    let embedder = ai_config.build_embedder().unwrap();

    // Create session fixture
    let sessions_dir = temp.path().join("antigravity_sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    let transcript_path = sessions_dir.join("transcript.jsonl");

    let fixture = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"Build the Recall Agent for Orbit"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","thinking":"Implementing 9 IDE transcript parsers.","content":"Implemented Antigravity, Claude Code, Codex, Cursor, Goose, OpenCode, ZCode, AGY CLI, and Kimi parsers."}"#;
    fs::write(&transcript_path, fixture).unwrap();

    let mut orchestrator = RecallOrchestrator::new(store.clone(), embedder.clone())
        .with_transcript_cache_dir(&cache_dir);
    orchestrator.register_plugin(Arc::new(AntigravityRecallPlugin::with_custom_root(&sessions_dir)));

    // ─── First Run ───────────────────────────────────────────────────────────
    let summary1 = orchestrator.ingest_all(&workspace_dir).await.unwrap();
    assert_eq!(summary1.sessions_ingested, 1);
    assert_eq!(summary1.sessions_skipped_dedup, 0);
    assert!(summary1.chunks_created > 0);
    assert!(summary1.vectors_saved > 0);

    let doc_count1 = store.metadata().count_documents().await.unwrap();
    let chunk_count1 = store.metadata().count_chunks().await.unwrap();
    let vector_count1 = store.vectors().count_vectors().await.unwrap();

    assert_eq!(doc_count1, 1);
    assert!(chunk_count1 > 0);
    assert_eq!(vector_count1, chunk_count1);

    // Verify transcript was cached in cache_dir/antigravity/
    let cached_files = fs::read_dir(cache_dir.join("antigravity")).unwrap().count();
    assert_eq!(cached_files, 1, "Transcript must be cached in local orbit_brain/transcripts");

    // ─── Second Run (Incremental Replay / Idempotency) ──────────────────────
    let summary2 = orchestrator.ingest_all(&workspace_dir).await.unwrap();
    assert_eq!(summary2.sessions_ingested, 0, "No new sessions should be ingested");
    assert_eq!(summary2.sessions_skipped_dedup, 1, "Session should be skipped by content-hash dedup");
    assert_eq!(summary2.chunks_created, 0, "Zero new chunks should be created");
    assert_eq!(summary2.vectors_saved, 0, "Zero new vectors should be embedded");

    // Verify counts in storage remained exactly the same
    assert_eq!(store.metadata().count_documents().await.unwrap(), doc_count1);
    assert_eq!(store.metadata().count_chunks().await.unwrap(), chunk_count1);
    assert_eq!(store.vectors().count_vectors().await.unwrap(), vector_count1);
}

#[tokio::test]
async fn test_secret_redaction_during_transcript_recall() {
    let temp = tempdir().unwrap();
    let brain_dir = temp.path().join("brain");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).unwrap());
    let mut ai_config = AiConfig::default();
    ai_config.embed_provider = EmbedProviderType::LocalOnnx;
    let embedder = ai_config.build_embedder().unwrap();

    let sessions_dir = temp.path().join("claude_sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    let session_file = sessions_dir.join("session_leak.json");

    let leak_json = serde_json::json!({
        "sessionId": "leak_001",
        "messages": [
            {
                "role": "user",
                "content": [{"type": "text", "text": "Deploy with key sk-proj1234567890abcdef123456 and token ghp_1234567890abcdefghijklmnopqrstuvwxyz"}]
            },
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "Acknowledged secret key sk-proj1234567890abcdef123456."}]
            }
        ]
    });
    fs::write(&session_file, leak_json.to_string()).unwrap();

    let mut orchestrator = RecallOrchestrator::new(store.clone(), embedder.clone());
    orchestrator.register_plugin(Arc::new(ClaudeCodeRecallPlugin::with_custom_root(&sessions_dir)));

    let summary = orchestrator.ingest_all(&workspace_dir).await.unwrap();
    assert_eq!(summary.sessions_ingested, 1);

    let doc = store
        .metadata()
        .get_document_by_uri(
            workspace_dir.to_str().unwrap(),
            &session_file.to_string_lossy(),
        )
        .unwrap()
        .expect("Document must exist in store");

    let chunks = store.metadata().get_chunks_by_document(&doc.id).await.unwrap();
    assert!(!chunks.is_empty(), "Chunks must be present for ingested session");
    for chunk in chunks {
        assert!(!chunk.content.contains("sk-proj1234567890abcdef123456"));
        assert!(!chunk.content.contains("ghp_1234567890abcdefghijklmnopqrstuvwxyz"));
        assert!(chunk.content.contains("[REDACTED:OPENAI_KEY]"));
        assert!(chunk.content.contains("[REDACTED:GITHUB_PAT]"));
    }
}

#[tokio::test]
async fn test_policy_eligible_context_filter() {
    let temp = tempdir().unwrap();
    let brain_dir = temp.path().join("brain");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).unwrap();

    let store = Arc::new(EmbeddedMemoryStore::open(&brain_dir).unwrap());
    let mut ai_config = AiConfig::default();
    ai_config.embed_provider = EmbedProviderType::LocalOnnx;
    let embedder = ai_config.build_embedder().unwrap();

    let codex_dir = temp.path().join("codex");
    fs::create_dir_all(&codex_dir).unwrap();
    let conv_file = codex_dir.join("conv_policy.json");
    let json = serde_json::json!({
        "id": "policy_test",
        "messages": [
            {"role": "user", "content": "How do we enforce local-only memory policies?"},
            {"role": "assistant", "content": "Check workspace boundaries and local-only permission fences before returning recalled chunks."}
        ]
    });
    fs::write(&conv_file, json.to_string()).unwrap();

    let mut orchestrator = RecallOrchestrator::new(store.clone(), embedder.clone());
    orchestrator.register_plugin(Arc::new(CodexRecallPlugin::with_custom_root(&codex_dir)));

    let summary = orchestrator.ingest_all(&workspace_dir).await.unwrap();
    assert_eq!(summary.sessions_ingested, 1);

    // Vector retrieval
    let query_vector = embedder.embed_batch(&["local-only memory policies"]).await.unwrap().into_iter().next().unwrap();
    let filter = VectorFilter {
        workspace_path: Some(workspace_dir.to_string_lossy().to_string()),
        document_id: None,
        scope: None,
    };
    let hits = store.search_vectors(&query_vector, &filter, 5).await.unwrap();
    assert!(!hits.is_empty());

    let chunks: Vec<_> = hits.into_iter().map(|h| {
        buzz_core::memory::Chunk {
            id: h.chunk_id,
            document_id: h.document_id,
            workspace_path: workspace_dir.to_string_lossy().to_string(),
            chunk_index: 0,
            content: h.content,
            token_count: 50,
            scope: "session".to_string(),
            agent_name: Some("codex".to_string()),
            line_start: Some(1),
            line_end: Some(5),
            created_at: chrono::Utc::now(),
        }
    }).collect();

    // Query from matching workspace
    let query_matching = PolicyContextQuery {
        workspace_path: workspace_dir.to_str().unwrap(),
        requesting_agent: Some("codex"),
        is_hosted: false,
    };
    let eligible = filter_policy_eligible_chunks(chunks.clone(), &query_matching);
    assert_eq!(eligible.len(), chunks.len());

    // Query from different workspace -> filtered out
    let query_different_ws = PolicyContextQuery {
        workspace_path: "/different/workspace",
        requesting_agent: Some("codex"),
        is_hosted: false,
    };
    let eligible_diff = filter_policy_eligible_chunks(chunks, &query_different_ws);
    assert_eq!(eligible_diff.len(), 0);
}
