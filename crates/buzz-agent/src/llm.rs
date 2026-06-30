use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::Client;
use serde_json::{json, Value};

use crate::auth::{PkceOAuthConfig, PkceOAuthTokenSource, StaticTokenSource, TokenSource};
use crate::config::{is_openai_host, Config, OpenAiApi, Provider};
use crate::types::{
    AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef, ToolResultContent,
};

/// Databricks OAuth client_id — the public Databricks-published CLI client.
/// PKCE-only, no secret. Same identifier goose uses, so a user's browser
/// consent for `databricks-cli` covers buzz-agent too.
const DATABRICKS_CLIENT_ID: &str = "databricks-cli";
const DATABRICKS_OAUTH_SCOPES: &[&str] = &["all-apis", "offline_access"];

const MAX_LLM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LLM_ERROR_BODY_BYTES: usize = 4 * 1024;

/// Parser for an OpenAI-family JSON response. Per-endpoint pair lives
/// alongside its `_body` serializer.
type OpenAiParse = fn(Value) -> Result<LlmResponse, AgentError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabricksV2Route {
    OpenAiResponses,
    AnthropicMessages,
    MlflowChatCompletions,
}

pub struct Llm {
    http: Client,
    /// One-shot sticky flag: set when a Chat Completions request comes
    /// back with a "use /v1/responses" provider error while `cfg.openai_api
    /// == Auto`. Subsequent OpenAI calls then go straight to Responses
    /// for the lifetime of the process.
    auto_upgraded: AtomicBool,
    /// Bearer-token source for OpenAI-family requests. Static for OpenAI
    /// (the `OPENAI_COMPAT_API_KEY` env var) and Databricks-with-token
    /// (the `DATABRICKS_TOKEN` env var); a refreshable PKCE engine for
    /// Databricks otherwise. Anthropic doesn't use this — it always
    /// reads `cfg.api_key` directly because the API expects `x-api-key`.
    auth: Arc<dyn TokenSource>,
}

impl Llm {
    pub fn new(cfg: &Config) -> Result<Self, AgentError> {
        let http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(cfg.llm_timeout)
            .build()
            .map_err(|e| AgentError::Llm(format!("http: {e}")))?;
        let auth = build_token_source(cfg)?;
        Ok(Self {
            http,
            auto_upgraded: AtomicBool::new(false),
            auth,
        })
    }

    pub async fn complete(
        &self,
        cfg: &Config,
        system_prompt: &str,
        history: &[HistoryItem],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let v = self
                    .post_anthropic(cfg, &anthropic_body(cfg, system_prompt, history, tools))
                    .await?;
                parse_anthropic(v)
            }
            Provider::OpenAi | Provider::Databricks => {
                self.openai_request(cfg, |use_responses| {
                    if use_responses {
                        (
                            responses_body(cfg, system_prompt, history, tools),
                            parse_responses as OpenAiParse,
                        )
                    } else {
                        (
                            openai_body(cfg, system_prompt, history, tools),
                            parse_openai as OpenAiParse,
                        )
                    }
                })
                .await
            }
            Provider::DatabricksV2 => {
                self.databricks_v2_request(cfg, |route| match route {
                    DatabricksV2Route::OpenAiResponses => (
                        responses_body(cfg, system_prompt, history, tools),
                        parse_responses as OpenAiParse,
                    ),
                    DatabricksV2Route::AnthropicMessages => (
                        anthropic_body(cfg, system_prompt, history, tools),
                        parse_anthropic as OpenAiParse,
                    ),
                    DatabricksV2Route::MlflowChatCompletions => (
                        openai_body(cfg, system_prompt, history, tools),
                        parse_openai as OpenAiParse,
                    ),
                })
                .await
            }
        }
    }

    pub async fn summarize(
        &self,
        cfg: &Config,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: u32,
    ) -> Result<String, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let body = json!({
                    "model": cfg.model,
                    "max_tokens": max_output_tokens,
                    "system": system_prompt,
                    "messages": [{
                        "role": "user",
                        "content": [{ "type": "text", "text": user_prompt }],
                    }],
                });
                Ok(parse_anthropic(self.post_anthropic(cfg, &body).await?)?.text)
            }
            Provider::OpenAi | Provider::Databricks => {
                let r = self
                    .openai_request(cfg, |use_responses| {
                        if use_responses {
                            (
                                json!({
                                    "model": cfg.model,
                                    "max_output_tokens": max_output_tokens,
                                    "instructions": system_prompt,
                                    "input": user_prompt,
                                }),
                                parse_responses as OpenAiParse,
                            )
                        } else {
                            (
                                json!({
                                    "model": cfg.model,
                                    "stream": false,
                                    "max_completion_tokens": max_output_tokens,
                                    "messages": [
                                        { "role": "system", "content": system_prompt },
                                        { "role": "user", "content": user_prompt },
                                    ],
                                }),
                                parse_openai as OpenAiParse,
                            )
                        }
                    })
                    .await?;
                Ok(r.text)
            }
            Provider::DatabricksV2 => {
                let r = self
                    .databricks_v2_request(cfg, |route| match route {
                        DatabricksV2Route::OpenAiResponses => (
                            json!({
                                "model": cfg.model,
                                "max_output_tokens": max_output_tokens,
                                "instructions": system_prompt,
                                "input": user_prompt,
                            }),
                            parse_responses as OpenAiParse,
                        ),
                        DatabricksV2Route::AnthropicMessages => (
                            json!({
                                "model": cfg.model,
                                "max_tokens": max_output_tokens,
                                "system": system_prompt,
                                "messages": [{
                                    "role": "user",
                                    "content": [{ "type": "text", "text": user_prompt }],
                                }],
                            }),
                            parse_anthropic as OpenAiParse,
                        ),
                        DatabricksV2Route::MlflowChatCompletions => (
                            json!({
                                "model": cfg.model,
                                "stream": false,
                                "max_completion_tokens": max_output_tokens,
                                "messages": [
                                    { "role": "system", "content": system_prompt },
                                    { "role": "user", "content": user_prompt },
                                ],
                            }),
                            parse_openai as OpenAiParse,
                        ),
                    })
                    .await?;
                Ok(r.text)
            }
        }
    }

    async fn post_anthropic(&self, cfg: &Config, body: &Value) -> Result<Value, AgentError> {
        let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
        post(&self.http, &url, body, |r| {
            r.header("x-api-key", &cfg.api_key)
                .header("anthropic-version", &cfg.anthropic_api_version)
        })
        .await
    }

    /// OpenAI dispatch: resolve endpoint (pinned > sticky-upgraded > auto by
    /// host), POST, and on `auto` retry once on Responses if the provider
    /// asks for it. `build` is called with `use_responses` so callers
    /// only construct the body actually needed.
    async fn openai_request<F>(&self, cfg: &Config, mut build: F) -> Result<LlmResponse, AgentError>
    where
        F: FnMut(bool) -> (Value, OpenAiParse) + Send,
    {
        let use_responses = self.auto_upgraded.load(Ordering::Relaxed)
            || matches!(cfg.openai_api, OpenAiApi::Responses)
            || matches!(cfg.openai_api, OpenAiApi::Auto) && is_openai_host(&cfg.base_url);

        if use_responses {
            let (b, p) = build(true);
            return p(self.post_openai(cfg, "/responses", &b).await?);
        }
        let (b, p) = build(false);
        match self.post_openai(cfg, "/chat/completions", &b).await {
            Ok(v) => p(v),
            Err(e) if cfg.openai_api == OpenAiApi::Auto && self.try_upgrade(&e) => {
                let (b, p) = build(true);
                p(self.post_openai(cfg, "/responses", &b).await?)
            }
            Err(e) => Err(e),
        }
    }

    async fn databricks_v2_request<F>(
        &self,
        cfg: &Config,
        build: F,
    ) -> Result<LlmResponse, AgentError>
    where
        F: FnOnce(DatabricksV2Route) -> (Value, OpenAiParse) + Send,
    {
        let route = databricks_v2_route_for_model(&cfg.model);
        let (body, parse) = build(route);
        parse(
            self.post_openai(cfg, databricks_v2_path(route), &body)
                .await?,
        )
    }

    /// POST to an OpenAI-family endpoint. For OpenAI-compat this is just
    /// `{base_url}{path}` with the body untouched. For Databricks the URL
    /// becomes `{base_url}/serving-endpoints/{model}/invocations` and the
    /// `model` field is stripped from the body (Databricks rejects it —
    /// the endpoint path already names the model).
    async fn post_openai(
        &self,
        cfg: &Config,
        path: &str,
        body: &Value,
    ) -> Result<Value, AgentError> {
        let (url, body_owned);
        let body_ref: &Value = match cfg.provider {
            Provider::Databricks => {
                url = format!(
                    "{}/serving-endpoints/{}/invocations",
                    cfg.base_url.trim_end_matches('/'),
                    cfg.model
                );
                body_owned = strip_model(body);
                &body_owned
            }
            _ => {
                url = format!("{}{}", cfg.base_url.trim_end_matches('/'), path);
                body
            }
        };

        // A 401 or 403 can mean the local expiry clock disagreed with the
        // server (skew, revocation, a node that never saw the token). On the
        // first such rejection, force a refresh keyed off the rejected bearer
        // and retry once. The guard is local to this call so an earlier turn's
        // rejection can never suppress a later turn's legitimate retry. Both
        // statuses map to `LlmAuth` in `post`: a 403 is indistinguishable from
        // an expired-token 403 here, so we refresh once and let it propagate.
        let mut bearer = self.auth.bearer().await?;
        let mut refreshed = false;
        loop {
            match post(&self.http, &url, body_ref, |r| r.bearer_auth(&bearer)).await {
                Err(AgentError::LlmAuth(_)) if !refreshed => {
                    refreshed = true;
                    bearer = self.auth.refresh_now(&bearer).await?;
                }
                result => return result,
            }
        }
    }

    /// If `err` names `/v1/responses` / "use the Responses API", latch a
    /// sticky upgrade so subsequent OpenAI calls hit Responses. Logged once.
    fn try_upgrade(&self, err: &AgentError) -> bool {
        let body = match err {
            AgentError::Llm(s) => s.as_str(),
            _ => return false, // auth/transport aren't "use the other endpoint" signals
        };
        if !is_responses_required_error(body) {
            return false;
        }
        if !self.auto_upgraded.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                provider_message = body,
                "openai: provider asked for the Responses API; \
                 routing subsequent OpenAI calls to /v1/responses for this process"
            );
        }
        true
    }
}

fn anthropic_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending: Vec<Value> = Vec::new();
    let flush = |out: &mut Vec<Value>, p: &mut Vec<Value>| {
        if !p.is_empty() {
            out.push(json!({ "role": "user", "content": std::mem::take(p) }));
        }
    };
    for item in history {
        match item {
            HistoryItem::User(text) => {
                flush(&mut messages, &mut pending);
                messages.push(json!({ "role": "user",
                    "content": [{ "type": "text", "text": text }] }));
            }
            HistoryItem::Assistant { text, tool_calls } => {
                flush(&mut messages, &mut pending);
                let mut content: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for c in tool_calls {
                    content.push(json!({ "type": "tool_use", "id": c.provider_id,
                        "name": c.name, "input": c.arguments }));
                }
                if content.is_empty() {
                    // Empty assistant turn (no text, no tool calls) — skip it.
                    // Anthropic rejects empty text blocks, and a placeholder
                    // just defers the problem. No tool_use = no pairing
                    // constraint, so omitting is safe.
                    continue;
                }
                messages.push(json!({ "role": "assistant", "content": content }));
            }
            HistoryItem::ToolResult(r) => pending.push(json!({
                "type": "tool_result", "tool_use_id": r.provider_id,
                "content": anthropic_tool_result_content(&r.content), "is_error": r.is_error })),
        }
    }
    flush(&mut messages, &mut pending);
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
        "name": t.name, "description": t.description, "input_schema": t.input_schema })
        })
        .collect();
    let mut body = json!({ "model": cfg.model, "max_tokens": cfg.max_output_tokens,
        "system": system_prompt, "messages": messages });
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
    }
    body
}

fn anthropic_tool_result_content(content: &[ToolResultContent]) -> Vec<Value> {
    content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(text) => json!({ "type": "text", "text": text }),
            ToolResultContent::Image { data, mime_type } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": mime_type, "data": data },
            }),
        })
        .collect()
}

fn openai_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
) -> Value {
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": system_prompt })];
    // Images returned from tool calls ride on a trailing `role:"user"`
    // message because OpenAI Chat's `role:"tool"` content is text-only. We
    // batch them across a run of adjacent ToolResult items so that all
    // `role:"tool"` messages stay contiguous — splitting them with a user
    // turn breaks OpenAI-Chat-compatible frontends that translate back to
    // Anthropic `tool_result` (notably Databricks model serving), since
    // Anthropic requires every `tool_use` in one assistant turn to be
    // answered by a single immediately-following user message.
    let mut pending_images: Vec<Value> = Vec::new();
    let flush_images = |messages: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if !pending.is_empty() {
            messages.push(json!({ "role": "user", "content": std::mem::take(pending) }));
        }
    };
    for item in history {
        match item {
            HistoryItem::User(text) => {
                flush_images(&mut messages, &mut pending_images);
                messages.push(json!({ "role": "user", "content": text }));
            }
            HistoryItem::Assistant { text, tool_calls } => {
                flush_images(&mut messages, &mut pending_images);
                let mut msg = serde_json::Map::new();
                msg.insert("role".into(), json!("assistant"));
                msg.insert("content".into(), json!(text.as_str()));
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                        "id": c.provider_id, "type": "function",
                        "function": { "name": c.name,
                            "arguments": serde_json::to_string(&c.arguments)
                                .unwrap_or_else(|_| "{}".into()) } })
                        })
                        .collect();
                    msg.insert("tool_calls".into(), Value::Array(calls));
                }
                messages.push(Value::Object(msg));
            }
            HistoryItem::ToolResult(r) => {
                messages.push(json!({
                    "role": "tool", "tool_call_id": r.provider_id,
                    "content": openai_tool_text_content(&r.content) }));
                pending_images.extend(openai_image_user_content(&r.content));
            }
        }
    }
    flush_images(&mut messages, &mut pending_images);
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
        "type": "function",
        "function": { "name": t.name, "description": t.description,
            "parameters": t.input_schema } })
        })
        .collect();
    let mut body = json!({ "model": cfg.model, "stream": false,
        "max_completion_tokens": cfg.max_output_tokens, "messages": messages });
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
        body["tool_choice"] = json!("auto");
    }
    body
}

fn openai_tool_text_content(content: &[ToolResultContent]) -> String {
    let mut parts = Vec::new();
    for c in content {
        match c {
            ToolResultContent::Text(text) => parts.push(text.clone()),
            ToolResultContent::Image { data, mime_type } => parts.push(format!(
                "This tool result included an image ({mime_type}, {} base64 bytes) that is provided in the next user message.",
                data.len()
            )),
        }
    }
    parts.join("\n")
}

fn openai_image_user_content(content: &[ToolResultContent]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|c| match c {
            ToolResultContent::Image { data, mime_type } => Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime_type};base64,{data}") },
            })),
            ToolResultContent::Text(_) => None,
        })
        .collect()
}

// Spec: https://platform.openai.com/docs/api-reference/responses
//
// Replay invariant: each assistant `function_call` input item **must**
// precede its matching `function_call_output`, or the API rejects with
// "No tool call found for call_id ...". `HistoryItem` ordering already
// guarantees this.

fn responses_body(
    cfg: &Config,
    system_prompt: &str,
    history: &[HistoryItem],
    tools: &[ToolDef],
) -> Value {
    let mut input: Vec<Value> = Vec::with_capacity(history.len());
    for item in history {
        match item {
            HistoryItem::User(text) => input.push(json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": text }],
            })),
            HistoryItem::Assistant { text, tool_calls } => {
                if !text.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                for c in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": c.provider_id,
                        "name": c.name,
                        "arguments": serde_json::to_string(&c.arguments)
                            .unwrap_or_else(|_| "{}".into()),
                    }));
                }
            }
            HistoryItem::ToolResult(r) => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": r.provider_id,
                    "output": openai_tool_text_content(&r.content),
                }));
                // Responses takes images as `input_image` parts on a user message.
                let images: Vec<Value> = r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ToolResultContent::Image { data, mime_type } => Some(json!({
                            "type": "input_image",
                            "image_url": format!("data:{mime_type};base64,{data}"),
                        })),
                        ToolResultContent::Text(_) => None,
                    })
                    .collect();
                if !images.is_empty() {
                    input.push(json!({ "role": "user", "content": images }));
                }
            }
        }
    }

    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            })
        })
        .collect();

    let mut body = json!({
        "model": cfg.model,
        "instructions": system_prompt,
        "max_output_tokens": cfg.max_output_tokens,
        "input": input,
    });
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
        body["tool_choice"] = json!("auto");
    }
    body
}

/// Narrow matcher for "you should be on the Responses API" provider errors,
/// the signal we use to auto-upgrade. Triggers on the literal path
/// `/v1/responses` (Databricks GPT-5.5 phrasing) or the prose
/// "use the Responses API" / "Responses API instead".
fn is_responses_required_error(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("/v1/responses")
        || b.contains("responses api instead")
        || b.contains("use the responses api")
}

fn databricks_v2_route_for_model(model: &str) -> DatabricksV2Route {
    // Databricks v2 catalog names currently identify OpenAI-shaped GPT-5
    // models and Anthropic-shaped Claude models by these substrings.
    let lower = model.to_ascii_lowercase();
    if lower.contains("gpt-5") || lower.contains("gpt5") {
        DatabricksV2Route::OpenAiResponses
    } else if lower.contains("claude") {
        DatabricksV2Route::AnthropicMessages
    } else {
        DatabricksV2Route::MlflowChatCompletions
    }
}

fn databricks_v2_path(route: DatabricksV2Route) -> &'static str {
    match route {
        DatabricksV2Route::OpenAiResponses => "/ai-gateway/openai/v1/responses",
        DatabricksV2Route::AnthropicMessages => "/ai-gateway/anthropic/v1/messages",
        DatabricksV2Route::MlflowChatCompletions => "/ai-gateway/mlflow/v1/chat/completions",
    }
}

fn parse_responses(v: Value) -> Result<LlmResponse, AgentError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut saw_function_call = false;

    for item in v
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for p in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    // Responses emits "output_text"; accept "text" forward-compat.
                    if matches!(
                        p.get("type").and_then(Value::as_str),
                        Some("output_text" | "text")
                    ) {
                        if let Some(t) = p.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                }
            }
            Some("function_call") => {
                saw_function_call = true;
                let raw = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let args: Value = serde_json::from_str(raw).map_err(|e| {
                    AgentError::Llm(format!("function_call.arguments not valid JSON: {e}"))
                })?;
                tool_calls.push(make_tool_call(
                    str_field(item, "call_id"),
                    str_field(item, "name"),
                    args,
                )?);
            }
            // Reasoning items are opaque/internal; we don't replay them.
            // Unknown types ignored for forward-compat.
            _ => {}
        }
    }

    let stop = match v.get("status").and_then(Value::as_str) {
        Some("incomplete") => {
            let reason = v
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str);
            if reason == Some("max_output_tokens") {
                ProviderStop::MaxTokens
            } else {
                ProviderStop::Other
            }
        }
        Some("completed") if saw_function_call => ProviderStop::ToolUse,
        Some("completed") => ProviderStop::EndTurn,
        _ => ProviderStop::Other,
    };
    let input_tokens = sum_usage(&v, &["input_tokens"]);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
    })
}

fn map_stop(s: Option<&str>) -> ProviderStop {
    match s {
        Some("end_turn" | "stop") => ProviderStop::EndTurn,
        Some("tool_use" | "tool_calls") => ProviderStop::ToolUse,
        Some("max_tokens" | "length") => ProviderStop::MaxTokens,
        Some("refusal" | "content_filter") => ProviderStop::Refusal,
        _ => ProviderStop::Other,
    }
}

/// Sum a set of `usage` token fields, returning `None` only when the `usage`
/// object is absent or carries none of the requested fields. A field that is
/// present is added; a field that is missing contributes 0. This keeps the
/// result an inclusive total (so cached tokens are never silently dropped)
/// while still distinguishing "no usage reported" from "usage was zero".
fn sum_usage(v: &Value, fields: &[&str]) -> Option<u64> {
    let usage = v.get("usage")?;
    let mut total: u64 = 0;
    let mut saw_any = false;
    for f in fields {
        if let Some(n) = usage.get(*f).and_then(Value::as_u64) {
            total = total.saturating_add(n);
            saw_any = true;
        }
    }
    saw_any.then_some(total)
}

/// Input-token total for Anthropic / Databricks (Anthropic-style) responses.
/// `input_tokens` alone EXCLUDES cached tokens, so we sum it with the two
/// cache fields to get the inclusive total the context budget must gate on.
fn anthropic_input_tokens(v: &Value) -> Option<u64> {
    sum_usage(
        v,
        &[
            "input_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ],
    )
}

/// Input-token total for OpenAI Chat Completions and Databricks responses.
/// OpenAI's `prompt_tokens` is already inclusive. Databricks uses the same
/// `prompt_tokens` wire field but ALSO reports Anthropic-style cache fields
/// alongside it, so we sum them; the cache fields are simply absent (and
/// contribute 0) for vanilla OpenAI.
fn openai_chat_input_tokens(v: &Value) -> Option<u64> {
    sum_usage(
        v,
        &[
            "prompt_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ],
    )
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn parse_anthropic(v: Value) -> Result<LlmResponse, AgentError> {
    let stop = map_stop(v.get("stop_reason").and_then(Value::as_str));
    let mut tool_calls = Vec::new();
    let mut text = String::new();
    if let Some(blocks) = v.get("content").and_then(Value::as_array) {
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => tool_calls.push(make_tool_call(
                    str_field(b, "id"),
                    str_field(b, "name"),
                    b.get("input").cloned().unwrap_or(Value::Null),
                )?),
                _ => {}
            }
        }
    }
    let input_tokens = anthropic_input_tokens(&v);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
    })
}

fn parse_openai(v: Value) -> Result<LlmResponse, AgentError> {
    let choice = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| AgentError::Llm("response missing choices".into()))?;
    let stop = map_stop(choice.get("finish_reason").and_then(Value::as_str));
    let msg = choice
        .get("message")
        .ok_or_else(|| AgentError::Llm("missing message".into()))?;
    let text = str_field(msg, "content");
    let mut tool_calls = Vec::new();
    if let Some(arr) = msg.get("tool_calls").and_then(Value::as_array) {
        for tc in arr {
            let f = tc
                .get("function")
                .ok_or_else(|| AgentError::Llm("tool_call missing function".into()))?;
            let raw = f.get("arguments").and_then(Value::as_str).unwrap_or("{}");
            let args: Value = serde_json::from_str(raw)
                .map_err(|e| AgentError::Llm(format!("tool_call.arguments not valid JSON: {e}")))?;
            tool_calls.push(make_tool_call(
                str_field(tc, "id"),
                str_field(f, "name"),
                args,
            )?);
        }
    }
    let input_tokens = openai_chat_input_tokens(&v);
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
        input_tokens,
    })
}

fn make_tool_call(id: String, name: String, args: Value) -> Result<ToolCall, AgentError> {
    if id.is_empty() || name.is_empty() {
        return Err(AgentError::Llm("tool_call missing id or name".into()));
    }
    let arguments = match args {
        Value::Object(_) => args,
        Value::Null => Value::Object(Default::default()),
        _ => {
            return Err(AgentError::Llm(
                "tool_call arguments must be a JSON object".into(),
            ))
        }
    };
    Ok(ToolCall {
        provider_id: id,
        name,
        arguments,
    })
}

async fn read_error_body(mut resp: reqwest::Response) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < MAX_LLM_ERROR_BODY_BYTES {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let take = chunk.len().min(MAX_LLM_ERROR_BODY_BYTES - buf.len());
                buf.extend_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    break;
                }
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

const MAX_RETRIES: u32 = 3;
const BASE_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 8_000;

async fn backoff_with_jitter(attempt: u32) {
    let base = BASE_BACKOFF_MS
        .saturating_mul(1u64 << attempt)
        .min(MAX_BACKOFF_MS);
    let mut buf = [0u8; 8];
    let jitter_range = base / 2;
    let delay = if jitter_range > 0 && getrandom::fill(&mut buf).is_ok() {
        let r = u64::from_le_bytes(buf) % jitter_range;
        base - jitter_range + r
    } else {
        base
    };
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
}

/// Transport-layer errors safe to retry for non-streaming LLM POSTs.
///
/// Covers timeouts, connect failures, and the broader request-class errors
/// reqwest reports for pre-response failures: TLS handshake aborts, sockets
/// dropped or reset mid-send, h2 GOAWAY/RST_STREAM, hyper protocol errors.
/// Body-serialization happens before the retry loop, so `is_request()` here
/// is always a network failure, never a malformed request we'd just resend.
fn is_retryable_transport_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
}

async fn post<F>(http: &Client, url: &str, body: &Value, apply: F) -> Result<Value, AgentError>
where
    F: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
{
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| AgentError::Llm(format!("serialize: {e}")))?;
    for attempt in 0..MAX_RETRIES {
        let resp = match apply(
            http.post(url)
                .header("content-type", "application/json")
                .body(body_bytes.clone()),
        )
        .send()
        .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        "llm: transport error, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(AgentError::Llm(format!("transport: {e}")));
            }
        };
        let status = resp.status();
        // Both 401 and 403 are treated as refreshable: a 403 can mean an
        // expired or revoked token, not just a pure authorization verdict, and
        // the two are indistinguishable at the HTTP-status layer. The caller's
        // retry loop keys off `LlmAuth` and refreshes once; the per-call guard
        // bounds a pure-authz 403 to one wasted refresh before it propagates.
        if status == 401 || status == 403 {
            return Err(AgentError::LlmAuth(read_error_body(resp).await));
        }
        if (status.is_server_error() || status == 429) && attempt + 1 < MAX_RETRIES {
            tracing::warn!(
                attempt = attempt + 1,
                max_attempts = MAX_RETRIES,
                %status,
                "llm: retryable status, retrying"
            );
            backoff_with_jitter(attempt).await;
            continue;
        }
        if !status.is_success() {
            return Err(AgentError::Llm(format!(
                "{status}: {}",
                read_error_body(resp).await
            )));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_LLM_RESPONSE_BYTES {
                return Err(AgentError::Llm(format!(
                    "response too large: {len} > {MAX_LLM_RESPONSE_BYTES}"
                )));
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp;
        loop {
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                        return Err(AgentError::Llm(format!(
                            "response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        )));
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => return Err(AgentError::Llm(format!("read: {e}"))),
            }
        }
        return serde_json::from_slice(&buf).map_err(|e| AgentError::Llm(format!("json: {e}")));
    }
    Err(AgentError::Llm("exhausted retries".into()))
}

/// Build the `TokenSource` for the configured provider.
///
/// - `Provider::Anthropic`: a static source seeded from `cfg.api_key`. It's
///   never read for Anthropic requests (those go through `post_anthropic` with
///   `x-api-key`), but Llm holds one to keep the field non-`Option`.
/// - `Provider::OpenAi`: a static source over `OPENAI_COMPAT_API_KEY`.
/// - `Provider::Databricks`: if `DATABRICKS_TOKEN` is set, a static source.
///   Otherwise a `PkceOAuthTokenSource` pointed at the workspace's OIDC
///   discovery URL. First request without a cached token triggers a browser
///   flow; subsequent requests use the cache + refresh transparently.
pub(crate) fn build_token_source(cfg: &Config) -> Result<Arc<dyn TokenSource>, AgentError> {
    match cfg.provider {
        Provider::Anthropic | Provider::OpenAi => {
            Ok(Arc::new(StaticTokenSource::new(cfg.api_key.clone())))
        }
        Provider::Databricks | Provider::DatabricksV2 => {
            if !cfg.api_key.is_empty() {
                return Ok(Arc::new(StaticTokenSource::new(cfg.api_key.clone())));
            }
            let discovery_url = format!(
                "{}/oidc/.well-known/oauth-authorization-server",
                cfg.base_url.trim_end_matches('/')
            );
            let pkce = PkceOAuthConfig {
                discovery_url,
                client_id: DATABRICKS_CLIENT_ID.into(),
                scopes: DATABRICKS_OAUTH_SCOPES
                    .iter()
                    .map(|s| (*s).into())
                    .collect(),
                cache_namespace: "databricks".into(),
                cache_dir_override: None,
            };
            Ok(PkceOAuthTokenSource::new(pkce)?)
        }
    }
}

/// Return a clone of `body` with any top-level `"model"` field removed.
/// Used for Databricks model-serving, which encodes the model in the URL
/// path and rejects the field in the body.
fn strip_model(body: &Value) -> Value {
    match body {
        Value::Object(map) => {
            let mut m = map.clone();
            m.remove("model");
            Value::Object(m)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, HookServers, OpenAiApi, Provider};
    use crate::types::{HistoryItem, ToolCall, ToolResult, ToolResultContent};
    use std::time::Duration;

    fn cfg(provider: Provider) -> Config {
        Config {
            provider,
            system_prompt: "system".into(),
            max_rounds: 10,
            max_output_tokens: 1024,
            llm_timeout: Duration::from_secs(10),
            tool_timeout: Duration::from_secs(10),
            mcp_init_timeout: Duration::from_secs(10),
            mcp_max_restart_attempts: 1,
            mcp_restart_base_ms: 1,
            mcp_restart_max_ms: 1,
            max_sessions: 1,
            max_line_bytes: 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
            max_tool_result_text_bytes: 50 * 1024,
            max_context_tokens: 200_000,
            max_handoffs: 1,
            max_parallel_tools: 1,
            hook_timeout: Duration::from_secs(1),
            stop_max_rejections: 0,
            hook_servers: HookServers::None,
            api_key: "key".into(),
            model: "model".into(),
            base_url: "http://example.invalid".into(),
            anthropic_api_version: "2023-06-01".into(),
            openai_api: OpenAiApi::Chat,
            hints_enabled: true,
        }
    }

    fn image_history() -> Vec<HistoryItem> {
        vec![
            HistoryItem::User("describe the image".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "toolu_1".into(),
                    name: "dev__view_image".into(),
                    arguments: serde_json::json!({"source":"x.png"}),
                }],
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_1".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from x.png)".into()),
                    ToolResultContent::Image {
                        data: "aW1n".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
        ]
    }

    #[test]
    fn anthropic_tool_result_preserves_image_block() {
        let body = anthropic_body(&cfg(Provider::Anthropic), "system", &image_history(), &[]);
        let content = &body["messages"][2]["content"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aW1n");
    }

    fn cfg_responses() -> Config {
        let mut c = cfg(Provider::OpenAi);
        c.openai_api = OpenAiApi::Responses;
        c
    }

    fn tool_call_history() -> Vec<HistoryItem> {
        vec![
            HistoryItem::User("call the tool".into()),
            HistoryItem::Assistant {
                text: "ok, calling".into(),
                tool_calls: vec![ToolCall {
                    provider_id: "call_abc".into(),
                    name: "dev__shell".into(),
                    arguments: serde_json::json!({"command": "ls"}),
                }],
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "call_abc".into(),
                content: vec![ToolResultContent::Text("file.txt".into())],
                is_error: false,
            }),
        ]
    }

    #[test]
    fn responses_body_top_level_shape() {
        let tools = vec![ToolDef {
            name: "dev__shell".into(),
            description: "run a shell command".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
            }),
        }];
        let body = responses_body(
            &cfg_responses(),
            "system",
            &[HistoryItem::User("hi".into())],
            &tools,
        );
        assert_eq!(body["model"], "model");
        assert_eq!(body["instructions"], "system");
        assert_eq!(body["max_output_tokens"], 1024);
        assert!(
            body.get("messages").is_none(),
            "must use `input`, not `messages`"
        );
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());

        // Tools are flat — top-level type/name/description/parameters.
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "dev__shell");
        assert!(
            tool.get("function").is_none(),
            "Responses tool schema is flat"
        );
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn responses_body_replay_emits_function_call_before_output() {
        // Replay requirement from the live API: the assistant's prior
        // function_call item *must* appear in `input[]` before its matching
        // function_call_output, otherwise the API rejects with
        // "No tool call found for call_id ...".
        let body = responses_body(&cfg_responses(), "system", &tool_call_history(), &[]);
        let input = body["input"].as_array().unwrap();

        // [0] user, [1] assistant text, [2] function_call, [3] function_call_output
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "call the tool");

        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["content"][0]["text"], "ok, calling");

        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_abc");
        assert_eq!(input[2]["name"], "dev__shell");
        // Arguments are a JSON-encoded string per spec.
        assert_eq!(input[2]["arguments"], "{\"command\":\"ls\"}");

        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_abc");
        assert_eq!(input[3]["output"], "file.txt");
    }

    #[test]
    fn responses_body_skips_empty_assistant_text() {
        // Mirrors the Chat Completions behavior (#559/#560): empty assistant
        // turns are skipped so we don't emit an empty `output_text` block,
        // but the tool_call(s) on that assistant turn still go through.
        let history = vec![
            HistoryItem::User("u".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "call_x".into(),
                    name: "t".into(),
                    arguments: serde_json::json!({}),
                }],
            },
        ];
        let body = responses_body(&cfg_responses(), "system", &history, &[]);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
    }

    #[test]
    fn responses_body_image_tool_result_attaches_input_image() {
        let body = responses_body(&cfg_responses(), "system", &image_history(), &[]);
        let input = body["input"].as_array().unwrap();
        // function_call_output carries the text part; image rides on a
        // trailing user message as `input_image`.
        let fco = input
            .iter()
            .find(|i| i["type"] == "function_call_output")
            .unwrap();
        assert_eq!(fco["call_id"], "toolu_1");
        let img_msg = input.iter().rev().find(|i| i["role"] == "user").unwrap();
        assert_eq!(img_msg["content"][0]["type"], "input_image");
        assert_eq!(
            img_msg["content"][0]["image_url"],
            "data:image/png;base64,aW1n"
        );
    }

    #[test]
    fn parse_responses_completed_with_text_is_end_turn() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hello"}],
            }],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.text, "hello");
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.stop, ProviderStop::EndTurn);
    }

    #[test]
    fn parse_responses_completed_with_function_call_is_tool_use() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": []},
                {
                    "type": "function_call",
                    "call_id": "call_z",
                    "name": "dev__shell",
                    "arguments": "{\"command\":\"ls\"}",
                },
            ],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.text, "");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].provider_id, "call_z");
        assert_eq!(r.tool_calls[0].name, "dev__shell");
        assert_eq!(
            r.tool_calls[0].arguments,
            serde_json::json!({"command": "ls"})
        );
        assert_eq!(r.stop, ProviderStop::ToolUse);
    }

    #[test]
    fn parse_responses_incomplete_max_output_tokens() {
        let v = serde_json::json!({
            "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [],
        });
        let r = parse_responses(v).unwrap();
        assert_eq!(r.stop, ProviderStop::MaxTokens);
    }

    #[test]
    fn is_responses_required_error_matrix() {
        for (body, want) in [
            // Databricks GPT-5.5 (the actual case we observed).
            ("Function tools with reasoning_effort are not supported for gpt-5.5 in /v1/chat/completions. Please use /v1/responses instead.", true),
            // Forward-compat: OpenAI saying the same thing in prose.
            ("This model requires the Responses API. Please use the Responses API instead.", true),
            // Negatives — must NOT trigger on unrelated 4xx.
            ("{\"error\":\"invalid_api_key\"}", false),
            ("max_tokens is not supported with this model", false),
            ("", false),
        ] {
            assert_eq!(is_responses_required_error(body), want, "body={body:?}");
        }
    }

    #[test]
    fn databricks_v2_routes_by_model_family() {
        for (model, route, path) in [
            (
                "databricks-gpt-5-5",
                DatabricksV2Route::OpenAiResponses,
                "/ai-gateway/openai/v1/responses",
            ),
            (
                "databricks-claude-opus-4-7",
                DatabricksV2Route::AnthropicMessages,
                "/ai-gateway/anthropic/v1/messages",
            ),
            (
                "custom-tool-model",
                DatabricksV2Route::MlflowChatCompletions,
                "/ai-gateway/mlflow/v1/chat/completions",
            ),
        ] {
            let got = databricks_v2_route_for_model(model);
            assert_eq!(got, route, "model={model}");
            assert_eq!(databricks_v2_path(got), path, "model={model}");
        }
    }

    #[test]
    fn parse_responses_rejects_malformed_function_arguments() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_z",
                "name": "t",
                "arguments": "not json {",
            }],
        });
        assert!(matches!(parse_responses(v), Err(AgentError::Llm(_))));
    }

    #[test]
    fn openai_tool_result_adds_followup_image_user_message() {
        let body = openai_body(&cfg(Provider::OpenAi), "system", &image_history(), &[]);
        assert_eq!(body["messages"][3]["role"], "tool");
        assert!(body["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("provided in the next user message"));
        assert_eq!(body["messages"][4]["role"], "user");
        assert_eq!(body["messages"][4]["content"][0]["type"], "image_url");
        assert_eq!(
            body["messages"][4]["content"][0]["image_url"]["url"],
            "data:image/png;base64,aW1n"
        );
    }

    /// Regression for Databricks model serving (and any OpenAI-Chat frontend
    /// that translates to Anthropic on the way to the model). Parallel tool
    /// calls where one or more return images previously produced an
    /// interleaved sequence:
    ///   role:"tool"  (A)
    ///   role:"user"  (image A)
    ///   role:"tool"  (B)
    ///   role:"user"  (image B)
    /// The intervening user message split the run of tool results, so the
    /// translator could no longer fold them into a single Anthropic
    /// `tool_result`-bearing user message — Anthropic then rejected the
    /// request with "tool_use ids were found without tool_result blocks
    /// immediately after". Fix: every `role:"tool"` for a run of adjacent
    /// ToolResults emits contiguously, then a single trailing user message
    /// carries all of the images from the batch.
    #[test]
    fn openai_parallel_image_tool_results_stay_contiguous() {
        let history = vec![
            HistoryItem::User("describe both images".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        provider_id: "toolu_a".into(),
                        name: "dev__view_image".into(),
                        arguments: serde_json::json!({"source": "a.png"}),
                    },
                    ToolCall {
                        provider_id: "toolu_b".into(),
                        name: "dev__view_image".into(),
                        arguments: serde_json::json!({"source": "b.png"}),
                    },
                ],
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_a".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from a.png)".into()),
                    ToolResultContent::Image {
                        data: "aaa".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
            HistoryItem::ToolResult(ToolResult {
                provider_id: "toolu_b".into(),
                content: vec![
                    ToolResultContent::Text("10×10, 70 B (image/png from b.png)".into()),
                    ToolResultContent::Image {
                        data: "bbb".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
        ];
        let body = openai_body(&cfg(Provider::OpenAi), "system", &history, &[]);
        let messages = body["messages"].as_array().unwrap();
        // [0] system, [1] user, [2] assistant(tool_calls), [3] tool A, [4] tool B, [5] user(images)
        assert_eq!(messages.len(), 6, "messages: {messages:#?}");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "toolu_a");
        assert_eq!(
            messages[4]["role"], "tool",
            "tool results must stay adjacent; intervening user message breaks Databricks/Anthropic pairing"
        );
        assert_eq!(messages[4]["tool_call_id"], "toolu_b");
        assert_eq!(messages[5]["role"], "user");
        let imgs = messages[5]["content"].as_array().unwrap();
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0]["image_url"]["url"], "data:image/png;base64,aaa");
        assert_eq!(imgs[1]["image_url"]["url"], "data:image/png;base64,bbb");
    }

    /// Regression: a connection that is accepted and then dropped before any
    /// HTTP response bytes are written surfaces as a reqwest request-class
    /// error (not `is_connect()`, not `is_timeout()`). The retry predicate
    /// must recognize it; otherwise transient TLS/h2/proxy hiccups bubble
    /// out of the agent as `transport: error sending request ...`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_retries_on_dropped_connection_before_response() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/x", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicU32::new(0));
        let accepts_srv = accepts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let n = accepts_srv.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    // First attempt: read the request, then drop the socket
                    // without writing a response. reqwest surfaces this as
                    // a request-class error (is_request() == true).
                    let mut tmp = [0u8; 4096];
                    let _ = sock.read(&mut tmp).await;
                    drop(sock);
                    continue;
                }
                // Subsequent attempts: serve a tiny JSON body.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                let body = "{\"ok\":true}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let out = post(&client, &url, &serde_json::json!({}), |b| b)
            .await
            .expect("post should succeed after retry");
        assert_eq!(out, serde_json::json!({ "ok": true }));
        assert!(
            accepts.load(Ordering::SeqCst) >= 2,
            "server should have seen at least 2 connection attempts, saw {}",
            accepts.load(Ordering::SeqCst)
        );
    }

    // ---- usage / input-token extraction -------------------------------------

    #[test]
    fn parse_anthropic_sums_input_and_cache_tokens() {
        // input_tokens alone excludes cached tokens; the inclusive total must
        // sum all three so a cache-heavy turn can't undercount the budget.
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {
                "input_tokens": 100,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 50,
                "output_tokens": 7
            }
        });
        let r = parse_anthropic(v).unwrap();
        assert_eq!(r.input_tokens, Some(1050));
    }

    #[test]
    fn parse_anthropic_input_tokens_only() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}],
            "usage": {"input_tokens": 42, "output_tokens": 3}
        });
        assert_eq!(parse_anthropic(v).unwrap().input_tokens, Some(42));
    }

    #[test]
    fn parse_anthropic_missing_usage_is_none() {
        let v = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}]
        });
        assert_eq!(parse_anthropic(v).unwrap().input_tokens, None);
    }

    #[test]
    fn parse_openai_uses_prompt_tokens() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 123, "completion_tokens": 4, "total_tokens": 127}
        });
        assert_eq!(parse_openai(v).unwrap().input_tokens, Some(123));
    }

    #[test]
    fn parse_openai_databricks_sums_cache_fields() {
        // Databricks uses the OpenAI chat wire format (prompt_tokens) but also
        // reports Anthropic-style cache fields; the inclusive total sums them.
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}],
            "usage": {
                "prompt_tokens": 200,
                "completion_tokens": 4,
                "total_tokens": 204,
                "cache_read_input_tokens": 800,
                "cache_creation_input_tokens": 0
            }
        });
        assert_eq!(parse_openai(v).unwrap().input_tokens, Some(1000));
    }

    #[test]
    fn parse_openai_missing_usage_is_none() {
        let v = serde_json::json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "hi"}}]
        });
        assert_eq!(parse_openai(v).unwrap().input_tokens, None);
    }

    #[test]
    fn parse_responses_uses_input_tokens() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }],
            "usage": {"input_tokens": 321, "output_tokens": 9, "total_tokens": 330}
        });
        assert_eq!(parse_responses(v).unwrap().input_tokens, Some(321));
    }

    #[test]
    fn parse_responses_missing_usage_is_none() {
        let v = serde_json::json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }]
        });
        assert_eq!(parse_responses(v).unwrap().input_tokens, None);
    }

    #[test]
    fn sum_usage_empty_object_is_none() {
        // A `usage` object present but carrying none of the requested fields
        // is "no usable reading" -> None, not Some(0).
        let v = serde_json::json!({"usage": {"output_tokens": 5}});
        assert_eq!(sum_usage(&v, &["input_tokens", "prompt_tokens"]), None);
    }

    /// A token source whose `bearer()` always hands back the same stale
    /// token and whose `refresh_now()` mints a distinct fresh one, counting
    /// each refresh. Lets a test assert exactly how many forced refreshes a
    /// `post_openai` call provoked.
    struct CountingAuth {
        refreshes: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl TokenSource for CountingAuth {
        async fn bearer(&self) -> Result<String, AgentError> {
            Ok("stale".into())
        }
        async fn refresh_now(&self, _rejected: &str) -> Result<String, AgentError> {
            self.refreshes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("fresh".into())
        }
    }

    /// Stub that answers `reject_status` to any request carrying `Bearer
    /// stale` and 200 to `Bearer fresh`. When `always_reject` is set it rejects
    /// unconditionally, simulating a token the refresh can never satisfy.
    /// Counts requests so a test can assert "one retry, not a loop".
    async fn spawn_auth_stub(
        always_reject: std::sync::Arc<std::sync::atomic::AtomicBool>,
        reject_status: u16,
    ) -> String {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let reject_line = match reject_status {
            401 => "401 Unauthorized",
            403 => "403 Forbidden",
            other => panic!("unsupported reject_status {other}"),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let always_reject = always_reject.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(k) => buf.extend_from_slice(&tmp[..k]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                    let stale = head.contains("authorization: bearer stale");
                    let resp = if always_reject.load(Ordering::SeqCst) || stale {
                        format!(
                            "HTTP/1.1 {reject_line}\r\nContent-Length: 11\r\n\
                             Connection: close\r\n\r\ntoken stale"
                        )
                    } else {
                        let body = "{\"ok\":true}";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body,
                        )
                    };
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        url
    }

    fn llm_with(auth: Arc<dyn TokenSource>) -> Llm {
        Llm {
            http: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            auto_upgraded: std::sync::atomic::AtomicBool::new(false),
            auth,
        }
    }

    /// A single 401 forces exactly one refresh, the retry with the fresh
    /// token succeeds, and a *later* call gets its own refresh — proving the
    /// one-shot guard is per-call, not stored on the source.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_refreshes_once_per_call_on_401() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_401 = Arc::new(AtomicBool::new(false));
        let base = spawn_auth_stub(always_401, 401).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let out = llm
            .post_openai(&c, "/v1/x", &json!({}))
            .await
            .expect("retry with fresh token should succeed");
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1, "one refresh");

        // Second call's 401 must trigger its own refresh — the guard cannot
        // be a stored flag that an earlier turn already tripped.
        let out2 = llm.post_openai(&c, "/v1/x", &json!({})).await.unwrap();
        assert_eq!(out2, json!({ "ok": true }));
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            2,
            "later call gets its own retry"
        );
    }

    /// A persistent 401 (even the refreshed token is rejected) propagates as
    /// `LlmAuth` after exactly one refresh — no infinite loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_persistent_401_propagates_after_one_retry() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_401 = Arc::new(AtomicBool::new(true));
        let base = spawn_auth_stub(always_401, 401).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let err = llm.post_openai(&c, "/v1/x", &json!({})).await.unwrap_err();
        assert!(matches!(err, AgentError::LlmAuth(_)), "got {err:?}");
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            1,
            "exactly one refresh, then propagate"
        );
    }

    /// A 403 is treated as refreshable: a persistent 403 forces exactly one
    /// refresh-and-retry, then propagates as `LlmAuth`. Proves a revoked-token
    /// 403 takes the same recovery path as a 401, bounded by the per-call guard.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_persistent_403_propagates_after_one_retry() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_403 = Arc::new(AtomicBool::new(true));
        let base = spawn_auth_stub(always_403, 403).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let err = llm.post_openai(&c, "/v1/x", &json!({})).await.unwrap_err();
        assert!(matches!(err, AgentError::LlmAuth(_)), "got {err:?}");
        assert_eq!(
            auth.refreshes.load(Ordering::SeqCst),
            1,
            "403 refreshes exactly once, then propagates"
        );
    }

    /// A recoverable 403 (stale token 403s, fresh token 200s) forces exactly
    /// one refresh and the retry succeeds — proving a 403 enters the refresh
    /// path and a refreshed token clears it, the stale-token-403 recovery case.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_openai_refreshes_once_on_403() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        let always_403 = Arc::new(AtomicBool::new(false));
        let base = spawn_auth_stub(always_403, 403).await;
        let auth = Arc::new(CountingAuth {
            refreshes: AtomicU32::new(0),
        });
        let llm = llm_with(auth.clone());
        let mut c = cfg(Provider::OpenAi);
        c.base_url = base;

        let out = llm
            .post_openai(&c, "/v1/x", &json!({}))
            .await
            .expect("retry with fresh token should clear the 403");
        assert_eq!(out, json!({ "ok": true }));
        assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1, "one refresh");
    }

    /// The default `refresh_now()` on a static source returns the static
    /// token unchanged — a key that can't refresh still answers harmlessly.
    #[tokio::test]
    async fn static_token_source_refresh_now_returns_static_token() {
        let src = StaticTokenSource::new("static-key");
        assert_eq!(src.refresh_now("rejected").await.unwrap(), "static-key");
    }
}
