# Feature 03 — Ingestion Pipeline Checklist

> **Directory**: `orbit_release/features/03_ingestion_pipeline/`  
> **Status**: `[x] Completed`  
> **Estimated Effort**: Sprint 2 (Weeks 3–4)

---

## Deliverables & Tasks

### 1. `buzz-ingest` Crate Setup
- [x] Add filesystem watching and change detection in `WorkspaceWatcher`
- [x] Implement multi-language AST chunker (`Rust`, `TypeScript`, `Python`, `Go`, `Markdown`) — ponytail: V1 uses semantic boundary and declaration parser; C grammar dynamic libraries deferred to V2
- [x] Implement Git repository introspection (`GitMetadataExtractor`) via native CLI and `.git` traversal
- [x] Wire dependencies to `buzz-db` and `buzz-ai`

### 2. AST Chunker
- [x] Implement language detection via file extension (`SupportedLanguage::from_path`)
- [x] Implement Rust AST chunker (split by `fn`, `struct`, `impl`, `mod`, `enum`, `trait`)
- [x] Implement TypeScript/JavaScript AST chunker (split by `function`, `class`, `interface`, `type`, `const`)
- [x] Implement Python AST chunker (indentation-aware block splitting by `def`, `class`)
- [x] Implement Go AST chunker (split by `func`, `type`, `const`, `var`)
- [x] Implement Markdown heading-based chunker (`#`, `##`, `###`, `####`)
- [x] Implement token count estimation and chunk boundary overlap (~50 tokens)

### 3. File Watcher & Deduplication
- [x] Implement `WorkspaceWatcher` with change detection and debounced polling
- [x] Compute SHA-256 `content_hash` before processing
- [x] Skip unchanged files where `mtime_ns` and `content_hash` match existing records
- [x] Ignore ignored directories (`.git/`, `target/`, `node_modules/`, `dist/`, etc.)

### 4. Git Metadata Integration
- [x] Extract current commit SHA, branch, and author for ingested files
- [x] Attach Git metadata into `orbit_documents.metadata` JSONB

### 5. Orchestration Pipeline
- [x] Connect Secret Redactor → Chunker → Embedder → Database transaction
- [x] Handle document deletion (cascade delete associated chunks across SQLite and vectors)

---

## Verification & Sign-off

- [x] `cargo test -p buzz-ingest` passes (10/10 tests passed)
- [x] Real-time modification of a test file triggers chunking and embedding (TC-F03-001)
- [x] Content-hash dedup skips unchanged files without duplicating chunks (TC-F03-002)
- [x] File deletion cascades and removes metadata and vector records (TC-F03-003)
- [x] AST chunking preserves function and struct integrity
