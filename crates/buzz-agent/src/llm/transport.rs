//! Provider HTTP status handling and retry budgets. Semantic streaming is decoded separately.
use super::stream::{self, StreamRequest};
use super::{
    backoff_with_jitter, classify_body_read_error, classify_transport_error, escalated_timeout,
    is_context_length_error, is_retryable_transport_error, is_unsupported_image_input_error,
    openrouter_parameter_routing_error, read_error_body, terminal_llm_error, AgentError,
    MAX_LLM_RESPONSE_BYTES, MAX_RETRIES,
};
use super::{classify_openrouter_error, parse_retry_after_header, OpenRouterErrorClass};
use reqwest::Client;
use serde_json::Value;

/// Internal HTTP failure wrapper for the OpenAI-family `post` helper.
#[derive(Debug)]
pub(super) enum PostError {
    Agent(AgentError),
}

impl PostError {
    pub(super) fn into_agent(self) -> AgentError {
        match self {
            Self::Agent(error) => error,
        }
    }
}

impl From<AgentError> for PostError {
    fn from(error: AgentError) -> Self {
        Self::Agent(error)
    }
}

pub(super) async fn post<F>(
    http: &Client,
    url: &str,
    body: &Value,
    base_timeout: std::time::Duration,
    apply: F,
    streaming: Option<StreamRequest<'_>>,
) -> Result<Value, PostError>
where
    F: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
{
    let body_bytes = serde_json::to_vec(body)
        .map_err(|e| PostError::Agent(AgentError::Llm(format!("serialize: {e}"))))?;
    let call_start = std::time::Instant::now();
    // Count prior attempts that ended with a timeout so we can double the
    // budget on the next attempt. Non-timeout retryable failures (429, 5xx,
    // connect resets) are not evidence of a slow model and do NOT escalate.
    let mut timeout_failures: u32 = 0;
    'attempt: for attempt in 0..MAX_RETRIES {
        let per_request_timeout = escalated_timeout(base_timeout, timeout_failures);
        let resp = match apply(
            http.post(url)
                .header("content-type", "application/json")
                .body(body_bytes.clone())
                .timeout(per_request_timeout),
        )
        .send()
        .await
        {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    timeout_failures += 1;
                }
                if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        is_timeout = e.is_timeout(),
                        timeout_failures,
                        "llm: transport error, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(PostError::Agent(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &classify_transport_error(&e, per_request_timeout),
                )));
            }
        };
        let status = resp.status();
        // Both 401 and 403 are treated as refreshable: a 403 can mean an
        // expired or revoked token, not just a pure authorization verdict, and
        // the two are indistinguishable at the HTTP-status layer. The caller's
        // retry loop keys off `LlmAuth` and refreshes once; the per-call guard
        // bounds a pure-authz 403 to one wasted refresh before it propagates.
        // Not a stall path: auth failures are not surfaced through
        // terminal_llm_error, they resolve on the next call after refresh.
        if status == 401 || status == 403 {
            return Err(PostError::Agent(AgentError::LlmAuth(
                read_error_body(resp).await,
            )));
        }
        if status.is_server_error() || status == 429 || status.as_u16() == 499 {
            let body = read_error_body(resp).await;
            if attempt + 1 < MAX_RETRIES {
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = MAX_RETRIES,
                    %status,
                    "llm: retryable status, retrying"
                );
                backoff_with_jitter(attempt).await;
                continue;
            }
            return Err(PostError::Agent(terminal_llm_error(
                call_start.elapsed(),
                attempt + 1,
                &format!("exhausted retries: {status}: {body}"),
            )));
        }
        // Not a stall path: the model is misconfigured, not the transport or
        // upstream capacity — no retry was attempted, so cumulative duration
        // would be misleading.
        if status == 404 {
            let error_body = read_error_body(resp).await;
            if is_unsupported_image_input_error(&error_body) {
                return Err(PostError::Agent(AgentError::UnsupportedImageInput(
                    error_body,
                )));
            }
            return Err(PostError::Agent(AgentError::LlmModelNotFound(format!(
                "{status}: {error_body}"
            ))));
        }
        if !status.is_success() {
            let body = read_error_body(resp).await;
            // Context-window overflow is a recovery signal, not a terminal
            // error: classify it here, where status and body are still separate
            // values. Callers must never re-derive this from the formatted
            // string — `Llm::complete` stamps the model name onto it before the
            // agent loop ever sees it.
            if status == 400 && is_context_length_error(&body) {
                return Err(PostError::Agent(AgentError::LlmContextExceeded(format!(
                    "{status}: {body}"
                ))));
            }
            // Image-capability rejection is equally recoverable and equally
            // deterministic: a text-only deployment 400s the same request
            // forever. Typed here (not just on the 404 arm) because
            // OpenAI-compatible providers report it as a 400.
            if status == 400 && is_unsupported_image_input_error(&body) {
                return Err(PostError::Agent(AgentError::UnsupportedImageInput(body)));
            }
            return Err(PostError::Agent(AgentError::Llm(format!(
                "{status}: {body}"
            ))));
        }
        if let Some(request) = streaming.filter(|_| stream::is_event_stream(&resp)) {
            return stream::read_response(resp, request)
                .await
                .map_err(PostError::from);
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_LLM_RESPONSE_BYTES {
                return Err(PostError::Agent(AgentError::Llm(format!(
                    "response too large: {len} > {MAX_LLM_RESPONSE_BYTES}"
                ))));
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp;
        loop {
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    if buf.len() + chunk.len() > MAX_LLM_RESPONSE_BYTES {
                        // Size overflow: not a timeout, not retryable.
                        return Err(PostError::Agent(AgentError::Llm(format!(
                            "response exceeded {MAX_LLM_RESPONSE_BYTES} bytes"
                        ))));
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    // A timeout during body streaming means the total
                    // per-request budget expired mid-body (headers arrived but
                    // the response stalled). Retry with an escalated budget,
                    // same as a transport-phase timeout — the server may be
                    // slow to flush, not permanently broken.
                    if e.is_timeout() && attempt + 1 < MAX_RETRIES {
                        timeout_failures += 1;
                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = MAX_RETRIES,
                            is_timeout = true,
                            timeout_failures,
                            error = %e,
                            "llm: body-read timeout, retrying with escalated budget"
                        );
                        backoff_with_jitter(attempt).await;
                        continue 'attempt;
                    }
                    return Err(PostError::Agent(terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &classify_body_read_error(&e, per_request_timeout),
                    )));
                }
            }
        }
        // A 2xx body that fails to parse (e.g. a provider flushing a
        // truncated JSON document and closing the stream cleanly) is a
        // transient upstream fault, not a request problem: retry it like a
        // 5xx. Safe to re-send — a malformed body produced no parsed
        // response, so no tool call was ever extracted from it, and the
        // retried request is the identical completion POST captured in
        // `body_bytes` at function entry.
        match serde_json::from_slice(&buf) {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt + 1 < MAX_RETRIES {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        "llm: malformed response body, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(PostError::Agent(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("json: {e}"),
                )));
            }
        }
    }
    // Unreachable in practice: every iteration either returns or continues.
    // A fallthrough here would mean MAX_RETRIES was 0, which is rejected at
    // config validation. Return a terminal error rather than panic so the
    // invariant is not load-bearing.
    Err(PostError::Agent(terminal_llm_error(
        call_start.elapsed(),
        MAX_RETRIES,
        "exhausted retries",
    )))
}

pub(super) async fn openrouter_post(
    http: &Client,
    url: &str,
    body: &Value,
    bearer: &str,
    base_timeout: std::time::Duration,
    streaming: Option<StreamRequest<'_>>,
) -> Result<Value, AgentError> {
    let body_bytes =
        serde_json::to_vec(body).map_err(|e| AgentError::Llm(format!("serialize: {e}")))?;
    let call_start = std::time::Instant::now();
    // Count prior timeout failures for per-attempt budget escalation; see
    // `escalated_timeout` for the doubling strategy.
    let mut timeout_failures: u32 = 0;
    'attempt: for attempt in 0..MAX_RETRIES {
        let per_request_timeout = escalated_timeout(base_timeout, timeout_failures);
        let resp = match http
            .post(url)
            .header("content-type", "application/json")
            .header("HTTP-Referer", "https://github.com/block/buzz")
            .header("X-OpenRouter-Title", "Buzz")
            .bearer_auth(bearer)
            .body(body_bytes.clone())
            .timeout(per_request_timeout)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    timeout_failures += 1;
                }
                if attempt + 1 < MAX_RETRIES && is_retryable_transport_error(&e) {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        is_timeout = e.is_timeout(),
                        timeout_failures,
                        "llm: openrouter transport error, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &classify_transport_error(&e, per_request_timeout),
                ));
            }
        };
        let status = resp.status();
        // Unlike the generic `post` path, OpenRouter's static-key auth makes
        // 401 and 403 distinguishable: 401 is an invalid/expired key (worth
        // one refresh-and-retry), while OpenRouter documents 403 as a
        // guardrail/moderation/permission rejection the same key will always
        // reproduce. Refreshing a static key returns the identical key, so
        // classifying 403 as `LlmAuth` would just waste a duplicate request
        // and surface Desktop's unrelated "access denied" copy.
        if status == 401 {
            return Err(AgentError::LlmAuth(read_error_body(resp).await));
        }
        if status == 403 {
            return Err(AgentError::Llm(format!(
                "{status}: {}",
                read_error_body(resp).await
            )));
        }
        if status == 402 {
            return Err(AgentError::Llm(
                "OpenRouter credits exhausted — check https://openrouter.ai/credits".into(),
            ));
        }
        if status == 404 {
            // OpenRouter overloads 404: a genuinely unknown/unavailable model id
            // and a valid model whose parameter set no endpoint can serve both
            // land here. Discriminate on the full parameter-routing phrase rather
            // than a prefix — "No endpoints found for <model>"-style bodies are
            // about the model, and reporting a parameter problem as
            // `LlmModelNotFound` (or vice versa) sends the user to the wrong fix.
            let error_body = read_error_body(resp).await;
            if is_unsupported_image_input_error(&error_body) {
                return Err(AgentError::UnsupportedImageInput(error_body));
            }
            if error_body.contains("No endpoints found that can handle the requested parameters") {
                return Err(openrouter_parameter_routing_error(&error_body));
            }
            return Err(AgentError::LlmModelNotFound(format!(
                "{status}: {error_body}"
            )));
        }
        // A6: status+error_type retry matrix
        // 499 (Client Closed Request) is included: OpenRouter may emit it when a
        // turn times out mid-stream. Will added 499 to the shared `post()` path in
        // #2175 for the same reason; OpenRouter must match to avoid silently opting
        // out of that stall-surfacing recovery.
        if status.is_server_error() || status == 429 || status.as_u16() == 499 {
            let header_retry_after = parse_retry_after_header(resp.headers());
            let error_body = read_error_body(resp).await;
            let should_retry = if attempt + 1 < MAX_RETRIES {
                match classify_openrouter_error(status.as_u16(), &error_body, header_retry_after) {
                    OpenRouterErrorClass::Retryable(delay) => {
                        if let Some(d) = delay {
                            tokio::time::sleep(d).await;
                        } else {
                            backoff_with_jitter(attempt).await;
                        }
                        true
                    }
                    OpenRouterErrorClass::Unknown => {
                        backoff_with_jitter(attempt).await;
                        true
                    }
                }
            } else {
                false
            };
            if should_retry {
                continue;
            }
            // Terminal: classify for the user
            return if status == 429 {
                Err(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("rate limited: {error_body}"),
                ))
            } else {
                let parsed: Option<Value> = serde_json::from_str(&error_body).ok();
                let has_error_type = parsed
                    .as_ref()
                    .and_then(|v| v.get("error"))
                    .and_then(|e| e.get("metadata"))
                    .and_then(|m| m.get("error_type"))
                    .and_then(Value::as_str)
                    .is_some();
                Err(if !has_error_type && status.as_u16() == 503 {
                    openrouter_parameter_routing_error(&error_body)
                } else {
                    terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &format!("exhausted retries: {status}: {error_body}"),
                    )
                })
            };
        }
        if !status.is_success() {
            let body = read_error_body(resp).await;
            // Same recovery classification as the shared `post()` terminal:
            // `openrouter_post` is a separate implementation with its own retry
            // loop and status ladder, so it needs its own arm or OpenRouter
            // agents keep the permanent context-400 stuck loop.
            if status == 400 && is_context_length_error(&body) {
                return Err(AgentError::LlmContextExceeded(format!("{status}: {body}")));
            }
            // Same 400-shaped image rejection as the shared `post()` terminal:
            // OpenRouter normally reports this as a 404 (handled above), but a
            // BYOK/passthrough upstream can surface the provider's own 400.
            if status == 400 && is_unsupported_image_input_error(&body) {
                return Err(AgentError::UnsupportedImageInput(body));
            }
            return Err(AgentError::Llm(format!("{status}: {body}")));
        }
        if let Some(request) = streaming.filter(|_| stream::is_event_stream(&resp)) {
            return stream::read_response(resp, request).await;
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
                Err(e) => {
                    // Body-read timeout: retry with escalated budget, same as
                    // transport-phase timeouts — see the corresponding arm in
                    // `post()` for the full rationale.
                    if e.is_timeout() && attempt + 1 < MAX_RETRIES {
                        timeout_failures += 1;
                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = MAX_RETRIES,
                            is_timeout = true,
                            timeout_failures,
                            error = %e,
                            "llm: openrouter body-read timeout, retrying with escalated budget"
                        );
                        backoff_with_jitter(attempt).await;
                        continue 'attempt;
                    }
                    return Err(terminal_llm_error(
                        call_start.elapsed(),
                        attempt + 1,
                        &classify_body_read_error(&e, per_request_timeout),
                    ));
                }
            }
        }
        // Same malformed-body retry as the shared `post()` terminal: a
        // truncated 2xx JSON body is transient upstream trouble, and
        // re-sending is provably tool-safe — nothing was parsed, so no tool
        // call could have been extracted, and the retry re-uses the
        // identical `body_bytes` completion request.
        match serde_json::from_slice(&buf) {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt + 1 < MAX_RETRIES {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_RETRIES,
                        error = %e,
                        "llm: openrouter malformed response body, retrying"
                    );
                    backoff_with_jitter(attempt).await;
                    continue;
                }
                return Err(terminal_llm_error(
                    call_start.elapsed(),
                    attempt + 1,
                    &format!("json: {e}"),
                ));
            }
        }
    }
    Err(terminal_llm_error(
        call_start.elapsed(),
        MAX_RETRIES,
        "exhausted retries",
    ))
}
