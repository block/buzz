# Feature 01 — Storage Foundation (pgvector + buzz-db Extension)

> **Priority**: P0 — Foundation layer. All other Orbit features depend on this.  
> **Sprint**: Sprint 1 (Weeks 1–2)  
> **Dependencies**: None  
> **Crates**: `buzz-db` (modify), `migrations/` (new SQL migration)  
> **Environment Variables**: `BUZZ_DATABASE_URL`, `BUZZ_DATA_DIR`

---

## Overview

Feature 01 establishes the foundational storage layer for the Orbit memory system inside the existing `buzz-db` crate. It activates the PostgreSQL `pgvector` extension and introduces four core tables following Cognee and Graphiti data models.

All data is housed under the structured `orbit_brain/` directory:
- `orbit_brain/storage/` — PostgreSQL data directory and pgvector indexes
- `orbit_brain/sync/` — Staging changelog for future cloud synchronization

---

## Architecture & Schemas

### 1. Database Migration: `migrations/0047_buzz_memory_pgvector.sql`

```sql
-- 1. Enable pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

-- 2. Raw Source Documents Table (Cognee Tier 1)
CREATE TABLE IF NOT EXISTS buzz_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_type VARCHAR(64) NOT NULL,            -- 'file', 'git', 'session', 'slack', 'manual'
    source_uri TEXT NOT NULL,                     -- Absolute path or URL
    workspace_path TEXT NOT NULL,                 -- Canonical root path of the workspace
    content_hash CHAR(64) NOT NULL,               -- SHA-256 for change detection & deduplication
    mtime_ns BIGINT NOT NULL,                     -- Filesystem modification timestamp in nanoseconds
    size_bytes BIGINT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,  -- Author, commit SHA, PR number, branch, etc.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_buzz_doc_workspace_hash UNIQUE (workspace_path, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_buzz_documents_workspace ON buzz_documents(workspace_path);
CREATE INDEX IF NOT EXISTS idx_buzz_documents_source_uri ON buzz_documents(source_uri);

-- 3. Semantic Chunks Table (Cognee Tier 2 + Hybrid Search)
CREATE TABLE IF NOT EXISTS buzz_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES buzz_documents(id) ON DELETE CASCADE,
    workspace_path TEXT NOT NULL,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,
    token_count INT NOT NULL,
    -- Default 384 dimensions for local BGE-small-en-v1.5
    embedding VECTOR(384),
    -- Full-Text Search column for BM25 lexical ranking
    search_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED,
    scope VARCHAR(64) NOT NULL DEFAULT 'project', -- 'project', 'global', 'agent_private'
    agent_name VARCHAR(64),                       -- 'antigravity', 'claude', 'cursor', etc.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- HNSW Vector Index (Cosine Distance) for sub-millisecond similarity search
CREATE INDEX IF NOT EXISTS idx_buzz_chunks_embedding_hnsw 
ON buzz_chunks USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- GIN Full-Text Index for BM25 lexical search
CREATE INDEX IF NOT EXISTS idx_buzz_chunks_search_tsv 
ON buzz_chunks USING gin (search_tsv);

CREATE INDEX IF NOT EXISTS idx_buzz_chunks_workspace ON buzz_chunks(workspace_path);
CREATE INDEX IF NOT EXISTS idx_buzz_chunks_agent ON buzz_chunks(agent_name);

-- 4. Knowledge Graph Entities Table (Graphiti Tier 3)
CREATE TABLE IF NOT EXISTS buzz_entities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_path TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    entity_type VARCHAR(64) NOT NULL,             -- 'Technology', 'Person', 'File', 'Concept', 'Architecture'
    description TEXT,
    summary_embedding VECTOR(384),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_buzz_entity_workspace_name_type UNIQUE (workspace_path, name, entity_type)
);

CREATE INDEX IF NOT EXISTS idx_buzz_entities_workspace ON buzz_entities(workspace_path);

-- 5. Bi-Temporal Relations Table (Graphiti Edge Model)
CREATE TABLE IF NOT EXISTS buzz_relations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_path TEXT NOT NULL,
    source_entity_id UUID NOT NULL REFERENCES buzz_entities(id) ON DELETE CASCADE,
    target_entity_id UUID NOT NULL REFERENCES buzz_entities(id) ON DELETE CASCADE,
    relation_type VARCHAR(64) NOT NULL,           -- 'implements', 'depends_on', 'decided_by', 'authored_by'
    confidence REAL NOT NULL DEFAULT 1.0,         -- 0.0 to 1.0 confidence score
    -- Bi-temporal validity timestamps
    valid_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),  -- When this became true in real world
    invalid_at TIMESTAMPTZ,                       -- NULL = actively true, timestamp = superseded
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),-- When system learned this fact
    provenance_chunk_id UUID REFERENCES buzz_chunks(id) ON DELETE SET NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_buzz_relations_source ON buzz_relations(source_entity_id);
CREATE INDEX IF NOT EXISTS idx_buzz_relations_target ON buzz_relations(target_entity_id);
CREATE INDEX IF NOT EXISTS idx_buzz_relations_active ON buzz_relations(workspace_path, relation_type) 
WHERE invalid_at IS NULL;
```

---

## Crate Implementation: `crates/buzz-db`

### 1. New Data Models in `crates/buzz-db/src/memory_models.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use pgvector::Vector;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BuzzDocument {
    pub id: Uuid,
    pub source_type: String,
    pub source_uri: String,
    pub workspace_path: String,
    pub content_hash: String,
    pub mtime_ns: i64,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BuzzChunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub workspace_path: String,
    pub chunk_index: i32,
    pub content: String,
    pub token_count: i32,
    pub embedding: Option<Vector>,
    pub scope: String,
    pub agent_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BuzzEntity {
    pub id: Uuid,
    pub workspace_path: String,
    pub name: String,
    pub entity_type: String,
    pub description: Option<String>,
    pub summary_embedding: Option<Vector>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BuzzRelation {
    pub id: Uuid,
    pub workspace_path: String,
    pub source_entity_id: Uuid,
    pub target_entity_id: Uuid,
    pub relation_type: String,
    pub confidence: f32,
    pub valid_at: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
    pub provenance_chunk_id: Option<Uuid>,
    pub metadata: serde_json::Value,
}
```

### 2. Secret Redactor: `crates/buzz-db/src/redactor.rs`

Protects against storing sensitive credentials:
- AWS access keys (`AKIA...`)
- GitHub Personal Access Tokens (`ghp_...`)
- Bearer tokens & JWTs
- RSA / OpenSSH private keys (`BEGIN PRIVATE KEY`)
- Generic API keys (`sk-...`, `Bearer ...`)

### 3. Data Storage & Cloud-Sync Staging Directory Layout

```
~/.buzz/orbit_brain/
├── storage/              ← PostgreSQL cluster directory
├── sync/
│   ├── changelog.wal     ← Append-only write-ahead log for cloud sync
│   └── snapshot_meta.json← Sync watermarks & replication state
```

---

## Verification & Quality Gates

1. Run `cargo test -p buzz-db` to verify SQLx queries and redactor tests.
2. Run `just ci` to ensure formatting, Clippy, and pre-push hooks pass.
