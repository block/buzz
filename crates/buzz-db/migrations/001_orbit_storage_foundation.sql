-- 001_orbit_storage_foundation.sql
-- Authoritative schema for Orbit Brain local-first embedded SQLite store (~/.orbit/brain/db/orbit.db)

-- Layer 1: Source Layer (Raw immutable files, chats, git commits, notes, ACLs)
CREATE TABLE IF NOT EXISTS orbit_documents (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_uri TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mtime_ns INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    permissions TEXT NOT NULL,
    metadata TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(workspace_path, content_hash)
);
CREATE INDEX IF NOT EXISTS idx_orbit_documents_workspace ON orbit_documents(workspace_path);
CREATE INDEX IF NOT EXISTS idx_orbit_documents_uri ON orbit_documents(source_uri);

-- Layer 2: Processed Context Layer (Normalized AST chunks with line provenance)
CREATE TABLE IF NOT EXISTS orbit_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES orbit_documents(id) ON DELETE CASCADE,
    workspace_path TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    scope TEXT NOT NULL,
    agent_name TEXT,
    line_start INTEGER,
    line_end INTEGER,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_orbit_chunks_document ON orbit_chunks(document_id);
CREATE INDEX IF NOT EXISTS idx_orbit_chunks_workspace ON orbit_chunks(workspace_path);

-- Layer 3: Memory Layer — Semantic Knowledge Graph Nodes
CREATE TABLE IF NOT EXISTS orbit_entities (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    name TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    description TEXT NOT NULL,
    metadata TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(workspace_path, name, entity_type)
);
CREATE INDEX IF NOT EXISTS idx_orbit_entities_workspace ON orbit_entities(workspace_path);

-- Layer 3: Memory Layer — Bi-temporal Relationship Edges
CREATE TABLE IF NOT EXISTS orbit_relations (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    source_entity_id TEXT NOT NULL REFERENCES orbit_entities(id) ON DELETE CASCADE,
    target_entity_id TEXT NOT NULL REFERENCES orbit_entities(id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL,
    confidence REAL NOT NULL,
    valid_at TEXT NOT NULL,
    invalid_at TEXT,
    recorded_at TEXT NOT NULL,
    provenance_chunk_id TEXT REFERENCES orbit_chunks(id) ON DELETE SET NULL,
    metadata TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_orbit_relations_source ON orbit_relations(source_entity_id);
CREATE INDEX IF NOT EXISTS idx_orbit_relations_target ON orbit_relations(target_entity_id);

-- Layer 4: Retrieval Layer — Query Routing & Semantic Cache
CREATE TABLE IF NOT EXISTS orbit_query_cache (
    query_hash TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    intent_type TEXT NOT NULL,
    candidate_chunk_ids TEXT NOT NULL,
    reranked_chunk_ids TEXT NOT NULL,
    compiled_tokens INTEGER NOT NULL,
    hit_count INTEGER NOT NULL DEFAULT 0,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_orbit_query_cache_workspace ON orbit_query_cache(workspace_path);

-- Layer 5: Working Context Layer (Task-Specific Active Working Context)
CREATE TABLE IF NOT EXISTS orbit_working_contexts (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    task_id TEXT NOT NULL,
    agent_name TEXT NOT NULL,
    token_budget INTEGER NOT NULL,
    allocated_tokens INTEGER NOT NULL,
    active_memory_ids TEXT NOT NULL,
    active_chunk_ids TEXT NOT NULL,
    context_xml TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_orbit_working_contexts_task ON orbit_working_contexts(workspace_path, task_id);

-- Durable Memory Items (ADRs, Architectural Guidelines, Facts)
CREATE TABLE IF NOT EXISTS orbit_memory_items (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL,
    scope TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Ingested Sources Metadata
CREATE TABLE IF NOT EXISTS orbit_sources (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    source_type TEXT NOT NULL,
    uri TEXT NOT NULL,
    status TEXT NOT NULL,
    last_synced_at TEXT,
    created_at TEXT NOT NULL
);

-- Historical Agent Sessions
CREATE TABLE IF NOT EXISTS orbit_sessions (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    summary TEXT NOT NULL,
    turns_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

-- Architectural Decisions
CREATE TABLE IF NOT EXISTS orbit_decisions (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    rationale TEXT NOT NULL,
    superseded_by TEXT,
    created_at TEXT NOT NULL
);

-- Cloud & Team Event Sync Log
CREATE TABLE IF NOT EXISTS orbit_sync_log (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

-- Active Background Ingestion Jobs
CREATE TABLE IF NOT EXISTS orbit_ingestion_jobs (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    status TEXT NOT NULL,
    total_files INTEGER NOT NULL DEFAULT 0,
    processed_files INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    completed_at TEXT
);

-- Memory Feedback & Reinforcement Scoring
CREATE TABLE IF NOT EXISTS orbit_memory_feedback (
    id TEXT PRIMARY KEY,
    workspace_path TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    user_rating INTEGER NOT NULL,
    feedback_text TEXT,
    created_at TEXT NOT NULL
);
