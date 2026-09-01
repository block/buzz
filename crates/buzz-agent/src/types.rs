//! Wire-facing types shared with `wire.rs`.
//!
//! This is a deliberate subset of `crates/buzz-agent/src/types.rs` (353 lines).
//! The original carried the hand-written loop's internal vocabulary —
//! `HistoryItem`, `ToolCall`, `ToolResult`, `LlmResponse`, `ProviderStop`,
//! `ToolDef`, plus byte-accounting helpers (`estimated_bytes`,
//! `context_pressure_bytes`) that fed the bespoke handoff heuristic.
//!
//! Goose owns all of that now: conversation state is `goose::conversation`,
//! tool plumbing is `rmcp`, and compaction is `goose_context_management`. What
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

/// Publisher-side billing identity for a turn.
///
/// Mirrors `buzz_core::agent_turn_metric::PricingIdentity` but is local to
/// `buzz-agent` (which does not depend on `buzz-core`). The two structs have
/// the same camelCase wire representation and are deserialized identically by
/// `buzz-acp`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingIdentity {
    /// Registered billing-namespace identifier (bare lowercase hostname).
    pub authority: String,
    /// Actually-requested billable model identifier.
    pub model: String,
    /// Cache-write class when applicable; omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_class: Option<String>,
}

/// Per-turn and per-session accumulator for one optional cache category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheTotalState {
    #[default]
    Unseen,
    Exact(u64),
    Unknown,
}

impl CacheTotalState {
    pub fn fold(self, value: Option<u64>) -> Self {
        match (self, value) {
            (Self::Unknown, _) | (_, None) => Self::Unknown,
            (Self::Unseen, Some(n)) => Self::Exact(n),
            (Self::Exact(acc), Some(n)) => acc.checked_add(n).map_or(Self::Unknown, Self::Exact),
        }
    }

    pub fn merge_session(self, turn: Self) -> Self {
        match (self, turn) {
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (acc, Self::Unseen) => acc,
            (Self::Unseen, Self::Exact(n)) => Self::Exact(n),
            (Self::Exact(acc), Self::Exact(n)) => {
                acc.checked_add(n).map_or(Self::Unknown, Self::Exact)
            }
        }
    }

    pub fn exact_value(self) -> Option<u64> {
        match self {
            Self::Exact(n) => Some(n),
            Self::Unseen | Self::Unknown => None,
        }
    }
}

/// Tri-state accumulator for provider-reported total tokens within one ACP turn.
///
/// - `Unseen`: no usage-bearing response observed yet (initial state for each turn).
/// - `Exact(n)`: every response so far reported a total; `n` is their sum.
/// - `Unknown`: at least one response lacked a total — permanently poisoned for
///   this turn. The session-cumulative also transitions to Unknown when any turn
///   lands Unknown, and stays there until a new session resets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnTotalState {
    #[default]
    Unseen,
    Exact(u64),
    Unknown,
}

impl TurnTotalState {
    /// Add two exact token counts with overflow protection.
    ///
    /// Returns `Exact(acc + n)` on success or `Unknown` on overflow.
    /// This is the single implementation of the checked-add / overflow-poisons
    /// contract; both `fold()` and `merge_session()` call this helper so a
    /// change to overflow semantics needs to be made in exactly one place.
    fn checked_exact_sum(acc: u64, n: u64) -> TurnTotalState {
        match acc.checked_add(n) {
            Some(sum) => TurnTotalState::Exact(sum),
            None => TurnTotalState::Unknown,
        }
    }

    /// Fold one provider-reported total into the current state.
    ///
    /// `total`: `Some(n)` when the provider included a genuine total on this
    /// response; `None` when it was absent (e.g. Anthropic, or an OpenAI
    /// response that omits usage). Absence of a total on any usage-bearing
    /// response poisons the whole turn.
    ///
    /// Overflow is handled by `checked_exact_sum`: a saturated value would
    /// not be a genuine provider-reported total, so overflow → `Unknown`.
    pub fn fold(self, total: Option<u64>) -> TurnTotalState {
        match (self, total) {
            // Already poisoned — stays Unknown regardless.
            (TurnTotalState::Unknown, _) => TurnTotalState::Unknown,
            // No total from this response — poison the accumulator.
            (_, None) => TurnTotalState::Unknown,
            // First response with a total.
            (TurnTotalState::Unseen, Some(n)) => TurnTotalState::Exact(n),
            // Subsequent response — delegate to the shared checked-sum helper.
            (TurnTotalState::Exact(acc), Some(n)) => Self::checked_exact_sum(acc, n),
        }
    }

    /// Merge a completed turn's total state into the session-cumulative state.
    ///
    /// This is the turn→session boundary accumulation:
    /// - An `Unseen` turn (no usage-bearing responses) leaves the cumulative unchanged.
    /// - Any `Unknown` side poisons the session permanently.
    /// - Two `Exact` values are summed via `checked_exact_sum`; overflow → `Unknown`.
    ///
    /// The checked-add logic lives in `checked_exact_sum`; both this function and
    /// `fold()` call that helper so overflow semantics are defined once.
    pub fn merge_session(self, turn: TurnTotalState) -> TurnTotalState {
        match (self, turn) {
            // Either side poisoned → session is poisoned.
            (TurnTotalState::Unknown, _) | (_, TurnTotalState::Unknown) => TurnTotalState::Unknown,
            // Turn had no usage-bearing responses → no change to cumulative.
            (acc, TurnTotalState::Unseen) => acc,
            // First exact turn — adopt its value.
            (TurnTotalState::Unseen, TurnTotalState::Exact(n)) => TurnTotalState::Exact(n),
            // Add to running exact sum — delegate to the shared checked-sum helper.
            (TurnTotalState::Exact(acc), TurnTotalState::Exact(n)) => {
                Self::checked_exact_sum(acc, n)
            }
        }
    }

    /// Consume the exact value if present; `None` for `Unseen` or `Unknown`.
    pub fn exact_value(self) -> Option<u64> {
        match self {
            TurnTotalState::Exact(n) => Some(n),
            _ => None,
        }
    }
}
