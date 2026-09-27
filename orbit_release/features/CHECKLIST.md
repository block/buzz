# Orbit Brain V1 — Master Implementation Checklist

> **Progress Tracker**: Use this master checklist to monitor the implementation status of all 10 feature modules across the Orbit Brain V1 build.
> Each feature has its own dedicated directory in `orbit_release/features/XX_feature_name/` containing a granular `CHECKLIST.md` and `IMPLEMENTATION.md`.

---

## High-Level Status Dashboard

| # | Feature Module | Directory | Priority | Sprint | Dependencies | Status |
|---|----------------|-----------|----------|--------|--------------|--------|
| **01** | Storage Foundation | `01_storage_foundation/` | P0 | Sprint 1 | None | `[ ] Not Started` |
| **02** | Embedding Engine | `02_embedding_engine/` | P0 | Sprint 1 | F01 | `[ ] Not Started` |
| **03** | Ingestion Pipeline | `03_ingestion_pipeline/` | P0 | Sprint 2 | F01, F02 | `[ ] Not Started` |
| **04** | Multi-RAG Search Engine | `04_multi_rag_search/` | P0 | Sprint 2 | F01, F02 | `[ ] Not Started` |
| **05** | Knowledge Graph | `05_knowledge_graph/` | P1 | Sprint 3 | F01, F03 | `[ ] Not Started` |
| **06** | Recall Agent (9 IDE Parsers)| `06_recall_agent/` | P1 | Sprint 3 | F01, F02, F03 | `[ ] Not Started` |
| **07** | MCP Server Tools | `07_mcp_server_tools/` | P0 | Sprint 4 | F01, F02, F04, F05, F06 | `[ ] Not Started` |
| **08** | Desktop 1-Click Harness Hub| `08_agent_auto_wiring/` | P1 | Sprint 4 | F07 | `[ ] Not Started` |
| **09** | Obsidian-Style Brain Graph | `09_desktop_brain_panel/` | P1 | Sprint 5 | F04, F05, F07 | `[ ] Not Started` |
| **10** | Packaging & Hardening | `10_packaging_hardening/` | P0 | Sprint 6 | All Features | `[ ] Not Started` |

---

## Milestones & Gates

### Phase 1: Core Foundation (Sprint 1)
- [ ] **F01**: `0047_buzz_memory_pgvector.sql` migration runs cleanly on PostgreSQL 16
- [ ] **F01**: `buzz_documents`, `buzz_chunks`, `buzz_entities`, `buzz_relations` created
- [ ] **F01**: Secret redactor prevents sensitive tokens from entering the DB
- [ ] **F02**: `ort` ONNX runtime initializes `bge-small-en-v1.5` on CPU (<5ms / 384-dim)
- [ ] **F02**: Cloud providers (OpenAI, Voyage, Gemini, Ollama) implemented behind `EmbedProvider` trait
- [ ] **F02**: API keys securely stored via `buzz-auth` OS keyring

### Phase 2: Ingestion & Retrieval (Sprint 2)
- [ ] **F03**: `orbit-ingest` file watcher monitors workspace changes
- [ ] **F03**: Tree-sitter AST chunker chunks Rust, TS, Python, Go, and Markdown
- [ ] **F03**: Git history ingested (commits, authors, diffs)
- [ ] **F04**: Layer 1 (Dense Vector) search executes over pgvector HNSW index
- [ ] **F04**: Layer 2 (Lexical BM25) search executes over GIN `search_tsv`
- [ ] **F04**: Reciprocal Rank Fusion (RRF) combines scores with temporal decay
- [ ] **F04**: Token budget knapsack packing formats context into `<orbit_context>` tags

### Phase 3: Knowledge Graph & Historical Recall (Sprint 3)
- [ ] **F05**: Bi-temporal entity and relation extraction operational (Graphiti model)
- [ ] **F05**: Contradiction resolver invalidates obsolete facts (`invalid_at = NOW()`)
- [ ] **F05**: 2-hop neighborhood traversal integrated into RRF search (Layer 3)
- [ ] **F06**: 9 IDE transcript parsers implemented:
  - [ ] Antigravity (`~/.gemini/...`)
  - [ ] Claude Code (`~/.claude/...`)
  - [ ] Codex (`~/.codex/...`)
  - [ ] Cursor (`~/.cursor/...`)
  - [ ] Goose (`~/.config/goose/...`)
  - [ ] OpenCode (`~/.opencode/...`)
  - [ ] ZCode (`~/.zcode/...`)
  - [ ] AGY CLI (`~/.gemini/...`)
  - [ ] Kimi (`~/.kimi/...`)
- [ ] **F06**: Incremental sync ensures no duplicate transcript ingestion

### Phase 4: MCP Tools & Desktop Agent Hub (Sprint 4)
- [ ] **F07**: 8 `orbit.*` MCP tools implemented and registered in `buzz-dev-mcp`:
  - [ ] `orbit.search_context`
  - [ ] `orbit.store_memory`
  - [ ] `orbit.get_project_context`
  - [ ] `orbit.recall_session`
  - [ ] `orbit.get_file_history`
  - [ ] `orbit.mark_decision`
  - [ ] `orbit.get_index_stats`
  - [ ] `orbit.delete_memory`
- [ ] **F07**: Context Arbiter security fencing operational
- [ ] **F08**: Desktop App Onboarding / Settings "Agent Harnesses" screen implemented
- [ ] **F08**: 1-Click "Connect to Orbit Brain" writes MCP config & skill files without terminal
- [ ] **F08**: Centralized `orbit_brain/` shared automatically across all connected harnesses

### Phase 5: Obsidian-Style Brain Graph (Sprint 5)
- [ ] **F09**: D3 force-directed physics graph renders in React 19 desktop (`/memory/graph`)
- [ ] **F09**: Multi-project, chat, agent, document, and entity nodes rendered with distinct styles
- [ ] **F09**: Interactive physics controls (gravity, charge repulsion, link distance, collision)
- [ ] **F09**: Node inspection drawer reveals connected chats, agent sessions, and code diffs
- [ ] **F09**: Real-time filtering by agent, project, entity type, and temporal date slider

### Phase 6: Hardening & Packaging (Sprint 6)
- [ ] **F10**: Zero-Docker embedded PostgreSQL 16 + pgvector runner functional
- [ ] **F10**: Data directory structured under `orbit_brain/` (storage, models, transcripts, exports, sync)
- [ ] **F10**: Cloud sync staging log operational (`sync/changelog.wal`)
- [ ] **F10**: Windows `.exe` installer (NSIS) and portable `.exe` built and verified
- [ ] **F10**: macOS `.dmg` and `.app` bundle built and verified
- [ ] **F10**: Latency benchmarks verified (P99 search `<25ms`, embed `<5ms`, idle RAM `<150MB`)
- [ ] **F10**: `just ci` passes cleanly
