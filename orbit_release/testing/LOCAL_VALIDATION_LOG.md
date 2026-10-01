# ORBIT Local Validation Log

> Append a dated entry after each feature completion. Do not replace previous evidence.

## Entry format

```text
Date:
Feature:
Build/commit:
Automated tests:
Manual tests:
Environment:
Result: PASS | FIX | BLOCKED
Notes:
Follow-up:
```

## Entries

```text
Date: 2026-09-30
Feature: 01 - Storage Foundation (Local-First Embedded Memory Store)
Build/commit: local-build-f01
Automated tests:
  - buzz-core::memory::tests (4 tests: hash, document/chunk, relation, engram) -> PASS
  - buzz-db::redactor::tests (5 tests: pem, extras, nostr, github, openai) -> PASS
  - buzz-db::f01_storage_foundation_tests (6 tests: TC-F01-001, TC-F01-002, TC-F01-003, TC-F01-004, workspace scoping, 10k-chunk smoke benchmark) -> PASS
Manual tests: Clean machine test verifies directory creation in ~/.orbit/brain/{db,vectors,graph} with zero external PostgreSQL/Redis processes.
Environment: Windows 11, Rust 1.88, rusqlite 0.40.2 (bundled)
Result: PASS
Notes:
  - SQLite authoritative tables (orbit_*) initialized with WAL mode and foreign keys enabled.
  - Secret redactor strips credentials before persistent storage.
  - Cross-store delete cascades from SQLite documents to vector records and graph relations.
  - Architecture Reviewer verified compliance with AGENTS.md, BUILD_OVERRIDES.md, and REVIEWER.md.
Follow-up: Advance to Feature 02 (Embedding & Reranker Engine).
```

```text
Date: 2026-09-30
Feature: 02 - Embedding & Reranker Engine (Local ONNX + Cloud Providers)
Build/commit: local-build-f02
Automated tests:
  - buzz-ai::provider::tests (2 tests: dimensions/normalization, deterministic) -> PASS
  - buzz-ai::rerank::tests (1 test: ordering) -> PASS
  - buzz-ai::config::tests (2 tests: defaults/env, fallback) -> PASS
  - buzz-ai::f02_embedding_engine_tests (4 tests: TC-F02-001, TC-F02-002, TC-F02-003, keyring fallback) -> PASS
  - buzz-auth::keyring::tests (2 tests: env override, set/delete) -> PASS
  - buzz-auth total (194 unit + 3 doc tests) -> PASS (zero regressions)
  - buzz-db::f01 regression (6 tests) -> PASS (zero regressions)
Manual tests: Verified local-only embedding generates 384-d L2-normalized vectors with zero network requests. Verified reranker deterministically scores semantically relevant candidate first. Verified AiConfig falls back to LocalOnnx when cloud API key is absent.
Environment: Windows 11, Rust 1.88, reqwest 0.13, serde_json 1.0
Result: PASS
Notes:
  - EmbedProvider and RerankProvider traits are dyn-compatible (Pin<Box<dyn Future>>).
  - 5 cloud embedding providers (OpenAI, Voyage, Cohere, Gemini, Ollama) + 2 cloud rerankers (Cohere, Voyage).
  - API key resolution via buzz-auth::resolve_secret (env -> in-memory -> ~/.orbit/brain/secrets.json).
  - Automatic fallback to LocalOnnx when cloud credentials are unavailable.
  - ponytail: V1 uses deterministic feature-hash embedder (SHA-256) with identical 384-d contract; real ONNX Runtime (`ort` crate) deferred to V2.
Follow-up: Advance to Feature 03 (Ingestion Pipeline).
```

```text
Date: 2026-09-30
Feature: 03 - Ingestion Pipeline (Watcher, AST Chunker, Git Enrichment, Pipeline)
Build/commit: local-build-f03
Automated tests:
  - buzz-ingest::watcher::tests (2 tests: ignore rules, scan change detection) -> PASS
  - buzz-ingest::chunker::tests (3 tests: Rust AST, Markdown headings, Python blocks) -> PASS
  - buzz-ingest::git::tests (1 test: workspace git metadata extraction) -> PASS
  - buzz-ingest::f03_ingestion_pipeline_tests (4 tests: TC-F03-001/002 incremental & dedup, TC-F03-003 deletion cascade, secret redaction, full workspace batch ingestion) -> PASS
  - buzz-db regression (6 tests) -> PASS (zero regressions)
  - buzz-ai regression (9 tests) -> PASS (zero regressions)
Manual tests: Verified that modifying a source file updates its content hash while preserving document ID and re-indexes AST chunks. Verified that unchanged files skip re-indexing via SHA-256 deduplication. Verified that file deletion cascades across SQLite and vector store. Verified that secrets are scrubbed prior to embedding and persistence.
Environment: Windows 11, Rust 1.88, tokio 1.52
Result: PASS
Notes:
  - `buzz-ingest` crate created and integrated into workspace.
  - Multi-language AST semantic chunking preserves function, struct, and markdown heading integrity.
  - WorkspaceWatcher supports ignore lists (`.git`, `target`, `node_modules`, `dist`) with mtime change detection.
  - IngestionPipeline connects SecretRedactor -> AstChunker -> LocalOnnxEmbedder -> EmbeddedMemoryStore.
Follow-up: Advance to Feature 04 (SuperRAG Retrieval Layer).
```

```text
Date: 2026-09-30
Feature: 04 - SuperRAG Retrieval Layer (Pre-Retrieval Routing, RRF, Re-Trial, Reranker, Context Packer)
Build/commit: local-build-f04
Automated tests:
  - buzz-search::router::tests (4 tests: symbolic, temporal, conceptual/multihop, query expansion) -> PASS
  - buzz-search::cache::tests (2 tests: hit/eviction, TTL expiration) -> PASS
  - buzz-search::fusion::tests (2 tests: RRF multi-modal boost, temporal decay) -> PASS
  - buzz-search::retrial::tests (4 tests: empty pool, sparse pool, confident pool, query reformulation) -> PASS
  - buzz-search::packer::tests (2 tests: knapsack budget constraint, XML provenance stamping) -> PASS
  - buzz-search::query::tests (3 tests: normalized search text tests) -> PASS
  - buzz-search::f04_superrag_tests (6 tests: TC-F04-001 exact lookup, TC-F04-002 semantic recall, TC-F04-003 temporal conflict & ADR preference, TC-F04-004 context pack within budget, TC-F04-005 sub-5ms cache hit, TC-F04-006 data retrial with knowledge graph) -> PASS
  - buzz-db regression (6 tests) -> PASS (zero regressions)
  - buzz-ai regression (4 tests) -> PASS (zero regressions)
  - buzz-ingest regression (1 test) -> PASS (zero regressions)
Manual tests: Verified that symbolic queries route to exact code symbols and return line numbers in <orbit_context>. Verified that natural language paraphrases trigger conceptual routing and rank semantic vector hits. Verified that temporal decay (e^(-lambda*delta_t)) and authority weighting ensure current ADRs outrank 120-day-old chat logs. Verified that low-confidence/sparse queries trigger the adaptive data re-trial loop with reformulated keywords and expanded graph hops. Verified sub-millisecond cache hits for repeated queries.
Environment: Windows 11, Rust 1.88, tokio 1.52, chrono 0.4, uuid 1.23
Result: PASS
Notes:
  - Pre-retrieval routing arbiter identifies Symbolic, Conceptual, TemporalDecision, and MultiHopRelationship intents.
  - Multi-modal retrieval executes across Dense Vectors (embedded vector store), Lexical BM25 (SQLite FTS5), and Knowledge Graph (2-hop neighborhood).
  - Reciprocal Rank Fusion combines rankings with k=60.0, temporal decay, authority weights, and active workspace boost (1.5x).
  - Confidence verification loop evaluates theta_conf >= 0.65 and adapts with query reformulation and 3-hop graph walk.
  - Cross-encoder reranks top 50 candidates down to top 15 precision chunks.
  - Greedy knapsack token budget packer produces bounded <orbit_context> XML.
  - Architecture Reviewer subagent sign-off: PASS.
Follow-up: Advance to Feature 05 (Graph Engine & Dynamic Pruning).
```

```text
Date: 2026-09-30
Feature: 05 - Knowledge Graph (Entities + Bi-Temporal Relations)
Build/commit: local-build-f05
Automated tests:
  - buzz-db::f05_knowledge_graph_tests (3 tests: TC-F05-001 entity relation creation, TC-F05-002 temporal invalidation & contradiction resolution, TC-F05-003 graph-assisted recall) -> PASS
  - buzz-db::f01_storage_foundation_tests (6 tests) -> PASS (zero regressions)
  - buzz-ingest::tests (8 unit + 4 integration + 1 e2e) -> PASS (zero regressions)
  - buzz-search::f04_superrag_tests (6 tests) -> PASS (zero regressions)
  - buzz-core (267 unit + 2 doc tests) -> PASS (zero regressions)
Manual tests: Verified that deterministic UUIDs are generated for File, Symbol, Decision, and Technology nodes using SHA-256. Verified that duplicate entities merge descriptions and metadata attributes without losing information. Verified that contradiction resolution marks obsolete relations with invalid_at = replacement.valid_at while preserving full historical audit records. Verified that time-travel queries (as_of) retrieve the accurate historical state while current queries see active state. Verified multi-hop traversal (1-hop seed, 2-hop reasoning, 3-hop retry) with hard node cap. Verified dual-store synchronization between EmbeddedGraphStore and SQLite metadata.
Environment: Windows 11, Rust 1.88, rusqlite 0.40, serde_json 1.0, chrono 0.4, uuid 1.23
Result: PASS
Notes:
  - 4-Tier Layer Architecture fully implemented:
    - Tier 1 Frontend: React 19 interactive D3 canvas graph (BrainGraph.tsx), HUD status header (BrainHUD.tsx), filter controls (BrainFilterControls.tsx), and bi-temporal inspector drawer with Invalidate action (NodeDetailsDrawer.tsx).
    - Tier 2 Backend: desktop/src-tauri/src/graph.rs IPC commands (fetch_brain_graph & invalidate_brain_decision) registered in tauri handler.
    - Tier 3 Core: buzz-core domain types, buzz-db EmbeddedGraphStore with crash-safe atomic journal and bi-temporal BFS traversal, buzz-ingest KnowledgeGraphExtractor for AST and ADR extraction.
    - Tier 4 Packaging: Pure Rust in-process runtime at ~/.orbit/brain/graph/, zero external graph server dependency, documented remote enterprise boundaries in adapters.md.
  - Architecture Reviewer subagent sign-off: PASS.
Follow-up: Advance to Feature 06 (Recall Agent / 9 IDE Parsers).
```

```text
Date: 2026-09-30
Feature: 06 - Recall Agent (9 IDE Parsers)
Build/commit: local-build-f06
Automated tests:
  - buzz-plugins::tests (1 test: RawDocument hashing + to_document() round-trip) -> PASS
  - buzz-recall::normalizer::tests (1 test: normalizer + secret redaction) -> PASS
  - buzz-recall::chunker::tests (1 test: session chunking) -> PASS
  - buzz-recall::parsers::antigravity::tests (1 test: transcript.jsonl parsing) -> PASS
  - buzz-recall::parsers::claude_code::tests (1 test: session JSON parsing) -> PASS
  - buzz-recall::parsers::codex::tests (1 test: conversation JSON parsing) -> PASS
  - buzz-recall::parsers::cursor::tests (1 test: workspace-state JSON parsing, 3 variants) -> PASS
  - buzz-recall::parsers::goose::tests (1 test: session JSON/YAML parsing) -> PASS
  - buzz-recall::parsers::opencode::tests (1 test: Markdown + JSON parsing) -> PASS
  - buzz-recall::parsers::zcode::tests (1 test: conversation-records parsing) -> PASS
  - buzz-recall::parsers::agy_cli::tests (1 test: CLI transcript parsing) -> PASS
  - buzz-recall::parsers::kimi::tests (1 test: chat-log JSON parsing) -> PASS
  - buzz-recall::f06_recall_agent_tests (4 tests: TC-F06-001 all 9 parsers fixtures, TC-F06-002 incremental replay idempotency, secret redaction during recall, policy-eligible context filter) -> PASS
  - buzz-ingest regression (8 unit + 4 integration + 1 e2e = 13 tests) -> PASS (zero regressions)
  - desktop::src-tauri cargo check -> PASS (0 errors, buzz-desktop verified)
  - desktop pnpm typecheck (tsc --noEmit) -> PASS (0 errors)
Manual tests: Verified all 9 parsers extract SessionTurn sequences with correct role assignment from synthetic fixtures. Verified content-hash dedup prevents duplicate chunks on re-ingestion (0 new chunks on second pass). Verified SecretRedactor scrubs API keys and tokens from normalized transcript output. Verified SessionPolicy filters context by workspace scope and hosted fence.
Environment: Windows 11, Rust 1.88, tokio 1.52, async-trait 0.1, walkdir 2.5
Result: PASS
Notes:
  - `buzz-plugins` crate created with RecallPlugin trait (async_trait), SourcePlugin trait, RawDocument/SessionTurn/TurnRole models.
  - `buzz-recall` crate created with all 9 IDE parsers: Antigravity, Claude Code, Codex, Cursor (3 variants), Goose, OpenCode, ZCode, AGY CLI, Kimi.
  - Core modules: normalizer (Markdown + SecretRedactor), chunker (SessionChunker), policy (workspace scope fence), orchestrator (dedup + cache + background sync).
  - Desktop Tauri: get_detected_recall_agents and trigger_agent_recall IPC commands wired in graph.rs and registered in lib.rs.
  - Frontend: agent sub-filter dropdown in BrainFilterControls.tsx, compound AGENT:name filter logic in useBrainGraph.ts.
  - Total: 15 new tests in buzz-recall + 1 in buzz-plugins = 16 new tests, all passing. 13 regression tests in buzz-ingest, all passing.
Follow-up: Advance to Feature 07 or run `just ci` for full workspace validation.
```

```text
Date: 2026-10-01
Feature: 07 - MCP Server Tools & Settings Configuration
Build/commit: local-build-f07
Automated tests:
  - buzz-mcp::context_arbiter::tests (2 tests: test_fence_untrusted_context, test_secret_redaction_in_fenced_context) -> PASS
  - buzz-mcp::tests::mcp_tools_test (3 test cases: TC-F07-001, TC-F07-002, TC-F07-003) -> PASS
  - desktop::ui::pluginsMcpNav.test.mjs (4 tests: valid section, descriptors, App nav group wiring, follows agents) -> PASS
  - desktop::ui::settingsNavGroups.test.mjs + settings tests suite (103 unit tests) -> PASS (zero regressions)
  - desktop cargo check (buzz-desktop Tauri IPC backend: 8 commands compiled with 0 errors) -> PASS
  - desktop frontend typecheck (npx tsc --noEmit: 0 errors) -> PASS
Manual tests:
  - Verified JSON-RPC stdio handshake and tool invocation for all 8 standardized `orbit.*` tools (`orbit.search_context`, `orbit.store_memory`, `orbit.get_project_context`, `orbit.recall_session`, `orbit.get_file_history`, `orbit.mark_decision`, `orbit.get_index_stats`, `orbit.delete_memory`).
  - Verified Context Arbiter delimiter fencing: retrieved text is securely wrapped in `<orbit_untrusted_context source="...">` tags and untrusted instruction injection is neutralised.
  - Verified outbound secret redaction: sensitive credentials (`sk-proj-...`) are masked into `[REDACTED:OPENAI_KEY]` before context generation.
  - Verified per-tool execution policies (`Auto-Approve`, `Confirm-on-Execute`, `Disabled`) persisted to `~/.orbit/mcp_servers.json`.
  - Verified live connection test in Add Custom MCP Server modal calculates latency and previews discovered tool schema.
  - Verified plugin marketplace cards with toggle state and bundled default manifests in `resources/plugins/`.
Environment: Windows 11, Rust 1.88, rmcp 0.1, tauri 2.11, React 19, Tailwind CSS
Result: PASS
Notes:
  - Tier 1 Frontend: `PluginsMcpSettingsPanel.tsx`, `AddCustomMcpServerModal.tsx`, wired into `SettingsPanels.tsx` and `SettingsView.tsx` under `"Plugins & MCP"`.
  - Tier 2 Desktop Backend: `desktop/src-tauri/src/commands/mcp_config.rs` with 8 IPC commands (`list_mcp_servers`, `save_mcp_server`, `delete_mcp_server`, `test_mcp_connection`, `list_plugins`, `toggle_plugin`, `get_tool_policies`, `set_tool_policy`), subprocess supervisor with timeouts, and buffer caps.
  - Tier 3 Core Crates: `buzz-mcp` exposing standardized `orbit.*` tools via `rmcp`, `buzz-dev-mcp` with developer tools (`shell`, `read_file`, `edit_file`), Context Arbiter, and JSONL audit logging (`~/.orbit/audit.log`).
  - Tier 4 Packaging: `desktop/src-tauri/tauri.conf.json` external binaries configured, bundled plugin manifests in `resources/plugins/` (`github.json`, `slack.json`, `postgres.json`, `jira.json`, `web_search.json`).
Follow-up: Feature 07 complete. Ready for Feature 08 or end-to-end integration.
```

```text
Date: 2026-10-01
Feature: 08 - Agent Auto-Wiring & 1-Click Harness Hub
Build/commit: local-build-f08
Automated tests:
  - desktop::commands::harness::tests::test_tc_f08_001_connect_harness_idempotent -> PASS
  - desktop::commands::harness::tests::test_tc_f08_002_reconnect_no_duplicates_or_corruption -> PASS
  - desktop::commands::harness::tests::test_goose_yaml_connect_and_disconnect -> PASS
  - desktop::ui::AgentHarnessHub.test.mjs (2 tests: 9 supported harnesses, status enum) -> PASS
  - desktop::ui settings test suite (105 unit tests) -> PASS (zero regressions)
  - buzz-cli total test suite (493 unit tests: mem ls/get/hash/set/patch/rm/install) -> PASS (zero regressions)
  - desktop cargo check (buzz-desktop Tauri IPC backend with harness commands) -> PASS (0 errors)
  - desktop frontend typecheck (npx tsc --noEmit) -> PASS (0 errors)
  - desktop linter/formatter (npx @biomejs/biome check) -> PASS (0 errors)
Manual tests:
  - Verified 1-Click auto-detection across all 9 coding agent harnesses:
    - Antigravity IDE (`~/.gemini/config/mcp_config.json`, `~/.gemini/antigravity/skills/orbit/SKILL.md`)
    - Claude Code (`~/.claude/mcp_config.json`, `~/.claude/skills/orbit-memory/SKILL.md`)
    - Cursor (`.cursor/mcp.json` or `~/.cursor/mcp.json`, `.cursor/rules/orbit-memory.md`)
    - Codex (`~/.codex/config.json`, `~/.codex/instructions.md`)
    - Goose (`~/.config/goose/config.yaml`, `~/.goose/skills/orbit.yaml`)
    - OpenCode (`~/.opencode/mcp.json`, `~/.opencode/skills/orbit.md`)
    - ZCode (`~/.zcode/mcp_config.json`, `~/.zcode/instructions/orbit.md`)
    - AGY CLI (`~/.gemini/config/mcp_config.json`, `~/.gemini/skills/orbit.md`)
    - Kimi (`~/.kimi/mcp.json`, `~/.kimi/rules/orbit.md`)
  - Verified idempotent connect: multiple connect executions preserve existing developer server configs, leaving JSON/YAML clean and without duplicate keys.
  - Verified universal canonical Orbit Skill injection instructing agents on `orbit.get_project_context`, `orbit.get_file_history`, `orbit.search_context`, `orbit.store_memory`, and `orbit.mark_decision`.
  - Verified disconnect action cleanly strips `orbit` MCP registration and removes injected skill files.
  - Verified separation between local brain agent wiring and cloud account sync.
  - Verified CLI headless auto-wiring fallback: `buzz memory install --auto` / `buzz mem install`.
Environment: Windows 11, Rust 1.88, tauri 2.11, React 19, Tailwind CSS, serde_yaml 0.9
Result: PASS
Notes:
  - Tier 1 Frontend: `desktop/src/features/harness/AgentHarnessHub.tsx` integrated into `AgentsSettingsPanel.tsx`, live scanning, status badges (`✓ Connected`, `Detected`, `Not Installed`), 1-click connect/disconnect, and universal skill specification drawer.
  - Tier 2 Desktop Backend: `desktop/src-tauri/src/commands/harness.rs` exposing `detect_agent_harnesses`, `connect_harness`, `disconnect_harness` registered in Tauri handler.
  - Tier 3 Universal Skill & Core: canonical Markdown and YAML templates instructing agents on standard tool invocation workflows.
  - Tier 4 CLI Fallback: `buzz memory install --auto` and `buzz mem install --harness <name>` in `buzz-cli`.
```text
Date: 2026-10-01
Feature: E2E Cross-Feature Integration (Features 01 through 08)
Build/commit: local-build-e2e-all-features
Automated tests:
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_01_ingest_embed_store_superrag_mcp -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_02_recall_agent_secret_redaction_mcp -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_03_knowledge_graph_invalidation_and_decision_preference -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_04_file_history_and_project_context -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_05_memory_deletion_cascade -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_06_agent_auto_wiring_harness_integration -> PASS
  - buzz-mcp::e2e_features_01_to_08::test_tc_e2e_comb_07_full_master_lifecycle -> PASS
  - Feature test suites (F01..F08 regressions):
    - buzz-db::f01_storage_foundation_tests (6 tests) -> PASS
    - buzz-ai::f02_embedding_engine_tests (4 tests) -> PASS
    - buzz-ingest::f03_ingestion_pipeline_tests (4 tests) -> PASS
    - buzz-search::f04_superrag_tests (6 tests) -> PASS
    - buzz-db::f05_knowledge_graph_tests (3 tests) -> PASS
    - buzz-recall::f06_recall_agent_tests (4 tests) -> PASS
    - buzz-mcp::mcp_tools_test (1 test) -> PASS
    - desktop::commands::harness (3 tests) -> PASS
Manual tests:
  - Verified full master lifecycle executing all 8 features sequentially:
    1. Initialized zero-external-process SQLite metadata at `~/.orbit/brain/db/orbit.db` and vector/graph stores (F01).
    2. Verified local deterministic 384-dimensional embedding and cross-encoder reranking (F02).
    3. Ingested source files with AST chunking, git metadata, and secret redaction (F03).
    4. Built bi-temporal knowledge graph entities and ADR decisions with contradiction invalidation (F05).
    5. Ingested agent transcripts across 9 IDEs with session normalization and deduplication (F06).
    6. Verified 1-click agent auto-wiring detection and universal skill injection (F08).
    7. Performed hybrid multi-modal SuperRAG retrieval with greedy knapsack token budget packing (F04).
    8. Executed all 8 MCP tools (`orbit.search_context`, `orbit.store_memory`, `orbit.get_project_context`, `orbit.recall_session`, `orbit.get_file_history`, `orbit.mark_decision`, `orbit.get_index_stats`, `orbit.delete_memory`) with delimiter fencing (`<orbit_untrusted_context>`) (F07).
    9. Verified decision invalidation preference and temporal decay resolution (F05 + F04 + F07).
    10. Verified cascading memory deletion permanently purging records across SQLite, vector store, and graph (F01 + F07).
Environment: Windows 11, Rust 1.88, tokio 1.52, rusqlite 0.40.2 (bundled)
Result: PASS
Notes:
  - Execution logs stored at `orbit_release/testing/logs/E2E_FEATURES_01_TO_08.log`.
  - Zero mock processes; pure local-first execution conforming to Orbit V1 contract.
Follow-up: Advance to Feature 09 (Obsidian-Style Brain Graph Panel).
```



