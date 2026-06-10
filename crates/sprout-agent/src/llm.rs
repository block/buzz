use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::Client;
use serde_json::{json, Value};

use crate::auth::{PkceOAuthConfig, PkceOAuthTokenSource, StaticTokenSource, TokenSource};
use crate::config::{is_openai_host, Config, OpenAiApi, Provider};
use crate::types::{
    AgentError, HistoryItem, LlmResponse, ProviderStop, ToolCall, ToolDef, ToolResultContent,
};
use crate::wire::{self, WireSender};

/// Databricks OAuth client_id — the public Databricks-published CLI client.
/// PKCE-only, no secret. Same identifier goose uses, so a user's browser
/// consent for `databricks-cli` covers sprout-agent too.
const DATABRICKS_CLIENT_ID: &str = "databricks-cli";
const DATABRICKS_OAUTH_SCOPES: &[&str] = &["all-apis", "offline_access"];

const MAX_LLM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LLM_ERROR_BODY_BYTES: usize = 4 * 1024;

/// Parser for an OpenAI-family JSON response. Per-endpoint pair lives
/// alongside its `_body` serializer.
type OpenAiParse = fn(Value) -> Result<LlmResponse, AgentError>;

pub struct Llm {
    http: Client,
    /// Streaming client — no global timeout so long-running SSE streams
    /// survive. First-byte and inter-chunk timeouts are enforced externally
    /// via `tokio::time::timeout`.
    http_stream: Client,
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
    /// Max gap allowed between response-body chunks. Enforced per
    /// `chunk()` poll rather than as a total request deadline, so a
    /// slow-but-progressing completion survives arbitrarily long while a
    /// connection that goes silent mid-body is torn down promptly.
    chunk_timeout: std::time::Duration,
    /// Inter-chunk timeout for SSE streaming after the first content delta
    /// has arrived. Default 30s, configurable via
    /// `SPROUT_AGENT_STREAM_CHUNK_TIMEOUT_SECS`.
    stream_chunk_timeout: std::time::Duration,
    /// Timeout from stream-open until the first text or tool delta. Uses
    /// `llm_timeout` (120s default) to accommodate reasoning models that
    /// pause before producing content.
    first_byte_timeout: std::time::Duration,
}

impl Llm {
    pub fn new(cfg: &Config) -> Result<Self, AgentError> {
        let http = Client::builder()
            // Fixed short connect timeout: a dead/unroutable endpoint must
            // fail fast at the TCP/TLS handshake, independent of llm_timeout
            // (which governs in-flight inter-chunk stalls, a different concern).
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AgentError::Llm(format!("http: {e}")))?;
        // Streaming client has no global timeout — SSE streams can run
        // indefinitely. Timeouts are enforced per-event via tokio::time::timeout.
        let http_stream = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AgentError::Llm(format!("http_stream: {e}")))?;
        let auth = build_token_source(cfg)?;
        Ok(Self {
            http,
            http_stream,
            auto_upgraded: AtomicBool::new(false),
            auth,
            chunk_timeout: cfg.llm_body_chunk_timeout,
            stream_chunk_timeout: cfg.stream_chunk_timeout,
            first_byte_timeout: cfg.llm_timeout,
        })
    }

    /// Non-streaming completion — kept as fallback for providers/scenarios
    /// where SSE streaming is unavailable or undesirable.
    #[allow(dead_code)]
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
        }
    }

    async fn post_anthropic(&self, cfg: &Config, body: &Value) -> Result<Value, AgentError> {
        let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
        post(&self.http, &url, body, self.chunk_timeout, |r| {
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
        let bearer = self.auth.bearer().await?;
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
        post(&self.http, &url, body_ref, self.chunk_timeout, |r| {
            r.bearer_auth(&bearer)
        })
        .await
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

    // ── SSE Streaming ────────────────────────────────────────────────────────

    /// Stream a completion, emitting text deltas as `agent_message_chunk`
    /// session updates and accumulating tool-call arguments per content-block
    /// index.
    pub async fn complete_stream(
        &self,
        cfg: &Config,
        system_prompt: &str,
        history: &[HistoryItem],
        tools: &[ToolDef],
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        match cfg.provider {
            Provider::Anthropic => {
                let mut body = anthropic_body(cfg, system_prompt, history, tools);
                body["stream"] = json!(true);
                self.post_stream_anthropic(cfg, &body, emitter).await
            }
            Provider::OpenAi | Provider::Databricks => {
                self.openai_stream_request(cfg, system_prompt, history, tools, emitter)
                    .await
            }
        }
    }

    async fn post_stream_anthropic(
        &self,
        cfg: &Config,
        body: &Value,
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        let url = format!("{}/v1/messages", cfg.base_url.trim_end_matches('/'));
        let body_bytes =
            serde_json::to_vec(body).map_err(|e| AgentError::Llm(format!("serialize: {e}")))?;
        let resp = self
            .send_stream_with_retry(|| {
                self.http_stream
                    .post(&url)
                    .header("content-type", "application/json")
                    .header("x-api-key", &cfg.api_key)
                    .header("anthropic-version", &cfg.anthropic_api_version)
                    .body(body_bytes.clone())
            })
            .await?;
        self.consume_sse_anthropic(resp, emitter).await
    }

    /// Open an SSE stream with the buffered path's retry semantics applied to
    /// the *initial* request only. Once this returns Ok, the caller begins
    /// consuming the stream and emitting chunks; retrying past that point would
    /// duplicate already-emitted output, so the retry window stops here.
    /// Transport errors, 5xx, and 429 retry with backoff; auth and other
    /// non-success statuses surface immediately.
    async fn send_stream_with_retry<F>(&self, build: F) -> Result<reqwest::Response, AgentError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        for attempt in 0..MAX_RETRIES {
            let resp = match build().send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = MAX_RETRIES,
                            error = %e,
                            "llm: stream transport error, retrying"
                        );
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
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = MAX_RETRIES,
                    %status,
                    "llm: stream retryable status, retrying"
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
            return Ok(resp);
        }
        Err(AgentError::Llm("exhausted retries".into()))
    }

    async fn consume_sse_anthropic(
        &self,
        resp: reqwest::Response,
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        let mut text = String::new();
        let mut tool_blocks: HashMap<usize, (String, String)> = HashMap::new();
        let mut stop = ProviderStop::Other;
        let mut input_tokens: Option<u64> = None;
        let mut saw_content_delta = false;
        let mut accumulated_bytes: usize = 0;

        let mut sse_reader = SseReader::new(resp);
        loop {
            let timeout = if saw_content_delta {
                self.stream_chunk_timeout
            } else {
                self.first_byte_timeout
            };
            let event = match tokio::time::timeout(timeout, sse_reader.next_event()).await {
                Ok(Ok(Some(ev))) => ev,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(AgentError::Llm(format!(
                        "stream stalled: no SSE event within {}s",
                        timeout.as_secs()
                    )));
                }
            };
            let data: Value = match serde_json::from_str(&event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event_type = data.get("type").and_then(Value::as_str).unwrap_or("");
            match event_type {
                "content_block_start" => {
                    let idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let cb = &data["content_block"];
                    if cb.get("type").and_then(Value::as_str) == Some("tool_use") {
                        let id = str_field(cb, "id");
                        let name = str_field(cb, "name");
                        tool_blocks.insert(idx, (format!("{id}:{name}"), String::new()));
                    }
                }
                "content_block_delta" => {
                    saw_content_delta = true;
                    let idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let delta = &data["delta"];
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(t) = delta.get("text").and_then(Value::as_str) {
                                accumulated_bytes += t.len();
                                if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                                    return Err(AgentError::Llm(format!(
                                        "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                                    )));
                                }
                                text.push_str(t);
                                emitter.emit_chunk(t).await;
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            {
                                accumulated_bytes += partial.len();
                                if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                                    return Err(AgentError::Llm(format!(
                                        "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                                    )));
                                }
                                if let Some(entry) = tool_blocks.get_mut(&idx) {
                                    entry.1.push_str(partial);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                "message_start" => {
                    if let Some(usage) = data.get("message").and_then(|m| m.get("usage")) {
                        input_tokens = anthropic_usage_sum(usage);
                    }
                }
                "message_delta" => {
                    if let Some(sr) = data["delta"].get("stop_reason").and_then(Value::as_str) {
                        stop = map_stop(Some(sr));
                    }
                    if let Some(usage) = data.get("usage") {
                        if let Some(t) = anthropic_usage_sum(usage) {
                            input_tokens = Some(t);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut tool_calls = Vec::new();
        let mut indices: Vec<usize> = tool_blocks.keys().copied().collect();
        indices.sort_unstable();
        for idx in indices {
            let (id_name, args_str) = tool_blocks.remove(&idx).unwrap();
            let (provider_id, name) = id_name
                .split_once(':')
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
                .unwrap_or((String::new(), id_name));
            let arguments: Value =
                serde_json::from_str(&args_str).unwrap_or(Value::Object(Default::default()));
            tool_calls.push(make_tool_call(provider_id, name, arguments)?);
        }

        if !tool_calls.is_empty() && stop == ProviderStop::Other {
            stop = ProviderStop::ToolUse;
        }
        if tool_calls.is_empty() && stop == ProviderStop::Other {
            stop = ProviderStop::EndTurn;
        }

        Ok(LlmResponse {
            text,
            tool_calls,
            stop,
            input_tokens,
        })
    }

    async fn openai_stream_request(
        &self,
        cfg: &Config,
        system_prompt: &str,
        history: &[HistoryItem],
        tools: &[ToolDef],
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        let use_responses = self.auto_upgraded.load(Ordering::Relaxed)
            || matches!(cfg.openai_api, OpenAiApi::Responses)
            || (matches!(cfg.openai_api, OpenAiApi::Auto) && is_openai_host(&cfg.base_url));

        if use_responses {
            let mut body = responses_body(cfg, system_prompt, history, tools);
            body["stream"] = json!(true);
            let resp = self.send_openai_stream(cfg, "/responses", &body).await?;
            return self.consume_sse_responses(resp, emitter).await;
        }

        let mut body = openai_body(cfg, system_prompt, history, tools);
        body["stream"] = json!(true);
        match self
            .send_openai_stream(cfg, "/chat/completions", &body)
            .await
        {
            Ok(resp) => self.consume_sse_openai_chat(resp, emitter).await,
            Err(e) if cfg.openai_api == OpenAiApi::Auto && self.try_upgrade(&e) => {
                let mut body = responses_body(cfg, system_prompt, history, tools);
                body["stream"] = json!(true);
                let resp = self.send_openai_stream(cfg, "/responses", &body).await?;
                self.consume_sse_responses(resp, emitter).await
            }
            Err(e) => Err(e),
        }
    }

    async fn send_openai_stream(
        &self,
        cfg: &Config,
        path: &str,
        body: &Value,
    ) -> Result<reqwest::Response, AgentError> {
        let bearer = self.auth.bearer().await?;
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
        let body_bytes =
            serde_json::to_vec(body_ref).map_err(|e| AgentError::Llm(format!("serialize: {e}")))?;
        self.send_stream_with_retry(|| {
            self.http_stream
                .post(&url)
                .header("content-type", "application/json")
                .bearer_auth(&bearer)
                .body(body_bytes.clone())
        })
        .await
    }

    async fn consume_sse_openai_chat(
        &self,
        resp: reqwest::Response,
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        let mut text = String::new();
        let mut tool_calls_acc: Vec<(String, String, String)> = Vec::new();
        let mut stop = ProviderStop::Other;
        let mut input_tokens: Option<u64> = None;
        let mut saw_content_delta = false;
        let mut accumulated_bytes: usize = 0;

        let mut sse_reader = SseReader::new(resp);
        loop {
            let timeout = if saw_content_delta {
                self.stream_chunk_timeout
            } else {
                self.first_byte_timeout
            };
            let event = match tokio::time::timeout(timeout, sse_reader.next_event()).await {
                Ok(Ok(Some(ev))) => ev,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(AgentError::Llm(format!(
                        "stream stalled: no SSE event within {}s",
                        timeout.as_secs()
                    )));
                }
            };
            if event == "[DONE]" {
                break;
            }
            let data: Value = match serde_json::from_str(&event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(usage) = data.get("usage") {
                if let Some(pt) = usage.get("prompt_tokens").and_then(Value::as_u64) {
                    let mut total = pt;
                    for f in ["cache_read_input_tokens", "cache_creation_input_tokens"] {
                        if let Some(n) = usage.get(f).and_then(Value::as_u64) {
                            total = total.saturating_add(n);
                        }
                    }
                    input_tokens = Some(total);
                }
            }
            let choice = match data
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
            {
                Some(c) => c,
                None => continue,
            };
            if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                stop = map_stop(Some(fr));
            }
            let delta = &choice["delta"];
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                if !t.is_empty() {
                    saw_content_delta = true;
                    accumulated_bytes += t.len();
                    if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                        return Err(AgentError::Llm(format!(
                            "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        )));
                    }
                    text.push_str(t);
                    emitter.emit_chunk(t).await;
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                saw_content_delta = true;
                for tc in tcs {
                    let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    while tool_calls_acc.len() <= idx {
                        tool_calls_acc.push((String::new(), String::new(), String::new()));
                    }
                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                        tool_calls_acc[idx].0 = id.to_owned();
                    }
                    if let Some(f) = tc.get("function") {
                        if let Some(name) = f.get("name").and_then(Value::as_str) {
                            tool_calls_acc[idx].1 = name.to_owned();
                        }
                        if let Some(args) = f.get("arguments").and_then(Value::as_str) {
                            accumulated_bytes += args.len();
                            if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                                return Err(AgentError::Llm(format!(
                                    "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                                )));
                            }
                            tool_calls_acc[idx].2.push_str(args);
                        }
                    }
                }
            }
        }

        let mut tool_calls = Vec::new();
        for (id, name, args_str) in tool_calls_acc {
            if id.is_empty() && name.is_empty() {
                continue;
            }
            let arguments: Value =
                serde_json::from_str(&args_str).unwrap_or(Value::Object(Default::default()));
            tool_calls.push(make_tool_call(id, name, arguments)?);
        }

        Ok(LlmResponse {
            text,
            tool_calls,
            stop,
            input_tokens,
        })
    }

    async fn consume_sse_responses(
        &self,
        resp: reqwest::Response,
        emitter: &StreamEmitter,
    ) -> Result<LlmResponse, AgentError> {
        let mut text = String::new();
        // Insertion-ordered accumulator keyed by call_id. A HashMap here would
        // yield non-deterministic tool-call ordering when a model returns
        // several calls; the buffered Responses parser preserves the JSON array
        // order, so the streaming path matches it via first-seen ordering.
        let mut tool_calls_acc: Vec<(String, String, String)> = Vec::new();
        let mut stop = ProviderStop::Other;
        let mut input_tokens: Option<u64> = None;
        let mut saw_content_delta = false;
        let mut saw_function_call = false;
        let mut accumulated_bytes: usize = 0;

        let mut sse_reader = SseReader::new(resp);
        loop {
            let timeout = if saw_content_delta {
                self.stream_chunk_timeout
            } else {
                self.first_byte_timeout
            };
            let event = match tokio::time::timeout(timeout, sse_reader.next_event()).await {
                Ok(Ok(Some(ev))) => ev,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(AgentError::Llm(format!(
                        "stream stalled: no SSE event within {}s",
                        timeout.as_secs()
                    )));
                }
            };
            let data: Value = match serde_json::from_str(&event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event_type = data.get("type").and_then(Value::as_str).unwrap_or("");
            match event_type {
                "response.output_text.delta" => {
                    saw_content_delta = true;
                    if let Some(t) = data.get("delta").and_then(Value::as_str) {
                        accumulated_bytes += t.len();
                        if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                            return Err(AgentError::Llm(format!(
                                "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                            )));
                        }
                        text.push_str(t);
                        emitter.emit_chunk(t).await;
                    }
                }
                "response.function_call_arguments.delta" => {
                    saw_content_delta = true;
                    saw_function_call = true;
                    let call_id = str_field(&data, "call_id");
                    if let Some(d) = data.get("delta").and_then(Value::as_str) {
                        accumulated_bytes += d.len();
                        if accumulated_bytes > MAX_LLM_RESPONSE_BYTES {
                            return Err(AgentError::Llm(format!(
                                "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                            )));
                        }
                        match tool_calls_acc.iter_mut().find(|(id, _, _)| *id == call_id) {
                            Some(entry) => entry.2.push_str(d),
                            None => tool_calls_acc.push((call_id, String::new(), d.to_owned())),
                        }
                    }
                }
                "response.output_item.added"
                    if data
                        .get("item")
                        .and_then(|i| i.get("type"))
                        .and_then(Value::as_str)
                        == Some("function_call") =>
                {
                    saw_function_call = true;
                    let call_id = str_field(&data["item"], "call_id");
                    let name = str_field(&data["item"], "name");
                    if !call_id.is_empty() {
                        match tool_calls_acc.iter_mut().find(|(id, _, _)| *id == call_id) {
                            Some(entry) if entry.1.is_empty() => entry.1 = name,
                            Some(_) => {}
                            None => tool_calls_acc.push((call_id, name, String::new())),
                        }
                    }
                }
                "response.completed" => {
                    if let Some(response) = data.get("response") {
                        match response.get("status").and_then(Value::as_str) {
                            Some("incomplete") => {
                                let reason = response
                                    .get("incomplete_details")
                                    .and_then(|d| d.get("reason"))
                                    .and_then(Value::as_str);
                                stop = if reason == Some("max_output_tokens") {
                                    ProviderStop::MaxTokens
                                } else {
                                    ProviderStop::Other
                                };
                            }
                            Some("completed") if saw_function_call => {
                                stop = ProviderStop::ToolUse;
                            }
                            Some("completed") => {
                                stop = ProviderStop::EndTurn;
                            }
                            _ => {}
                        }
                        if let Some(usage) = response.get("usage") {
                            if let Some(it) = usage.get("input_tokens").and_then(Value::as_u64) {
                                input_tokens = Some(it);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let mut tool_calls = Vec::new();
        for (call_id, name, args_str) in tool_calls_acc {
            let arguments: Value =
                serde_json::from_str(&args_str).unwrap_or(Value::Object(Default::default()));
            tool_calls.push(make_tool_call(call_id, name, arguments)?);
        }

        Ok(LlmResponse {
            text,
            tool_calls,
            stop,
            input_tokens,
        })
    }
}

// ── Stream Emitter ──────────────────────────────────────────────────────────

pub struct StreamEmitter {
    wire: WireSender,
    session_id: String,
}

impl StreamEmitter {
    pub fn new(wire: WireSender, session_id: String) -> Self {
        Self { wire, session_id }
    }

    async fn emit_chunk(&self, text: &str) {
        wire::send(
            &self.wire,
            wire::session_update(
                &self.session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text }
                }),
            ),
        )
        .await;
    }

    #[cfg(test)]
    pub fn noop() -> Self {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        Self {
            wire: tx,
            session_id: String::new(),
        }
    }
}

// ── SSE Parser ──────────────────────────────────────────────────────────────

struct SseReader {
    resp: reqwest::Response,
    buf: String,
}

impl SseReader {
    fn new(resp: reqwest::Response) -> Self {
        Self {
            resp,
            buf: String::new(),
        }
    }

    async fn next_event(&mut self) -> Result<Option<String>, AgentError> {
        loop {
            if let Some(pos) = self.buf.find("\n\n") {
                let event_block = self.buf[..pos].to_owned();
                self.buf.drain(..pos + 2);
                if let Some(d) = Self::extract_data(&event_block) {
                    return Ok(Some(d));
                }
                continue;
            }
            // Bound the raw read buffer independently of the per-event byte
            // accounting downstream: a stream that never emits an event
            // boundary (malformed, or a proxy that breaks event framing)
            // would otherwise grow `buf` without limit before any timeout fires.
            if self.buf.len() > MAX_LLM_RESPONSE_BYTES {
                return Err(AgentError::Llm(format!(
                    "streaming response exceeded {MAX_LLM_RESPONSE_BYTES} bytes without an event boundary"
                )));
            }
            match self.resp.chunk().await {
                Ok(Some(chunk)) => {
                    // Normalize CR and CRLF to LF so event framing (split on
                    // "\n\n") works regardless of how a provider, proxy, or CDN
                    // terminates lines. The SSE spec treats CR, LF, and CRLF as
                    // equivalent line terminators.
                    let text = String::from_utf8_lossy(&chunk);
                    self.buf
                        .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
                }
                Ok(None) => {
                    if !self.buf.trim().is_empty() {
                        let event_block = std::mem::take(&mut self.buf);
                        if let Some(d) = Self::extract_data(&event_block) {
                            return Ok(Some(d));
                        }
                    }
                    return Ok(None);
                }
                Err(e) => return Err(AgentError::Llm(format!("stream read: {e}"))),
            }
        }
    }

    fn extract_data(block: &str) -> Option<String> {
        let mut data_parts: Vec<&str> = Vec::new();
        for line in block.lines() {
            if line.is_empty() {
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                data_parts.push(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        if data_parts.is_empty() {
            None
        } else {
            Some(data_parts.join("\n"))
        }
    }
}

fn anthropic_usage_sum(usage: &Value) -> Option<u64> {
    let mut total = 0u64;
    let mut saw = false;
    for f in [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ] {
        if let Some(n) = usage.get(f).and_then(Value::as_u64) {
            total = total.saturating_add(n);
            saw = true;
        }
    }
    if saw {
        Some(total)
    } else {
        None
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

// ── OpenAI Responses API ───────────────────────────────────────────────────
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
    // `is_decode` covers mid-body read failures: reqwest's `chunk()` wraps a
    // reset/truncated response body as a decode error, which is a transient
    // transport fault and safe to resend. JSON-parse failures take a separate
    // path (`serde_json` on the fully-buffered body), so they never reach here.
    e.is_timeout() || e.is_connect() || e.is_request() || e.is_decode()
}

async fn post<F>(
    http: &Client,
    url: &str,
    body: &Value,
    chunk_timeout: std::time::Duration,
    apply: F,
) -> Result<Value, AgentError>
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
            // Per-chunk deadline: a stall between body chunks means the
            // provider connection went silent mid-response. Bounding each
            // poll (instead of the whole request) lets a slow-but-progressing
            // completion run arbitrarily long while still tearing down a
            // dead connection. A stall is transport-class, so retry the whole
            // request when attempts remain, then surface a clear error.
            let next = match tokio::time::timeout(chunk_timeout, stream.chunk()).await {
                Ok(r) => r,
                Err(_) => {
                    if attempt + 1 < MAX_RETRIES {
                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = MAX_RETRIES,
                            timeout_secs = chunk_timeout.as_secs(),
                            "llm: response body stalled between chunks, retrying"
                        );
                        backoff_with_jitter(attempt).await;
                        break;
                    }
                    return Err(AgentError::Llm(format!(
                        "response body stalled: no chunk within {}s",
                        chunk_timeout.as_secs()
                    )));
                }
            };
            match next {
                Ok(Some(chunk)) => {
                    if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                        return Err(AgentError::Llm(format!(
                            "response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        )));
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => {
                    return serde_json::from_slice(&buf)
                        .map_err(|e| AgentError::Llm(format!("json: {e}")));
                }
                Err(e) => {
                    // A read error mid-body (connection reset, broken pipe,
                    // TLS failure) is transport-class, same as a stall. With
                    // the total request timeout gone this is the main body-read
                    // failure, so retry the whole request when attempts remain
                    // and the error is transient, mirroring the stall arm.
                    if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = MAX_RETRIES,
                            error = %e,
                            "llm: response body read error, retrying"
                        );
                        backoff_with_jitter(attempt).await;
                        break;
                    }
                    return Err(AgentError::Llm(format!("read: {e}")));
                }
            }
        }
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
fn build_token_source(cfg: &Config) -> Result<Arc<dyn TokenSource>, AgentError> {
    match cfg.provider {
        Provider::Anthropic | Provider::OpenAi => {
            Ok(Arc::new(StaticTokenSource::new(cfg.api_key.clone())))
        }
        Provider::Databricks => {
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
            llm_body_chunk_timeout: Duration::from_secs(120),
            stream_chunk_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(10),
            mcp_init_timeout: Duration::from_secs(10),
            mcp_max_restart_attempts: 1,
            mcp_restart_base_ms: 1,
            mcp_restart_max_ms: 1,
            max_sessions: 1,
            max_line_bytes: 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
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
        let out = post(
            &client,
            &url,
            &serde_json::json!({}),
            Duration::from_secs(5),
            |b| b,
        )
        .await
        .expect("post should succeed after retry");
        assert_eq!(out, serde_json::json!({ "ok": true }));
        assert!(
            accepts.load(Ordering::SeqCst) >= 2,
            "server should have seen at least 2 connection attempts, saw {}",
            accepts.load(Ordering::SeqCst)
        );
    }

    /// A response that stops sending chunks mid-body (stalls longer than the
    /// chunk timeout) should be retried and eventually surface a clear error
    /// if all retries exhaust.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_times_out_on_stalled_response_body() {
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
                accepts_srv.fetch_add(1, Ordering::SeqCst);
                // Handle each connection in its own task so a stalled
                // response never blocks accepting the next retry.
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(k) => buf.extend_from_slice(&tmp[..k]),
                        }
                    }
                    // Send headers with chunked encoding, write one chunk, then stall.
                    let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                                   Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(headers.as_bytes()).await;
                    // Send a partial chunk then go silent.
                    let _ = sock.write_all(b"4\r\n{\"ok\r\n").await;
                    let _ = sock.flush().await;
                    // Hold the connection open but never send more data. Just
                    // longer than the chunk timeout so the client's per-chunk
                    // deadline fires; no need to outlast the whole retry budget.
                    tokio::time::sleep(Duration::from_secs(2)).await;
                });
            }
        });

        // Disable idle-connection reuse so each retry opens a fresh socket
        // instead of waiting on the stalled one still draining in the pool.
        let client = Client::builder().pool_max_idle_per_host(0).build().unwrap();
        // Very short chunk timeout so the test runs fast.
        let chunk_timeout = Duration::from_millis(100);
        let err = post(&client, &url, &serde_json::json!({}), chunk_timeout, |b| b)
            .await
            .unwrap_err();
        // Should surface a stall error after exhausting retries.
        match err {
            AgentError::Llm(msg) => {
                assert!(msg.contains("stalled"), "expected stall error, got: {msg}")
            }
            other => panic!("expected AgentError::Llm, got: {other:?}"),
        }
        // All retry attempts should have been used.
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            MAX_RETRIES,
            "should have attempted all retries"
        );
    }

    /// A slow-but-progressing response (chunks arrive within the timeout)
    /// should succeed regardless of total elapsed time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_succeeds_with_slow_but_progressing_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/x", listener.local_addr().unwrap());

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => return,
                    Ok(k) => buf.extend_from_slice(&tmp[..k]),
                }
            }
            // Chunked response: send the JSON body in small pieces with delays
            // that are within the chunk timeout.
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                           Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(headers.as_bytes()).await;
            // Split `{"ok":true}` across multiple chunks with delays.
            for piece in ["{\"ok\"", ":tru", "e}"] {
                tokio::time::sleep(Duration::from_millis(30)).await;
                let chunk = format!("{:x}\r\n{}\r\n", piece.len(), piece);
                let _ = sock.write_all(chunk.as_bytes()).await;
                let _ = sock.flush().await;
            }
            // Terminating chunk.
            let _ = sock.write_all(b"0\r\n\r\n").await;
            let _ = sock.shutdown().await;
        });

        let client = Client::builder().build().unwrap();
        // Chunk timeout is longer than the inter-chunk delay, so this should succeed.
        let chunk_timeout = Duration::from_millis(200);
        let out = post(&client, &url, &serde_json::json!({}), chunk_timeout, |b| b)
            .await
            .expect("slow-but-progressing response should succeed");
        assert_eq!(out, serde_json::json!({ "ok": true }));
    }

    /// A connection that delivers partial body bytes then is reset mid-body
    /// (before the terminating chunk) surfaces as a transport read error on
    /// `chunk().await`. That error is transient and must retry like a stall:
    /// the next attempt serves a complete body and the call succeeds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_retries_on_read_error_mid_body() {
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
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(k) => buf.extend_from_slice(&tmp[..k]),
                        }
                    }
                    if n == 0 {
                        // First attempt: send chunked headers and a partial
                        // chunk, then drop the socket without the terminating
                        // chunk. The truncated chunked body makes reqwest's
                        // `chunk()` return a transport read error.
                        let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                        let _ = sock.write_all(headers.as_bytes()).await;
                        let _ = sock.write_all(b"4\r\n{\"ok\r\n").await;
                        let _ = sock.flush().await;
                        drop(sock);
                        return;
                    }
                    // Subsequent attempts: serve a complete body.
                    let body = "{\"ok\":true}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body,
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });

        // Fresh socket per retry so the reset connection isn't reused.
        let client = Client::builder().pool_max_idle_per_host(0).build().unwrap();
        // Generous chunk timeout: this test exercises the read-error arm, not
        // the stall arm — the failure must come from the reset, not a timeout.
        let out = post(
            &client,
            &url,
            &serde_json::json!({}),
            Duration::from_secs(5),
            |b| b,
        )
        .await
        .expect("post should succeed after retrying the read error");
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

    // ── SSE Parser Tests ────────────────────────────────────────────────────

    #[test]
    fn sse_extract_data_basic() {
        let block = "data: {\"hello\":\"world\"}";
        assert_eq!(
            SseReader::extract_data(block),
            Some("{\"hello\":\"world\"}".to_owned())
        );
    }

    #[test]
    fn sse_extract_data_discards_comments() {
        let block = ": this is a comment\ndata: payload";
        assert_eq!(SseReader::extract_data(block), Some("payload".to_owned()));
    }

    #[test]
    fn sse_extract_data_multiline() {
        let block = "data: line1\ndata: line2";
        assert_eq!(
            SseReader::extract_data(block),
            Some("line1\nline2".to_owned())
        );
    }

    #[test]
    fn sse_extract_data_ignores_event_id_retry() {
        let block = "event: message\nid: 123\nretry: 5000\ndata: actual";
        assert_eq!(SseReader::extract_data(block), Some("actual".to_owned()));
    }

    #[test]
    fn sse_extract_data_empty_block() {
        assert_eq!(SseReader::extract_data(""), None);
        assert_eq!(SseReader::extract_data(": only a comment"), None);
    }

    #[test]
    fn sse_extract_data_no_space_after_colon() {
        // SSE spec: if no space after "data:", the value starts immediately
        let block = "data:no-space";
        assert_eq!(SseReader::extract_data(block), Some("no-space".to_owned()));
    }

    /// Helper: spin up a one-shot HTTP server that returns the given body as
    /// `text/event-stream`, then send a GET request to it and return the
    /// `reqwest::Response` for the streaming consumer under test.
    async fn sse_response(body: String) -> reqwest::Response {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            // Drain the request
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
        reqwest::get(format!("http://{addr}")).await.unwrap()
    }

    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn sse_reader_parses_stream() {
        let body = "data: {\"type\":\"first\"}\n\n: comment\n\ndata: {\"type\":\"second\"}\n\n";
        let resp = sse_response(body.to_owned()).await;
        let mut reader = SseReader::new(resp);

        let ev1 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev1, "{\"type\":\"first\"}");

        let ev2 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev2, "{\"type\":\"second\"}");

        let ev3 = reader.next_event().await.unwrap();
        assert!(ev3.is_none());
    }

    #[tokio::test]
    async fn anthropic_stream_text_and_tool() {
        let events = vec![
            json!({"type":"message_start","message":{"usage":{"input_tokens":50}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_weather"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ation\":\"NYC\"}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":30}}),
        ];
        let mut sse_body = String::new();
        for ev in &events {
            sse_body.push_str(&format!("data: {}\n\n", ev));
        }

        let resp = sse_response(sse_body).await;
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::Anthropic);
        let llm = Llm::new(&cfg).unwrap();
        let result = llm.consume_sse_anthropic(resp, &emitter).await.unwrap();

        assert_eq!(result.text, "Hello world");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(result.tool_calls[0].arguments, json!({"location": "NYC"}));
        assert_eq!(result.stop, ProviderStop::ToolUse);
        assert_eq!(result.input_tokens, Some(50));
    }

    #[tokio::test]
    async fn openai_chat_stream_text() {
        let events = vec![
            json!({"choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"content":" there"},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":15}}),
        ];
        let mut sse_body = String::new();
        for ev in &events {
            sse_body.push_str(&format!("data: {}\n\n", ev));
        }
        sse_body.push_str("data: [DONE]\n\n");

        let resp = sse_response(sse_body).await;
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::OpenAi);
        let llm = Llm::new(&cfg).unwrap();
        let result = llm.consume_sse_openai_chat(resp, &emitter).await.unwrap();

        assert_eq!(result.text, "Hi there");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop, ProviderStop::EndTurn);
        assert_eq!(result.input_tokens, Some(15));
    }

    #[tokio::test]
    async fn openai_chat_stream_tool_calls() {
        let events = vec![
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"foo.rs\"}"}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20}}),
        ];
        let mut sse_body = String::new();
        for ev in &events {
            sse_body.push_str(&format!("data: {}\n\n", ev));
        }
        sse_body.push_str("data: [DONE]\n\n");

        let resp = sse_response(sse_body).await;
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::OpenAi);
        let llm = Llm::new(&cfg).unwrap();
        let result = llm.consume_sse_openai_chat(resp, &emitter).await.unwrap();

        assert!(result.text.is_empty());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.tool_calls[0].arguments, json!({"path": "foo.rs"}));
        assert_eq!(result.stop, ProviderStop::ToolUse);
    }

    #[tokio::test]
    async fn responses_stream_text_and_function() {
        let events = vec![
            json!({"type":"response.output_item.added","item":{"type":"function_call","call_id":"fc_1","name":"search"}}),
            json!({"type":"response.function_call_arguments.delta","call_id":"fc_1","delta":"{\"q\":"}),
            json!({"type":"response.function_call_arguments.delta","call_id":"fc_1","delta":"\"rust\"}"}),
            json!({"type":"response.output_text.delta","delta":"Found results"}),
            json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":42}}}),
        ];
        let mut sse_body = String::new();
        for ev in &events {
            sse_body.push_str(&format!("data: {}\n\n", ev));
        }

        let resp = sse_response(sse_body).await;
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::OpenAi);
        let llm = Llm::new(&cfg).unwrap();
        let result = llm.consume_sse_responses(resp, &emitter).await.unwrap();

        assert_eq!(result.text, "Found results");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "search");
        assert_eq!(result.tool_calls[0].arguments, json!({"q": "rust"}));
        assert_eq!(result.stop, ProviderStop::ToolUse);
        assert_eq!(result.input_tokens, Some(42));
    }

    #[tokio::test]
    async fn stream_enforces_max_bytes() {
        // Send multiple chunks that together exceed MAX_LLM_RESPONSE_BYTES.
        // Each chunk is small enough to fit in TCP buffers, but accumulated
        // semantic content crosses the limit.
        let chunk_size = 1024 * 1024; // 1MB per chunk
        let num_chunks = (MAX_LLM_RESPONSE_BYTES / chunk_size) + 2;
        let chunk_text = "x".repeat(chunk_size);

        let mut sse_body = String::new();
        for _ in 0..num_chunks {
            let ev = json!({"choices":[{"index":0,"delta":{"content": chunk_text},"finish_reason":null}]});
            sse_body.push_str(&format!("data: {}\n\n", ev));
        }
        sse_body.push_str("data: [DONE]\n\n");

        // Use a streaming server that sends chunks incrementally
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener as TcpL;

        let listener = TcpL::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(header.as_bytes()).await;
            // Write body in small pieces so TCP buffers don't block
            let body_bytes = sse_body.as_bytes();
            for chunk in body_bytes.chunks(64 * 1024) {
                if sock.write_all(chunk).await.is_err() {
                    break;
                }
            }
            let _ = sock.shutdown().await;
        });

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::OpenAi);
        let llm = Llm::new(&cfg).unwrap();
        let result = llm.consume_sse_openai_chat(resp, &emitter).await;

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("exceeded"), "error: {err_msg}");
    }

    #[tokio::test]
    async fn stream_empty_body_completes_gracefully() {
        let resp = sse_response(String::new()).await;
        let emitter = StreamEmitter::noop();
        let cfg = cfg(Provider::OpenAi);
        let llm = Llm::new(&cfg).unwrap();

        let result = llm.consume_sse_openai_chat(resp, &emitter).await.unwrap();
        assert!(result.text.is_empty());
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn sse_reader_handles_crlf_line_endings() {
        // Events delimited by \r\n\r\n should be parsed correctly after
        // normalization to \n\n.
        let body = "data: {\"type\":\"first\"}\r\n\r\ndata: {\"type\":\"second\"}\r\n\r\n";
        let resp = sse_response(body.to_owned()).await;
        let mut reader = SseReader::new(resp);

        let ev1 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev1, "{\"type\":\"first\"}");

        let ev2 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev2, "{\"type\":\"second\"}");

        assert!(reader.next_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn sse_reader_handles_bare_cr_line_endings() {
        // Bare \r should also be normalized to \n per the SSE spec.
        let body = "data: {\"val\":\"cr\"}\r\rdata: {\"val\":\"end\"}\r\r";
        let resp = sse_response(body.to_owned()).await;
        let mut reader = SseReader::new(resp);

        let ev1 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev1, "{\"val\":\"cr\"}");

        let ev2 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev2, "{\"val\":\"end\"}");

        assert!(reader.next_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn sse_reader_errors_on_unbounded_buffer() {
        // A stream that never produces an event boundary should be capped by
        // the MAX_LLM_RESPONSE_BYTES guard on SseReader.buf.
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener as TcpL;

        let listener = TcpL::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(header.as_bytes()).await;
            // Send data without ever producing a double-newline boundary.
            // Each chunk is "data: <big payload>" with only a single \n.
            let chunk = "x".repeat(1024 * 1024); // 1MB
            for _ in 0..20 {
                let line = format!("data: {}\n", chunk);
                let _ = sock.write_all(line.as_bytes()).await;
            }
            let _ = sock.shutdown().await;
        });

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let mut reader = SseReader::new(resp);
        let result = reader.next_event().await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("exceeded") && err_msg.contains("without"),
            "unexpected error: {err_msg}"
        );
    }

    // ── Delayed-write TCP server helper ────────────────────────────────────
    //
    // Spawns a one-shot HTTP server that writes SSE chunks with configurable
    // delays between them. Used by Gap 1 (split-boundary) and Gap 3 (timeout
    // switchover) tests.

    /// A scheduled piece of an SSE response: write `data` after waiting `delay`.
    struct ScheduledWrite {
        data: String,
        delay: Duration,
    }

    /// Spawn a TCP server that accepts one connection, drains the HTTP request,
    /// sends SSE response headers, then writes each `ScheduledWrite` in order
    /// (delay first, then data). Returns the server address.
    async fn spawn_delayed_sse_server(writes: Vec<ScheduledWrite>) -> std::net::SocketAddr {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            // Drain the HTTP request headers.
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(header.as_bytes()).await;
            let _ = sock.flush().await;
            for w in writes {
                if !w.delay.is_zero() {
                    tokio::time::sleep(w.delay).await;
                }
                let _ = sock.write_all(w.data.as_bytes()).await;
                let _ = sock.flush().await;
            }
            let _ = sock.shutdown().await;
        });
        addr
    }

    // ── Gap 3: Dual-timeout switchover ─────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_timeout_uses_first_byte_before_content() {
        // first_byte=80ms, stream_chunk=20ms. A non-content event arrives,
        // then a 50ms gap, then a content delta. The 80ms first_byte window
        // governs until content arrives, so the 50ms gap is fine.
        let events = vec![
            ScheduledWrite {
                data: "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\n".into(),
                delay: Duration::ZERO,
            },
            ScheduledWrite {
                data: "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n".into(),
                delay: Duration::from_millis(50),
            },
            ScheduledWrite {
                data: "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n".into(),
                delay: Duration::ZERO,
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let mut c = cfg(Provider::Anthropic);
        c.llm_timeout = Duration::from_millis(80);
        c.stream_chunk_timeout = Duration::from_millis(20);
        let llm = Llm::new(&c).unwrap();
        let emitter = StreamEmitter::noop();

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let result = llm.consume_sse_anthropic(resp, &emitter).await.unwrap();
        assert_eq!(result.text, "hello");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_timeout_switches_to_stream_chunk_after_content() {
        // first_byte=500ms, stream_chunk=30ms. One content delta arrives,
        // then a 50ms stall. The tighter stream_chunk timeout (30ms) should
        // fire because we already saw content.
        let events = vec![
            ScheduledWrite {
                data: "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n".into(),
                delay: Duration::ZERO,
            },
            ScheduledWrite {
                // This arrives after 50ms, but stream_chunk is only 30ms.
                data: "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n".into(),
                delay: Duration::from_millis(50),
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let mut c = cfg(Provider::Anthropic);
        c.llm_timeout = Duration::from_millis(500);
        c.stream_chunk_timeout = Duration::from_millis(30);
        let llm = Llm::new(&c).unwrap();
        let emitter = StreamEmitter::noop();

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let result = llm.consume_sse_anthropic(resp, &emitter).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("stalled"),
            "expected stall error, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_timeout_first_byte_fires_when_no_content_arrives() {
        // first_byte=30ms. Server sends nothing then closes after 100ms.
        // The first_byte timeout should fire within ~30ms.
        let events = vec![
            ScheduledWrite {
                data: String::new(), // nothing useful
                delay: Duration::from_millis(100),
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let mut c = cfg(Provider::Anthropic);
        c.llm_timeout = Duration::from_millis(30);
        c.stream_chunk_timeout = Duration::from_millis(500);
        let llm = Llm::new(&c).unwrap();
        let emitter = StreamEmitter::noop();

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let start = tokio::time::Instant::now();
        let result = llm.consume_sse_anthropic(resp, &emitter).await;
        let elapsed = start.elapsed();
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("stalled"), "expected stall error, got: {err}");
        // Should fire around 30ms, not wait for the full 100ms server delay.
        assert!(
            elapsed < Duration::from_millis(80),
            "timeout fired too late: {elapsed:?}"
        );
    }

    // ── Gap 1: SseReader split-boundary framing ────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_reader_event_split_across_chunks() {
        // The "\n\n" event boundary is split across two TCP writes.
        let events = vec![
            ScheduledWrite {
                data: "data: {\"type\":\"first\"}\n".into(),
                delay: Duration::ZERO,
            },
            ScheduledWrite {
                // Second write starts with the second \n completing the boundary,
                // then delivers the next event.
                data: "\ndata: {\"type\":\"second\"}\n\n".into(),
                delay: Duration::from_millis(5),
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let mut reader = SseReader::new(resp);

        let ev1 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev1, "{\"type\":\"first\"}");

        let ev2 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev2, "{\"type\":\"second\"}");

        assert!(reader.next_event().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_reader_data_field_split_mid_keyword() {
        // The "data:" prefix is split across two TCP writes: "da" | "ta: value"
        let events = vec![
            ScheduledWrite {
                data: "da".into(),
                delay: Duration::ZERO,
            },
            ScheduledWrite {
                data: "ta: {\"split\":true}\n\n".into(),
                delay: Duration::from_millis(5),
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let mut reader = SseReader::new(resp);

        let ev = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev, "{\"split\":true}");

        assert!(reader.next_event().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sse_reader_trailing_data_without_boundary() {
        // Stream ends with data but no trailing \n\n — the drain path should
        // still yield the event.
        let events = vec![
            ScheduledWrite {
                data: "data: {\"type\":\"complete\"}\n\n".into(),
                delay: Duration::ZERO,
            },
            ScheduledWrite {
                // Trailing data without a boundary before EOF.
                data: "data: {\"type\":\"trailing\"}".into(),
                delay: Duration::from_millis(5),
            },
        ];
        let addr = spawn_delayed_sse_server(events).await;

        let resp = reqwest::get(format!("http://{addr}")).await.unwrap();
        let mut reader = SseReader::new(resp);

        let ev1 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev1, "{\"type\":\"complete\"}");

        let ev2 = reader.next_event().await.unwrap().unwrap();
        assert_eq!(ev2, "{\"type\":\"trailing\"}");

        assert!(reader.next_event().await.unwrap().is_none());
    }

    // ── Gap 6: send_stream_with_retry ──────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_retry_on_5xx_then_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_srv = attempts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let n = attempts_srv.fetch_add(1, Ordering::SeqCst);
                // Drain request.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                if n == 0 {
                    // First attempt: 503
                    let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(resp.as_bytes()).await;
                } else {
                    // Second attempt: 200 with valid SSE
                    let body = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }
                let _ = sock.shutdown().await;
            }
        });

        let c = cfg(Provider::Anthropic);
        let llm = Llm::new(&c).unwrap();
        let emitter = StreamEmitter::noop();

        let url = format!("http://{addr}/v1/messages");
        let body_bytes = b"{}".to_vec();
        let resp = llm
            .send_stream_with_retry(|| {
                llm.http_stream
                    .post(&url)
                    .header("content-type", "application/json")
                    .body(body_bytes.clone())
            })
            .await
            .unwrap();
        let result = llm.consume_sse_anthropic(resp, &emitter).await.unwrap();
        assert_eq!(result.text, "ok");
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "should have retried at least once"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_retry_exhausted_surfaces_error() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_srv = attempts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                attempts_srv.fetch_add(1, Ordering::SeqCst);
                // Drain request.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                // Always 503.
                let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let c = cfg(Provider::Anthropic);
        let llm = Llm::new(&c).unwrap();

        let url = format!("http://{addr}/v1/messages");
        let body_bytes = b"{}".to_vec();
        let result = llm
            .send_stream_with_retry(|| {
                llm.http_stream
                    .post(&url)
                    .header("content-type", "application/json")
                    .body(body_bytes.clone())
            })
            .await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("503") || err.contains("exhausted"),
            "expected 503 or exhausted error, got: {err}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            MAX_RETRIES,
            "should have exhausted all retries"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stream_auth_error_does_not_retry() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_srv = attempts.clone();

        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                attempts_srv.fetch_add(1, Ordering::SeqCst);
                // Drain request.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    }
                }
                // 401 Unauthorized — should not retry.
                let body = "invalid api key";
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let c = cfg(Provider::Anthropic);
        let llm = Llm::new(&c).unwrap();

        let url = format!("http://{addr}/v1/messages");
        let body_bytes = b"{}".to_vec();
        let result = llm
            .send_stream_with_retry(|| {
                llm.http_stream
                    .post(&url)
                    .header("content-type", "application/json")
                    .body(body_bytes.clone())
            })
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AgentError::LlmAuth(msg) => {
                assert!(
                    msg.contains("invalid api key"),
                    "expected auth error body, got: {msg}"
                );
            }
            other => panic!("expected LlmAuth error, got: {other:?}"),
        }
        // Auth errors should NOT retry — only 1 attempt.
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "auth error should not trigger retries"
        );
    }
}
