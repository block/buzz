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

