use reqwest::Client;
use serde_json::{json, Value};

use crate::config::{Config, Provider};
use crate::types::{
    AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef, ToolResultContent,
};

const MAX_LLM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LLM_ERROR_BODY_BYTES: usize = 4 * 1024;

pub struct Llm {
    http: Client,
}

impl Llm {
    pub fn new(cfg: &Config) -> Result<Self, AgentError> {
        let http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(cfg.llm_timeout)
            .build()
            .map_err(|e| AgentError::Llm(format!("http: {e}")))?;
        Ok(Self { http })
    }

    pub async fn complete(
        &self,
        cfg: &Config,
        history: &[HistoryItem],
        tools: &[ToolDef],
    ) -> Result<LlmResponse, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let body = anthropic_body(cfg, history, tools);
                let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| {
                    r.header("x-api-key", &cfg.api_key)
                        .header("anthropic-version", &cfg.anthropic_api_version)
                })
                .await?;
                parse_anthropic(v)
            }
            Provider::OpenAi => {
                let body = openai_body(cfg, history, tools);
                let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| r.bearer_auth(&cfg.api_key)).await?;
                parse_openai(v)
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
                let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| {
                    r.header("x-api-key", &cfg.api_key)
                        .header("anthropic-version", &cfg.anthropic_api_version)
                })
                .await?;
                Ok(parse_anthropic(v)?.text)
            }
            Provider::OpenAi => {
                let body = json!({
                    "model": cfg.model,
                    "stream": false,
                    "max_completion_tokens": max_output_tokens,
                    "messages": [
                        { "role": "system", "content": system_prompt },
                        { "role": "user", "content": user_prompt },
                    ],
                });
                let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| r.bearer_auth(&cfg.api_key)).await?;
                Ok(parse_openai(v)?.text)
            }
        }
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
    use crate::config::{Config, HookServers, Provider};
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
