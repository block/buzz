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



