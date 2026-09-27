# Feature 01 — Storage Foundation Checklist

> **Directory**: `orbit_release/features/01_storage_foundation/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 1 (Weeks 1–2)

---

## Deliverables & Tasks

### 1. Database & Migrations
- [ ] Create `migrations/0047_buzz_memory_pgvector.sql`
- [ ] Add `CREATE EXTENSION IF NOT EXISTS vector;`
- [ ] Define `buzz_documents` table with unique constraint on `(workspace_path, content_hash)`
- [ ] Define `buzz_chunks` table with `VECTOR(384)` and generated `search_tsv` column
- [ ] Add HNSW cosine index `idx_buzz_chunks_embedding_hnsw` (`m = 16, ef_construction = 64`)
- [ ] Add GIN index `idx_buzz_chunks_search_tsv` for full-text search
- [ ] Define `buzz_entities` table with unique constraint on `(workspace_path, name, entity_type)`
- [ ] Define `buzz_relations` table with bi-temporal columns (`valid_at`, `invalid_at`, `recorded_at`)

### 2. Rust Types & Models (`crates/buzz-db`)
- [ ] Add `pgvector = { version = "0.4", features = ["sqlx"] }` to `crates/buzz-db/Cargo.toml`
- [ ] Implement `BuzzDocument` struct with SQLx `FromRow`
- [ ] Implement `BuzzChunk` struct with `Vector` support
- [ ] Implement `BuzzEntity` struct
- [ ] Implement `BuzzRelation` struct with bi-temporal helpers

### 3. Storage Data Access Layer (CRUD)
- [ ] `insert_document(pool, doc) -> Result<Uuid>`
- [ ] `insert_chunk(pool, chunk) -> Result<Uuid>`
- [ ] `insert_entity(pool, entity) -> Result<Uuid>`
- [ ] `insert_relation(pool, relation) -> Result<Uuid>`
- [ ] `invalidate_relation(pool, relation_id, invalid_at) -> Result<()>`
- [ ] `delete_chunk(pool, chunk_id) -> Result<()>`
- [ ] `get_storage_stats(pool, workspace_path) -> Result<StorageStats>`

### 4. Secret Redactor & Sanitizer
- [ ] Implement regex patterns in `crates/buzz-db/src/redactor.rs`
  - [ ] AWS Keys (`AKIA...`)
  - [ ] GitHub PATs (`ghp_...`, `github_pat_...`)
  - [ ] Bearer tokens & JWTs
  - [ ] Private Keys (`BEGIN RSA/OPENSSH PRIVATE KEY`)
- [ ] Unit tests for redactor with synthetic secret payloads

### 5. Data Directory & Cloud Sync Preparation
- [ ] Configure `orbit_brain/` directory structure under `BUZZ_DATA_DIR`
  - [ ] `orbit_brain/storage/`
  - [ ] `orbit_brain/sync/`
- [ ] Verify WAL / sync logging scaffolding

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-db` passes all tests
- [ ] Migration applies successfully on clean PostgreSQL 16
- [ ] Chunks can be inserted and queried with vector similarity
- [ ] `just ci` passes cleanly
