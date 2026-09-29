# Feature 03 — Ingestion Pipeline Checklist

> **Directory**: `orbit_release/features/03_ingestion_pipeline/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 2 (Weeks 3–4)

---

## Deliverables & Tasks

### 1. `buzz-ingest` Crate Setup
- [ ] Add `notify = "6.1"` for filesystem watching
- [ ] Add `tree-sitter` and grammars (`tree-sitter-rust`, `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-go`, `tree-sitter-md`)
- [ ] Add `git2` for Git repository introspection
- [ ] Wire dependencies to `buzz-db` and `buzz-ai`

### 2. AST Chunker
- [ ] Implement language detection via file extension
- [ ] Implement Rust AST chunker (split by `fn`, `struct`, `impl`, `mod`)
- [ ] Implement TypeScript/JavaScript AST chunker
- [ ] Implement Python AST chunker
- [ ] Implement Go AST chunker
- [ ] Implement Markdown heading-based chunker
- [ ] Implement token count estimation and chunk boundary overlap (~50 tokens)

### 3. File Watcher & Deduplication
- [ ] Implement `WorkspaceWatcher` using `notify` with debouncing (500ms)
- [ ] Compute SHA-256 `content_hash` before processing
- [ ] Skip unchanged files where `mtime_ns` and `content_hash` match existing records
- [ ] Ignore ignored directories (`.git/`, `target/`, `node_modules/`, `dist/`)

### 4. Git Metadata Integration
- [ ] Extract current commit SHA, branch, and author for ingested files
- [ ] Attach Git metadata into `orbit_documents.metadata` JSONB

### 5. Orchestration Pipeline
- [ ] Connect Secret Redactor → Chunker → Embedder → Database transaction
- [ ] Handle document deletion (cascade delete associated chunks)

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-ingest` passes
- [ ] Real-time modification of a test file triggers chunking and embedding
- [ ] AST chunking preserves function and struct integrity
- [ ] `just ci` passes cleanly
