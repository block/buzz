use std::sync::atomic::{AtomicBool, Ordering};

use reqwest::Client;
use serde_json::{json, Value};

use crate::config::{is_openai_host, Config, OpenAiApi, Provider};
use crate::types::{
    AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef, ToolResultContent,
};

const MAX_LLM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LLM_ERROR_BODY_BYTES: usize = 4 * 1024;

/// Parser for an OpenAI-family JSON response. Per-endpoint pair lives
/// alongside its `_body` serializer.
type OpenAiParse = fn(Value) -> Result<LlmResponse, AgentError>;

pub struct Llm {
    http: Client,
    /// One-shot sticky flag: set when a Chat Completions request comes
    /// back with a "use /v1/responses" provider error while `cfg.openai_api
    /// == Auto`. Subsequent OpenAI calls then go straight to Responses
    /// for the lifetime of the process.
    auto_upgraded: AtomicBool,
}

impl Llm {
    pub fn new(cfg: &Config) -> Result<Self, AgentError> {
        let http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(cfg.llm_timeout)
            .build()
            .map_err(|e| AgentError::Llm(format!("http: {e}")))?;
        Ok(Self {
            http,
            auto_upgraded: AtomicBool::new(false),
        })
    }

    pub async fn complete(
        &self,
        cfg: &Config,
        history: &[HistoryItem],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let v = self
                    .post_anthropic(cfg, &anthropic_body(cfg, history, tools))
                    .await?;
                parse_anthropic(v)
            }
            Provider::OpenAi => {
                self.openai_request(cfg, |use_responses| {
                    if use_responses {
                        (
                            responses_body(cfg, history, tools),
                            parse_responses as OpenAiParse,
                        )
                    } else {
                        (
                            openai_body(cfg, history, tools),
                            parse_openai as OpenAiParse,
                        )
                    }
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
            Provider::OpenAi => {
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

    async fn post_openai(
        &self,
        cfg: &Config,
        path: &str,
        body: &Value,
    ) -> Result<Value, AgentError> {
        let url = format!("{}{}", cfg.base_url.trim_end_matches('/'), path);
        post(&self.http, &url, body, |r| r.bearer_auth(&cfg.api_key)).await
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

fn anthropic_body(cfg: &Config, history: &[HistoryItem], tools: &[ToolDef]) -> Value {
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
        "system": cfg.system_prompt, "messages": messages });
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

fn openai_body(cfg: &Config, history: &[HistoryItem], tools: &[ToolDef]) -> Value {
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": cfg.system_prompt })];
    for item in history {
        match item {
            HistoryItem::User(text) => messages.push(json!({ "role": "user", "content": text })),
            HistoryItem::Assistant { text, tool_calls } => {
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
                let image_content = openai_image_user_content(&r.content);
                if !image_content.is_empty() {
                    messages.push(json!({ "role": "user", "content": image_content }));
                }
            }
        }
    }
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

// ── OpenAI Responses API ───────────────────────────────────────────────────
// Spec: https://platform.openai.com/docs/api-reference/responses
//
// Replay invariant: each assistant `function_call` input item **must**
// precede its matching `function_call_output`, or the API rejects with
// "No tool call found for call_id ...". `HistoryItem` ordering already
// guarantees this.

fn responses_body(cfg: &Config, history: &[HistoryItem], tools: &[ToolDef]) -> Value {
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
        "instructions": cfg.system_prompt,
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
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
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
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
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
    Ok(LlmResponse {
        text,
        tool_calls,
        stop,
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
    let delay = if jitter_range > 0 && getrandom::getrandom(&mut buf).is_ok() {
        let r = u64::from_le_bytes(buf) % jitter_range;
        base - jitter_range + r
    } else {
        base
    };
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
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
                if attempt + 1 < MAX_RETRIES && (e.is_timeout() || e.is_connect()) {
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(AgentError::Llm(format!("transport: {e}")));
            }
        };
        let status = resp.status();
        if status == 401 || status == 403 {
            return Err(AgentError::LlmAuth(read_error_body(resp).await));
        }
        if (status.is_server_error() || status == 429) && attempt + 1 < MAX_RETRIES {
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
        let body = anthropic_body(&cfg(Provider::Anthropic), &image_history(), &[]);
        let content = &body["messages"][2]["content"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aW1n");
    }

    // ── Responses API unit tests ───────────────────────────────────────

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
        let body = responses_body(&cfg_responses(), &[HistoryItem::User("hi".into())], &tools);
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
        let body = responses_body(&cfg_responses(), &tool_call_history(), &[]);
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
        let body = responses_body(&cfg_responses(), &history, &[]);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
    }

    #[test]
    fn responses_body_image_tool_result_attaches_input_image() {
        let body = responses_body(&cfg_responses(), &image_history(), &[]);
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
        let body = openai_body(&cfg(Provider::OpenAi), &image_history(), &[]);
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
}
