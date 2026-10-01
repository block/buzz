#![deny(unsafe_code)]
//! Policy engine for multi-device transcript synchronization and policy-aware context filtering.

use buzz_core::memory::{Chunk, Document};
use serde::{Deserialize, Serialize};

/// Policy configuration for a session transcript and its derived chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPolicy {
    /// If true, raw transcript bytes stay strictly on the local workstation.
    pub local_only: bool,
    /// If true, only durable extracted decisions and compact summaries can be synced.
    pub sync_summaries_only: bool,
    /// Optional allowed agent identities for recalling this context.
    pub allowed_agents: Option<Vec<String>>,
    /// Workspace isolation boundary.
    pub workspace_boundary: Option<String>,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            local_only: true,
            sync_summaries_only: true,
            allowed_agents: None,
            workspace_boundary: None,
        }
    }
}

/// Context filtering query to evaluate if a chunk or document is policy-eligible.
#[derive(Debug, Clone, Default)]
pub struct PolicyContextQuery<'a> {
    /// Workspace path where the requesting agent/task is executing.
    pub workspace_path: &'a str,
    /// Name of the invoking agent requesting recall (e.g. "antigravity", "claude_code").
    pub requesting_agent: Option<&'a str>,
    /// Whether the execution environment is hosted/cloud or strictly local.
    pub is_hosted: bool,
}

/// Evaluates whether a chunk is policy-eligible for the requesting execution context.
pub fn is_chunk_policy_eligible(chunk: &Chunk, query: &PolicyContextQuery<'_>) -> bool {
    // 1. Workspace scope check
    if !chunk.workspace_path.is_empty()
        && !query.workspace_path.is_empty()
        && chunk.workspace_path != query.workspace_path
    {
        return false;
    }

    // 2. Local-only fence: if running in a hosted environment, local_only chunks must not be returned
    if query.is_hosted && chunk.scope == "agent_private" {
        return false;
    }

    true
}

/// Filters a list of retrieved chunks to return only policy-eligible items.
pub fn filter_policy_eligible_chunks(
    chunks: Vec<Chunk>,
    query: &PolicyContextQuery<'_>,
) -> Vec<Chunk> {
    chunks
        .into_iter()
        .filter(|c| is_chunk_policy_eligible(c, query))
        .collect()
}

/// Creates a compact summary of a session document suitable for sync.
pub fn create_syncable_summary(doc: &Document, decisions: &[String]) -> serde_json::Value {
    serde_json::json!({
        "document_id": doc.id,
        "source_uri": doc.source_uri,
        "workspace_path": doc.workspace_path,
        "created_at": doc.created_at,
        "content_hash": doc.content_hash,
        "decisions": decisions,
        "is_summary_only": true,
    })
}
