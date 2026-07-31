//! Wire-facing types shared with `wire.rs`.
//!
//! This is a deliberate subset of `crates/buzz-agent/src/types.rs` (353 lines).
//! The original carried the hand-written loop's internal vocabulary —
//! `HistoryItem`, `ToolCall`, `ToolResult`, `LlmResponse`, `ProviderStop`,
//! `ToolDef`, plus byte-accounting helpers (`estimated_bytes`,
//! `context_pressure_bytes`) that fed the bespoke handoff heuristic.
//!
//! Goose owns all of that now: conversation state is `goose::conversation`,
//! tool plumbing is `rmcp`, and compaction is `goose::context_mgmt`. What
//! survives here is only what crosses the ACP wire.

use serde::Deserialize;

/// A stdio MCP server declaration from `session/new`.
#[derive(Debug, Deserialize, Clone)]
pub struct McpServerStdio {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// A single block of an ACP `session/prompt` payload.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ResourceLink {
        uri: String,
    },
    #[serde(other)]
    Unsupported,
}

/// ACP stop reasons. Wire strings are load-bearing: `buzz-acp` parses
/// `stopReason` off the `session/prompt` response and errors without it
/// (`acp.rs:1757-1761`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
}

impl StopReason {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::Cancelled => "cancelled",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
        }
    }
}

/// Errors surfaced to the client as JSON-RPC error responses.
#[derive(Debug)]
pub enum AgentError {
    InvalidParams(String),
    Llm(String),
    LlmAuth(String),
    LlmModelNotFound(String),
    Mcp(String),
    Cancelled,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(s) => write!(f, "invalid params: {s}"),
            Self::Llm(s) => write!(f, "llm: {s}"),
            Self::LlmAuth(s) => write!(f, "llm auth: {s}"),
            Self::LlmModelNotFound(s) => write!(f, "llm model not found: {s}"),
            Self::Mcp(s) => write!(f, "mcp: {s}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for AgentError {}

impl AgentError {
    /// Preserved verbatim from buzz-agent: the harness's error taxonomy keys
    /// off these codes.
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::LlmAuth(_) => -32001,
            Self::LlmModelNotFound(_) => -32002,
            _ => -32000,
        }
    }
}

/// A tool definition advertised to the model. Used by [`crate::builtin`] for
/// the in-process `load_skill` tool.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// One piece of a tool result.
#[derive(Debug, Clone)]
pub enum ToolResultContent {
    Text(String),
    Image { data: String, mime_type: String },
}

impl ToolResultContent {
    pub fn as_text_lossy(&self) -> String {
        match self {
            Self::Text(t) => t.clone(),
            Self::Image { mime_type, .. } => format!("[image: {mime_type}]"),
        }
    }
}

/// Result of an in-process tool call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub provider_id: String,
    pub content: Vec<ToolResultContent>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(&self) -> String {
        self.content
            .iter()
            .map(ToolResultContent::as_text_lossy)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Truncate `s` to at most `max` bytes without splitting a UTF-8 character.
///
/// Lifted from the deleted `mcp.rs`; `hints` and `builtin` both cap what they
/// read off disk with it.
pub fn truncate_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}
