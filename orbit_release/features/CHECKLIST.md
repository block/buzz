# Orbit Brain V1 — Master Implementation Checklist

> **Progress Tracker**: Use this master checklist to monitor the implementation status of all 13 feature modules across the Orbit Brain V1 build.
> Each feature has its own dedicated directory in `orbit_release/features/XX_feature_name/` containing a granular `CHECKLIST.md` and `IMPLEMENTATION.md`.
> Built upon Orbit's **5-Layer Context Architecture**: Source Layer (L1), Processed Context Layer (L2), Memory Layer (L3), Retrieval Layer (L4), and Working Context Layer (L5).

---

## High-Level Status Dashboard

| # | Feature Module | Directory | Priority | Sprint | Dependencies | Status |
|---|----------------|-----------|----------|--------|--------------|--------|
| **01** | Storage Foundation (Embedded Local-First Store) | `01_storage_foundation/` | P0 | Sprint 1 | None | `[ ] Not Started` |
| **02** | Embedding & Reranker Engine | `02_embedding_engine/` | P0 | Sprint 1 | F01 | `[ ] Not Started` |
| **03** | Ingestion Pipeline (L1 $\rightarrow$ L2) | `03_ingestion_pipeline/` | P0 | Sprint 2 | F01, F02 | `[ ] Not Started` |
| **04** | SuperRAG Retrieval Layer (Data Re-Trial & Reranking)| `04_multi_rag_search/` | P0 | Sprint 2 | F01, F02 | `[ ] Not Started` |
| **05** | Knowledge Graph (L3 Bi-temporal Memory) | `05_knowledge_graph/` | P1 | Sprint 3 | F01, F03 | `[ ] Not Started` |
| **06** | Recall Agent (9 IDE Parsers) | `06_recall_agent/` | P1 | Sprint 3 | F01, F02, F03 | `[ ] Not Started` |
| **07** | MCP Server Tools (`orbit.*`) | `07_mcp_server_tools/` | P0 | Sprint 4 | F01, F02, F04, F05, F06 | `[ ] Not Started` |
| **08** | Desktop 1-Click Harness Hub | `08_agent_auto_wiring/` | P1 | Sprint 4 | F07 | `[ ] Not Started` |
| **09** | Obsidian-Style Brain Graph | `09_desktop_brain_panel/` | P1 | Sprint 5 | F04, F05, F07 | `[ ] Not Started` |
| **10** | Packaging & Hardening | `10_packaging_hardening/` | P0 | Sprint 6 | F01–F10 | `[ ] Not Started` |
| **11** | Hosted / Enterprise Foundation | `11_hosted_enterprise/` | P1 | Post-V1 | F01–F10 | `[ ] Planned` |
| **12** | Identity, Subscription & Multi-Device Sync | `12_identity_subscription_sync/` | P1 | Post-V1 | F10, F11 | `[ ] Planned` |
| **13** | Auth Server & Data Governance | `13_auth_server_and_data_governance/` | P1 | Post-V1 | F10, F11, F12 | `[ ] Planned` |

---

## Milestones & Gates

### Phase 1: Core Foundation & Runtimes (Sprint 1)
- [ ] **F01**: embedded SQLite + LanceDB + Ladybug/Kuzu stores initialize without external services
- [ ] **F01**: Authoritative SQLite metadata + vector store + graph store initialized
- [ ] **F01**: Secret redactor prevents sensitive credentials from entering storage
- [ ] **F02**: `ort` ONNX runtime initializes `bge-small-en-v1.5` embedder on CPU (<5ms / 384-dim)
- [ ] **F02**: `ort` ONNX runtime initializes `bge-reranker-small` cross-encoder on CPU (<10ms / 50 pairs)
- [ ] **F02**: Cloud providers (OpenAI, Voyage, Cohere, Gemini, Ollama) implemented behind traits
- [ ] **F02**: API keys securely stored via `buzz-auth` OS keyring (DPAPI, Keychain, Secret Service)

### Phase 2: Ingestion & SuperRAG Retrieval (Sprint 2)
- [ ] **F03**: `orbit-ingest` file watcher monitors workspace changes using `notify`
- [ ] **F03**: Tree-sitter AST chunker chunks Rust, TS, Python, Go, and Markdown with provenance
- [ ] **F03**: Git history ingested (commits, authors, diffs) with SHA hashes
- [ ] **F04**: Pre-Retrieval Data Re-Trial & Query Routing Arbiter classifies intents and checks semantic cache
- [ ] **F04**: Multi-Modal candidate retrieval executes over embedded VectorStore + SQLite FTS5
- [ ] **F04**: Reciprocal Rank Fusion (RRF) combines scores with temporal decay ($e^{-\lambda \Delta t}$) and authority weighting
- [ ] **F04**: Data Re-Trial & Confidence Verification loop triggers adaptive reformulation on ambiguous queries
- [ ] **F04**: Native MVP Cross-Encoder Reranker (`bge-reranker-small`) re-scores top 50 candidates to top 15 precision chunks
- [ ] **F04**: Token budget knapsack packing formats Layer 5 context into `<orbit_context>` semantic XML tags

### Phase 3: Knowledge Graph & Historical Recall (Sprint 3)
- [ ] **F05**: Bi-temporal entity and relation extraction operational (Graphiti model)
- [ ] **F05**: Contradiction resolver invalidates obsolete facts (`invalid_at = NOW()`)
- [ ] **F05**: 2-hop neighborhood traversal integrated into SuperRAG retrieval
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
- [ ] **F07**: Context Arbiter security fencing operational (`<orbit_untrusted_context>`)
- [ ] **F08**: Desktop App Onboarding / Settings "Agent Harnesses" screen implemented
- [ ] **F08**: 1-Click "Connect to Orbit Brain" writes MCP config & skill files without terminal
- [ ] **F08**: Centralized `orbit_brain/` shared automatically across all connected harnesses

### Phase 5: Obsidian-Style Brain Graph (Sprint 5)
- [ ] **F09**: D3 force-directed physics graph renders in React 19 desktop (`/memory/graph`)
- [ ] **F09**: Multi-project, chat, agent, document, and entity nodes rendered with distinct styles
- [ ] **F09**: Full compliance with codebase typography contract (`--buzz-type-scale`, `--text-xs`, `--text-sm`)
- [ ] **F09**: Interactive physics controls (gravity, charge repulsion, link distance, collision)
- [ ] **F09**: Node inspection drawer reveals connected chats, agent sessions, and code diffs
- [ ] **F09**: Real-time filtering by agent, project, entity type, and temporal date slider

### Phase 6: Hardening & Packaging (Sprint 6)
- [ ] **F10**: Zero-Docker embedded SQLite + LanceDB + Ladybug runner functional
- [ ] **F10**: Data directory structured under `orbit_brain/` (storage, models, transcripts, sync)
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
