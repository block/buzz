use serde::Deserialize;
use serde_json::Value;

/// Byte-equivalent charged to the handoff/context-pressure gate for a single
/// image tool result. The gate maps bytes to tokens at 1 byte/token (see
/// `handoff::CONSERVATIVE_BYTES_PER_TOKEN`), so this is also the per-image
/// token budget. Providers bill an image as visual *tiles*, not its base64
/// length: Anthropic caps at ~1600 tokens/image and OpenAI high-detail lands
/// ~1.1K–1.5K. We charge 16 KiB — a generous ceiling that still over-counts
/// the real ~2K cost, while being ~190× smaller than the base64 length of a
/// typical multi-MiB screenshot. Charging `data.len()` to the gate instead
/// made a single `view_image` (~3.1M base64 bytes) trip the handoff gate on a
/// fresh context.
const IMAGE_CONTEXT_TOKEN_EQUIV: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub enum ToolResultContent {
    Text(String),
    Image { data: String, mime_type: String },
}

impl ToolResultContent {
    /// Real serialized size in bytes. Used by `truncate_history` to keep the
    /// outgoing request body under `max_history_bytes` — an image rides the
    /// wire as its full base64 string, so that string's length is what counts
    /// here. For context-window/handoff pressure use
    /// [`Self::context_pressure_bytes`] instead, which charges an image its
    /// (far smaller) visual-token equivalent.
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Image { data, mime_type } => data.len() + mime_type.len(),
        }
    }

    /// Token-equivalent context-window pressure, in bytes (the handoff gate
    /// maps bytes→tokens at 1:1). Identical to [`Self::estimated_bytes`] for
    /// text, but an image is charged a flat [`IMAGE_CONTEXT_TOKEN_EQUIV`]
    /// budget rather than its base64 length — providers bill it as visual
    /// tiles (~2K tokens), so counting `data.len()` over-counts by ~1500× and
    /// forces a handoff on a single image.
    pub fn context_pressure_bytes(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Image { data: _, mime_type } => IMAGE_CONTEXT_TOKEN_EQUIV + mime_type.len(),
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
        self.size_with(ToolResultContent::estimated_bytes)
    }

    /// Token-equivalent context-window pressure, in bytes. Mirrors
    /// [`Self::estimated_bytes`] but charges image tool results their visual-
    /// token equivalent rather than their base64 length — see
    /// [`ToolResultContent::context_pressure_bytes`]. The handoff gate uses
    /// this; `truncate_history` (request-body sizing) uses `estimated_bytes`.
    pub fn context_pressure_bytes(&self) -> usize {
        self.size_with(ToolResultContent::context_pressure_bytes)
    }

    fn size_with(&self, content_size: fn(&ToolResultContent) -> usize) -> usize {
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
                r.provider_id.len() + r.content.iter().map(content_size).sum::<usize>()
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
    /// Output tokens the provider reported for this request, or `None` if the
    /// response carried no usage. Used to accumulate per-turn output counts
    /// for NIP-AM metric publishing.
    pub output_tokens: Option<u64>,
    /// Reasoning/thinking content emitted by the model before its answer, if
    /// any. Non-empty when the provider returns extended-thinking tokens:
    ///
    /// - Responses API: concatenated `summary[].text` from `type == "reasoning"` output items.
    /// - Anthropic: concatenated `thinking` from `type == "thinking"` content blocks.
    /// - OpenAI chat/completions: not exposed; always empty.
    ///
    /// Empty string when the provider returned no reasoning content.
    pub reasoning: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn image_item(base64_len: usize) -> HistoryItem {
        HistoryItem::ToolResult(ToolResult {
            provider_id: "call_1".into(),
            content: vec![ToolResultContent::Image {
                data: "A".repeat(base64_len),
                mime_type: "image/png".into(),
            }],
            is_error: false,
        })
    }

    #[test]
    fn image_estimated_bytes_is_real_wire_size() {
        // `truncate_history` relies on this to keep the request body under
        // `max_history_bytes`, so an image must report its full base64 length.
        let img = ToolResultContent::Image {
            data: "A".repeat(3_000_000),
            mime_type: "image/png".into(),
        };
        assert_eq!(img.estimated_bytes(), 3_000_000 + "image/png".len());
    }

    #[test]
    fn image_context_pressure_is_token_equivalent_not_base64_len() {
        // The handoff gate must charge an image its visual-token equivalent,
        // not its base64 length — otherwise one screenshot trips the gate.
        let img = ToolResultContent::Image {
            data: "A".repeat(3_000_000),
            mime_type: "image/png".into(),
        };
        assert_eq!(
            img.context_pressure_bytes(),
            IMAGE_CONTEXT_TOKEN_EQUIV + "image/png".len()
        );
        // And it must be independent of the (huge) base64 payload length.
        let bigger = ToolResultContent::Image {
            data: "A".repeat(10_000_000),
            mime_type: "image/png".into(),
        };
        assert_eq!(
            img.context_pressure_bytes(),
            bigger.context_pressure_bytes()
        );
    }

    #[test]
    fn single_image_does_not_trip_default_handoff_threshold() {
        // Regression: a single ~3.1M-base64-byte `view_image` result on an
        // otherwise-empty history must NOT exceed the default pre-usage
        // handoff cap. The gate's byte-fallback threshold with the shipped
        // defaults (max_context_tokens=200_000, max_output_tokens=32_768) is
        // min(200_000*9/10, 200_000-32_768) = 167_232 "bytes". Before the fix
        // this item counted ~3.1M and tripped instantly.
        let item = image_item(3_118_884);
        const DEFAULT_PRE_USAGE_THRESHOLD: usize = 167_232;
        assert!(
            item.context_pressure_bytes() <= DEFAULT_PRE_USAGE_THRESHOLD,
            "one image charged {} bytes of context pressure, over the {} threshold",
            item.context_pressure_bytes(),
            DEFAULT_PRE_USAGE_THRESHOLD
        );
        // The real wire size, by contrast, is still the full base64 payload.
        assert!(item.estimated_bytes() >= 3_118_884);
    }

    #[test]
    fn text_content_size_is_identical_for_both_measures() {
        // Only images diverge; text must size the same under both paths.
        let text = ToolResultContent::Text("hello world".into());
        assert_eq!(text.estimated_bytes(), text.context_pressure_bytes());
        let item = HistoryItem::User("a user message".into());
        assert_eq!(item.estimated_bytes(), item.context_pressure_bytes());
    }
}
