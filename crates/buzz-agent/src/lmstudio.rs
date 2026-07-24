use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::types::{clamp, AgentError, ExecutedToolCall, ExecutedToolProvider};

/// Default upper bound for native LM Studio output generation.
pub const MAX_OUTPUT_TOKENS: u32 = 32_768;

/// Default context length requested from the native LM Studio endpoint.
pub const MAX_CONTEXT_TOKENS: u64 = 200_000;

/// Maximum accepted non-streaming native LM Studio response body.
pub const MAX_NATIVE_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

const MAX_NATIVE_OUTPUT_ITEMS: usize = 1_024;
const MAX_DIAGNOSTIC_BYTES: usize = 512;
const MAX_RESPONSE_ID_SUFFIX_BYTES: usize = 256;

/// Native LM Studio reasoning modes accepted by `/api/v1/chat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LmStudioReasoning {
    /// Disable LM Studio native reasoning output.
    Off,
    /// Request LM Studio's native low reasoning level.
    Low,
    /// Request LM Studio's native medium reasoning level.
    Medium,
    /// Request LM Studio's native high reasoning level.
    High,
    /// Enable LM Studio native reasoning with model-selected intensity.
    On,
}

/// An ephemeral MCP integration supplied directly to LM Studio for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EphemeralMcpIntegration {
    #[serde(rename = "type")]
    integration_type: EphemeralMcpType,
    server_label: String,
    server_url: String,
    allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EphemeralMcpType {
    EphemeralMcp,
}

impl EphemeralMcpIntegration {
    /// Creates the native wire representation for one ephemeral MCP server.
    pub fn new(
        server_label: impl Into<String>,
        server_url: impl Into<String>,
        allowed_tools: Vec<String>,
    ) -> Self {
        Self {
            integration_type: EphemeralMcpType::EphemeralMcp,
            server_label: server_label.into(),
            server_url: server_url.into(),
            allowed_tools,
        }
    }
}

/// Non-streaming native LM Studio chat request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LmStudioChatRequest {
    model: String,
    input: String,
    system_prompt: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    integrations: Vec<EphemeralMcpIntegration>,
    stream: bool,
    reasoning: LmStudioReasoning,
    max_output_tokens: u32,
    context_length: u64,
    store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
}

impl LmStudioChatRequest {
    /// Creates a new stateful native chat request with bounded token resources.
    ///
    /// Model, input, and system-prompt size policy remains at the future
    /// configuration and ACP transport boundaries; Task 1 has no HTTP transport
    /// and validates only the native wire contract's explicit numeric resource
    /// boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: impl Into<String>,
        input: impl Into<String>,
        system_prompt: impl Into<String>,
        integrations: Vec<EphemeralMcpIntegration>,
        reasoning: LmStudioReasoning,
        max_output_tokens: u32,
        context_length: u64,
    ) -> Result<Self, AgentError> {
        if !(1..=MAX_OUTPUT_TOKENS).contains(&max_output_tokens) {
            return Err(AgentError::InvalidParams(format!(
                "LM Studio max output tokens must be between 1 and {MAX_OUTPUT_TOKENS}"
            )));
        }
        let widened_output_tokens = u64::from(max_output_tokens);
        if context_length <= widened_output_tokens || context_length > MAX_CONTEXT_TOKENS {
            return Err(AgentError::InvalidParams(format!(
                "LM Studio context length must be greater than max output tokens and at most {MAX_CONTEXT_TOKENS}"
            )));
        }
        Ok(Self {
            model: model.into(),
            input: input.into(),
            system_prompt: system_prompt.into(),
            integrations,
            stream: false,
            reasoning,
            max_output_tokens,
            context_length,
            store: true,
            previous_response_id: None,
        })
    }

    /// Continues a stateful native chat from a prior valid response ID.
    pub fn continue_from(mut self, response_id: impl Into<String>) -> Result<Self, AgentError> {
        let response_id = response_id.into();
        validate_response_id(&response_id)?;
        self.previous_response_id = Some(response_id);
        Ok(self)
    }

    /// Returns the exact native MCP integrations carried by this request.
    pub fn integrations(&self) -> &[EphemeralMcpIntegration] {
        &self.integrations
    }

    /// Returns the prior native response ID for a continuation request.
    pub fn previous_response_id(&self) -> Option<&str> {
        self.previous_response_id.as_deref()
    }
}

fn validate_response_id(response_id: &str) -> Result<(), AgentError> {
    let suffix = response_id
        .strip_prefix("resp_")
        .filter(|suffix| !suffix.is_empty() && suffix.len() <= MAX_RESPONSE_ID_SUFFIX_BYTES);
    if suffix.is_some_and(|suffix| {
        suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }) {
        Ok(())
    } else {
        Err(AgentError::InvalidParams(
            "LM Studio response ID must start with resp_ and contain only bounded ASCII identifier characters"
                .into(),
        ))
    }
}

/// Token statistics reported by a native LM Studio chat response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LmStudioStats {
    /// Tokens consumed by the input, including prior state and tool definitions.
    pub input_tokens: u64,
    /// Total tokens generated by the model.
    pub total_output_tokens: u64,
    /// Generated tokens attributed to reasoning.
    pub reasoning_output_tokens: u64,
}

/// One native LM Studio output item, preserving provider order.
#[derive(Debug, Clone, PartialEq)]
pub enum LmStudioOutput {
    /// Model-visible answer text. Tool-looking content remains inert text.
    Message {
        /// Message text exactly as returned by LM Studio.
        content: String,
    },
    /// Model reasoning. Tool-looking content remains inert reasoning text.
    Reasoning {
        /// Reasoning text exactly as returned by LM Studio.
        content: String,
    },
    /// Structured evidence of a tool call LM Studio already executed.
    ToolCall(ExecutedToolCall),
}

/// Validated, non-streaming native LM Studio chat response.
#[derive(Debug, Clone, PartialEq)]
pub struct LmStudioChatResponse {
    /// Loaded model instance that generated the response.
    pub model_instance_id: String,
    /// Ordered native output items.
    pub output: Vec<LmStudioOutput>,
    /// Provider token statistics.
    pub stats: LmStudioStats,
    /// Stateful response identifier used for the next request.
    pub response_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChatResponse {
    model_instance_id: String,
    output: Vec<RawOutput>,
    stats: RawStats,
    response_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RawOutput {
    Message {
        content: String,
    },
    Reasoning {
        content: String,
    },
    ToolCall {
        tool: String,
        arguments: Map<String, Value>,
        output: String,
        provider_info: RawProviderInfo,
    },
    InvalidToolCall {
        reason: String,
        metadata: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RawProviderInfo {
    EphemeralMcp { server_label: String },
    Plugin { plugin_id: String },
}

#[derive(Debug, Deserialize)]
struct RawStats {
    input_tokens: u64,
    total_output_tokens: u64,
    reasoning_output_tokens: u64,
}

/// Parses and validates a bounded non-streaming native LM Studio response.
///
/// `request_identity` must uniquely identify the originating ACP request. It
/// is combined with each provider output index to create stable synthetic ACP
/// call IDs for already-executed native MCP calls.
pub fn parse_chat_response(
    request_identity: &str,
    body: &[u8],
) -> Result<LmStudioChatResponse, AgentError> {
    if body.len() > MAX_NATIVE_RESPONSE_BYTES {
        return Err(parse_error(format!(
            "native response body exceeds {MAX_NATIVE_RESPONSE_BYTES} bytes"
        )));
    }
    if request_identity.is_empty() || request_identity.len() > 1_024 {
        return Err(parse_error("invalid bounded request identity".into()));
    }

    let raw: RawChatResponse = serde_json::from_slice(body)
        .map_err(|error| parse_error(format!("malformed native response: {error}")))?;
    if raw.model_instance_id.is_empty()
        || raw.model_instance_id.len() > 512
        || raw.model_instance_id.chars().any(char::is_control)
    {
        return Err(parse_error("malformed model instance ID".into()));
    }
    validate_response_id(&raw.response_id)
        .map_err(|_| parse_error("malformed native response ID".into()))?;
    if raw.output.len() > MAX_NATIVE_OUTPUT_ITEMS {
        return Err(parse_error(format!(
            "native response contains more than {MAX_NATIVE_OUTPUT_ITEMS} output items"
        )));
    }

    if let Some((reason, metadata)) = raw.output.iter().find_map(|item| match item {
        RawOutput::InvalidToolCall { reason, metadata } => Some((reason, metadata)),
        _ => None,
    }) {
        let metadata_kind = metadata
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(parse_error(format!(
            "native invalid tool call ({metadata_kind}): {reason}"
        )));
    }
    if !matches!(raw.output.last(), Some(RawOutput::Message { .. })) {
        return Err(parse_error(
            "native response is missing a terminal message".into(),
        ));
    }

    let mut output = Vec::with_capacity(raw.output.len());
    for (index, item) in raw.output.into_iter().enumerate() {
        let parsed = match item {
            RawOutput::Message { content } => LmStudioOutput::Message { content },
            RawOutput::Reasoning { content } => LmStudioOutput::Reasoning { content },
            RawOutput::ToolCall {
                tool,
                arguments,
                output,
                provider_info,
            } => {
                validate_tool_name(&tool)?;
                let provider = match provider_info {
                    RawProviderInfo::EphemeralMcp { server_label } => {
                        validate_provider_identifier("server label", &server_label)?;
                        ExecutedToolProvider::EphemeralMcp { server_label }
                    }
                    RawProviderInfo::Plugin { plugin_id } => {
                        validate_provider_identifier("plugin ID", &plugin_id)?;
                        ExecutedToolProvider::Plugin { plugin_id }
                    }
                };
                LmStudioOutput::ToolCall(ExecutedToolCall {
                    provider_id: synthetic_call_id(request_identity, index),
                    name: tool,
                    arguments: Value::Object(arguments),
                    output,
                    provider,
                })
            }
            RawOutput::InvalidToolCall { .. } => {
                return Err(parse_error("native invalid tool call".into()))
            }
        };
        output.push(parsed);
    }

    Ok(LmStudioChatResponse {
        model_instance_id: raw.model_instance_id,
        output,
        stats: LmStudioStats {
            input_tokens: raw.stats.input_tokens,
            total_output_tokens: raw.stats.total_output_tokens,
            reasoning_output_tokens: raw.stats.reasoning_output_tokens,
        },
        response_id: raw.response_id,
    })
}

fn validate_tool_name(name: &str) -> Result<(), AgentError> {
    validate_provider_identifier("tool name", name)
}

fn validate_provider_identifier(kind: &str, value: &str) -> Result<(), AgentError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(parse_error(format!("malformed native {kind}")))
    } else {
        Ok(())
    }
}

fn synthetic_call_id(request_identity: &str, output_index: usize) -> String {
    let mut hash = Sha256::new();
    hash.update((request_identity.len() as u64).to_be_bytes());
    hash.update(request_identity.as_bytes());
    hash.update((output_index as u64).to_be_bytes());
    let digest = hash.finalize();
    format!("lmstudio_{}", hex::encode(&digest[..16]))
}

fn parse_error(message: String) -> AgentError {
    AgentError::Llm(clamp(message, MAX_DIAGNOSTIC_BYTES))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        parse_chat_response, EphemeralMcpIntegration, LmStudioChatRequest, LmStudioOutput,
        LmStudioReasoning, MAX_CONTEXT_TOKENS, MAX_NATIVE_RESPONSE_BYTES, MAX_OUTPUT_TOKENS,
    };
    use crate::types::{AgentError, ExecutedToolProvider};

    #[test]
    fn new_chat_request_uses_only_native_fields() {
        let request = LmStudioChatRequest::new(
            "qwen/qwen3.6-27b",
            "Prepare the brief.",
            "You are the Chief of Staff.",
            vec![EphemeralMcpIntegration::new(
                "memory",
                "http://127.0.0.1:8765/mcp",
                vec!["recall_for_entity".into(), "search_events".into()],
            )],
            LmStudioReasoning::On,
            MAX_OUTPUT_TOKENS,
            MAX_CONTEXT_TOKENS,
        )
        .unwrap();

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            json!({
                "model": "qwen/qwen3.6-27b",
                "input": "Prepare the brief.",
                "system_prompt": "You are the Chief of Staff.",
                "integrations": [{
                    "type": "ephemeral_mcp",
                    "server_label": "memory",
                    "server_url": "http://127.0.0.1:8765/mcp",
                    "allowed_tools": ["recall_for_entity", "search_events"]
                }],
                "stream": false,
                "reasoning": "on",
                "max_output_tokens": MAX_OUTPUT_TOKENS,
                "context_length": MAX_CONTEXT_TOKENS,
                "store": true
            })
        );
        assert!(value.get("previous_response_id").is_none());
        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
        assert!(value.get("messages").is_none());
    }

    #[test]
    fn continued_chat_request_serializes_previous_response_id() {
        let request = LmStudioChatRequest::new(
            "qwen/qwen3.6-27b",
            "Continue.",
            "System",
            Vec::new(),
            LmStudioReasoning::Off,
            2_048,
            32_768,
        )
        .unwrap()
        .continue_from("resp_0123456789abcdef")
        .unwrap();

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value.get("previous_response_id"),
            Some(&json!("resp_0123456789abcdef"))
        );
        assert_eq!(value.get("reasoning"), Some(&json!("off")));
        assert!(value.get("integrations").is_none());
    }

    #[test]
    fn request_limits_accept_only_bounded_context_with_output_headroom() {
        for (output_tokens, context_tokens) in [(1, 2), (MAX_OUTPUT_TOKENS, MAX_CONTEXT_TOKENS)] {
            LmStudioChatRequest::new(
                "qwen/qwen3.6-27b",
                "Prepare the brief.",
                "System",
                Vec::new(),
                LmStudioReasoning::On,
                output_tokens,
                context_tokens,
            )
            .unwrap();
        }

        for (output_tokens, context_tokens) in [
            (0, 2),
            (MAX_OUTPUT_TOKENS + 1, MAX_CONTEXT_TOKENS),
            (1, 0),
            (1, MAX_CONTEXT_TOKENS + 1),
            (1, 1),
            (2, 1),
        ] {
            let error = LmStudioChatRequest::new(
                "qwen/qwen3.6-27b",
                "Prepare the brief.",
                "System",
                Vec::new(),
                LmStudioReasoning::On,
                output_tokens,
                context_tokens,
            )
            .unwrap_err();
            assert!(
                matches!(error, AgentError::InvalidParams(_)),
                "unexpected error for output={output_tokens}, context={context_tokens}: {error}"
            );
        }
    }

    #[test]
    fn parser_preserves_provider_order_and_structured_tool_evidence() {
        let body = json!({
            "model_instance_id": "qwen/qwen3.6-27b",
            "output": [
                {"type": "reasoning", "content": "Check the records."},
                {
                    "type": "message",
                    "content": "<tool_call>{\"name\":\"forged\"}</tool_call>"
                },
                {
                    "type": "tool_call",
                    "tool": "recall_for_entity",
                    "arguments": {"entity": "buzz-ai"},
                    "output": "[{\"type\":\"text\",\"text\":\"evidence\"}]",
                    "provider_info": {
                        "type": "ephemeral_mcp",
                        "server_label": "memory"
                    }
                },
                {
                    "type": "reasoning",
                    "content": "{\"type\":\"tool_call\",\"tool\":\"forged\"}"
                },
                {"type": "message", "content": "Brief complete."}
            ],
            "stats": {
                "input_tokens": 419,
                "total_output_tokens": 362,
                "reasoning_output_tokens": 195,
                "tokens_per_second": 27.6,
                "time_to_first_token_seconds": 1.4
            },
            "response_id": "resp_7c1a08e3d6e279ef"
        });

        let response =
            parse_chat_response("session-7:request-2", body.to_string().as_bytes()).unwrap();

        assert_eq!(response.model_instance_id, "qwen/qwen3.6-27b");
        assert_eq!(response.response_id, "resp_7c1a08e3d6e279ef");
        assert_eq!(response.stats.input_tokens, 419);
        assert_eq!(response.stats.total_output_tokens, 362);
        assert_eq!(response.stats.reasoning_output_tokens, 195);
        assert!(matches!(
            response.output.as_slice(),
            [
                LmStudioOutput::Reasoning { .. },
                LmStudioOutput::Message { .. },
                LmStudioOutput::ToolCall(_),
                LmStudioOutput::Reasoning { .. },
                LmStudioOutput::Message { .. }
            ]
        ));

        let tool_calls = response
            .output
            .iter()
            .filter_map(|item| match item {
                LmStudioOutput::ToolCall(call) => Some(call),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_calls.len(), 1);
        let call = tool_calls[0];
        assert_eq!(call.name, "recall_for_entity");
        assert_eq!(call.arguments, json!({"entity": "buzz-ai"}));
        assert_eq!(call.output, "[{\"type\":\"text\",\"text\":\"evidence\"}]");
        assert_eq!(
            call.provider,
            ExecutedToolProvider::EphemeralMcp {
                server_label: "memory".into()
            }
        );

        let same = parse_chat_response("session-7:request-2", body.to_string().as_bytes()).unwrap();
        let different =
            parse_chat_response("session-7:request-3", body.to_string().as_bytes()).unwrap();
        let call_id = tool_call_id(&response);
        assert_eq!(call_id, tool_call_id(&same));
        assert_ne!(call_id, tool_call_id(&different));
    }

    #[test]
    fn invalid_tool_call_fails_closed_with_bounded_diagnostic() {
        let reason = "invalid arguments ".repeat(1_000);
        let body = json!({
            "model_instance_id": "qwen/qwen3.6-27b",
            "output": [{
                "type": "invalid_tool_call",
                "reason": reason,
                "metadata": {
                    "type": "invalid_arguments",
                    "tool_name": "search_events",
                    "arguments": {"query": 7},
                    "provider_info": {
                        "type": "ephemeral_mcp",
                        "server_label": "memory"
                    }
                }
            }],
            "stats": {
                "input_tokens": 1,
                "total_output_tokens": 1,
                "reasoning_output_tokens": 1
            },
            "response_id": "resp_invalid"
        });

        let error = parse_chat_response("request", body.to_string().as_bytes()).unwrap_err();
        let AgentError::Llm(diagnostic) = error else {
            panic!("expected LM Studio parser failure");
        };
        assert!(diagnostic.contains("invalid tool call"));
        assert!(diagnostic.len() <= 512);
    }

    #[test]
    fn parser_requires_a_terminal_message() {
        let body = json!({
            "model_instance_id": "qwen/qwen3.6-27b",
            "output": [{"type": "reasoning", "content": "No final answer"}],
            "stats": {
                "input_tokens": 1,
                "total_output_tokens": 1,
                "reasoning_output_tokens": 1
            },
            "response_id": "resp_no_message"
        });

        let error = parse_chat_response("request", body.to_string().as_bytes()).unwrap_err();
        assert!(error.to_string().contains("terminal message"));
    }

    #[test]
    fn parser_rejects_invalid_tool_argument_and_output_shapes() {
        for malformed in [
            json!({
                "type": "tool_call",
                "tool": "search_events",
                "arguments": ["not", "an", "object"],
                "output": "ok",
                "provider_info": {
                    "type": "ephemeral_mcp",
                    "server_label": "memory"
                }
            }),
            json!({
                "type": "tool_call",
                "tool": "search_events",
                "arguments": {"query": "buzz"},
                "output": {"not": "a string"},
                "provider_info": {
                    "type": "ephemeral_mcp",
                    "server_label": "memory"
                }
            }),
        ] {
            let body = response_with_outputs(vec![
                malformed,
                json!({"type": "message", "content": "Done"}),
            ]);
            assert!(parse_chat_response("request", body.as_bytes()).is_err());
        }
    }

    #[test]
    fn parser_rejects_missing_or_malformed_response_ids() {
        let missing = json!({
            "model_instance_id": "qwen/qwen3.6-27b",
            "output": [{"type": "message", "content": "Done"}],
            "stats": {
                "input_tokens": 1,
                "total_output_tokens": 1,
                "reasoning_output_tokens": 0
            }
        });
        assert!(parse_chat_response("request", missing.to_string().as_bytes()).is_err());

        for malformed in ["response_123", "resp_", "resp_💥"] {
            let mut body: serde_json::Value =
                serde_json::from_str(&response_with_outputs(vec![json!({
                    "type": "message",
                    "content": "Done"
                })]))
                .unwrap();
            body["response_id"] = json!(malformed);
            assert!(parse_chat_response("request", body.to_string().as_bytes()).is_err());
        }
    }

    #[test]
    fn parser_rejects_duplicate_fields_and_unknown_item_types() {
        let duplicate = br#"{
            "model_instance_id":"qwen/qwen3.6-27b",
            "output":[{"type":"message","content":"Done"}],
            "stats":{"input_tokens":1,"total_output_tokens":1,"reasoning_output_tokens":0},
            "response_id":"resp_first",
            "response_id":"resp_second"
        }"#;
        assert!(parse_chat_response("request", duplicate).is_err());

        let unknown = response_with_outputs(vec![
            json!({"type": "computer_action", "content": "not supported"}),
            json!({"type": "message", "content": "Done"}),
        ]);
        assert!(parse_chat_response("request", unknown.as_bytes()).is_err());
    }

    #[test]
    fn parser_rejects_over_large_bodies_before_json_decoding() {
        let body = vec![b' '; MAX_NATIVE_RESPONSE_BYTES + 1];
        let error = parse_chat_response("request", &body).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
        assert!(error.to_string().len() <= 512);
    }

    fn response_with_outputs(outputs: Vec<serde_json::Value>) -> String {
        json!({
            "model_instance_id": "qwen/qwen3.6-27b",
            "output": outputs,
            "stats": {
                "input_tokens": 1,
                "total_output_tokens": 1,
                "reasoning_output_tokens": 0
            },
            "response_id": "resp_valid"
        })
        .to_string()
    }

    fn tool_call_id(response: &super::LmStudioChatResponse) -> &str {
        response
            .output
            .iter()
            .find_map(|item| match item {
                LmStudioOutput::ToolCall(call) => Some(call.provider_id.as_str()),
                _ => None,
            })
            .unwrap()
    }
}
