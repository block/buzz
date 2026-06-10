use std::sync::Arc;

use serde_json::json;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;

use crate::config::{Config, MAX_PROMPT_BYTES, MAX_TOOL_CALLS_PER_TURN, MAX_TOOL_RESULT_BYTES};
use crate::handoff::HandoffOutcome;
use crate::llm::Llm;
use crate::mcp::McpRegistry;

use crate::types::{
    AgentError, ContentBlock, HistoryItem, ProviderStop, StopReason, ToolCall, ToolResult,
    ToolResultContent,
};
use crate::wire::{self, WireSender};

const ERROR_REFLECTION_SUFFIX: &str =
    "\n\n[Reflect] Before retrying, identify the cause and change your approach.";

pub struct RunCtx<'a> {
    pub cfg: &'a Config,
    pub session_id: &'a str,
    pub system_prompt: &'a str,
    pub llm: &'a Llm,
    pub mcp: &'a Arc<McpRegistry>,
    pub wire: &'a WireSender,
    pub cancel: &'a mut watch::Receiver<bool>,
    pub history: &'a mut Vec<HistoryItem>,
    pub original_task: &'a mut Option<String>,
    pub handoff_count: &'a mut usize,
    /// Cumulative `_Stop` objection count for this session (persists
    /// across `session/prompt` calls). Once it hits
    /// `cfg.stop_max_rejections` we stop calling `_Stop` for that
    /// session — a runaway hook can't burn rejections on every prompt.
    pub stop_rejections: &'a mut u32,
    /// Cache-summed input tokens reported by the provider on this session's
    /// most recent request (persists across `session/prompt` calls), or `None`
    /// before the first response and immediately after a handoff resets the
    /// context. The handoff gate reads this to compare against the token
    /// budget; falls back to the byte heuristic when `None`.
    pub last_request_input_tokens: &'a mut Option<u64>,
    /// History byte size at the moment `last_request_input_tokens` was
    /// measured. Paired with it so the gate can add a conservative token
    /// estimate of history that has grown since (tool results, next prompt),
    /// which the exact-but-stale token count would otherwise miss. Cleared and
    /// preserved in lockstep with `last_request_input_tokens`.
    pub last_request_history_bytes: &'a mut Option<usize>,
}

impl RunCtx<'_> {
    pub async fn run(&mut self, prompt: Vec<ContentBlock>) -> Result<StopReason, AgentError> {
        let user_text = prompt_to_text(prompt)?;
        if user_text.len() > MAX_PROMPT_BYTES {
            return Err(AgentError::InvalidParams(format!(
                "prompt: exceeds {MAX_PROMPT_BYTES} bytes"
            )));
        }
        if self.original_task.is_none() {
            *self.original_task = Some(user_text.clone());
        }
        self.history.push(HistoryItem::User(user_text));

        let mut round = 0u32;
        // Per-prompt latch: only used to detect "LLM said end_turn twice
        // in a row with no tool calls between" within this single prompt.
        // The cumulative rejection budget lives on the session.
        let mut last_was_end_turn = false;
        loop {
            if self.cfg.max_rounds > 0 && round >= self.cfg.max_rounds {
                return Ok(StopReason::MaxTurnRequests);
            }
            if *self.cancel.borrow() {
                return Ok(StopReason::Cancelled);
            }
            match self.maybe_handoff().await {
                HandoffOutcome::Cancelled => return Ok(StopReason::Cancelled),
                // Context was just reset — the prior request's token count no
                // longer describes the (now much smaller) history. Clear both
                // the token count and its byte baseline so a stale over-
                // threshold reading can't immediately re-fire the handoff
                // before the next response reports fresh usage.
                HandoffOutcome::Performed => {
                    *self.last_request_input_tokens = None;
                    *self.last_request_history_bytes = None;
                }
                HandoffOutcome::Skipped => {
                    truncate_history(self.history, self.cfg.max_history_bytes)
                }
            }

            let tools = self.mcp.tools();
            round = round.saturating_add(1);
            let response = tokio::select! {
                biased;
                _ = self.cancel.changed() => return Ok(StopReason::Cancelled),
                r = self.llm.complete(self.cfg, self.system_prompt, self.history, &tools) => r?,
                _ = async {
                    // Keepalive ticker: emit a lightweight session update every 30s
                    // while waiting on the LLM provider. This resets the ACP harness
                    // idle clock so long provider responses don't trigger timeout.
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                    interval.tick().await; // first tick fires immediately — skip it
                    loop {
                        interval.tick().await;
                        tracing::debug!("llm keepalive tick");
                        wire::send(
                            self.wire,
                            wire::session_update(
                                self.session_id,
                                json!({
                                    "sessionUpdate": "keepalive",
                                }),
                            ),
                        )
                        .await;
                    }
                } => unreachable!(),
            };

            // Record provider-reported input usage so the next loop iteration's
            // handoff gate can compare it against the token budget. We capture
            // it together with the history byte size AT THIS MOMENT — which is
            // exactly the history that was just sent to `complete()` (the
            // assistant response is appended below, after this point). Pairing
            // them lets the gate add a conservative estimate for any history
            // appended before the next request. Preserve both when a response
            // omits usage (`None`) rather than clobbering — a one-off missing
            // field shouldn't blind the gate or zero the growth baseline.
            if let Some(tokens) = response.input_tokens {
                *self.last_request_input_tokens = Some(tokens);
                *self.last_request_history_bytes =
                    Some(self.history.iter().map(HistoryItem::estimated_bytes).sum());
            }

            if !response.text.is_empty() {
                wire::send(
                    self.wire,
                    wire::session_update(
                        self.session_id,
                        json!({
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": &response.text }
                        }),
                    ),
                )
                .await;
            }

            if response.tool_calls.is_empty() {
                if response.stop == ProviderStop::ToolUse {
                    return Err(AgentError::Llm(
                        "provider: stop=tool_use but zero tool_calls".into(),
                    ));
                }
                self.history.push(HistoryItem::Assistant {
                    text: response.text,
                    tool_calls: Vec::new(),
                });
                let stop = map_stop(response.stop);
                // Only gate genuine end_turn — don't override max_tokens/refusal.
                if stop == StopReason::EndTurn {
                    // Consecutive-rejection rule: LLM responded to our last
                    // objection with no tool calls — accept the end and
                    // move on rather than loop forever.
                    if last_was_end_turn {
                        return Ok(stop);
                    }
                    if *self.stop_rejections >= self.cfg.stop_max_rejections {
                        return Ok(stop);
                    }
                    let objections = self
                        .mcp
                        .call_hooks(
                            "_Stop",
                            &json!({}),
                            self.cfg.hook_timeout,
                            &self.cfg.hook_servers,
                        )
                        .await;
                    if !objections.is_empty() {
                        *self.stop_rejections = self.stop_rejections.saturating_add(1);
                        last_was_end_turn = true;
                        push_hook_outputs_as_tool_results(self.history, "_Stop", &objections);
                        continue;
                    }
                }
                return Ok(stop);
            }

            let mut calls = response.tool_calls;
            if calls.len() > MAX_TOOL_CALLS_PER_TURN {
                tracing::warn!(
                    "capping tool_calls {} -> {MAX_TOOL_CALLS_PER_TURN}",
                    calls.len()
                );
                calls.truncate(MAX_TOOL_CALLS_PER_TURN);
            }
            self.history.push(HistoryItem::Assistant {
                text: response.text,
                tool_calls: calls.clone(),
            });

            // Tool calls executed → reset the consecutive-rejection latch.
            last_was_end_turn = false;

            if let Some(stop) = self.execute_calls(&calls).await {
                return Ok(stop);
            }
        }
    }

    /// Unified tool-call execution. Three phases:
    ///   1. Preflight (sequential): emit `pending`; unknown tools fail fast
    ///      with a synthetic result. Cancel here fills every still-empty
    ///      slot as cancelled.
    ///   2. Execute: spawn runnable calls into a `JoinSet` bounded by a
    ///      `Semaphore(max_parallel_tools)`. `select!` between cancel and
    ///      `join_next`. On cancel: close semaphore, drain in-flight tasks
    ///      (each sends `notifications/cancelled` internally), synthesize
    ///      cancelled for unfilled slots and emit `failed`.
    ///   3. Append: push results into history in original call order.
    ///
    /// `max_parallel_tools = 1` makes phase 2 effectively sequential
    /// (one in-flight call at a time via the semaphore). Larger values
    /// run that many calls concurrently.
    async fn execute_calls(&mut self, calls: &[ToolCall]) -> Option<StopReason> {
        let mut results: Vec<Option<ToolResult>> = vec![None; calls.len()];
        let mut runnable: Vec<usize> = Vec::with_capacity(calls.len());

        for (idx, call) in calls.iter().enumerate() {
            if *self.cancel.borrow() {
                for (j, c) in calls.iter().enumerate() {
                    if results[j].is_none() {
                        // Calls 0..idx already had `pending` emitted; emit
                        // a terminal `failed` so the client doesn't see
                        // them stuck.
                        if j < idx {
                            emit_failed(self.wire, self.session_id, c, "cancelled").await;
                        }
                        results[j] = Some(synthetic_tool_result(c, "cancelled".into()));
                    }
                }
                self.append_results(calls, &mut results);
                return Some(StopReason::Cancelled);
            }
            emit_pending(self.wire, self.session_id, call).await;
            // Hook tools (bare name starts with `_`) are invisible to the
            // LLM and only callable via `call_hooks`. Treat any direct
            // invocation as if the tool didn't exist.
            if !self.mcp.has(&call.name) || self.mcp.is_hook(&call.name) {
                let err = format!("unknown tool: {}", call.name);
                emit_failed(self.wire, self.session_id, call, &err).await;
                results[idx] = Some(synthetic_tool_result(call, err));
                continue;
            }
            runnable.push(idx);
        }

        self.execute_parallel(calls, &runnable, &mut results).await;

        self.append_results(calls, &mut results);

        if *self.cancel.borrow() {
            Some(StopReason::Cancelled)
        } else {
            None
        }
    }

    fn append_results(&mut self, calls: &[ToolCall], results: &mut [Option<ToolResult>]) {
        for (i, call) in calls.iter().enumerate() {
            let mut result = results[i].take().unwrap_or_else(|| ToolResult {
                provider_id: call.provider_id.clone(),
                content: vec![ToolResultContent::Text(
                    "internal error: missing result".into(),
                )],
                is_error: true,
            });
            // On tool error: append a reflection prompt so the LLM
            // diagnoses the failure before blindly retrying.
            if result.is_error {
                result
                    .content
                    .push(ToolResultContent::Text(ERROR_REFLECTION_SUFFIX.to_string()));
            }
            self.history.push(HistoryItem::ToolResult(result));
        }
    }

    async fn execute_parallel(
        &mut self,
        calls: &[ToolCall],
        runnable: &[usize],
        results: &mut [Option<ToolResult>],
    ) {
        let limit = self.cfg.max_parallel_tools.max(1);
        let sem = Arc::new(Semaphore::new(limit));
        let mut set: JoinSet<(usize, InvokeOutcome)> = JoinSet::new();

        for &i in runnable {
            let call = calls[i].clone();
            let mcp = Arc::clone(self.mcp);
            let wire = self.wire.clone();
            let session_id = self.session_id.to_owned();
            let timeout = self.cfg.tool_timeout;
            let cancel = self.cancel.clone();
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                // Acquire a permit; if the semaphore is closed (cancel),
                // emit a terminal wire update and skip the call.
                let _permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        emit_failed(&wire, &session_id, &call, "cancelled").await;
                        return (i, InvokeOutcome::Failed("cancelled".into()));
                    }
                };
                emit_in_progress(&wire, &session_id, &call).await;
                let outcome = invoke_tool_inner(&mcp, &call, timeout, cancel).await;
                match &outcome {
                    InvokeOutcome::Done(result) => {
                        emit_completed(&wire, &session_id, &call, result).await;
                    }
                    InvokeOutcome::Failed(msg) => {
                        emit_failed(&wire, &session_id, &call, msg).await;
                    }
                }
                (i, outcome)
            });
        }

        let mut cancel_rx = self.cancel.clone();
        let mut cancelled = if *cancel_rx.borrow() {
            sem.close();
            true
        } else {
            false
        };
        while !cancelled {
            tokio::select! {
                biased;
                _ = cancel_rx.changed() => {
                    // Cancel: stop accepting new permits. Do NOT abort
                    // tasks — each in-flight `mcp.call` observes the same
                    // cancel receiver via its internal `select!` and
                    // returns promptly with an "cancelled" error after
                    // sending `notifications/cancelled` to the server.
                    sem.close();
                    cancelled = true;
                    break;
                }
                next = set.join_next() => {
                    match next {
                        Some(Ok((i, outcome))) => {
                            results[i] = Some(outcome_to_result(&calls[i], outcome));
                        }
                        Some(Err(e)) => {
                            tracing::warn!("tool task join error: {e}");
                        }
                        None => break,
                    }
                }
            }
        }

        // After cancel, drain in-flight tasks. Each task's internal
        // `do_call` observes the cancel receiver and returns promptly
        // after sending `notifications/cancelled`. We bound the drain
        // to avoid hanging if a task is stuck in restart/reconnect.
        if cancelled {
            let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match tokio::time::timeout_at(drain_deadline, set.join_next()).await {
                    Ok(Some(Ok((i, outcome)))) => {
                        if results[i].is_none() {
                            results[i] = Some(outcome_to_result(&calls[i], outcome));
                        }
                    }
                    Ok(Some(Err(e))) => {
                        tracing::warn!("tool task join error (drain): {e}");
                    }
                    Ok(None) => break, // all tasks drained
                    Err(_) => {
                        // Drain timed out — abort remaining tasks.
                        set.abort_all();
                        tracing::warn!("cancel drain timed out; aborting remaining tasks");
                        break;
                    }
                }
            }
        }

        // Fill any remaining unfilled runnable slots as cancelled. Tasks
        // that didn't complete (timed out in drain or never started) need
        // a terminal wire update so the client doesn't see "pending" forever.
        for &i in runnable {
            if results[i].is_none() {
                results[i] = Some(synthetic_tool_result(&calls[i], "cancelled".into()));
                emit_failed(self.wire, self.session_id, &calls[i], "cancelled").await;
            }
        }
    }
}

/// Outcome of invoking a single tool. The wire notification is emitted by
/// the caller so the spawn loop and the (degenerate, max_parallel=1) path
/// share the same logic.
enum InvokeOutcome {
    Done(ToolResult),
    Failed(String),
}

/// Standalone tool invocation. Takes only owned/cloned handles so it can
/// run inside a spawned task. On timeout, kills the offending MCP server's
/// process group and marks it dead; the registry's lazy restart handles it
/// on the next call.
async fn invoke_tool_inner(
    mcp: &Arc<McpRegistry>,
    call: &ToolCall,
    tool_timeout: std::time::Duration,
    mut cancel: watch::Receiver<bool>,
) -> InvokeOutcome {
    if *cancel.borrow() {
        return InvokeOutcome::Failed("cancelled".into());
    }
    match tokio::time::timeout(
        tool_timeout,
        mcp.call(
            &call.name,
            &call.provider_id,
            &call.arguments,
            MAX_TOOL_RESULT_BYTES,
            &mut cancel,
        ),
    )
    .await
    {
        Ok(Ok(result)) => InvokeOutcome::Done(result),
        Ok(Err(AgentError::Cancelled)) => InvokeOutcome::Failed("cancelled".into()),
        Ok(Err(e)) => InvokeOutcome::Failed(e.to_string()),
        Err(_) => {
            // If the session was cancelled, the timeout fired because
            // do_call returned quickly with "cancelled" and the outer
            // timeout raced. Don't kill a healthy server for that.
            if *cancel.borrow() {
                return InvokeOutcome::Failed("cancelled".into());
            }
            if let Some(server) = mcp.server_of(&call.name) {
                mcp.kill_server(server, "tool timeout");
            }
            let msg = format!(
                "tool: timeout after {}s. The command took too long. Try a faster approach.",
                tool_timeout.as_secs()
            );
            InvokeOutcome::Failed(msg)
        }
    }
}

fn outcome_to_result(call: &ToolCall, outcome: InvokeOutcome) -> ToolResult {
    match outcome {
        InvokeOutcome::Done(r) => r,
        InvokeOutcome::Failed(m) => synthetic_tool_result(call, m),
    }
}

async fn emit_pending(wire: &WireSender, sid: &str, call: &ToolCall) {
    wire::send(
        wire,
        wire::session_update(
            sid,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": call.provider_id,
                "title": call.name,
                "kind": "other",
                "status": "pending",
                "rawInput": call.arguments,
            }),
        ),
    )
    .await;
}

async fn emit_in_progress(wire: &WireSender, sid: &str, call: &ToolCall) {
    wire::send(
        wire,
        wire::session_update(
            sid,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call.provider_id,
                "status": "in_progress",
            }),
        ),
    )
    .await;
}

async fn emit_completed(wire: &WireSender, sid: &str, call: &ToolCall, result: &ToolResult) {
    wire::send(
        wire,
        wire::session_update(
            sid,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call.provider_id,
                "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text", "text": result.text() } }],
                "rawOutput": { "isError": result.is_error },
            }),
        ),
    )
    .await;
}

async fn emit_failed(wire: &WireSender, sid: &str, call: &ToolCall, err: &str) {
    wire::send(
        wire,
        wire::session_update(
            sid,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call.provider_id,
                "status": "failed",
                "rawOutput": { "error": err },
            }),
        ),
    )
    .await;
}

fn prompt_to_text(prompt: Vec<ContentBlock>) -> Result<String, AgentError> {
    let mut parts = Vec::with_capacity(prompt.len());
    for block in prompt {
        match block {
            ContentBlock::Text { text } => parts.push(text),
            ContentBlock::ResourceLink { uri } => parts.push(format!("[resource: {uri}]")),
            ContentBlock::Unsupported => {
                return Err(AgentError::InvalidParams(
                    "prompt: unsupported content block (only text and resource_link are advertised)".into(),
                ));
            }
        }
    }
    Ok(parts.join("\n"))
}

/// Format a single hook output as a structured tool-result body.
///
/// We emit a JSON object rather than XML-style tags. JSON is unambiguous:
/// the inner `text` field is escaped, so a malicious hook cannot break
/// out by including a literal `</hook_output>` (or any other delimiter)
/// in its output. The LLM still sees the source attribution via the
/// `hook` and `server` fields.
fn format_hook_output_body(hook: &str, server: &str, text: &str) -> String {
    // serde_json::to_string never fails on owned strings.
    serde_json::to_string(&json!({
        "hook": hook,
        "server": server,
        "text": text,
    }))
    .unwrap_or_else(|_| String::from("{\"hook\":\"\",\"server\":\"\",\"text\":\"\"}"))
}

/// Synthetic provider id for an injected hook tool-call/result pair. Must
/// be unique per pair so the LLM wire format (which keys tool results by
/// id) stays valid across multiple objections in one session.
fn synthetic_hook_id(hook: &str, server: &str, ordinal: u64) -> String {
    format!("buzz_hook_{hook}_{server}_{ordinal}")
}

/// Append a synthetic Assistant tool-call + ToolResult pair for each hook
/// output. Modeling hook output as a tool result (rather than as a User
/// message) means a malicious hook can't impersonate the user or system
/// — the LLM treats tool results as lower-trust, structured data.
///
/// Each pair uses the hook's qualified tool name (e.g. `fake___Stop`) so
/// attribution is preserved in the wire format. Empty arguments are sent
/// as `{}`. The `Assistant` turn carries no text (tool_calls only).
pub(crate) fn push_hook_outputs_as_tool_results(
    history: &mut Vec<HistoryItem>,
    hook: &str,
    outputs: &[(String, String)],
) {
    for (server, text) in outputs.iter() {
        let provider_id = synthetic_hook_id(hook, server, unique_nonce());
        // Tool name is `<server>__<hook>` — same shape as a real qname
        // for that hook, so the LLM never sees an unknown synthetic name.
        let tool_name = format!("{server}__{hook}");
        history.push(HistoryItem::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall {
                provider_id: provider_id.clone(),
                name: tool_name,
                arguments: serde_json::json!({}),
            }],
        });
        history.push(HistoryItem::ToolResult(ToolResult {
            provider_id,
            content: vec![ToolResultContent::Text(format_hook_output_body(
                hook, server, text,
            ))],
            is_error: false,
        }));
    }
}

/// Monotonic counter for synthetic hook ids within a single process. The
/// uniqueness target is "no collision within the lifetime of one history
/// vec", which a process-wide counter satisfies trivially.
fn unique_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn synthetic_tool_result(call: &ToolCall, msg: String) -> ToolResult {
    ToolResult {
        provider_id: call.provider_id.clone(),
        content: vec![ToolResultContent::Text(msg)],
        is_error: true,
    }
}

pub(crate) fn truncate_history(history: &mut Vec<HistoryItem>, max_bytes: usize) {
    let mut total: usize = history.iter().map(HistoryItem::estimated_bytes).sum();
    if total <= max_bytes {
        return;
    }
    let original_len = history.len();
    while total > max_bytes && !history.is_empty() {
        let mut end = 1usize;
        while end < history.len() && !matches!(history[end], HistoryItem::User(_)) {
            end += 1;
        }
        if end >= history.len() {
            break;
        }
        let dropped: usize = history[..end]
            .iter()
            .map(HistoryItem::estimated_bytes)
            .sum();
        history.drain(..end);
        total = total.saturating_sub(dropped);
    }
    if history.len() < original_len {
        tracing::info!(
            "history truncated {original_len} -> {} items ({total} bytes)",
            history.len()
        );
    }
}

fn map_stop(p: ProviderStop) -> StopReason {
    match p {
        ProviderStop::EndTurn | ProviderStop::ToolUse | ProviderStop::Other => StopReason::EndTurn,
        ProviderStop::MaxTokens => StopReason::MaxTokens,
        ProviderStop::Refusal => StopReason::Refusal,
    }
}
