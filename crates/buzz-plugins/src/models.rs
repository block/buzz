#![deny(unsafe_code)]
//! Data models for raw ingested documents and normalized session transcripts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use buzz_core::memory::Document;

/// Participant role in an agent conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    /// Human developer or prompt author.
    User,
    /// AI assistant or autonomous coding agent.
    Assistant,
    /// Tool execution input or output.
    Tool,
    /// System instruction, prompt injection boundary, or environment notice.
    System,
}

impl TurnRole {
    /// Returns static string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for TurnRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A structured turn inside an agent conversation or session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTurn {
    /// Speaker or actor of the turn.
    pub role: TurnRole,
    /// Primary text or conversational content.
    pub content: String,
    /// Optional chain-of-thought or reasoning text.
    pub thinking: Option<String>,
    /// Tool invocations made during this turn (name, parameters).
    #[serde(default)]
    pub tool_calls: Vec<serde_json::Value>,
    /// Tool outputs or execution results returned.
    #[serde(default)]
    pub tool_results: Vec<serde_json::Value>,
    /// Timestamp when this turn was produced.
    pub timestamp: Option<DateTime<Utc>>,
}

impl SessionTurn {
    /// Creates a new user message turn.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: TurnRole::User,
            content: content.into(),
            thinking: None,
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            timestamp: Some(Utc::now()),
        }
    }

    /// Creates a new assistant turn with optional thinking.
    pub fn assistant(content: impl Into<String>, thinking: Option<String>) -> Self {
        Self {
            role: TurnRole::Assistant,
            content: content.into(),
            thinking,
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            timestamp: Some(Utc::now()),
        }
    }

    /// Creates a tool turn.
    pub fn tool(content: impl Into<String>, tool_calls: Vec<serde_json::Value>, tool_results: Vec<serde_json::Value>) -> Self {
        Self {
            role: TurnRole::Tool,
            content: content.into(),
            thinking: None,
            tool_calls,
            tool_results,
            timestamp: Some(Utc::now()),
        }
    }
}

/// Raw parsed document before chunking and embedding, representing an agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawDocument {
    /// Absolute local file path or URI of the raw transcript.
    pub source_uri: String,
    /// Workspace root boundary for scoping and tenant isolation.
    pub workspace_path: String,
    /// Clean, normalized session transcript text (secret-redacted).
    pub content: String,
    /// SHA-256 hash of `content` for idempotency and change detection.
    pub content_hash: String,
    /// Timestamp or modification time in nanoseconds.
    pub mtime_ns: i64,
    /// Extensible metadata (agent name, session ID, turn count, policy flags).
    pub metadata: serde_json::Value,
    /// Structured conversation turns for rich inspection.
    pub turns: Vec<SessionTurn>,
}

impl RawDocument {
    /// Creates a new `RawDocument`, automatically computing the SHA-256 `content_hash`.
    pub fn new(
        source_uri: impl Into<String>,
        workspace_path: impl Into<String>,
        content: impl Into<String>,
        mtime_ns: i64,
        metadata: serde_json::Value,
        turns: Vec<SessionTurn>,
    ) -> Self {
        let content_str = content.into();
        let mut hasher = Sha256::new();
        hasher.update(content_str.as_bytes());
        let content_hash = hex::encode(hasher.finalize());

        Self {
            source_uri: source_uri.into(),
            workspace_path: workspace_path.into(),
            content: content_str,
            content_hash,
            mtime_ns,
            metadata,
            turns,
        }
    }

    /// Converts this `RawDocument` into Orbit's persistent `buzz_core::memory::Document` model.
    pub fn to_document(&self) -> Document {
        let now = Utc::now();
        Document {
            id: Uuid::new_v4(),
            source_type: "session".to_string(),
            source_uri: self.source_uri.clone(),
            workspace_path: self.workspace_path.clone(),
            content_hash: self.content_hash.clone(),
            mtime_ns: self.mtime_ns,
            size_bytes: self.content.len() as i64,
            permissions: serde_json::json!({
                "public": true,
                "local_only": true,
                "cloud_sync": false
            }),
            metadata: self.metadata.clone(),
            created_at: now,
            updated_at: now,
        }
    }
}
