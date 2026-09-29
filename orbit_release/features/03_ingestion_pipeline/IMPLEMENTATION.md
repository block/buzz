# Feature 03 — Ingestion Pipeline (File Watcher + Tree-sitter + Normalizer)

> **Priority**: P0 — Ingestion engine. Populates the storage foundation.  
> **Sprint**: Sprint 2 (Weeks 3–4)  
> **Dependencies**: Feature 01 (Storage), Feature 02 (Embeddings)  
> **Crates**: `orbit-ingest` (implementation), `buzz-db` (storage), `orbit-ai` (vectorization)  
> **Environment Variables**: `BUZZ_DATA_DIR`, `BUZZ_WATCH_INTERVAL_MS`

---

## Overview

Feature 03 converts files, code, and documentation from developer workspaces into chunked, embedded records in the `buzz_documents` and `buzz_chunks` tables. It includes:
1. Continuous background workspace watching with `notify`.
2. AST-aware semantic chunking via Tree-sitter.
3. Git commit, author, and branch metadata enrichment.
4. Automatic secret redaction prior to vectorization and storage.

---

## Architecture & Components

### 1. Ingestion Flow

```
File Change Detected (notify)
            │
            ▼
Content Hash Check (SHA-256 Deduplication)
            │ (If changed or new)
            ▼
Secret Redactor (Strips API keys & credentials)
            │
            ▼
Tree-sitter AST Chunker (~512 token chunks with semantic boundaries)
            │
            ▼
Batch Embedding Generation (Feature 02)
            │
            ▼
Persist to `buzz_documents` & `buzz_chunks` (Feature 01)
```

### 2. Tree-sitter AST Chunker (`crates/orbit-ingest/src/chunker.rs`)

Language-specific AST boundary recognition:
- **Rust**: `function_item`, `struct_item`, `impl_item`, `mod_item`
- **TypeScript / JavaScript**: `function_declaration`, `class_declaration`, `interface_declaration`, `export_statement`
- **Python**: `function_definition`, `class_definition`
- **Go**: `function_declaration`, `method_declaration`, `type_declaration`
- **Markdown / Prose**: Heading boundaries (`#`, `##`, `###`) and paragraphs

### 3. Git Metadata Enrichment

Extracts context from the local `.git` repository:
- Current branch & commit SHA
- Author identity & commit message
- Correlates code changes to historical decisions

---

## Verification & Quality Gates

- Ingest sample project files; verify records created in `buzz_documents` and `buzz_chunks`.
- Verify modified files update `mtime_ns` and regenerate embeddings without duplicating documents.
- Verify secrets are redacted before embedding.
- Run `cargo test -p orbit-ingest`.

## Local-first processing and cloud outbox

Ingestion must complete local normalization, hashing, chunking and memory extraction before any network sync work is attempted. After a durable logical change is committed locally, the sync layer may append an encrypted mutation to the local outbox when cloud sync is enabled.

```text
file/session change
      ↓
local parse + chunk + extract
      ↓
local SQLite/LanceDB/graph update
      ↓
logical mutation recorded
      ↓
optional encrypted sync outbox
```

The ingestion hot path must remain functional when the user is offline.


## Governance gate before persistence

Every ingested item must resolve the current user, tenant, workspace and `WorkspacePolicy` before content leaves the ingestion boundary. Secret detection, source exclusions, classification, local-only rules, cloud-sync eligibility and hosted-processing eligibility are evaluated before persistence or outbound transfer.

For local-only data, the ingestion pipeline may build local indexes but must not enqueue cloud-sync events. For enterprise workspaces, the same policy context must be propagated to embedding, graph and memory-consolidation workers so a downstream worker cannot bypass the ingestion decision.
