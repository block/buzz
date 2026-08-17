-- Buzz Hive Flow Studio read-model tables (P3 projector target).
-- Source of truth remains Nostr events (kinds 46250–46399).

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS flow_knowledge_documents (
    community_id UUID NOT NULL,
    document_id TEXT NOT NULL,
    knowledge_base_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    token_count INTEGER NOT NULL DEFAULT 0,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, document_id)
);

CREATE INDEX IF NOT EXISTS flow_kb_docs_community_idx
    ON flow_knowledge_documents (community_id, knowledge_base_id);

CREATE TABLE IF NOT EXISTS flow_knowledge_embeddings (
    community_id UUID NOT NULL,
    embedding_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding vector(1536) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, embedding_id)
);

CREATE INDEX IF NOT EXISTS flow_kb_embeddings_document_idx
    ON flow_knowledge_embeddings (community_id, document_id);

CREATE TABLE IF NOT EXISTS flow_table_rows (
    community_id UUID NOT NULL,
    table_id TEXT NOT NULL,
    row_id TEXT NOT NULL,
    row_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, table_id, row_id)
);

CREATE INDEX IF NOT EXISTS flow_table_rows_table_idx
    ON flow_table_rows (community_id, table_id)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS flow_files (
    community_id UUID NOT NULL,
    file_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    media_url TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    deleted_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, file_id)
);

CREATE INDEX IF NOT EXISTS flow_files_community_idx
    ON flow_files (community_id)
    WHERE deleted_at IS NULL;
