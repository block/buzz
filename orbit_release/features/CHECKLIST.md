# Orbit Brain V1 — Master Implementation Checklist

> **Progress Tracker**: Use this master checklist to monitor the implementation status of all 13 feature modules across the Orbit Brain V1 build.
> Each feature has its own dedicated directory in `orbit_release/features/XX_feature_name/` containing a granular `CHECKLIST.md` and `IMPLEMENTATION.md`.
> Built upon Orbit's **5-Layer Context Architecture**: Source Layer (L1), Processed Context Layer (L2), Memory Layer (L3), Retrieval Layer (L4), and Working Context Layer (L5).

---

## High-Level Status Dashboard

| # | Feature Module | Directory | Priority | Sprint | Dependencies | Status |
|---|----------------|-----------|----------|--------|--------------|--------|
| **01** | Storage Foundation (Embedded Local-First Store) | `01_storage_foundation/` | P0 | Sprint 1 | None | `[x] Completed` |
| **02** | Embedding & Reranker Engine | `02_embedding_engine/` | P0 | Sprint 1 | F01 | `[x] Completed` |
| **03** | Ingestion Pipeline (L1 $\rightarrow$ L2) | `03_ingestion_pipeline/` | P0 | Sprint 2 | F01, F02 | `[x] Completed` |
| **04** | SuperRAG Retrieval Layer (Data Re-Trial & Reranking)| `04_multi_rag_search/` | P0 | Sprint 2 | F01, F02 | `[x] Completed` |
| **05** | Knowledge Graph (L3 Bi-temporal Memory) | `05_knowledge_graph/` | P1 | Sprint 3 | F01, F03 | `[x] Completed` |
| **06** | Recall Agent (9 IDE Parsers) | `06_recall_agent/` | P1 | Sprint 3 | F01, F02, F03 | `[x] Completed` |
| **07** | MCP Server Tools (`orbit.*`) | `07_mcp_server_tools/` | P0 | Sprint 4 | F01, F02, F04, F05, F06 | `[x] Completed` |
| **08** | Desktop 1-Click Harness Hub | `08_agent_auto_wiring/` | P1 | Sprint 4 | F07 | `[x] Completed` |
| **09** | Obsidian-Style Brain Graph | `09_desktop_brain_panel/` | P1 | Sprint 5 | F04, F05, F07 | `[ ] Not Started` |
| **10** | Packaging & Hardening | `10_packaging_hardening/` | P0 | Sprint 6 | F01–F10 | `[ ] Not Started` |
| **11** | Hosted / Enterprise Foundation | `11_hosted_enterprise/` | P1 | Post-V1 | F01–F10 | `[ ] Planned` |
| **12** | Identity, Subscription & Multi-Device Sync | `12_identity_subscription_sync/` | P1 | Post-V1 | F10, F11 | `[ ] Planned` |
| **13** | Auth Server & Data Governance | `13_auth_server_and_data_governance/` | P1 | Post-V1 | F10, F11, F12 | `[ ] Planned` |

---

## Milestones & Gates

### Phase 1: Core Foundation & Runtimes (Sprint 1)
- [x] **F01**: embedded SQLite + LanceDB + Ladybug/Kuzu stores initialize without external services
- [x] **F01**: Authoritative SQLite metadata + vector store + graph store initialized
- [x] **F01**: Secret redactor prevents sensitive credentials from entering storage
- [x] **F02**: `ort` ONNX runtime initializes `bge-small-en-v1.5` embedder on CPU (<5ms / 384-dim)
- [x] **F02**: `ort` ONNX runtime initializes `bge-reranker-small` cross-encoder on CPU (<10ms / 50 pairs)
- [x] **F02**: Cloud providers (OpenAI, Voyage, Cohere, Gemini, Ollama) implemented behind traits
- [x] **F02**: API keys securely stored via `buzz-auth` OS keyring (DPAPI, Keychain, Secret Service)

### Phase 2: Ingestion & SuperRAG Retrieval (Sprint 2)
- [x] **F03**: `buzz-ingest` file watcher monitors workspace changes using `notify`
- [x] **F03**: Tree-sitter AST chunker chunks Rust, TS, Python, Go, and Markdown with provenance
- [x] **F03**: Git history ingested (commits, authors, diffs) with SHA hashes
- [x] **F04**: Pre-Retrieval Data Re-Trial & Query Routing Arbiter classifies intents and checks semantic cache
- [x] **F04**: Multi-Modal candidate retrieval executes over embedded VectorStore + SQLite FTS5
- [x] **F04**: Reciprocal Rank Fusion (RRF) combines scores with temporal decay ($e^{-\lambda \Delta t}$) and authority weighting
- [x] **F04**: Data Re-Trial & Confidence Verification loop triggers adaptive reformulation on ambiguous queries
- [x] **F04**: Native MVP Cross-Encoder Reranker (`bge-reranker-small`) re-scores top 50 candidates to top 15 precision chunks
- [x] **F04**: Token budget knapsack packing formats Layer 5 context into `<orbit_context>` semantic XML tags

### Phase 3: Knowledge Graph & Historical Recall (Sprint 3)
- [x] **F05**: Bi-temporal entity and relation extraction operational (Graphiti model)
- [x] **F05**: Contradiction resolver invalidates obsolete facts (`invalid_at = replacement.valid_at`)
- [x] **F05**: 2-hop neighborhood traversal integrated into SuperRAG retrieval
- [x] **F06**: 9 IDE transcript parsers implemented:
  - [x] Antigravity (`~/.gemini/...`)
  - [x] Claude Code (`~/.claude/...`)
  - [x] Codex (`~/.codex/...`)
  - [x] Cursor (`~/.cursor/...`)
  - [x] Goose (`~/.config/goose/...`)
  - [x] OpenCode (`~/.opencode/...`)
  - [x] ZCode (`~/.zcode/...`)
  - [x] AGY CLI (`~/.gemini/...`)
  - [x] Kimi (`~/.kimi/...`)
- [x] **F06**: Incremental sync ensures no duplicate transcript ingestion

### Phase 4: MCP Tools, Plugins & Desktop Settings Hub (Sprint 4)
- [x] **F07**: 8 `orbit.*` MCP tools implemented and registered in `buzz-dev-mcp` and `buzz-mcp`:
  - [x] `orbit.search_context`
  - [x] `orbit.store_memory`
  - [x] `orbit.get_project_context`
  - [x] `orbit.recall_session`
  - [x] `orbit.get_file_history`
  - [x] `orbit.mark_decision`
  - [x] `orbit.get_index_stats`
  - [x] `orbit.delete_memory`
- [x] **F07**: Context Arbiter security fencing operational (`<orbit_untrusted_context>`, secret redaction)
- [x] **F07**: Desktop Settings UI: **"Plugins & MCP Tools"** panel (`PluginsMcpSettingsPanel.tsx`)
- [x] **F07**: Settings tool toggles (enable/disable built-in tools) and execution policies (`Auto-Approve` vs `Confirm-on-Execute`)
- [x] **F07**: Custom MCP server registration (stdio & SSE) with live handshake & tool discovery tester
- [x] **F07**: Plugins directory, marketplace, and custom plugin installer
- [x] **F08**: Desktop App Onboarding / Settings "Agent Harnesses" screen implemented
- [x] **F08**: 1-Click "Connect to Orbit Brain" writes MCP config & skill files without terminal
- [x] **F08**: Centralized `~/.orbit/brain/` shared automatically across all connected harnesses

### Phase 5: "AI Brain" Primary Navigation & Obsidian-Style Brain Graph (Sprint 5)
- [ ] **F09**: Primary navigation entry **"AI Brain"** on main page left sidebar (`AppSidebar.tsx` / `CommunityRail.tsx`)
- [ ] **F09**: Route screen `desktop/src/app/routes/ai-brain.tsx` with Top Ingestion HUD and SuperRAG omnibar
- [ ] **F09**: D3 force-directed physics graph renders in React 19 desktop (`/ai-brain`)
- [ ] **F09**: Multi-project, chat, agent, document, entity, and decision nodes rendered with distinct styles
- [ ] **F09**: Full compliance with codebase typography contract (`--buzz-type-scale`, `--text-xs`, `--text-sm`)
- [ ] **F09**: Interactive physics controls (gravity, charge repulsion, link distance, collision)
- [ ] **F09**: Node inspection drawer reveals connected chats, agent sessions, code diffs, and ADR history
- [ ] **F09**: Real-time filtering by agent, project, entity type, and temporal date slider

### Phase 6: Hardening, Local Embedder Bundling & Packaging (Sprint 6)
- [ ] **F10**: Zero-Docker embedded SQLite + LanceDB + Ladybug/Kùzu runner functional
- [ ] **F10**: Data directory structured under `~/.orbit/brain/` (db, vectors, graph, models, transcripts, sync)
- [ ] **F10**: Pre-packaged quantized ONNX models (`bge-small-en-v1.5`, `bge-reranker-small`) bundled in `desktop/src-tauri/resources/models/`
- [ ] **F10**: First-boot model extractor populates `~/.orbit/brain/models/` offline
- [ ] **F10**: `tauri.conf.json` bundles external binaries (`buzz-mcp`, `buzz-dev-mcp`, `buzz-agent`, `buzz-acp`, `buzz`)
- [ ] **F10**: Container boundary verified: Docker Compose & Kubernetes reserved exclusively for hosted team relay
- [ ] **F10**: Cloud sync staging log operational (`sync/changelog.jsonl`)
- [ ] **F10**: Windows `.exe` installer (NSIS) and portable `.exe` built and verified
- [ ] **F10**: macOS `.dmg` and `.app` bundle built and verified
- [ ] **F10**: End-to-end latency, restart/recovery, and RAM/disk footprint benchmarks measured on release builds
- [ ] **F10**: `just ci` passes cleanly

### Phase 7: Hosted / Enterprise Foundation (Post-V1)
- [ ] **F11**: Hosted API, tenant model, PostgreSQL + pgvector schema, object storage and worker contracts implemented
- [ ] **F11**: Local/hosted logical memory schema parity tests pass
- [ ] **F11**: Server-side authorization is enforced before vector, graph or object retrieval
- [ ] **F11**: Hosted processing is optional and policy-controlled; desktop retrieval remains local-first

### Phase 8: Identity, Subscription & Multi-Device Sync (Post-V1)
- [ ] **F12**: Website-first account signup/login flow implemented
- [ ] **F12**: OAuth authorization-code + PKCE flow returns to desktop through HTTPS app/universal link or private-use scheme fallback
- [ ] **F12**: Device registration and revocation implemented
- [ ] **F12**: Subscription entitlements gate cloud sync without disabling local-only memory
- [ ] **F12**: Append-only logical event/mutation sync protocol implemented
- [ ] **F12**: Conflict resolution and idempotent replay tested across two devices
- [ ] **F12**: Local data remains usable offline after cloud disconnection
- [ ] **F12**: Future mobile client can hydrate a brain from the same canonical sync model


## Feature 13 — Auth Server & Data Governance

- [ ] Dedicated web authentication server
- [ ] Desktop `Log In` / `Sign Up` / `Continue Local` entry flow
- [ ] Email/password lifecycle outside Tauri
- [ ] Deep-link + one-time code exchange
- [ ] OS-keychain session storage
- [ ] Device registration/revocation
- [ ] Personal vs enterprise workspace ownership boundaries
- [ ] Enterprise data classification and policy engine
- [ ] Local-only / sync / hosted-processing enforcement
- [ ] Retention/delete/export/audit hooks
- [ ] Device/security telemetry separated from brain content
