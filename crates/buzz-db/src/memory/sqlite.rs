#![deny(unsafe_code)]
//! SQLite metadata and event store implementation for Orbit memory.
//!
//! Manages `orbit_*` authoritative tables in `~/.orbit/brain/db/orbit.db`
//! or a custom file path with zero external database dependencies.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use buzz_core::memory::{
    Chunk, DecisionRecord, Document, Entity, IngestionJob, MemoryFeedback, MemoryItem,
    QueryCacheEntry, Relation, SessionRecord, SourceRecord, SyncLogEntry, WorkingContext,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;
use crate::error::{DbError, Result};
use crate::memory::traits::MetadataStore;

/// SQLite metadata store managing all `orbit_*` authoritative tables.
#[derive(Debug, Clone)]
pub struct SqliteMetadataStore {
    db_path: PathBuf,
    // ponytail: mutex lock over connection, connection pool if high concurrent write throughput is needed
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMetadataStore {
    /// Opens or creates the SQLite database at the specified path and runs initial migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db_path = path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DbError::Internal(format!("Failed to create database directory {}: {e}", parent.display())))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| DbError::Internal(format!("Failed to open SQLite database at {}: {e}", db_path.display())))?;

        // Enable WAL mode, normal synchronous, and foreign keys
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| DbError::Internal(format!("Failed to set PRAGMAs: {e}")))?;

        let store = Self {
            db_path,
            conn: Arc::new(Mutex::new(conn)),
        };

        store.init_schema()?;
        Ok(store)
    }

    /// Opens the default database at `~/.orbit/brain/db/orbit.db`.
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let default_path = PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("db")
            .join("orbit.db");
        Self::open(default_path)
    }

    /// Initializes all authoritative `orbit_*` tables and indexes.
    pub fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
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

            CREATE TABLE IF NOT EXISTS orbit_memory_items (
                id TEXT PRIMARY KEY,
                workspace_path TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                tags TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orbit_sources (
                id TEXT PRIMARY KEY,
                workspace_path TEXT NOT NULL,
                source_type TEXT NOT NULL,
                uri TEXT NOT NULL,
                status TEXT NOT NULL,
                last_synced_at TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orbit_sessions (
                id TEXT PRIMARY KEY,
                agent_name TEXT NOT NULL,
                workspace_path TEXT NOT NULL,
                summary TEXT NOT NULL,
                turns_count INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orbit_decisions (
                id TEXT PRIMARY KEY,
                workspace_path TEXT NOT NULL,
                title TEXT NOT NULL,
                state TEXT NOT NULL,
                rationale TEXT NOT NULL,
                superseded_by TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orbit_sync_log (
                id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orbit_ingestion_jobs (
                id TEXT PRIMARY KEY,
                workspace_path TEXT NOT NULL,
                status TEXT NOT NULL,
                total_files INTEGER NOT NULL DEFAULT 0,
                processed_files INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS orbit_memory_feedback (
                id TEXT PRIMARY KEY,
                workspace_path TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                user_rating INTEGER NOT NULL,
                feedback_text TEXT,
                created_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| DbError::Internal(format!("Failed to execute schema initialization: {e}")))?;

        Ok(())
    }

    /// Returns the filesystem path to the database file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Stores or updates an entity record.
    pub fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let meta_json = serde_json::to_string(&entity.metadata).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "INSERT INTO orbit_entities (id, workspace_path, name, entity_type, description, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(workspace_path, name, entity_type) DO UPDATE SET
                description = excluded.description,
                metadata = excluded.metadata,
                updated_at = excluded.updated_at;",
            params![
                entity.id.to_string(),
                entity.workspace_path,
                entity.name,
                entity.entity_type,
                entity.description,
                meta_json,
                entity.created_at.to_rfc3339(),
                entity.updated_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Fetches an entity by ID.
    pub fn get_entity(&self, id: &Uuid) -> Result<Option<Entity>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, workspace_path, name, entity_type, description, metadata, created_at, updated_at FROM orbit_entities WHERE id = ?1;")
            .map_err(|e| DbError::Internal(e.to_string()))?;
        let res = stmt.query_row(params![id.to_string()], |row| {
            let id_str: String = row.get(0)?;
            let meta_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            Ok(Entity {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                workspace_path: row.get(1)?,
                name: row.get(2)?,
                entity_type: row.get(3)?,
                description: row.get(4)?,
                metadata: serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null),
                created_at: DateTime::parse_from_rfc3339(&created_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            })
        }).optional().map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(res)
    }

    /// Stores or updates a bi-temporal relation record.
    pub fn upsert_relation(&self, relation: &Relation) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let meta_json = serde_json::to_string(&relation.metadata).unwrap_or_else(|_| "{}".to_string());
        let invalid_at_str = relation.invalid_at.map(|dt| dt.to_rfc3339());
        let chunk_id_str = relation.provenance_chunk_id.map(|id| id.to_string());
        conn.execute(
            "INSERT INTO orbit_relations (id, workspace_path, source_entity_id, target_entity_id, relation_type, confidence, valid_at, invalid_at, recorded_at, provenance_chunk_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                confidence = excluded.confidence,
                invalid_at = excluded.invalid_at,
                metadata = excluded.metadata;",
            params![
                relation.id.to_string(),
                relation.workspace_path,
                relation.source_entity_id.to_string(),
                relation.target_entity_id.to_string(),
                relation.relation_type,
                relation.confidence,
                relation.valid_at.to_rfc3339(),
                invalid_at_str,
                relation.recorded_at.to_rfc3339(),
                chunk_id_str,
                meta_json,
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Stores or updates an ingested source record.
    pub fn upsert_source(&self, source: &SourceRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let last_synced_str = source.last_synced_at.map(|d| d.to_rfc3339());
        conn.execute(
            "INSERT INTO orbit_sources (id, workspace_path, source_type, uri, status, last_synced_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                last_synced_at = excluded.last_synced_at;",
            params![
                source.id.to_string(),
                source.workspace_path,
                source.source_type,
                source.uri,
                source.status,
                last_synced_str,
                source.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Lists sources registered for a workspace.
    pub fn list_sources_in_workspace(&self, workspace_path: &str) -> Result<Vec<SourceRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, workspace_path, source_type, uri, status, last_synced_at, created_at FROM orbit_sources WHERE workspace_path = ?1;")
            .map_err(|e| DbError::Internal(e.to_string()))?;
        let rows = stmt.query_map(params![workspace_path], |row| {
            let id_str: String = row.get(0)?;
            let last_synced_opt: Option<String> = row.get(5)?;
            let created_str: String = row.get(6)?;
            Ok(SourceRecord {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                workspace_path: row.get(1)?,
                source_type: row.get(2)?,
                uri: row.get(3)?,
                status: row.get(4)?,
                last_synced_at: last_synced_opt.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                created_at: DateTime::parse_from_rfc3339(&created_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            })
        }).map_err(|e| DbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Internal(e.to_string()))?);
        }
        Ok(out)
    }

    /// Stores or updates an agent interactive session record.
    pub fn upsert_session(&self, session: &SessionRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orbit_sessions (id, agent_name, workspace_path, summary, turns_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                summary = excluded.summary,
                turns_count = excluded.turns_count;",
            params![
                session.id.to_string(),
                session.agent_name,
                session.workspace_path,
                session.summary,
                session.turns_count,
                session.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Lists sessions for a workspace.
    pub fn list_sessions_in_workspace(&self, workspace_path: &str) -> Result<Vec<SessionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, agent_name, workspace_path, summary, turns_count, created_at FROM orbit_sessions WHERE workspace_path = ?1;")
            .map_err(|e| DbError::Internal(e.to_string()))?;
        let rows = stmt.query_map(params![workspace_path], |row| {
            let id_str: String = row.get(0)?;
            let created_str: String = row.get(5)?;
            Ok(SessionRecord {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                agent_name: row.get(1)?,
                workspace_path: row.get(2)?,
                summary: row.get(3)?,
                turns_count: row.get(4)?,
                created_at: DateTime::parse_from_rfc3339(&created_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            })
        }).map_err(|e| DbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Internal(e.to_string()))?);
        }
        Ok(out)
    }

    /// Stores a query cache entry.
    pub fn set_query_cache(&self, cache: &QueryCacheEntry) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let cands_json = serde_json::to_string(&cache.candidate_chunk_ids).unwrap_or_else(|_| "[]".to_string());
        let reranked_json = serde_json::to_string(&cache.reranked_chunk_ids).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO orbit_query_cache (query_hash, workspace_path, intent_type, candidate_chunk_ids, reranked_chunk_ids, compiled_tokens, hit_count, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(query_hash) DO UPDATE SET
                candidate_chunk_ids = excluded.candidate_chunk_ids,
                reranked_chunk_ids = excluded.reranked_chunk_ids,
                compiled_tokens = excluded.compiled_tokens,
                hit_count = orbit_query_cache.hit_count + 1,
                expires_at = excluded.expires_at;",
            params![
                cache.query_hash,
                cache.workspace_path,
                cache.intent_type,
                cands_json,
                reranked_json,
                cache.compiled_tokens,
                cache.hit_count,
                cache.expires_at.to_rfc3339(),
                cache.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Fetches a query cache entry by hash.
    pub fn get_query_cache(&self, query_hash: &str) -> Result<Option<QueryCacheEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT query_hash, workspace_path, intent_type, candidate_chunk_ids, reranked_chunk_ids, compiled_tokens, hit_count, expires_at, created_at FROM orbit_query_cache WHERE query_hash = ?1;")
            .map_err(|e| DbError::Internal(e.to_string()))?;
        let res = stmt.query_row(params![query_hash], |row| {
            let cands_str: String = row.get(3)?;
            let reranked_str: String = row.get(4)?;
            let expires_str: String = row.get(7)?;
            let created_str: String = row.get(8)?;
            Ok(QueryCacheEntry {
                query_hash: row.get(0)?,
                workspace_path: row.get(1)?,
                intent_type: row.get(2)?,
                candidate_chunk_ids: serde_json::from_str(&cands_str).unwrap_or_default(),
                reranked_chunk_ids: serde_json::from_str(&reranked_str).unwrap_or_default(),
                compiled_tokens: row.get(5)?,
                hit_count: row.get(6)?,
                expires_at: DateTime::parse_from_rfc3339(&expires_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
                created_at: DateTime::parse_from_rfc3339(&created_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            })
        }).optional().map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(res)
    }

    /// Stores or updates an architectural decision record.
    pub fn upsert_decision(&self, decision: &DecisionRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orbit_decisions (id, workspace_path, title, state, rationale, superseded_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                rationale = excluded.rationale,
                superseded_by = excluded.superseded_by;",
            params![
                decision.id.to_string(),
                decision.workspace_path,
                decision.title,
                decision.state,
                decision.rationale,
                decision.superseded_by,
                decision.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Appends a mutation entry to the sync log.
    pub fn append_sync_log(&self, entry: &SyncLogEntry) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orbit_sync_log (id, event_type, payload, synced, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5);",
            params![
                entry.id.to_string(),
                entry.event_type,
                entry.payload,
                if entry.synced { 1 } else { 0 },
                entry.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Fetches uncommitted/pending sync log entries.
    pub fn get_pending_sync_logs(&self) -> Result<Vec<SyncLogEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, event_type, payload, synced, created_at FROM orbit_sync_log WHERE synced = 0 ORDER BY created_at ASC;")
            .map_err(|e| DbError::Internal(e.to_string()))?;
        let rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let synced_int: i32 = row.get(3)?;
            let created_str: String = row.get(4)?;
            Ok(SyncLogEntry {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                event_type: row.get(1)?,
                payload: row.get(2)?,
                synced: synced_int == 1,
                created_at: DateTime::parse_from_rfc3339(&created_str).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            })
        }).map_err(|e| DbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Internal(e.to_string()))?);
        }
        Ok(out)
    }

    /// Stores or updates an ingestion job.
    pub fn upsert_ingestion_job(&self, job: &IngestionJob) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let completed_str = job.completed_at.map(|d| d.to_rfc3339());
        conn.execute(
            "INSERT INTO orbit_ingestion_jobs (id, workspace_path, status, total_files, processed_files, created_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                total_files = excluded.total_files,
                processed_files = excluded.processed_files,
                completed_at = excluded.completed_at;",
            params![
                job.id.to_string(),
                job.workspace_path,
                job.status,
                job.total_files,
                job.processed_files,
                job.created_at.to_rfc3339(),
                completed_str,
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Stores an active working context.
    pub fn upsert_working_context(&self, ctx: &WorkingContext) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let mem_ids_json = serde_json::to_string(&ctx.active_memory_ids).unwrap_or_else(|_| "[]".to_string());
        let chunk_ids_json = serde_json::to_string(&ctx.active_chunk_ids).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO orbit_working_contexts (id, workspace_path, task_id, agent_name, token_budget, allocated_tokens, active_memory_ids, active_chunk_ids, context_xml, is_active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                allocated_tokens = excluded.allocated_tokens,
                active_memory_ids = excluded.active_memory_ids,
                active_chunk_ids = excluded.active_chunk_ids,
                context_xml = excluded.context_xml,
                is_active = excluded.is_active,
                updated_at = excluded.updated_at;",
            params![
                ctx.id.to_string(),
                ctx.workspace_path,
                ctx.task_id,
                ctx.agent_name,
                ctx.token_budget,
                ctx.allocated_tokens,
                mem_ids_json,
                chunk_ids_json,
                ctx.context_xml,
                if ctx.is_active { 1 } else { 0 },
                ctx.created_at.to_rfc3339(),
                ctx.updated_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Stores or updates a durable memory item.
    pub fn upsert_memory_item(&self, item: &MemoryItem) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tags_json = serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO orbit_memory_items (id, workspace_path, title, content, tags, scope, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                content = excluded.content,
                tags = excluded.tags,
                scope = excluded.scope;",
            params![
                item.id.to_string(),
                item.workspace_path,
                item.title,
                item.content,
                tags_json,
                item.scope,
                item.created_at.to_rfc3339(),
            ],
        ).map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(())
    }
}

impl MetadataStore for SqliteMetadataStore {
    async fn upsert_document(&self, doc: &Document) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let permissions_json = serde_json::to_string(&doc.permissions).unwrap_or_else(|_| "{}".to_string());
        let metadata_json = serde_json::to_string(&doc.metadata).unwrap_or_else(|_| "{}".to_string());

        conn.execute(
            "INSERT INTO orbit_documents (
                id, source_type, source_uri, workspace_path, content_hash,
                mtime_ns, size_bytes, permissions, metadata, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(workspace_path, content_hash) DO UPDATE SET
                source_type = excluded.source_type,
                source_uri = excluded.source_uri,
                mtime_ns = excluded.mtime_ns,
                size_bytes = excluded.size_bytes,
                permissions = excluded.permissions,
                metadata = excluded.metadata,
                updated_at = excluded.updated_at;",
            params![
                doc.id.to_string(),
                doc.source_type,
                doc.source_uri,
                doc.workspace_path,
                doc.content_hash,
                doc.mtime_ns,
                doc.size_bytes,
                permissions_json,
                metadata_json,
                doc.created_at.to_rfc3339(),
                doc.updated_at.to_rfc3339(),
            ],
        )
        .map_err(|e| DbError::Internal(format!("Failed to upsert document: {e}")))?;

        Ok(())
    }

    async fn get_document(&self, id: &Uuid) -> Result<Option<Document>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, source_type, source_uri, workspace_path, content_hash,
                        mtime_ns, size_bytes, permissions, metadata, created_at, updated_at
                 FROM orbit_documents WHERE id = ?1;",
            )
            .map_err(|e| DbError::Internal(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let permissions_str: String = row.get(7)?;
                let metadata_str: String = row.get(8)?;
                let created_str: String = row.get(9)?;
                let updated_str: String = row.get(10)?;

                Ok(Document {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    source_type: row.get(1)?,
                    source_uri: row.get(2)?,
                    workspace_path: row.get(3)?,
                    content_hash: row.get(4)?,
                    mtime_ns: row.get(5)?,
                    size_bytes: row.get(6)?,
                    permissions: serde_json::from_str(&permissions_str).unwrap_or(serde_json::Value::Null),
                    metadata: serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null),
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&updated_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .optional()
            .map_err(|e| DbError::Internal(e.to_string()))?;

        Ok(res)
    }

    async fn get_document_by_hash(&self, workspace_path: &str, content_hash: &str) -> Result<Option<Document>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, source_type, source_uri, workspace_path, content_hash,
                        mtime_ns, size_bytes, permissions, metadata, created_at, updated_at
                 FROM orbit_documents WHERE workspace_path = ?1 AND content_hash = ?2;",
            )
            .map_err(|e| DbError::Internal(e.to_string()))?;

        let res = stmt
            .query_row(params![workspace_path, content_hash], |row| {
                let id_str: String = row.get(0)?;
                let permissions_str: String = row.get(7)?;
                let metadata_str: String = row.get(8)?;
                let created_str: String = row.get(9)?;
                let updated_str: String = row.get(10)?;

                Ok(Document {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    source_type: row.get(1)?,
                    source_uri: row.get(2)?,
                    workspace_path: row.get(3)?,
                    content_hash: row.get(4)?,
                    mtime_ns: row.get(5)?,
                    size_bytes: row.get(6)?,
                    permissions: serde_json::from_str(&permissions_str).unwrap_or(serde_json::Value::Null),
                    metadata: serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null),
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&updated_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .optional()
            .map_err(|e| DbError::Internal(e.to_string()))?;

        Ok(res)
    }

    async fn delete_document(&self, id: &Uuid) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn
            .execute("DELETE FROM orbit_documents WHERE id = ?1;", params![id.to_string()])
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn upsert_chunk(&self, chunk: &Chunk) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orbit_chunks (
                id, document_id, workspace_path, chunk_index, content,
                token_count, scope, agent_name, line_start, line_end, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                token_count = excluded.token_count,
                scope = excluded.scope,
                agent_name = excluded.agent_name,
                line_start = excluded.line_start,
                line_end = excluded.line_end;",
            params![
                chunk.id.to_string(),
                chunk.document_id.to_string(),
                chunk.workspace_path,
                chunk.chunk_index,
                chunk.content,
                chunk.token_count,
                chunk.scope,
                chunk.agent_name,
                chunk.line_start,
                chunk.line_end,
                chunk.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| DbError::Internal(format!("Failed to upsert chunk: {e}")))?;

        Ok(())
    }

    async fn get_chunk(&self, id: &Uuid) -> Result<Option<Chunk>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, document_id, workspace_path, chunk_index, content,
                        token_count, scope, agent_name, line_start, line_end, created_at
                 FROM orbit_chunks WHERE id = ?1;",
            )
            .map_err(|e| DbError::Internal(e.to_string()))?;

        let res = stmt
            .query_row(params![id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let doc_id_str: String = row.get(1)?;
                let created_str: String = row.get(10)?;

                Ok(Chunk {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    document_id: Uuid::parse_str(&doc_id_str).unwrap_or_default(),
                    workspace_path: row.get(2)?,
                    chunk_index: row.get(3)?,
                    content: row.get(4)?,
                    token_count: row.get(5)?,
                    scope: row.get(6)?,
                    agent_name: row.get(7)?,
                    line_start: row.get(8)?,
                    line_end: row.get(9)?,
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .optional()
            .map_err(|e| DbError::Internal(e.to_string()))?;

        Ok(res)
    }

    async fn get_chunks_by_document(&self, document_id: &Uuid) -> Result<Vec<Chunk>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, document_id, workspace_path, chunk_index, content,
                        token_count, scope, agent_name, line_start, line_end, created_at
                 FROM orbit_chunks WHERE document_id = ?1 ORDER BY chunk_index ASC;",
            )
            .map_err(|e| DbError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map(params![document_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let doc_id_str: String = row.get(1)?;
                let created_str: String = row.get(10)?;

                Ok(Chunk {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    document_id: Uuid::parse_str(&doc_id_str).unwrap_or_default(),
                    workspace_path: row.get(2)?,
                    chunk_index: row.get(3)?,
                    content: row.get(4)?,
                    token_count: row.get(5)?,
                    scope: row.get(6)?,
                    agent_name: row.get(7)?,
                    line_start: row.get(8)?,
                    line_end: row.get(9)?,
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .map_err(|e| DbError::Internal(e.to_string()))?;

        let mut chunks = Vec::new();
        for chunk in rows {
            chunks.push(chunk.map_err(|e| DbError::Internal(e.to_string()))?);
        }

        Ok(chunks)
    }

    async fn delete_chunk(&self, id: &Uuid) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn
            .execute("DELETE FROM orbit_chunks WHERE id = ?1;", params![id.to_string()])
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(rows > 0)
    }

    async fn delete_chunks_by_document(&self, document_id: &Uuid) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = conn
            .execute("DELETE FROM orbit_chunks WHERE document_id = ?1;", params![document_id.to_string()])
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(rows)
    }

    async fn record_feedback(&self, feedback: &MemoryFeedback) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orbit_memory_feedback (
                id, workspace_path, memory_id, user_rating, feedback_text, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6);",
            params![
                feedback.id.to_string(),
                feedback.workspace_path,
                feedback.memory_id.to_string(),
                feedback.user_rating,
                feedback.feedback_text,
                feedback.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| DbError::Internal(format!("Failed to record memory feedback: {e}")))?;

        Ok(())
    }

    async fn count_documents(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orbit_documents;", [], |row| row.get(0))
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(count as usize)
    }

    async fn count_chunks(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orbit_chunks;", [], |row| row.get(0))
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sqlite_store_crud_and_reopen() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_test_{}", Uuid::new_v4()));
        let db_file = temp_dir.join("orbit.db");

        // 1. Initialize and insert
        {
            let store = SqliteMetadataStore::open(&db_file).expect("open store");
            let doc = Document::new(
                "file",
                "/test/file.rs",
                "/workspace",
                b"fn hello() {}",
                123456,
                serde_json::json!({}),
            );
            store.upsert_document(&doc).await.expect("upsert doc");

            let chunk = Chunk::new(doc.id, &doc.workspace_path, 0, "fn hello() {}", "project");
            store.upsert_chunk(&chunk).await.expect("upsert chunk");

            let fetched_doc = store.get_document(&doc.id).await.expect("get doc");
            assert!(fetched_doc.is_some());
            assert_eq!(fetched_doc.unwrap().content_hash, doc.content_hash);

            let fetched_chunk = store.get_chunk(&chunk.id).await.expect("get chunk");
            assert!(fetched_chunk.is_some());
            assert_eq!(fetched_chunk.unwrap().content, "fn hello() {}");
        }

        // 2. Reopen and verify persistence (crash/restart recovery)
        {
            let store = SqliteMetadataStore::open(&db_file).expect("reopen store");
            assert_eq!(store.count_documents().await.unwrap(), 1);
            assert_eq!(store.count_chunks().await.unwrap(), 1);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
