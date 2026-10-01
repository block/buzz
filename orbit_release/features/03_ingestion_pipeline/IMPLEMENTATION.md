# Feature 03 — Ingestion Pipeline (File Watcher + Tree-sitter + Normalizer)

> **Priority**: P0 — Ingestion engine. Populates the storage foundation.  
> **Sprint**: Sprint 2 (Weeks 3–4)  
> **Dependencies**: Feature 01 (Storage), Feature 02 (Embeddings)  
> **Crates**: `buzz-ingest` (implementation), `buzz-db` (storage), `buzz-ai` (vectorization)  
> **Environment Variables**: `BUZZ_DATA_DIR`, `BUZZ_WATCH_INTERVAL_MS`

---

## Overview

Feature 03 converts files, code, and documentation from developer workspaces into chunked, embedded records in the `orbit_documents` and `orbit_chunks` tables. It includes:
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
Persist to `orbit_documents` & `orbit_chunks` (Feature 01)
```

### 2. Tree-sitter AST Chunker (`crates/buzz-ingest/src/chunker.rs`)

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

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: Ingestion & Workspace UX
- **AI Brain Top HUD (`desktop/src/features/memory/BrainHUD.tsx`)**:
  - Displays live count of monitored workspaces and indexing progress.
  - Active file watcher status indicator (pulsing green dot for idle/watching, amber for scanning/indexing).
- **Workspace Manager Dialog (`desktop/src/features/workspace/`)**:
  - "Add Monitored Folder" action utilizing native Tauri file picker.
  - Exclude patterns editor (`.gitignore`, `node_modules`, `target`, `dist`).

### 2. Desktop Backend Tier: Tauri Rust IPC & File Watcher Task
- **Location**: `desktop/src-tauri/src/commands/ingest_control.rs`
- Spawns background asynchronous watcher task using `notify` crate.
- Batches rapid filesystem events with debounce (~300ms) to avoid churning on compiler build artifacts.
- Tauri IPC commands: `start_workspace_watch`, `stop_workspace_watch`, `get_ingest_progress`.

### 3. Core Workspace Crates Tier (`crates/buzz-ingest`, `crates/buzz-db`, `crates/buzz-ai`)
- `crates/buzz-ingest`: Tree-sitter multi-language chunker, SHA-256 deduplication, Git commit enricher.
- `crates/buzz-db`: Persists metadata and chunks to `orbit_documents` and `orbit_chunks`.
- `crates/buzz-ai`: Generates dense 384-d embeddings in batches via local ONNX runtime.

### 4. Packaging, Bundling & Container Tier
- **Zero Docker**: Ingestion operates 100% locally in-process inside the desktop application.
- Tree-sitter language grammars are statically compiled into `buzz-ingest`.

---

## Verification & Quality Gates

- Ingest sample project files; verify records created in `orbit_documents` and `orbit_chunks`.
- Verify modified files update `mtime_ns` and regenerate embeddings without duplicating documents.
- Verify secrets are redacted before embedding.
- Run `cargo test -p buzz-ingest`.

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
