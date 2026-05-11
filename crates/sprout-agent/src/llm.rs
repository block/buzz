use reqwest::Client;
use serde_json::{json, Value};

use crate::config::{Config, Provider};
use crate::types::{AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef};

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
                        .header("content-type", "application/json")
                })
                .await?;
                parse_anthropic(v)
            }
            Provider::OpenAi => {
                let body = openai_body(cfg, history, tools);
                let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| {
                    r.bearer_auth(&cfg.api_key)
                        .header("content-type", "application/json")
                })
                .await?;
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
                        .header("content-type", "application/json")
                })
                .await?;
                Ok(parse_anthropic(v)?.text)
            }
            Provider::OpenAi => {
                let body = json!({
                    "model": cfg.model,
                    "stream": false,
                    "max_tokens": max_output_tokens,
                    "messages": [
                        { "role": "system", "content": system_prompt },
                        { "role": "user", "content": user_prompt },
                    ],
                });
                let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
                let v = post(&self.http, &url, &body, |r| {
                    r.bearer_auth(&cfg.api_key)
                        .header("content-type", "application/json")
                })
                .await?;
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
                    content.push(json!({ "type": "text", "text": "" }));
                }
                messages.push(json!({ "role": "assistant", "content": content }));
            }
            HistoryItem::ToolResult(r) => pending.push(json!({
                "type": "tool_result", "tool_use_id": r.provider_id,
                "content": [{ "type": "text", "text": r.text }], "is_error": r.is_error })),
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
            HistoryItem::ToolResult(r) => messages.push(json!({
                "role": "tool", "tool_call_id": r.provider_id, "content": r.text })),
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
    let mut body = json!({ "model": cfg.model, "stream": false, "max_tokens": cfg.max_output_tokens,
        "messages": messages });
    if !tools_json.is_empty() {
        body["tools"] = Value::Array(tools_json);
        body["tool_choice"] = json!("auto");
    }
    body
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
    for attempt in 0..MAX_RETRIES {
        let resp = match apply(http.post(url).json(body)).send().await {
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
