use std::sync::Arc;

use serde_json::json;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;

use crate::builtin;
use crate::config::{
    Config, Provider, MAX_PROMPT_BYTES, MAX_TOOL_CALLS_PER_TURN, MAX_TOOL_RESULT_BYTES,
};
use crate::handoff::HandoffOutcome;
use crate::hints::SkillEntry;
use crate::llm::{Llm, LmStudioNativeClient};
use crate::lmstudio::{LmStudioChatRequest, LmStudioOutput};
use crate::mcp::McpRegistry;
use crate::mcp::ResultBudget;

use crate::types::{
    clamp, AgentError, ContentBlock, ExecutedToolCall, ExecutedToolProvider, HistoryItem,
    ProviderStop, StopReason, ToolCall, ToolResult, ToolResultContent,
};
use crate::wire::{self, WireSender};

const ERROR_REFLECTION_SUFFIX: &str =
    "\n\n[Reflect] Before retrying, identify the cause and change your approach.";

/// Native evidence text is serialized into a single ACP JSON line. JSON can
/// expand one input byte to six bytes (`\u00xx`), so 512 KiB leaves at least
/// 960 KiB for the fixed envelope and bounded metadata below the 4 MiB line
/// budget even in the worst case. This outbound boundary is intentionally
/// independent of the larger generic MCP history/result setting.
const MAX_NATIVE_ACP_EVIDENCE_TEXT_BYTES: usize = 512 * 1024;

pub struct RunCtx<'a> {
    pub cfg: &'a Config,
    /// Effective model for this session. Usually equals `cfg.model`; overridden
    /// per-session by `session/set_model`. All LLM calls use this value.
    pub effective_model: &'a str,
    pub session_id: &'a str,
    pub system_prompt: &'a str,
    pub llm: &'a Llm,
    /// Dedicated stateful native transport, present only for LM Studio sessions.
    pub native_llm: Option<&'a Arc<LmStudioNativeClient>>,
    pub mcp: &'a Arc<McpRegistry>,
    /// Skills discovered at session creation; used by the built-in `load_skill` tool.
    pub skills: &'a [SkillEntry],
    pub wire: &'a WireSender,
    pub cancel: &'a mut watch::Receiver<bool>,
    /// Mid-turn steer queue. Drained at each round boundary (before the next
    /// LLM call): queued messages are appended to history as user turns so the
    /// model sees them on its next request, without restarting the turn. Fed by
    /// the `_goose/unstable/session/steer` handler.
    pub steer: &'a mut mpsc::UnboundedReceiver<Vec<ContentBlock>>,
    pub history: &'a mut Vec<HistoryItem>,
    pub original_task: &'a mut Option<String>,
    pub handoff_count: &'a mut usize,
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
    /// Accumulated input tokens across all LLM rounds in this turn, for
    /// NIP-AM metric publishing. Reset to `None` at turn start in `run()`.
    pub turn_input_tokens: &'a mut Option<u64>,
    /// Accumulated output tokens across all LLM rounds in this turn, for
    /// NIP-AM metric publishing. Reset to `None` at turn start in `run()`.
    pub turn_output_tokens: &'a mut Option<u64>,
    /// Private native response state for this ACP session.
    pub native_response_id: &'a mut Option<String>,
    /// Per-session request sequence used for stable native evidence identifiers.
    pub native_request_sequence: &'a mut u64,
}

impl RunCtx<'_> {
    pub async fn run(&mut self, prompt: Vec<ContentBlock>) -> Result<StopReason, AgentError> {
        if self.cfg.provider == Provider::LmStudioNative {
            return self.run_native(prompt).await;
        }
        let user_text = prompt_to_text(prompt)?;
        if user_text.len() > MAX_PROMPT_BYTES {
            return Err(AgentError::InvalidParams(format!(
                "prompt: exceeds {MAX_PROMPT_BYTES} bytes"
            )));
        }
        if self.original_task.is_none() {
            *self.original_task = Some(user_text.clone());
        }
        let n2_destination = n2_publish_destination(&user_text);
        self.history.push(HistoryItem::User(user_text));

        let (n2_evidence_prefetched, prefetch_stop) = self.prefetch_n2_evidence().await;
        if let Some(stop) = prefetch_stop {
            return Ok(stop);
        }

        // Reset per-turn token accumulators for this prompt.
        *self.turn_input_tokens = None;
        *self.turn_output_tokens = None;

        let mut round = 0u32;
        // Per-prompt `_Stop` objection count. Bounded per prompt (not per
        // session) so a stubborn exchange can't permanently disable the stop
        // guard for a long-lived session; `max_rounds` still caps the loop.
        let mut stop_rejections = 0u32;
        let mut n2_empty_retry_used = false;
        loop {
            if self.cfg.max_rounds > 0 && round >= self.cfg.max_rounds {
                return Ok(StopReason::MaxTurnRequests);
            }
            if *self.cancel.borrow() {
                return Ok(StopReason::Cancelled);
            }
            // Round boundary: fold in any steer messages queued since the last
            // round. They land as user turns so the model incorporates them on
            // its next request — the turn continues, it is not restarted. Drain
            // non-blocking; an empty queue is the common case.
            self.drain_steers();
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

            let mut tools = self.mcp.tools();
            // Inject the built-in load_skill tool when skills are available.
            if !self.skills.is_empty() {
                tools.push(builtin::load_skill_def());
            }
            round = round.saturating_add(1);
            let response = tokio::select! {
                biased;
                _ = self.cancel.changed() => return Ok(StopReason::Cancelled),
                r = self.llm.complete(self.cfg, self.system_prompt, self.history, &tools, self.effective_model) => r?,
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
            // appended before the next request. Uses `context_pressure_bytes`
            // (the same measure the gate's `current_bytes` uses) so the
            // `grown` delta is coherent — an image contributes its visual-
            // token equivalent here, not its base64 length. Preserve both when
            // a response omits usage (`None`) rather than clobbering — a
            // one-off missing field shouldn't blind the gate or zero the
            // growth baseline.
            if let Some(tokens) = response.input_tokens {
                *self.last_request_input_tokens = Some(tokens);
                *self.last_request_history_bytes = Some(
                    self.history
                        .iter()
                        .map(HistoryItem::context_pressure_bytes)
                        .sum(),
                );
                // Accumulate per-turn input tokens for NIP-AM metric publishing.
                *self.turn_input_tokens =
                    Some(self.turn_input_tokens.unwrap_or(0).saturating_add(tokens));
            }
            // Accumulate per-turn output tokens for NIP-AM metric publishing.
            if let Some(out) = response.output_tokens {
                *self.turn_output_tokens =
                    Some(self.turn_output_tokens.unwrap_or(0).saturating_add(out));
            }

            if !response.reasoning.is_empty() {
                wire::send(
                    self.wire,
                    wire::session_update(
                        self.session_id,
                        json!({
                            "sessionUpdate": "agent_thought_chunk",
                            "content": { "type": "text", "text": &response.reasoning }
                        }),
                    ),
                )
                .await;
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
                if n2_evidence_prefetched {
                    tracing::info!(
                        stop = ?response.stop,
                        text_bytes = response.text.len(),
                        reasoning_bytes = response.reasoning.len(),
                        "Maritime N2 model round completed after evidence prefetch"
                    );
                }
                if response.stop == ProviderStop::ToolUse {
                    return Err(AgentError::Llm(
                        "provider: stop=tool_use but zero tool_calls".into(),
                    ));
                }
                if should_retry_empty_n2_response(
                    n2_evidence_prefetched,
                    n2_empty_retry_used,
                    response.stop,
                    &response.text,
                ) {
                    tracing::warn!(
                        "Maritime N2 model returned no answer after evidence prefetch; retrying once"
                    );
                    self.history.push(HistoryItem::Assistant {
                        text: response.text,
                        tool_calls: Vec::new(),
                    });
                    self.history.push(HistoryItem::User(
                        "The World Monitor evidence fetch has completed. Answer the original \
                         question now with a substantive intelligence assessment. State key \
                         developments, uncertainties, and implications; do not reply with a \
                         pickup acknowledgement or a promise to investigate later."
                            .to_string(),
                    ));
                    n2_empty_retry_used = true;
                    continue;
                }
                if should_auto_publish_n2_response(
                    n2_evidence_prefetched,
                    response.tool_calls.is_empty(),
                    &response.text,
                ) && matches!(response.stop, ProviderStop::EndTurn | ProviderStop::Other)
                {
                    let destination = n2_destination.as_ref().ok_or_else(|| {
                        AgentError::InvalidParams(
                            "Maritime N2 response cannot be published because the current Buzz \
                             channel is missing from the prompt context"
                                .into(),
                        )
                    })?;
                    if let Some(stop) = self
                        .publish_n2_response(destination, &response.text)
                        .await?
                    {
                        return Ok(stop);
                    }
                    return Ok(map_stop(response.stop));
                }
                self.history.push(HistoryItem::Assistant {
                    text: response.text,
                    tool_calls: Vec::new(),
                });
                let stop = map_stop(response.stop);
                // Only gate genuine end_turn — don't override max_tokens/refusal.
                if stop == StopReason::EndTurn {
                    if stop_rejections >= self.cfg.stop_max_rejections {
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
                        stop_rejections = stop_rejections.saturating_add(1);
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

            if let Some(stop) = self.execute_calls(&calls).await {
                return Ok(stop);
            }
        }
    }

    async fn run_native(&mut self, prompt: Vec<ContentBlock>) -> Result<StopReason, AgentError> {
        let user_text = prompt_to_text(prompt)?;
        if user_text.len() > MAX_PROMPT_BYTES {
            return Err(AgentError::InvalidParams(format!(
                "prompt: exceeds {MAX_PROMPT_BYTES} bytes"
            )));
        }
        *self.turn_input_tokens = None;
        *self.turn_output_tokens = None;
        let client = self.native_llm.ok_or_else(|| {
            AgentError::InvalidParams(
                "LM Studio native runtime is missing its dedicated client".into(),
            )
        })?;
        let runtime = self.cfg.lmstudio_runtime.as_ref().ok_or_else(|| {
            AgentError::InvalidParams(
                "LM Studio native runtime is missing validated egress policy".into(),
            )
        })?;
        let mut request = LmStudioChatRequest::new(
            self.effective_model,
            user_text.as_str(),
            self.system_prompt,
            runtime.wire_integrations(),
            self.cfg.lmstudio_reasoning,
            self.cfg.max_output_tokens,
            self.cfg.max_context_tokens,
        )?;
        if let Some(previous_response_id) = self.native_response_id.as_deref() {
            request = request.continue_from(previous_response_id)?;
        }
        let history_start = self.history.len();
        let original_task_was_empty = self.original_task.is_none();
        if original_task_was_empty {
            *self.original_task = Some(user_text.clone());
        }
        self.history.push(HistoryItem::User(user_text));
        if *self.cancel.borrow() {
            self.rollback_native_prompt(history_start, original_task_was_empty);
            return Ok(StopReason::Cancelled);
        }
        if self.native_response_id.is_some() && self.should_handoff() {
            self.rollback_native_prompt(history_start, original_task_was_empty);
            return Err(AgentError::InvalidParams(
                "LM Studio native context handoff is unavailable; start a new ACP session".into(),
            ));
        }

        let request_sequence = self.native_request_sequence.saturating_add(1);
        *self.native_request_sequence = request_sequence;
        let request_identity = format!("{}:{request_sequence}", self.session_id);
        let response_result = tokio::select! {
            biased;
            _ = self.cancel.changed() => Err(AgentError::Cancelled),
            result = async {
                if request.previous_response_id().is_some() {
                    client.continue_chat(&request, &request_identity).await
                } else {
                    client.chat(&request, &request_identity).await
                }
            } => result,
            _ = async {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    wire::send(
                        self.wire,
                        wire::session_update(
                            self.session_id,
                            json!({ "sessionUpdate": "keepalive" }),
                        ),
                    )
                    .await;
                }
            } => unreachable!(),
        };
        let response = match response_result {
            Ok(response) => response,
            Err(AgentError::Cancelled) => {
                self.rollback_native_prompt(history_start, original_task_was_empty);
                return Ok(StopReason::Cancelled);
            }
            Err(error) => {
                self.rollback_native_prompt(history_start, original_task_was_empty);
                return Err(error);
            }
        };

        *self.native_response_id = Some(response.response_id.clone());
        *self.last_request_input_tokens = Some(response.stats.input_tokens);
        *self.last_request_history_bytes = Some(
            self.history
                .iter()
                .map(HistoryItem::context_pressure_bytes)
                .sum(),
        );
        *self.turn_input_tokens = Some(response.stats.input_tokens);
        *self.turn_output_tokens = Some(response.stats.total_output_tokens);

        let mut assistant_text = Vec::new();
        for (output_index, output) in response.output.into_iter().enumerate() {
            let message_id = format!(
                "lmstudio_{}_{}",
                request_identity.replace(':', "_"),
                output_index
            );
            match output {
                LmStudioOutput::Reasoning { content } => {
                    let content = clamp(content, MAX_PROMPT_BYTES);
                    wire::send(
                        self.wire,
                        wire::session_update(
                            self.session_id,
                            json!({
                                "sessionUpdate": "agent_thought_chunk",
                                "messageId": message_id,
                                "content": { "type": "text", "text": content }
                            }),
                        ),
                    )
                    .await;
                }
                LmStudioOutput::Message { content } => {
                    let content = clamp(content, MAX_PROMPT_BYTES);
                    assistant_text.push(content.clone());
                    wire::send(
                        self.wire,
                        wire::session_update(
                            self.session_id,
                            json!({
                                "sessionUpdate": "agent_message_chunk",
                                "messageId": message_id,
                                "content": { "type": "text", "text": content }
                            }),
                        ),
                    )
                    .await;
                }
                LmStudioOutput::ToolCall(call) => {
                    emit_native_completed(
                        self.wire,
                        self.session_id,
                        &call,
                        &response.model_instance_id,
                        self.cfg
                            .max_tool_result_text_bytes
                            .min(MAX_NATIVE_ACP_EVIDENCE_TEXT_BYTES),
                    )
                    .await;
                }
            }
        }
        self.history.push(HistoryItem::Assistant {
            text: assistant_text.join("\n"),
            tool_calls: Vec::new(),
        });
        Ok(StopReason::EndTurn)
    }

    fn rollback_native_prompt(&mut self, history_start: usize, original_task_was_empty: bool) {
        self.history.truncate(history_start);
        if original_task_was_empty {
            *self.original_task = None;
        }
    }

    /// Non-blocking drain of the steer queue. Each queued steer is appended to
    /// history as a user turn so the model picks it up on its next request. A
    /// steer whose blocks all fail to render (e.g. unsupported content) is
    /// skipped rather than aborting the turn — steering is best-effort
    /// augmentation, not a hard input contract like the initial prompt.
    fn drain_steers(&mut self) {
        while let Ok(blocks) = self.steer.try_recv() {
            match prompt_to_text(blocks) {
                Ok(text) if !text.trim().is_empty() => {
                    self.history.push(HistoryItem::User(text));
                }
                Ok(_) => {
                    tracing::debug!("dropping empty steer message");
                }
                Err(e) => {
                    tracing::warn!("dropping unrenderable steer message: {e}");
                }
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

            // Built-in load_skill: execute inline, no MCP round-trip.
            if call.name == builtin::LOAD_SKILL_TOOL {
                emit_in_progress(self.wire, self.session_id, call).await;
                let mut result = builtin::call_load_skill(&call.arguments, self.skills).await;
                result.provider_id = call.provider_id.clone();
                emit_completed(self.wire, self.session_id, call, &result).await;
                results[idx] = Some(result);
                continue;
            }

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

    async fn prefetch_n2_evidence(&mut self) -> (bool, Option<StopReason>) {
        let Some(user_text) = self.history.iter().rev().find_map(|item| match item {
            HistoryItem::User(text) => Some(text.as_str()),
            _ => None,
        }) else {
            return (false, None);
        };
        let persona_id = std::env::var("COMMAND_ADVISER_PERSONA_ID").ok();
        let requested = n2_prefetches_for(persona_id.as_deref(), user_text);
        if requested.is_empty() {
            return (false, None);
        }
        let available = self.mcp.tools();
        let mut calls = Vec::with_capacity(requested.len());
        for (bare_name, arguments) in &requested {
            let Some(qualified_name) = available
                .iter()
                .find(|tool| {
                    tool.name
                        .rsplit_once("__")
                        .is_some_and(|(_, bare)| bare == *bare_name)
                })
                .map(|tool| tool.name.clone())
            else {
                tracing::warn!(tool = bare_name, "Maritime N2 evidence tool is unavailable");
                continue;
            };
            calls.push(ToolCall {
                provider_id: format!("buzz_n2_prefetch_{}", unique_nonce()),
                name: qualified_name,
                arguments: arguments.clone(),
            });
        }
        if calls.is_empty() {
            return (false, None);
        }
        let call_ids = calls
            .iter()
            .map(|call| call.provider_id.clone())
            .collect::<Vec<_>>();
        tracing::info!(
            calls = calls.len(),
            requested = requested.len(),
            "prefetching Maritime N2 evidence and doctrine"
        );
        self.history.push(HistoryItem::Assistant {
            text: String::new(),
            tool_calls: calls.clone(),
        });
        let stop = self.execute_calls(&calls).await;
        if stop.is_none() {
            let succeeded = self
                .history
                .iter()
                .filter_map(|item| match item {
                    HistoryItem::ToolResult(result)
                        if call_ids.contains(&result.provider_id) && !result.is_error =>
                    {
                        Some(())
                    }
                    _ => None,
                })
                .count();
            self.history
                .push(HistoryItem::User(n2_prefetch_status_instruction(
                    succeeded,
                    requested.len(),
                )));
            tracing::info!(
                succeeded,
                requested = requested.len(),
                "Maritime N2 evidence prefetch completed"
            );
        }
        (true, stop)
    }

    async fn publish_n2_response(
        &mut self,
        destination: &N2PublishDestination,
        assessment: &str,
    ) -> Result<Option<StopReason>, AgentError> {
        let qualified_name = self
            .mcp
            .tools()
            .into_iter()
            .find(|tool| {
                tool.name
                    .rsplit_once("__")
                    .is_some_and(|(_, bare)| bare == "shell")
            })
            .map(|tool| tool.name)
            .ok_or_else(|| {
                AgentError::Mcp(
                    "Maritime N2 response cannot be published because the Buzz shell tool is \
                     unavailable"
                        .into(),
                )
            })?;
        let call = ToolCall {
            provider_id: format!("buzz_n2_publish_{}", unique_nonce()),
            name: qualified_name,
            arguments: n2_publish_shell_arguments(destination, assessment),
        };
        self.history.push(HistoryItem::Assistant {
            text: assessment.to_string(),
            tool_calls: vec![call.clone()],
        });
        if let Some(stop) = self.execute_calls(std::slice::from_ref(&call)).await {
            return Ok(Some(stop));
        }
        let result = self
            .history
            .iter()
            .rev()
            .find_map(|item| match item {
                HistoryItem::ToolResult(result) if result.provider_id == call.provider_id => {
                    Some(result)
                }
                _ => None,
            })
            .ok_or_else(|| {
                AgentError::Mcp("Maritime N2 Buzz publication returned no tool result".into())
            })?;
        validate_n2_publish_result(result.is_error, &result.text()).map_err(|message| {
            AgentError::Mcp(format!("Maritime N2 publication failed: {message}"))
        })?;
        tracing::info!(
            channel = destination.channel_id,
            threaded = destination.reply_to.is_some(),
            text_bytes = assessment.len(),
            "published Maritime N2 assessment to Buzz"
        );
        Ok(None)
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
            let budget = ResultBudget {
                total: MAX_TOOL_RESULT_BYTES,
                text: self.cfg.max_tool_result_text_bytes,
            };
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
                let outcome = invoke_tool_inner(&mcp, &call, timeout, budget, cancel).await;
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
    budget: ResultBudget,
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
            budget,
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

async fn emit_native_completed(
    wire: &WireSender,
    sid: &str,
    call: &ExecutedToolCall,
    model_instance_id: &str,
    evidence_text_limit: usize,
) {
    let provider = match &call.provider {
        ExecutedToolProvider::EphemeralMcp { server_label } => {
            json!({ "type": "ephemeral_mcp", "serverLabel": server_label })
        }
        ExecutedToolProvider::Plugin { plugin_id } => {
            json!({ "type": "plugin", "pluginId": plugin_id })
        }
    };
    let bounded_arguments = bounded_native_arguments(&call.arguments, evidence_text_limit);
    let bounded_output = clamp(call.output.clone(), evidence_text_limit);
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
                "rawInput": bounded_arguments,
                "rawOutput": {
                    "executedByProvider": true,
                    "provider": provider.clone(),
                    "modelInstanceId": model_instance_id,
                },
            }),
        ),
    )
    .await;
    wire::send(
        wire,
        wire::session_update(
            sid,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": call.provider_id,
                "status": "completed",
                "content": [{
                    "type": "content",
                    "content": { "type": "text", "text": bounded_output }
                }],
                "rawOutput": {
                    "isError": false,
                    "executedByProvider": true,
                    "provider": provider,
                    "modelInstanceId": model_instance_id,
                    "tool": call.name,
                },
            }),
        ),
    )
    .await;
}

fn bounded_native_arguments(arguments: &serde_json::Value, limit: usize) -> serde_json::Value {
    let serialized = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".into());
    if serialized.len() <= limit {
        return arguments.clone();
    }
    let preview_limit = limit.saturating_sub(128);
    json!({
        "_buzzTruncated": true,
        "preview": clamp(serialized, preview_limit),
    })
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

fn n2_prefetches_for(
    persona_id: Option<&str>,
    user_text: &str,
) -> Vec<(&'static str, serde_json::Value)> {
    if persona_id != Some("builtin:command-intelligence") {
        return Vec::new();
    }
    if user_text.to_ascii_lowercase().contains("south china sea") {
        return vec![
            (
                "world_monitor_military_posture",
                json!({"country_code":"PH","limit":25}),
            ),
            (
                "world_monitor_maritime_activity",
                json!({"country_code":"PH","limit":25}),
            ),
            (
                "search_command_doctrine",
                json!({
                    "query":"maritime intelligence assessment, operational planning, logistics support and risk",
                    "top_k":5
                }),
            ),
        ];
    }
    vec![
        (
            "world_monitor_news_intelligence",
            json!({"topic":"intelligence","limit":25,"days":7}),
        ),
        (
            "search_command_doctrine",
            json!({
                "query":"intelligence assessment, operational planning, logistics support and risk",
                "top_k":5
            }),
        ),
    ]
}

fn n2_prefetch_status_instruction(succeeded: usize, requested: usize) -> String {
    if succeeded == requested {
        return format!(
            "All {requested} current-run evidence calls succeeded. Use the immediately preceding \
             World Monitor and doctrine tool results as the evidence for this answer. Disregard \
             earlier conversation messages that reported tool or connection failures; they \
             describe older runs. Answer the user's current question now, distinguish reported \
             information from assessment, and state any gaps in what the successful evidence \
             actually supports."
        );
    }
    if succeeded == 0 {
        return format!(
            "All {requested} current-run evidence calls failed. Disregard earlier promises to \
             investigate later. Continue with other available information, identify the current \
             source failure briefly, and answer the user's current question as far as the \
             available evidence permits."
        );
    }
    format!(
        "{succeeded} of {requested} current-run evidence calls succeeded. Use the successful \
         results, disregard earlier conversation messages that report a different connection \
         state, identify only the evidence that is currently missing, and answer the user's \
         current question now."
    )
}

#[derive(Debug, PartialEq, Eq)]
struct N2PublishDestination {
    channel_id: String,
    reply_to: Option<String>,
}

fn n2_publish_destination(prompt: &str) -> Option<N2PublishDestination> {
    let context = prompt
        .split_once("[Context]\n")?
        .1
        .split("\n[")
        .next()
        .unwrap_or("");
    let channel_line = context
        .lines()
        .find(|line| line.trim_start().starts_with("Channel:"))?;
    let channel_id = channel_line
        .split(|character: char| !character.is_ascii_hexdigit() && character != '-')
        .find(|candidate| candidate.len() == 36 && uuid::Uuid::parse_str(candidate).is_ok())?
        .to_ascii_lowercase();
    let reply_to = context.rfind("--reply-to ").and_then(|index| {
        let candidate = context[index + "--reply-to ".len()..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect::<String>();
        (candidate.len() == 64).then(|| candidate.to_ascii_lowercase())
    });
    Some(N2PublishDestination {
        channel_id,
        reply_to,
    })
}

fn n2_publish_shell_arguments(
    destination: &N2PublishDestination,
    assessment: &str,
) -> serde_json::Value {
    let encoded = hex::encode(assessment.as_bytes());
    let reply = destination
        .reply_to
        .as_ref()
        .map(|event_id| format!(" --reply-to '{event_id}'"))
        .unwrap_or_default();
    json!({
        "command": format!(
            "printf '%s' '{encoded}' | xxd -r -p | buzz messages send --channel '{}' --content -{reply}",
            destination.channel_id
        ),
        "timeout_ms": 120_000,
    })
}

fn should_auto_publish_n2_response(
    evidence_prefetched: bool,
    no_model_tool_calls: bool,
    text: &str,
) -> bool {
    evidence_prefetched && no_model_tool_calls && !text.trim().is_empty()
}

fn validate_n2_publish_result(is_error: bool, result_text: &str) -> Result<(), String> {
    if is_error {
        return Err("Buzz shell tool reported an error".into());
    }
    let shell_result: serde_json::Value = serde_json::from_str(result_text)
        .map_err(|_| "Buzz shell tool returned malformed output".to_string())?;
    if shell_result
        .get("exit_code")
        .and_then(|value| value.as_i64())
        != Some(0)
    {
        return Err("Buzz message command returned a non-zero exit status".into());
    }
    let stdout = shell_result
        .get("stdout")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Buzz message command returned no output".to_string())?;
    let write_result = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .ok_or_else(|| "Buzz message command returned an unreadable relay response".to_string())?;
    if write_result
        .get("accepted")
        .and_then(|value| value.as_bool())
        != Some(true)
    {
        return Err("Buzz relay did not accept the N2 assessment".into());
    }
    Ok(())
}

fn should_retry_empty_n2_response(
    evidence_prefetched: bool,
    retry_used: bool,
    stop: ProviderStop,
    text: &str,
) -> bool {
    evidence_prefetched
        && !retry_used
        && matches!(stop, ProviderStop::EndTurn | ProviderStop::Other)
        && text.trim().is_empty()
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

#[cfg(test)]
mod tests {
    use super::{
        n2_prefetch_status_instruction, n2_prefetches_for, n2_publish_destination,
        n2_publish_shell_arguments, should_auto_publish_n2_response,
        should_retry_empty_n2_response, validate_n2_publish_result,
    };
    use crate::types::ProviderStop;
    use serde_json::json;

    #[test]
    fn n2_prefetches_current_regional_evidence_and_doctrine_before_the_model_round() {
        assert_eq!(
            n2_prefetches_for(
                Some("builtin:command-intelligence"),
                "What is happening in the South China Sea today?"
            ),
            vec![
                (
                    "world_monitor_military_posture",
                    json!({"country_code":"PH","limit":25})
                ),
                (
                    "world_monitor_maritime_activity",
                    json!({"country_code":"PH","limit":25})
                ),
                (
                    "search_command_doctrine",
                    json!({
                        "query":"maritime intelligence assessment, operational planning, logistics support and risk",
                        "top_k":5
                    })
                ),
            ]
        );
        assert_eq!(
            n2_prefetches_for(
                Some("builtin:command-intelligence"),
                "Give me a current regional intelligence update."
            ),
            vec![
                (
                    "world_monitor_news_intelligence",
                    json!({"topic":"intelligence","limit":25,"days":7})
                ),
                (
                    "search_command_doctrine",
                    json!({
                        "query":"intelligence assessment, operational planning, logistics support and risk",
                        "top_k":5
                    })
                ),
            ]
        );
        assert_eq!(
            n2_prefetches_for(
                Some("builtin:command-operations"),
                "What is happening in the South China Sea today?"
            ),
            Vec::new()
        );
    }

    #[test]
    fn n2_current_run_status_overrides_stale_failure_messages_in_dm_history() {
        let succeeded = n2_prefetch_status_instruction(3, 3);
        assert!(succeeded.contains("current-run evidence calls succeeded"));
        assert!(succeeded.contains("disregard earlier conversation messages"));
        assert!(succeeded.contains("answer now"));

        let partial = n2_prefetch_status_instruction(2, 3);
        assert!(partial.contains("2 of 3"));
        assert!(partial.contains("use the successful results"));

        let failed = n2_prefetch_status_instruction(0, 3);
        assert!(failed.contains("current-run evidence calls failed"));
        assert!(failed.contains("continue with other available information"));
    }

    #[test]
    fn n2_retries_one_empty_end_turn_after_evidence_prefetch() {
        assert!(should_retry_empty_n2_response(
            true,
            false,
            ProviderStop::EndTurn,
            "  "
        ));
        assert!(should_retry_empty_n2_response(
            true,
            false,
            ProviderStop::Other,
            ""
        ));
        assert!(!should_retry_empty_n2_response(
            true,
            true,
            ProviderStop::EndTurn,
            ""
        ));
        assert!(!should_retry_empty_n2_response(
            false,
            false,
            ProviderStop::EndTurn,
            ""
        ));
        assert!(!should_retry_empty_n2_response(
            true,
            false,
            ProviderStop::Refusal,
            ""
        ));
        assert!(!should_retry_empty_n2_response(
            true,
            false,
            ProviderStop::EndTurn,
            "Assessment"
        ));
    }

    #[test]
    fn n2_extracts_the_current_buzz_dm_destination_from_context() {
        let destination = n2_publish_destination(
            "[Context]\n\
             Scope: dm\n\
             Channel: Maritime N2 (#ea5388d5-9e15-4858-a0ae-45e4f43472f6)\n\
             Thread root: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
             IMPORTANT: For ordinary replies in this turn, use `--reply-to \
             bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb` on \
             `buzz messages send`.\n\
             [Buzz event: current]",
        )
        .expect("current destination");

        assert_eq!(
            destination.channel_id,
            "ea5388d5-9e15-4858-a0ae-45e4f43472f6"
        );
        assert_eq!(
            destination.reply_to.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn n2_builds_a_shell_publish_call_without_interpolating_assessment_text() {
        let destination = n2_publish_destination(
            "[Context]\n\
             Scope: dm\n\
             Channel: ea5388d5-9e15-4858-a0ae-45e4f43472f6\n\
             Conversation context included below.\n\
             [Buzz event: current]",
        )
        .expect("current destination");
        let assessment = "Current picture: `$HOME`; $(touch /tmp/never-run).\n\nLogistics: fuel.";

        let arguments = n2_publish_shell_arguments(&destination, assessment);
        let command = arguments["command"].as_str().expect("command");

        assert_eq!(arguments["timeout_ms"], 120_000);
        assert!(command.contains(
            "buzz messages send --channel 'ea5388d5-9e15-4858-a0ae-45e4f43472f6' --content -"
        ));
        assert!(!command.contains(assessment));
        assert!(!command.contains("$HOME"));
        assert!(!command.contains("touch /tmp/never-run"));
        assert!(command.contains(&hex::encode(assessment.as_bytes())));
        assert!(!command.contains("--reply-to"));
    }

    #[test]
    fn n2_auto_publishes_only_a_substantive_plain_response_after_prefetch() {
        assert!(should_auto_publish_n2_response(true, true, "Assessment"));
        assert!(!should_auto_publish_n2_response(false, true, "Assessment"));
        assert!(!should_auto_publish_n2_response(true, false, "Assessment"));
        assert!(!should_auto_publish_n2_response(true, true, "  "));
    }

    #[test]
    fn n2_requires_a_successful_accepted_buzz_write() {
        let accepted = json!({
            "exit_code": 0,
            "stdout": "{\"event_id\":\"abc\",\"accepted\":true,\"message\":\"ok\"}\n",
            "stderr": "",
        })
        .to_string();
        assert!(validate_n2_publish_result(false, &accepted).is_ok());

        let rejected = json!({
            "exit_code": 0,
            "stdout": "{\"event_id\":\"abc\",\"accepted\":false,\"message\":\"rejected\"}\n",
            "stderr": "",
        })
        .to_string();
        assert!(validate_n2_publish_result(false, &rejected).is_err());

        let failed = json!({"exit_code": 2, "stdout": "", "stderr": "network"}).to_string();
        assert!(validate_n2_publish_result(false, &failed).is_err());
        assert!(validate_n2_publish_result(true, &accepted).is_err());
    }
}
