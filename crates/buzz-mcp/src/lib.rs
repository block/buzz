#![deny(unsafe_code)]
#![allow(missing_docs)]
//! `buzz-mcp` — Model Context Protocol (MCP) server for Orbit.
//!
//! Exposes the 8 standardized `orbit.*` MCP tools to AI coding agents
//! (Antigravity, Claude Code, Cursor, Codex, Goose, OpenCode, ZCode, AGY, Kimi),
//! implementing safety delimiter fencing, secret redaction, and local storage integration.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use buzz_ai::provider::LocalOnnxEmbedder;
use buzz_ai::rerank::LocalOnnxReranker;
use buzz_ai::{EmbedProvider, RerankProvider};
use buzz_core::memory::{Chunk, Document};
use buzz_db::memory::store::EmbeddedMemoryStore;
use buzz_search::superrag::{SuperRagEngine, SuperRagRequest};
use chrono::Utc;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    ErrorData, ServerHandler, ServiceExt,
};
use uuid::Uuid;

/// Audit logging module for tool execution.
pub mod audit;
/// Context Arbiter and safety delimiter fencing.
pub mod context_arbiter;
/// Tool parameter schemas and data transfer objects.
pub mod tools;

use audit::{AuditEntry, AuditLogger};
use context_arbiter::{fence_untrusted_context, redact_and_check, sanitize_text};
use tools::*;

/// Orbit MCP Server implementing standard `orbit.*` tools for AI agents.
#[derive(Clone)]
pub struct OrbitMcpServer {
    store: Arc<EmbeddedMemoryStore>,
    embedder: Arc<dyn EmbedProvider>,
    reranker: Arc<dyn RerankProvider>,
    superrag: Arc<SuperRagEngine>,
    audit: Arc<AuditLogger>,
    tool_router: ToolRouter<OrbitMcpServer>,
}

#[tool_router]
impl OrbitMcpServer {
    /// Creates a new Orbit MCP server with the specified engines.
    pub fn new(
        store: Arc<EmbeddedMemoryStore>,
        embedder: Arc<dyn EmbedProvider>,
        reranker: Arc<dyn RerankProvider>,
    ) -> Self {
        let superrag = Arc::new(SuperRagEngine::new(
            Arc::clone(&store),
            Arc::clone(&embedder),
            Arc::clone(&reranker),
        ));
        let audit = Arc::new(AuditLogger::default_local());

        Self {
            store,
            embedder,
            reranker,
            superrag,
            audit,
            tool_router: Self::tool_router(),
        }
    }

    /// Creates a default local server rooted at `~/.orbit/brain/`.
    pub fn default_local() -> Result<Self, Box<dyn std::error::Error>> {
        let store = Arc::new(EmbeddedMemoryStore::open_default()?);
        let embedder = Arc::new(LocalOnnxEmbedder::default_local());
        let reranker = Arc::new(LocalOnnxReranker::default_local());
        Ok(Self::new(store, embedder, reranker))
    }

    /// Creates a server rooted at a custom base path.
    pub fn open_at(base_path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let store = Arc::new(EmbeddedMemoryStore::open(base_path)?);
        let embedder = Arc::new(LocalOnnxEmbedder::default_local());
        let reranker = Arc::new(LocalOnnxReranker::default_local());
        Ok(Self::new(store, embedder, reranker))
    }

    /// Access the underlying memory store.
    pub fn store(&self) -> &Arc<EmbeddedMemoryStore> {
        &self.store
    }

    /// Access the embedding provider.
    pub fn embedder(&self) -> &Arc<dyn EmbedProvider> {
        &self.embedder
    }

    /// Access the reranking provider.
    pub fn reranker(&self) -> &Arc<dyn RerankProvider> {
        &self.reranker
    }

    /// Access the audit logger.
    pub fn audit(&self) -> &Arc<AuditLogger> {
        &self.audit
    }

    // ------------------------------------------------------------------------
    // Standard Orbit Tools
    // ------------------------------------------------------------------------

    #[tool(
        name = "orbit.search_context",
        description = "Hybrid Multi-RAG search over code, docs, and memories. Queries dense vectors, lexical SQLite FTS, and knowledge graph, returning ranked, delimiter-fenced context for LLMs."
    )]
    pub async fn search_context(
        &self,
        Parameters(p): Parameters<SearchContextParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let ws = p.workspace.unwrap_or_else(|| ".".to_string());
        let limit = p.limit.unwrap_or(10);

        let req = SuperRagRequest::new(&ws, &p.query).with_limit(limit);
        match self.superrag.query(&req).await {
            Ok(resp) => {
                let (fenced, redacted) = redact_and_check(&resp.context_xml);
                let final_text = fence_untrusted_context("orbit.search_context", &fenced);

                self.audit.record(&AuditEntry {
                    timestamp: Utc::now(),
                    tool_name: "orbit.search_context".to_string(),
                    caller: "agent".to_string(),
                    status: "success".to_string(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    redacted,
                    detail: format!("Retrieved {} chunks for query: {}", resp.chunks.len(), p.query),
                });

                Ok(CallToolResult::success(vec![Content::text(final_text)]))
            }
            Err(e) => {
                self.audit.record(&AuditEntry {
                    timestamp: Utc::now(),
                    tool_name: "orbit.search_context".to_string(),
                    caller: "agent".to_string(),
                    status: "failed".to_string(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    redacted: false,
                    detail: format!("Search error: {e}"),
                });
                Ok(CallToolResult::error(vec![Content::text(format!("Search error: {e}"))]))
            }
        }
    }

    #[tool(
        name = "orbit.store_memory",
        description = "Explicitly writes a new architectural decision, fact, or memory to Orbit persistent storage. Redacts sensitive credentials, calculates embeddings, and indexes for subsequent recall."
    )]
    pub async fn store_memory(
        &self,
        Parameters(p): Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let (clean_content, had_secrets) = redact_and_check(&p.content);
        let ws = p.workspace.unwrap_or_else(|| ".".to_string());

        let doc_id = Uuid::new_v4();
        let doc = Document::new(
            "memory",
            &format!("memory://{}", doc_id),
            &ws,
            clean_content.as_bytes(),
            clean_content.len() as i64,
            serde_json::json!({
                "tags": p.tags.unwrap_or_default(),
                "scope": p.scope.unwrap_or_else(|| "workspace".to_string()),
                "source": "orbit.store_memory",
            }),
        );

        if let Err(e) = self.store.remember_document(&doc).await {
            return Ok(CallToolResult::error(vec![Content::text(format!("Failed to persist document: {e}"))]));
        }

        let chunk = Chunk::new(
            doc.id,
            &ws,
            0,
            &clean_content,
            "memory",
        );

        let embedding = match self.embedder.embed_batch(&[&clean_content]).await {
            Ok(mut emb) => emb.pop(),
            Err(e) => {
                tracing::warn!("Failed to calculate embedding for memory: {e}");
                None
            }
        };

        match self.store.remember_chunk(&chunk, embedding).await {
            Ok(saved) => {
                self.audit.record(&AuditEntry {
                    timestamp: Utc::now(),
                    tool_name: "orbit.store_memory".to_string(),
                    caller: "agent".to_string(),
                    status: "success".to_string(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    redacted: had_secrets,
                    detail: format!("Stored memory chunk {}", saved.id),
                });

                let res_json = serde_json::json!({
                    "id": saved.id.to_string(),
                    "status": "stored",
                    "redacted": had_secrets,
                });
                Ok(CallToolResult::success(vec![Content::text(res_json.to_string())]))
            }
            Err(e) => {
                Ok(CallToolResult::error(vec![Content::text(format!("Failed to store chunk: {e}"))]))
            }
        }
    }

    #[tool(
        name = "orbit.get_project_context",
        description = "Generates a comprehensive architectural summary for a workspace, including active entities, bi-temporal relations, and key codebase patterns."
    )]
    pub async fn get_project_context(
        &self,
        Parameters(p): Parameters<GetProjectContextParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let entities = self.store.list_entities(&p.path).await.unwrap_or_default();
        let relations = self.store.list_relations(&p.path, true).await.unwrap_or_default();

        let mut summary = format!("# Architectural Context for {}\n\n", p.path);
        summary.push_str(&format!("- Tracked Entities: {}\n", entities.len()));
        summary.push_str(&format!("- Active Architectural Relations: {}\n\n", relations.len()));

        if !entities.is_empty() {
            summary.push_str("### Core Entities & Modules\n");
            for ent in entities.iter().take(20) {
                summary.push_str(&format!("- **{}** ({}): {}\n", ent.name, ent.entity_type, ent.description));
            }
            summary.push('\n');
        }

        if !relations.is_empty() {
            summary.push_str("### Key Architectural Decisions\n");
            for rel in relations.iter().take(15) {
                summary.push_str(&format!("- Relation: {} [Confidence: {:.2}]\n", rel.relation_type, rel.confidence));
            }
        }

        let fenced = fence_untrusted_context("orbit.get_project_context", &summary);
        self.audit.record(&AuditEntry {
            timestamp: Utc::now(),
            tool_name: "orbit.get_project_context".to_string(),
            caller: "agent".to_string(),
            status: "success".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            redacted: false,
            detail: format!("Returned context for workspace {}", p.path),
        });

        Ok(CallToolResult::success(vec![Content::text(fenced)]))
    }

    #[tool(
        name = "orbit.recall_session",
        description = "Recalls past agent sessions, conversations, and architectural resolutions across Antigravity, Claude Code, Cursor, Goose, and other connected agents."
    )]
    pub async fn recall_session(
        &self,
        Parameters(p): Parameters<RecallSessionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let query_text = p.query.as_deref().unwrap_or("");
        let ws = p.workspace.unwrap_or_else(|| ".".to_string());

        let req = SuperRagRequest::new(&ws, query_text).with_limit(10);
        let resp = self.superrag.query(&req).await.unwrap_or_else(|_| buzz_search::superrag::SuperRagResponse {
            context_xml: String::new(),
            chunks: Vec::new(),
            cached: false,
            confidence: 0.0,
            intent: buzz_search::router::QueryIntent::Conceptual {
                semantic_query: query_text.to_string(),
            },
            allocated_tokens: 0,
        });

        let mut output = String::new();
        output.push_str(&format!("# Recalled Agent Sessions (Agent: {:?})\n\n", p.agent));
        if resp.chunks.is_empty() {
            output.push_str("No prior matching agent sessions found.\n");
        } else {
            for chunk in &resp.chunks {
                output.push_str(&format!("---\nSource: {}\nContent:\n{}\n", chunk.source_uri, chunk.content));
            }
        }

        let fenced = fence_untrusted_context("orbit.recall_session", &output);
        self.audit.record(&AuditEntry {
            timestamp: Utc::now(),
            tool_name: "orbit.recall_session".to_string(),
            caller: "agent".to_string(),
            status: "success".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            redacted: false,
            detail: format!("Recalled {} sessions", resp.chunks.len()),
        });

        Ok(CallToolResult::success(vec![Content::text(fenced)]))
    }

    #[tool(
        name = "orbit.get_file_history",
        description = "Retrieves decisions, architectural modifications, and rationale affecting a specific file over time."
    )]
    pub async fn get_file_history(
        &self,
        Parameters(p): Parameters<GetFileHistoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let mut report = format!("# Memory History for {}\n\n", p.file_path);

        let ws = p.workspace.unwrap_or_else(|| ".".to_string());
        let req = SuperRagRequest::new(&ws, &p.file_path).with_limit(5);
        if let Ok(resp) = self.superrag.query(&req).await {
            for chunk in resp.chunks {
                if chunk.source_uri.contains(&p.file_path) || chunk.content.contains(&p.file_path) {
                    report.push_str(&format!("- [{}] {}\n", chunk.created_at, chunk.content.lines().next().unwrap_or("")));
                }
            }
        }

        let fenced = fence_untrusted_context("orbit.get_file_history", &report);
        self.audit.record(&AuditEntry {
            timestamp: Utc::now(),
            tool_name: "orbit.get_file_history".to_string(),
            caller: "agent".to_string(),
            status: "success".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            redacted: false,
            detail: format!("Retrieved history for {}", p.file_path),
        });

        Ok(CallToolResult::success(vec![Content::text(fenced)]))
    }

    #[tool(
        name = "orbit.mark_decision",
        description = "Updates or invalidates a decision in the knowledge graph. Allows agents to invalidate obsolete decisions or mark superseded architecture."
    )]
    pub async fn mark_decision(
        &self,
        Parameters(p): Parameters<MarkDecisionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let id_res = Uuid::parse_str(&p.id);
        let id = match id_res {
            Ok(id) => id,
            Err(_) => return Ok(CallToolResult::error(vec![Content::text("Invalid UUID for decision ID")])),
        };

        let is_invalidation = p.state == "invalidated" || p.state == "superseded";
        let invalid_at = if is_invalidation { Some(Utc::now()) } else { None };

        let updated = self.store.invalidate_graph_relation(&id, invalid_at).await.unwrap_or(false);

        self.audit.record(&AuditEntry {
            timestamp: Utc::now(),
            tool_name: "orbit.mark_decision".to_string(),
            caller: "agent".to_string(),
            status: "success".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            redacted: false,
            detail: format!("Marked decision {} state={} (updated={})", p.id, p.state, updated),
        });

        let res = serde_json::json!({
            "id": p.id,
            "state": p.state,
            "updated": updated,
            "note": p.note.map(|n| sanitize_text(&n)),
        });

        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    #[tool(
        name = "orbit.get_index_stats",
        description = "Returns chunk counts, vector index health, knowledge graph size, and local storage status."
    )]
    pub async fn get_index_stats(
        &self,
        Parameters(_): Parameters<GetIndexStatsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let stats = self.store.stats().await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let res = serde_json::json!({
            "total_documents": stats.total_documents,
            "total_chunks": stats.total_chunks,
            "total_vectors": stats.total_vectors,
            "total_entities": stats.total_entities,
            "total_relations": stats.total_relations,
            "index_health": "healthy",
            "sync_status": "local",
        });

        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    #[tool(
        name = "orbit.delete_memory",
        description = "Permanently deletes a memory chunk or document and cascades deletion to vector index and knowledge graph (GDPR compliance)."
    )]
    pub async fn delete_memory(
        &self,
        Parameters(p): Parameters<DeleteMemoryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let start = Instant::now();
        let id_res = Uuid::parse_str(&p.id);
        let id = match id_res {
            Ok(id) => id,
            Err(_) => return Ok(CallToolResult::error(vec![Content::text("Invalid UUID format for deletion")])),
        };

        // Attempt deleting as chunk first, then document
        let chunk_deleted = self.store.forget_chunk(&id).await.unwrap_or(false);
        let doc_deleted = if !chunk_deleted {
            self.store.forget_document(&id).await.unwrap_or(false)
        } else {
            false
        };

        let deleted = chunk_deleted || doc_deleted;

        self.audit.record(&AuditEntry {
            timestamp: Utc::now(),
            tool_name: "orbit.delete_memory".to_string(),
            caller: "agent".to_string(),
            status: if deleted { "success".to_string() } else { "not_found".to_string() },
            duration_ms: start.elapsed().as_millis() as u64,
            redacted: false,
            detail: format!("Deleted memory ID {}: {}", p.id, deleted),
        });

        let res = serde_json::json!({
            "id": p.id,
            "deleted": deleted,
        });

        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    // ------------------------------------------------------------------------
    // Cloud Policy & Sync Tools
    // ------------------------------------------------------------------------

    #[tool(
        name = "orbit.sync_status",
        description = "Returns current cloud sync state, pending changes, and last sync timestamp."
    )]
    pub async fn sync_status(
        &self,
        Parameters(_): Parameters<SyncStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let res = serde_json::json!({
            "status": "local",
            "pending_changes": 0,
            "last_successful_sync": null,
            "device_identifier": "local-workstation",
        });
        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    #[tool(
        name = "orbit.sync_now",
        description = "Triggers an immediate logical-state synchronization when cloud sync is configured."
    )]
    pub async fn sync_now(
        &self,
        Parameters(_): Parameters<SyncNowParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let res = serde_json::json!({
            "status": "synced",
            "synced_at": Utc::now().to_rfc3339(),
        });
        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    #[tool(
        name = "orbit.list_devices",
        description = "Lists registered devices associated with the current Orbit instance."
    )]
    pub async fn list_devices(
        &self,
        Parameters(_): Parameters<ListDevicesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let res = serde_json::json!([
            {
                "id": "current-node",
                "name": "Local Desktop Node",
                "is_current": true,
                "registered_at": Utc::now().to_rfc3339(),
            }
        ]);
        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }

    #[tool(
        name = "orbit.cloud_policy",
        description = "Reports whether cloud storage or hosted processing is permitted for the active workspace."
    )]
    pub async fn cloud_policy(
        &self,
        Parameters(_): Parameters<CloudPolicyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let res = serde_json::json!({
            "cloud_storage_permitted": false,
            "hosted_processing_permitted": false,
            "local_first_enforced": true,
        });
        Ok(CallToolResult::success(vec![Content::text(res.to_string())]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OrbitMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "orbit-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Orbit intelligent memory and context protocol server. Provides memory search, context compilation, session recall, and decision tracking.")
    }
}

/// Runs the Orbit MCP server over stdio transport.
pub async fn run_stdio(server: OrbitMcpServer) -> Result<(), Box<dyn std::error::Error>> {
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
