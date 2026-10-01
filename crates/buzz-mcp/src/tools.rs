#![deny(unsafe_code)]
//! Parameter definitions and response schemas for Orbit MCP tools.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Parameters for `orbit.search_context`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchContextParams {
    /// Natural language or symbolic query text.
    pub query: String,
    /// Maximum candidate chunks to retrieve and rank (default: 10).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Optional workspace path boundary. Defaults to current workspace.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Parameters for `orbit.store_memory`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct StoreMemoryParams {
    /// Memory content, architectural decision, or fact to store.
    pub content: String,
    /// Semantic tags for categorization (e.g. ["auth", "decision", "api"]).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Storage scope: "workspace" (default) or "global".
    #[serde(default)]
    pub scope: Option<String>,
    /// Optional workspace path. Defaults to active workspace.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Parameters for `orbit.get_project_context`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetProjectContextParams {
    /// Workspace root path or subfolder path.
    pub path: String,
    /// Maximum tokens for the generated architectural summary (default: 4000).
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

/// Parameters for `orbit.recall_session`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct RecallSessionParams {
    /// Agent identifier filter (e.g. "antigravity", "cursor", "claude_code", "goose").
    #[serde(default)]
    pub agent: Option<String>,
    /// Search query across past conversation turns and decisions.
    #[serde(default)]
    pub query: Option<String>,
    /// Lookback window in days (default: 30).
    #[serde(default)]
    pub days: Option<u32>,
    /// Optional workspace path filter.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Parameters for `orbit.get_file_history`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetFileHistoryParams {
    /// Relative or absolute path to the file.
    pub file_path: String,
    /// Optional workspace root.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// Parameters for `orbit.mark_decision`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MarkDecisionParams {
    /// UUID identifier of the decision, chunk, or entity to update.
    pub id: String,
    /// New decision status ("active", "superseded", "invalidated", "deprecated").
    pub state: String,
    /// Optional explanatory note or rationale for the state change.
    #[serde(default)]
    pub note: Option<String>,
}

/// Parameters for `orbit.get_index_stats`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct GetIndexStatsParams {}

/// Parameters for `orbit.delete_memory`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DeleteMemoryParams {
    /// UUID of the chunk or document to permanently delete (GDPR compliance).
    pub id: String,
}

/// Parameters for `orbit.sync_status`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct SyncStatusParams {}

/// Parameters for `orbit.sync_now`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct SyncNowParams {}

/// Parameters for `orbit.list_devices`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct ListDevicesParams {}

/// Parameters for `orbit.cloud_policy`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct CloudPolicyParams {}
