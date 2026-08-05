use serde::Deserialize;
use serde_json::{Map, Value};

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
        reasoning_details: Option<Value>,
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
            Self::Assistant {
                text,
                tool_calls,
                reasoning_details,
            } => {
                text.len()
                    + tool_calls
                        .iter()
                        .map(|c| {
                            c.provider_id.len()
                                + c.name.len()
                                + serde_json::to_vec(&c.arguments)
                                    .map(|b| b.len())
                                    .unwrap_or(0)
                                // `provider_extra` (e.g. a Gemini
                                // `thoughtSignature`) is re-serialized into
                                // every replayed call, so it counts toward the
                                // request body and the context-pressure gate.
                                + serde_json::to_vec(&c.provider_extra)
                                    .map(|b| b.len())
                                    .unwrap_or(0)
                        })
                        .sum::<usize>()
                    + reasoning_details
                        .as_ref()
                        .and_then(|v| serde_json::to_vec(v).ok())
                        .map(|b| b.len())
                        .unwrap_or(0)
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
    /// Fields the provider put on the tool call that we do not model, kept so
    /// the assistant turn can be replayed the way it arrived.
    ///
    /// Gemini on the Databricks MLflow route returns a `thoughtSignature` per
    /// call and *requires* it echoed back: replaying without it fails the whole
    /// request with `Function call is missing a thought_signature in functionCall
    /// parts`. For an agent loop that lands on the very first tool call, so the
    /// model is unusable without this. Carrying whatever we did not model,
    /// rather than naming that one field, means the next provider with an opaque
    /// per-call token needs no change here.
    pub provider_extra: Map<String, Value>,
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
    /// The portion of `input_tokens` the provider served from its prompt cache,
    /// or `None` when the response reported no cache split. Providers bill this
    /// slice at a large discount (roughly 10x for both OpenAI and Anthropic),
    /// so a consumer that prices all of `input_tokens` at the full rate
    /// *overstates* cost — by a lot on an append-only agent loop, where most of
    /// each request is a prefix the provider already has.
    ///
    /// This is a subset of `input_tokens`, never an addition to it: every
    /// provider we speak to reports an inclusive input total, so adding this
    /// would double-count.
    pub cached_input_tokens: Option<u64>,
    /// Output tokens the provider reported for this request, or `None` if the
    /// response carried no usage. Used to accumulate per-turn output counts
    /// for NIP-AM metric publishing.
    pub output_tokens: Option<u64>,
    /// Provider-reported total tokens for this request, or `None` when the
    /// provider does not report a genuine total. Present for OpenAI-shaped
    /// responses (`usage.total_tokens`). Always `None` for Anthropic, which
    /// reports only category counts; NIP-AM forbids summing categories into a
    /// total. Callers must not derive this by summing `input_tokens +
    /// output_tokens` — that is what the UI display approximation is for.
    pub total_tokens: Option<u64>,
    /// Reasoning/thinking content emitted by the model before its answer, if
    /// any. Non-empty when the provider returns extended-thinking tokens:
    ///
    /// - Responses API: concatenated `summary[].text` from `type == "reasoning"` output items.
    /// - Anthropic: concatenated `thinking` from `type == "thinking"` content blocks.
    /// - OpenAI chat/completions: not exposed; always empty.
    ///
    /// Empty string when the provider returned no reasoning content.
    pub reasoning: String,
    /// Raw `reasoning_details` array from an OpenRouter response, if present.
    /// Replayed on subsequent turns so the model can continue its chain-of-thought.
    /// `None` for all non-OpenRouter providers.
    pub reasoning_details: Option<Value>,
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

/// Tri-state accumulator for provider-reported total tokens within one ACP turn.
///
/// Tracks whether every usage-bearing LLM response in the turn supplied a genuine
/// provider total. Used to accumulate a reliable per-turn total and contribute to
/// the session-cumulative total.
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

/// The session-cumulative usage counters as of the START of a turn.
///
/// Copied out of the session under the lock when a turn begins and handed to
/// `RunCtx` by value, so the run loop can emit a cumulative `usage_update`
/// after every LLM round without reaching back into `App.sessions` (which it
/// holds no handle to, and which is locked by the turn's own bookkeeping at
/// both ends).
///
/// This exists so that usage is durable *during* a turn rather than only after
/// it. The counters a turn accrues live in the prompt task's stack frame until
/// the turn returns; a process killed mid-turn takes them with it and the
/// tokens are billed by the provider but recorded nowhere. That is not
/// hypothetical — it silently under-reported a long-horizon benchmark's cost by
/// several-fold, because every phase of a `continue_until_timeout` run is
/// terminated mid-turn by design.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionUsageBaseline {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The cache-served subset of `input_tokens`, not an addition to it.
    pub cached_input_tokens: u64,
    pub total_state: TurnTotalState,
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
    LlmModelNotFound(String),
    /// The provider explicitly rejected image content for the selected model.
    /// Kept distinct so the agent loop can remove the unsupported image from
    /// replayed history and give the model a recoverable tool error.
    UnsupportedImageInput(String),
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
            Self::UnsupportedImageInput(s) => write!(f, "llm image input unsupported: {s}"),
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
            Self::LlmModelNotFound(_) => -32002,
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
    fn assistant_size_counts_provider_extra() {
        // A Gemini `thoughtSignature` rides the wire on every replayed call, so
        // both size measures must see it — otherwise `truncate_history` and the
        // handoff gate under-count and let the real request exceed the budget.
        let mut extra = Map::new();
        extra.insert("thoughtSignature".into(), Value::String("S".repeat(500)));
        let with_extra = HistoryItem::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall {
                provider_id: "id".into(),
                name: "t".into(),
                arguments: Value::Null,
                provider_extra: extra,
            }],
            reasoning_details: None,
        };
        let without_extra = HistoryItem::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall {
                provider_id: "id".into(),
                name: "t".into(),
                arguments: Value::Null,
                provider_extra: Map::new(),
            }],
            reasoning_details: None,
        };
        assert!(with_extra.estimated_bytes() > without_extra.estimated_bytes() + 500);
        assert_eq!(
            with_extra.estimated_bytes(),
            with_extra.context_pressure_bytes(),
            "provider_extra is text, so both measures must agree"
        );
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

#[cfg(test)]
mod turn_total_state_tests {
    use super::TurnTotalState;

    // ── TurnTotalState::fold ───────────────────────────────────────────────

    #[test]
    fn fold_first_response_with_total_becomes_exact() {
        let state = TurnTotalState::Unseen;
        assert_eq!(state.fold(Some(100)), TurnTotalState::Exact(100));
    }

    #[test]
    fn fold_first_response_without_total_becomes_unknown() {
        // Missing total on any usage-bearing response poisons the turn.
        let state = TurnTotalState::Unseen;
        assert_eq!(state.fold(None), TurnTotalState::Unknown);
    }

    #[test]
    fn multiple_provider_rounds_all_with_totals_sum_correctly() {
        // Multiple rounds all reporting a genuine total → Exact with their sum.
        let state = TurnTotalState::Unseen;
        let state = state.fold(Some(100));
        let state = state.fold(Some(50));
        let state = state.fold(Some(75));
        assert_eq!(state, TurnTotalState::Exact(225));
    }

    #[test]
    fn mixed_present_and_missing_totals_within_one_turn_poisons_accumulator() {
        // First round has a total, second does not → Unknown (permanently poisoned).
        let state = TurnTotalState::Unseen;
        let state = state.fold(Some(100)); // Exact(100)
        let state = state.fold(None); // Missing → Unknown
        assert_eq!(state, TurnTotalState::Unknown);
        // Further rounds with totals don't un-poison.
        let state = state.fold(Some(50));
        assert_eq!(state, TurnTotalState::Unknown);
    }

    #[test]
    fn unknown_stays_unknown_regardless_of_subsequent_totals() {
        // Once poisoned, no subsequent total can recover the state.
        let state = TurnTotalState::Unknown;
        assert_eq!(state.fold(Some(999)), TurnTotalState::Unknown);
        assert_eq!(state.fold(None), TurnTotalState::Unknown);
    }

    #[test]
    fn exact_value_returns_some_only_for_exact_variant() {
        assert_eq!(TurnTotalState::Unseen.exact_value(), None);
        assert_eq!(TurnTotalState::Unknown.exact_value(), None);
        assert_eq!(TurnTotalState::Exact(42).exact_value(), Some(42));
    }

    #[test]
    fn default_is_unseen() {
        let state: TurnTotalState = Default::default();
        assert_eq!(state, TurnTotalState::Unseen);
    }

    // ── overflow: fold ─────────────────────────────────────────────────────

    #[test]
    fn fold_overflow_poisons_turn_not_saturates() {
        // u64::MAX + 1 would saturate; checked_add must poison instead.
        let state = TurnTotalState::Exact(u64::MAX);
        assert_eq!(
            state.fold(Some(1)),
            TurnTotalState::Unknown,
            "overflow in fold() must produce Unknown, not Exact(u64::MAX)"
        );
    }

    // ── TurnTotalState::merge_session ──────────────────────────────────────

    #[test]
    fn merge_session_unseen_turn_leaves_cumulative_unchanged() {
        // An Unseen turn (no usage-bearing responses) must not alter the cumulative.
        assert_eq!(
            TurnTotalState::Exact(100).merge_session(TurnTotalState::Unseen),
            TurnTotalState::Exact(100),
        );
        assert_eq!(
            TurnTotalState::Unseen.merge_session(TurnTotalState::Unseen),
            TurnTotalState::Unseen,
        );
    }

    #[test]
    fn merge_session_exact_turn_adds_to_exact_cumulative() {
        assert_eq!(
            TurnTotalState::Exact(100).merge_session(TurnTotalState::Exact(50)),
            TurnTotalState::Exact(150),
        );
    }

    #[test]
    fn merge_session_first_exact_turn_from_unseen_adopts_value() {
        assert_eq!(
            TurnTotalState::Unseen.merge_session(TurnTotalState::Exact(200)),
            TurnTotalState::Exact(200),
        );
    }

    #[test]
    fn merge_session_unknown_turn_poisons_cumulative_permanently() {
        assert_eq!(
            TurnTotalState::Exact(100).merge_session(TurnTotalState::Unknown),
            TurnTotalState::Unknown,
        );
        // Poisoned session stays poisoned even with Unseen turn.
        assert_eq!(
            TurnTotalState::Unknown.merge_session(TurnTotalState::Unseen),
            TurnTotalState::Unknown,
        );
        // Poisoned session stays poisoned even with another Exact turn.
        assert_eq!(
            TurnTotalState::Unknown.merge_session(TurnTotalState::Exact(999)),
            TurnTotalState::Unknown,
        );
    }

    #[test]
    fn merge_session_overflow_poisons_not_saturates() {
        // Overflow at the session boundary must also produce Unknown.
        assert_eq!(
            TurnTotalState::Exact(u64::MAX).merge_session(TurnTotalState::Exact(1)),
            TurnTotalState::Unknown,
            "overflow in merge_session() must produce Unknown, not Exact(u64::MAX)"
        );
    }
}
