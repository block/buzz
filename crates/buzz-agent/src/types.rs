use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum ToolResultContent {
    Text(String),
    Image { data: String, mime_type: String },
}

impl ToolResultContent {
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            // This is request-size pressure accounting, not a visual-token
            // estimate. Count the base64 bytes we will actually serialize so
            // image-heavy sessions cannot silently exceed provider/body caps.
            Self::Image { data, mime_type } => data.len() + mime_type.len(),
        }
    }

    pub fn as_text_lossy(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Image { data, mime_type } => {
                format!("[image: {mime_type}, {} base64 bytes]", data.len())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum HistoryItem {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult(ToolResult),
}

impl HistoryItem {
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::User(s) => s.len(),
            Self::Assistant { text, tool_calls } => {
                text.len()
                    + tool_calls
                        .iter()
                        .map(|c| {
                            c.provider_id.len()
                                + c.name.len()
                                + serde_json::to_vec(&c.arguments)
                                    .map(|b| b.len())
                                    .unwrap_or(0)
                        })
                        .sum::<usize>()
            }
            Self::ToolResult(r) => {
                r.provider_id.len()
                    + r.content
                        .iter()
                        .map(ToolResultContent::estimated_bytes)
                        .sum::<usize>()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub provider_id: String,
    pub name: String,
    pub arguments: Value,
}

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

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop: ProviderStop,
    /// Total input tokens the provider reported for this request, or `None`
    /// if the response carried no usage. For Anthropic/Databricks this is the
    /// inclusive sum `input_tokens + cache_read_input_tokens +
    /// cache_creation_input_tokens` (plain `input_tokens` excludes cached
    /// tokens, so reading it alone would undercount). Used to gate handoff on
    /// the real token budget rather than a byte estimate.
    pub input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderStop {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
    Other,
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug)]
pub enum AgentError {
    InvalidParams(String),
    Llm(String),
    LlmAuth(String),
    Mcp(String),
    Cancelled,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(s) => write!(f, "invalid params: {s}"),
            Self::Llm(s) => write!(f, "llm: {s}"),
            Self::LlmAuth(s) => write!(f, "llm auth: {s}"),
            Self::Mcp(s) => write!(f, "mcp: {s}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for AgentError {}

impl AgentError {
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::LlmAuth(_) => -32001,
            _ => -32000,
        }
    }
}

pub fn clamp(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    const MARKER: &str = "\n[truncated]";
    let budget = max.saturating_sub(MARKER.len());
    let mut cut = budget;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    if max >= MARKER.len() {
        s.push_str(MARKER);
    }
    s
}
