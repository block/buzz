use std::sync::Arc;

use serde_json::json;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;
use tracing::Instrument as _;

use crate::builtin;
use crate::config::{
    pricing_authority, Config, MAX_PROMPT_BYTES, MAX_TOOL_CALLS_PER_TURN, MAX_TOOL_RESULT_BYTES,
};
use crate::handoff::{ContextRecovery, HandoffOutcome};
use crate::hints::SkillEntry;
use crate::llm::Llm;
use crate::mcp::McpRegistry;
use crate::mcp::ResultBudget;

use crate::types::{
    AgentError, CacheTotalState, ContentBlock, HistoryItem, PricingIdentity, ProviderStop,
    SessionUsageBaseline, StopReason, ToolCall, ToolResult, ToolResultContent, TurnIOState,
    TurnTotalState,
};
use crate::wire::{self, WireSender};

const ERROR_REFLECTION_SUFFIX: &str =
    "\n\n[Reflect] Before retrying, identify the cause and change your approach.";

const UNSUPPORTED_IMAGE_TOOL_MESSAGE: &str = "The current model does not support image input. The image was removed from conversation history so this turn can continue. Use a text-based inspection tool or ask the user for a textual description instead.";

/// Model-visible feedback after the provider truncates an assistant response at
/// its output-token limit. This is a user message rather than a synthetic tool
/// result because truncation can happen without a tool call (and an unpaired
/// tool result is invalid on every provider wire format).
const MAX_TOKENS_RECOVERY_MESSAGE: &str = "Your previous response exceeded the model's output token limit and was truncated. Any incomplete tool call was not run. Continue the task, breaking the work or tool call into smaller steps and keeping the response concise.";

/// A provider can repeatedly spend its entire output allowance without making
/// progress, while `max_rounds` is unbounded by default. Keep the in-turn rescue
/// finite so a persistently truncating model eventually surfaces `max_tokens`.
const MAX_TOKENS_RECOVERIES_PER_RUN: u32 = 2;

/// Remove image blocks that the provider has explicitly rejected while keeping
/// their surrounding tool result (and therefore the tool-call/result pairing)
/// intact. Returns the number of images removed; zero means the provider error
/// cannot be safely recovered by mutating history.
fn replace_unsupported_images(history: &mut [HistoryItem]) -> usize {
    let mut replaced = 0;
    for item in history {
        let HistoryItem::ToolResult(result) = item else {
            continue;
        };
        let before = result.content.len();
        result
            .content
            .retain(|content| !matches!(content, ToolResultContent::Image { .. }));
        let removed = before - result.content.len();
        if removed > 0 {
            replaced += removed;
            result.is_error = true;
            result.content.push(ToolResultContent::Text(
                UNSUPPORTED_IMAGE_TOOL_MESSAGE.to_string(),
            ));
        }
    }
    replaced
}

/// Maximum reply reminders emitted per prompt when `require_reply` is on.
///
/// After this many, the turn is allowed to end whether or not anything was
/// published: the guard exists to catch accidental omission, not to compel
/// speech. The shared `stop_max_rejections` budget can cut this lower — see
/// [`Config::require_reply`](crate::config::Config::require_reply).
const MAX_REPLY_NAGS: u32 = 2;

/// Server label on the synthetic reply-guard objection.
///
/// Not a real MCP server. It rides the same tool-result path as `_Stop` hook
/// output, so the model sees `{hook, server, text}` attribution naming the
/// in-process guard rather than an MCP server that could be impersonated.
const REPLY_GUARD_SERVER: &str = "buzz-agent";

/// Reminder text emitted when a turn is about to end with nothing published.
///
/// Explicitly licenses silence. The base prompt tells agents that publishing is
/// optional and "silence is usually correct"; a reminder that argued otherwise
/// would fight that instruction and make agents chattier.
const REPLY_GUARD_NAG: &str = "You are about to end this turn without calling `buzz messages send`. \
Your assistant text and reasoning are never shown to anyone — if you did work, found an answer, \
or hit a blocker that someone is waiting on, it exists only if you publish it. \
If you already posted, or if silence is genuinely correct for this turn, ignore this and end your turn.";

/// Whether `call` is a recognized attempt to publish a reply to Buzz.
///
/// Recognizes an *attempt*, not a successful publish: the command text is
/// inspected, never the exit status. That is deliberate — a send that fails
/// already returns a non-zero exit and error JSON to the model, which is louder
/// feedback than the reminder this gates.
///
/// `has` + `!is_hook` are the same checks the dispatcher uses to accept a call
/// (see `execute_calls`), so a hallucinated `fake__shell` — rejected at preflight
/// and never executed — cannot disarm the guard. They must stay *before*
/// [`is_reply_shaped`]: together with them, and only with them, the `__shell`
/// suffix is exactly equivalent to "the bare tool name is `shell`".
fn is_buzz_reply_call(call: &ToolCall, mcp: &McpRegistry) -> bool {
    mcp.has(&call.name) && !mcp.is_hook(&call.name) && is_reply_shaped(&call.name, &call.arguments)
}

/// Whether a tool name and arguments have the shape of a Buzz publish command.
///
/// Split from [`is_buzz_reply_call`] only so the matcher is testable without a
/// live [`McpRegistry`]; callers must apply the registry checks first.
///
/// On the name: `ends_with("__shell")` is exact rather than approximate *given*
/// those checks. Registration rejects `__` in both server names and bare tool
/// names, and qualified names are `{server}__{bare}`, so a trailing `__shell` can
/// only straddle the separator if the bare name starts with `_` — which `is_hook`
/// already excludes. Dropping the separator would not be exact: `powershell` and
/// `noshell` both end in `shell`.
///
/// On the command: a deliberately coarse substring test, scoped to the structured
/// `command` field so unrelated metadata — a `description` that quotes a send —
/// cannot suppress the guard, and a non-string `command` is rejected rather than
/// coerced. Known limits, both accepted: a command assembled at runtime (`$CMD`)
/// or hidden in a wrapper script is missed, and text that merely quotes a send
/// (`echo "buzz messages send"`) matches. Missing a real post is the expensive
/// direction, and substring matching is the more forgiving one there.
fn is_reply_shaped(name: &str, arguments: &serde_json::Value) -> bool {
    name.ends_with("__shell")
        && arguments
            .get("command")
            .and_then(|v| v.as_str())
            .is_some_and(|cmd| {
                // `messages send` also covers `messages send-diff`. `reactions
                // add` counts because the base prompt directs agents to react
                // rather than post a bare acknowledgement, so nagging an agent
                // that reacted would punish documented-correct behavior.
                cmd.contains("messages send") || cmd.contains("reactions add")
            })
}

pub struct RunCtx<'a> {
    pub cfg: &'a Config,
    /// Effective model for this session. Usually equals `cfg.model`; overridden
    /// per-session by `session/set_model`. All LLM calls use this value.
    pub effective_model: &'a str,
    pub session_id: &'a str,
    pub system_prompt: &'a str,
    pub llm: &'a Llm,
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
    /// ACP v2 session identifier for this prompt turn. Used to derive
    /// per-message `messageId` values that are unique within the ACP session.
    /// Distinct from `session_id` (which is the ACP session); this is a
    /// per-`session/prompt` random token so that IDs from one prompt invocation
    /// never collide with those from another even within the same session.
    pub run_id: String,
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
    /// NIP-AM metric publishing. Reset to `Unseen` at turn start in `run()`.
    pub turn_input_tokens: &'a mut TurnIOState,
    /// Accumulated output tokens across all LLM rounds in this turn, for
    /// NIP-AM metric publishing. Reset to `Unseen` at turn start in `run()`.
    pub turn_output_tokens: &'a mut TurnIOState,
    /// The cache-served subset of `turn_input_tokens`, accumulated across all
    /// LLM rounds in this turn. Reset to `Unseen` at turn start in `run()`.
    /// Consumers price this slice at the provider's cached rate; without it
    /// every round of a growing conversation is billed at full price.
    ///
    /// `CacheTotalState` enforces the D1 rule: any usage-bearing round that
    /// omits this category poisons the accumulator permanently for the turn.
    pub turn_cached_input_tokens: &'a mut CacheTotalState,
    /// The cache-written subset of `turn_input_tokens`, accumulated across all
    /// LLM rounds in this turn. Reset to `Unseen` at turn start in `run()`.
    /// Consumers need this to price cache-creation at the provider's write rate
    /// (distinct from both the standard input rate and the cached-read rate).
    ///
    /// Same D1 tri-state contract as `turn_cached_input_tokens`.
    pub turn_cache_write_tokens: &'a mut CacheTotalState,
    /// Per-turn billing identity accumulator.
    ///
    /// - `None`: no usage-bearing response observed yet this turn (initial state).
    /// - `Some(Some(pi))`: all usage-bearing responses so far carry the same
    ///   proven identity `pi`. If a subsequent response carries a different or
    ///   unproven identity, this transitions to `Some(None)` (poisoned).
    /// - `Some(None)`: poisoned — mixed identities, unproven response, mesh
    ///   retry across models, or no identity derived. Never heals within the turn.
    ///
    /// Reset to `None` at turn start in `run()`. The wire payload emits the
    /// proven identity when `Some(Some(pi))`, omits it otherwise.
    pub turn_pricing_identity: &'a mut Option<Option<PricingIdentity>>,
    /// Tri-state total-token accumulator for this turn.
    ///
    /// - `Unseen`: no usage-bearing response observed yet this turn (initial state).
    /// - `Exact(n)`: every usage-bearing response so far reported a genuine
    ///   provider total; `n` is their sum.
    /// - `Unknown`: at least one usage-bearing response lacked a provider total;
    ///   this turn can never produce a reliable total.
    ///
    /// Reset to `Unseen` at turn start in `run()`. Callers must not derive a
    /// total by summing input+output — that is the UI display approximation only.
    pub turn_total_state: &'a mut TurnTotalState,
    /// Session-cumulative counters as they stood when this turn began. Added to
    /// the `turn_*` accumulators above to report a cumulative figure mid-turn;
    /// the session's own copy is only advanced once, after the turn returns.
    pub usage_baseline: SessionUsageBaseline,
}

/// Fold one round's proven identity into the per-turn identity accumulator.
///
/// Accumulator tri-state (NIP-AM §pricingIdentity):
/// - `None`: no usage-bearing round observed yet this turn.
/// - `Some(Some(pi))`: every usage-bearing round so far carries the same
///   proven identity `pi`.
/// - `Some(None)`: poisoned — mixed identities or unproven round seen.
///   Never heals within the turn.
///
/// `round`: `Some(pi)` when this round's `(base_url, request_model)` resolve
/// to a proven identity; `None` when the endpoint is unallowlisted or the
/// model is unknown.
#[inline]
fn fold_pricing_identity(
    acc: Option<Option<PricingIdentity>>,
    round: Option<PricingIdentity>,
) -> Option<Option<PricingIdentity>> {
    match acc {
        // First usage-bearing round: record whatever was derived.
        None => Some(round),
        // Already consistent: keep only if this round matches exactly.
        Some(Some(ref existing)) => {
            if Some(existing) == round.as_ref() {
                Some(round)
            } else {
                // Mismatch (different model, different authority,
                // or this round had no proven identity) → poison.
                Some(None)
            }
        }
        // Already poisoned: stays poisoned forever this turn.
        poisoned @ Some(None) => poisoned,
    }
}

impl RunCtx<'_> {
    /// Send a session-cumulative `usage_update` reflecting everything observed
    /// up to and including the most recent LLM response.
    ///
    /// The figure is the turn-start baseline plus this turn's running
    /// accumulators, which is exactly what `session/prompt` will fold into the
    /// session once the turn returns — so a mid-turn notification and the
    /// end-of-turn one agree, and a turn that never returns has still reported
    /// everything but its final in-flight request.
    async fn emit_usage_update(&self) {
        let base = self.usage_baseline;
        // Combine session baseline CacheTotalState with the per-turn delta:
        // merge_session produces Exact when both sides are Exact, Unknown when
        // either is Unknown, and leaves Unseen when both sides are Unseen.
        let cached_total = base
            .cached_input_tokens
            .merge_session(*self.turn_cached_input_tokens);
        let write_total = base
            .cache_write_tokens
            .merge_session(*self.turn_cache_write_tokens);
        let payload = wire::usage_update_payload(
            base.input_tokens
                .merge_session(*self.turn_input_tokens)
                .exact_value(),
            base.output_tokens
                .merge_session(*self.turn_output_tokens)
                .exact_value(),
            cached_total.exact_value(),
            write_total.exact_value(),
            base.total_state.merge_session(*self.turn_total_state),
            self.effective_model,
            // Extract the proven identity if this turn is consistent so far.
            self.turn_pricing_identity
                .as_ref()
                .and_then(|inner| inner.as_ref()),
        );
        wire::send(
            self.wire,
            wire::goose_session_update(self.session_id, payload),
        )
        .await;
    }

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

        // Reset per-turn token accumulators for this prompt.
        *self.turn_input_tokens = TurnIOState::Unseen;
        *self.turn_output_tokens = TurnIOState::Unseen;
        *self.turn_cached_input_tokens = CacheTotalState::Unseen;
        *self.turn_cache_write_tokens = CacheTotalState::Unseen;
        *self.turn_pricing_identity = None;
        *self.turn_total_state = TurnTotalState::Unseen;
        // Per-turn handoff-attempt counter. Scoped here (not persisted in the
        // session) so `BUZZ_AGENT_MAX_HANDOFFS` bounds compactions per
        // `session/prompt` turn rather than per session lifetime. A
        // long-lived session legitimately needs unbounded handoffs across
        // prompts; the cap only exists to stop runaway within a single turn.
        // The session-cumulative `handoff_count` (used in log lines) is not
        // reset: it reflects total compactions since session start.
        let mut handoff_attempts: usize = 0;

        let mut round = 0u32;
        // Per-prompt `_Stop` objection count. Bounded per prompt (not per
        // session) so a stubborn exchange can't permanently disable the stop
        // guard for a long-lived session; `max_rounds` still caps the loop.
        let mut stop_rejections = 0u32;
        // Reply-guard state for this prompt. `prompt()` *is* the turn, so
        // locals here are per-turn by construction — same shape as
        // `stop_rejections` above.
        //
        // Named for what it proves: a *recognized attempt* to publish, not a
        // successful publish. See `is_buzz_reply_call`.
        let mut buzz_reply_call_seen = false;
        let mut reply_nags = 0u32;
        // Per-`run()` reactive context-recovery budget. Per-turn, not
        // per-session: a fresh prompt deserves a fresh chance to recover, and
        // `max_rounds` defaults to 0 (unbounded) so it cannot bound this.
        let mut context_recoveries = 0u32;
        // Per-run output-truncation recovery budget. Unlike context recovery,
        // these successful provider requests consume a real round and are not
        // refunded; this counter only bounds the default-unlimited case.
        let mut max_tokens_recoveries = 0u32;
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
            match self.maybe_handoff(&mut handoff_attempts).await {
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
            let response_result = tokio::select! {
                biased;
                _ = self.cancel.changed() => return Ok(StopReason::Cancelled),
                r = self.llm.complete(self.cfg, self.system_prompt, self.history, &tools, self.effective_model)
                        .instrument(tracing::info_span!("llm", session_id = %self.session_id)) => r,
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
            let response = match response_result {
                Ok(response) => response,
                Err(AgentError::UnsupportedImageInput(detail)) => {
                    let removed = replace_unsupported_images(self.history);
                    if removed == 0 {
                        return Err(AgentError::UnsupportedImageInput(detail));
                    }
                    tracing::warn!(
                        model = self.effective_model,
                        removed_images = removed,
                        "provider rejected image input; removed images from history and continuing turn"
                    );
                    continue;
                }
                // Reactive context recovery. A context-window 400 is the only
                // ground-truth signal that history must shrink, and it arrives
                // exactly when the proactive gate cannot act: a failed request
                // reports no usage, so `last_request_input_tokens` stays frozen
                // at the last SUCCESSFUL (sub-threshold) reading and
                // `should_handoff()` returns false forever. Without this arm the
                // error propagates out of `run()`, the in-memory session keeps
                // the same oversized history, and every later prompt in that
                // session fails the same way — a stick that persists across
                // turns for the life of the session. (Restarting the agent DOES
                // clear it: history lives only in the in-memory session map, so
                // a restart is the manual workaround, not an exception to it.)
                //
                // Retried in-loop rather than returned so the recovered context
                // continues the turn the user is waiting on.
                Err(AgentError::LlmContextExceeded(e)) => {
                    match self
                        .recover_from_context_overflow(&mut context_recoveries)
                        .await
                    {
                        ContextRecovery::Recovered => {
                            // Refund the round the rejected request consumed.
                            // `round` is incremented before `complete()`, so
                            // without this a finite `max_rounds` is spent by a
                            // request the provider refused to serve: the loop
                            // would re-enter, hit the cap at the top, and return
                            // `MaxTurnRequests` having destroyed history and
                            // never asked the model again — a worse outcome than
                            // the error it replaced.
                            //
                            // This cannot become an unbounded amnesty: refunds
                            // happen only on a *successful* recovery, and
                            // recoveries are independently capped by
                            // `MAX_CONTEXT_RECOVERIES_PER_RUN`, so at most that
                            // many rounds can ever be refunded in one turn. An
                            // ordinary round is never refunded.
                            round = round.saturating_sub(1);
                            // Same reset as the proactive path (see
                            // `HandoffOutcome::Performed` above): the frozen
                            // token reading describes history that no longer
                            // exists. Clearing it is what lets the gate work
                            // again on later rounds.
                            *self.last_request_input_tokens = None;
                            *self.last_request_history_bytes = None;
                            continue;
                        }
                        ContextRecovery::Cancelled => return Ok(StopReason::Cancelled),
                        // No rescue left. Surface the provider's own error
                        // rather than a synthetic one: it names the model and
                        // the offending sizes, and a visible failure is the
                        // point — the alternative is retrying forever.
                        ContextRecovery::Exhausted => {
                            return Err(AgentError::LlmContextExceeded(e))
                        }
                    }
                }
                Err(error) => return Err(error),
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
            if response.input_tokens_overflowed {
                // The Anthropic-style inclusive sum (input_tokens +
                // cache_read_input_tokens + cache_creation_input_tokens)
                // overflowed u64::MAX during parsing. Permanently poison the
                // turn accumulator so wire emission omits this value and ACP
                // marks the delta unreliable. Do NOT update
                // last_request_input_tokens — freeze the context-gate
                // baseline at its prior reading rather than poisoning it with
                // a clamped value, exactly as the absent-usage path does.
                *self.turn_input_tokens = TurnIOState::Poisoned;
            } else if let Some(tokens) = response.input_tokens {
                *self.last_request_input_tokens = Some(tokens);
                *self.last_request_history_bytes = Some(
                    self.history
                        .iter()
                        .map(HistoryItem::context_pressure_bytes)
                        .sum(),
                );
                // Accumulate per-turn input tokens for NIP-AM metric publishing.
                // fold_round uses checked_add; overflow permanently poisons the
                // turn accumulator (and, via merge_session, the session cumulative).
                *self.turn_input_tokens = self.turn_input_tokens.fold_round(tokens);
            }
            // Accumulate per-turn output tokens for NIP-AM metric publishing.
            if let Some(out) = response.output_tokens {
                *self.turn_output_tokens = self.turn_output_tokens.fold_round(out);
            }
            // Fold the provider-reported total, cache subsets, and billing
            // identity — only when this response was usage-bearing (had input
            // or output tokens).  A response with no usage at all is not
            // evidence of a missing cache field or total and must not poison
            // either accumulator.
            //
            // `input_tokens_overflowed` counts as usage-bearing: the provider
            // reported an input total (which overflowed) so cache fields and
            // the total are meaningful and must be folded.
            //
            // D1: absent cache field on a usage-bearing round permanently
            // poisons the turn accumulator.  Some(0) stays Exact(0) (explicit
            // zero is distinct from absent).  Cache-read and cache-write are
            // tracked separately from `turn_input_tokens` rather than
            // subtracted from it: the input total must stay inclusive for the
            // handoff gate, which cares how much context was sent, not cost.
            //
            // Shape assumption: documented OpenAI-compatible responses that carry
            // `total_tokens` always co-report at least one of `prompt_tokens` /
            // `completion_tokens`. A response that supplies only `total_tokens`
            // with neither category is therefore not a supported shape and would
            // be silently ignored here. If that shape is ever encountered, extend
            // this gate rather than representing absent categories as zero.
            if response.input_tokens.is_some()
                || response.input_tokens_overflowed
                || response.output_tokens.is_some()
            {
                *self.turn_total_state = self.turn_total_state.fold(response.total_tokens);
                // Cache-read: the cache-served subset of input tokens.
                *self.turn_cached_input_tokens = self
                    .turn_cached_input_tokens
                    .fold(response.cached_input_tokens);
                // Cache-write: the cache-creation subset of input tokens.
                *self.turn_cache_write_tokens = self
                    .turn_cache_write_tokens
                    .fold(response.cache_write_tokens);

                // Derive billing identity for this round and fold it into the
                // per-turn accumulator.  Rules (NIP-AM §pricingIdentity):
                //
                // 1. Attempt to derive identity from (base_url, request_model).
                //    - base_url must canonically match an official allowlisted host.
                //    - request_model must be Some (mesh-auto with unknown model
                //      cannot prove identity).
                // 2. Fold the round identity into the turn accumulator:
                //    - Unseen (None): record this round's identity (or poison if None).
                //    - Consistent: if it matches, keep; otherwise poison.
                //    - Poisoned: stays poisoned forever this turn.
                {
                    let round_identity: Option<PricingIdentity> =
                        response.request_model.as_deref().and_then(|model| {
                            pricing_authority(&self.cfg.base_url).map(|auth| PricingIdentity {
                                authority: auth.to_string(),
                                model: model.to_string(),
                                cache_class: None, // no cache-class derivation yet
                            })
                        });

                    *self.turn_pricing_identity =
                        fold_pricing_identity(self.turn_pricing_identity.take(), round_identity);
                }

                // Report what the turn has burned SO FAR, before running the
                // next round. A turn is many provider round-trips over many
                // minutes, and until this point the only report was the one
                // `session/prompt` sends after the turn returns — so a turn
                // that was cancelled, timed out, or whose process was killed
                // reported nothing at all, and its tokens (already billed)
                // existed only in this stack frame. Reporting per round bounds
                // the loss to the single request in flight.
                //
                // Emitting more than one `usage_update` per turn is expected by
                // the consumer: buzz-acp's UsageTracker advances its committed
                // baseline only when the turn's metric is published, so every
                // notification within a turn measures from the same frozen
                // baseline and the last one seen is the turn's true total.
                // goose behaves the same way, which is why the tracker was
                // written to tolerate it.
                self.emit_usage_update().await;
            }

            // Stable per-kind message IDs for ACP v2 ContentChunk compliance.
            // ACP v2 requires every ContentChunk to carry `messageId`; all chunks
            // that belong to the same logical message must share the same ID, and
            // IDs must be unique per message within the ACP session.
            //
            // A provider round produces at most one thought and one assistant
            // message (the parsers collapse all provider output into one
            // LlmResponse.reasoning string and one LlmResponse.text string).
            // These are two *distinct* logical messages, so they get distinct IDs.
            //
            // `run_id` is a fresh random token per `session/prompt` invocation,
            // so `<run_id>-thought-<round>` and `<run_id>-message-<round>` are
            // unique within the ACP session even across multiple prompts.
            //
            // ACP v1 allows the field, so this is a backwards-safe addition.
            let thought_msg_id = format!("{}-thought-{round}", self.run_id);
            let message_msg_id = format!("{}-message-{round}", self.run_id);
            if !response.reasoning.is_empty() {
                wire::send(
                    self.wire,
                    wire::session_update(
                        self.session_id,
                        json!({
                            "sessionUpdate": "agent_thought_chunk",
                            "messageId": &thought_msg_id,
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
                            "messageId": &message_msg_id,
                            "content": { "type": "text", "text": &response.text }
                        }),
                    ),
                )
                .await;
            }

            // `max_tokens` describes a truncated assistant response, not turn
            // completion. Never execute tool calls from it: although one may
            // parse as valid, a later call (or surrounding instructions) may
            // have been cut off. Replay only the text, with no tool calls, so
            // the history remains valid without fabricated tool results; then
            // add actionable user-role feedback and ask the model to continue.
            if response.stop == ProviderStop::MaxTokens {
                self.history.push(HistoryItem::Assistant {
                    text: response.text,
                    tool_calls: Vec::new(),
                    reasoning_details: response.reasoning_details,
                });
                if max_tokens_recoveries >= MAX_TOKENS_RECOVERIES_PER_RUN {
                    tracing::warn!(
                        recoveries = max_tokens_recoveries,
                        "provider repeatedly hit output token limit; recovery budget exhausted"
                    );
                    return Ok(StopReason::MaxTokens);
                }
                max_tokens_recoveries = max_tokens_recoveries.saturating_add(1);
                tracing::warn!(
                    recovery = max_tokens_recoveries,
                    max_recoveries = MAX_TOKENS_RECOVERIES_PER_RUN,
                    discarded_tool_calls = response.tool_calls.len(),
                    "provider hit output token limit; asking model to continue in smaller steps"
                );
                self.history
                    .push(HistoryItem::User(MAX_TOKENS_RECOVERY_MESSAGE.to_string()));
                continue;
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
                    reasoning_details: response.reasoning_details.clone(),
                });
                let stop = map_stop(response.stop);
                // Only gate genuine end_turn — don't override max_tokens/refusal.
                if stop == StopReason::EndTurn {
                    if stop_rejections >= self.cfg.stop_max_rejections {
                        return Ok(stop);
                    }
                    let mut objections = self
                        .mcp
                        .call_hooks(
                            "_Stop",
                            &json!({}),
                            self.cfg.hook_timeout,
                            &self.cfg.hook_servers,
                        )
                        .await;
                    // Reply guard shares this gate and this budget, so a round
                    // carrying both a hook objection and a reply reminder costs
                    // one rejection and delivers both texts.
                    if self.cfg.require_reply
                        && !buzz_reply_call_seen
                        && reply_nags < MAX_REPLY_NAGS
                    {
                        reply_nags += 1;
                        objections
                            .push((REPLY_GUARD_SERVER.to_string(), REPLY_GUARD_NAG.to_string()));
                    }
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
            // Deliberately after truncation: a publish-shaped call that was
            // discarded never runs, so it must not suppress the reminder.
            if self.cfg.require_reply && !buzz_reply_call_seen {
                buzz_reply_call_seen = calls.iter().any(|c| is_buzz_reply_call(c, self.mcp));
            }
            self.history.push(HistoryItem::Assistant {
                text: response.text,
                tool_calls: calls.clone(),
                reasoning_details: response.reasoning_details,
            });

            if let Some(stop) = self.execute_calls(&calls).await {
                return Ok(stop);
            }
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
                // Synthesised locally, so there is no provider wire form to
                // preserve.
                provider_extra: Default::default(),
            }],
            reasoning_details: None,
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
    use super::*;
    use serde_json::json;

    /// `truncate_history` cannot serve as the context-window fallback: it is
    /// measured in BYTES (`max_history_bytes`, default 16 MiB, a request-body
    /// limiter) while the thing the fallback must defend is a TOKEN window
    /// (`max_context_tokens`, default 200k). A history large enough to blow a
    /// 200k-token window is nowhere near 16 MiB, so at the default budget the
    /// fallback evicts nothing at all — which is why the `Skipped ->
    /// truncate_history` path left the agent permanently stuck and the reactive
    /// ladder had to be built instead.
    ///
    /// The negative assertion is paired with a positive control (same helper,
    /// same fixture, budget set to the window instead) so that "evicted
    /// nothing" is a real observation about the unit mismatch rather than a
    /// blind probe that could never evict.
    #[test]
    fn truncate_history_is_a_noop_at_context_window_scale() {
        // ~800 KB of history. At any real bytes/token density (densest real
        // content is ~1.4 B/tok, typical prose ~3-4) this is >= 200k tokens,
        // i.e. already over a 200k window.
        let mut history: Vec<HistoryItem> = Vec::new();
        for i in 0..400 {
            history.push(HistoryItem::User(format!("q{i} {}", "x".repeat(1000))));
            history.push(HistoryItem::Assistant {
                text: format!("a{i} {}", "y".repeat(1000)),
                tool_calls: vec![],
                reasoning_details: None,
            });
        }
        let total: usize = history.iter().map(HistoryItem::estimated_bytes).sum();
        let pressure: usize = history
            .iter()
            .map(HistoryItem::context_pressure_bytes)
            .sum();
        assert!(
            total > 800_000,
            "fixture must be big enough to exceed a 200k-token window, got {total}"
        );

        // NEGATIVE: the real configured default budget.
        let default_budget = 16 * 1024 * 1024;
        let mut under_default = history.clone();
        truncate_history(&mut under_default, default_budget);
        assert_eq!(
            under_default.len(),
            history.len(),
            "16 MiB byte budget evicted nothing from a {total}-byte history \
             (pressure {pressure}) that already exceeds a 200k-token window"
        );

        // POSITIVE CONTROL: same helper, same fixture, budget set to the
        // window instead. If this also evicted nothing the assertion above
        // would prove nothing about the unit mismatch -- it would just mean
        // the probe is blind.
        let mut under_window = history.clone();
        truncate_history(&mut under_window, 200_000);
        assert!(
            under_window.len() < history.len(),
            "positive control must evict: probe is blind otherwise"
        );
    }

    /// The shapes the guard must recognize as a publish attempt. Callers apply
    /// the registry checks first; these cover the name suffix and command text.
    #[test]
    fn reply_shape_matches_documented_send_forms() {
        for cmd in [
            "buzz messages send --channel X --content Y",
            "buzz --relay wss://r messages send --channel X --content Y",
            "/abs/path/buzz messages send",
            "printf 'hi' | buzz messages send --content -",
            "buzz messages send-diff --diff -",
            "buzz reactions add --event E --emoji +",
            // Assembled through another shell: rev 3's tokenizer missed this.
            r#"sh -c "buzz messages send --channel X""#,
        ] {
            assert!(
                is_reply_shaped("dev__shell", &json!({ "command": cmd })),
                "expected {cmd:?} to count as a publish attempt"
            );
        }
    }

    /// Commands that do real work but do not reply in the originating
    /// conversation must still be nagged.
    #[test]
    fn reply_shape_rejects_non_reply_commands() {
        for cmd in [
            "buzz messages get --channel X",
            "buzz channels list",
            "buzz reactions remove --event E",
            "buzz pr open --title T",
            "buzz social publish --content hi",
            "buzz notes set --name n",
            "cargo test -p buzz-agent",
        ] {
            assert!(
                !is_reply_shaped("dev__shell", &json!({ "command": cmd })),
                "expected {cmd:?} not to count as a publish attempt"
            );
        }
    }

    /// The `__` separator is load-bearing: `ends_with("shell")` alone would
    /// accept any registered tool whose name merely ends in those letters, and
    /// `has()` proves registration, not the bare name.
    #[test]
    fn reply_shape_requires_the_qname_separator() {
        let args = json!({ "command": "buzz messages send --channel X" });
        for name in [
            "dev__powershell",
            "dev__noshell",
            "shell",
            "dev__send_message",
        ] {
            assert!(
                !is_reply_shaped(name, &args),
                "{name} must not satisfy the shell-tool check"
            );
        }
        assert!(is_reply_shaped("dev__shell", &args));
        assert!(is_reply_shaped("buzz-dev-mcp__shell", &args));
    }

    /// Only the field that carries the executable command counts. Searching
    /// serialized arguments instead would let arbitrary metadata disarm the
    /// guard, turning a description into an attempted send.
    #[test]
    fn reply_shape_reads_only_the_command_field() {
        assert!(!is_reply_shaped(
            "dev__shell",
            &json!({ "description": "buzz messages send --channel X" })
        ));
        assert!(!is_reply_shaped(
            "dev__shell",
            &json!({ "workdir": "buzz messages send" })
        ));
        // Malformed `command` is rejected, not coerced — and must not panic.
        assert!(!is_reply_shaped("dev__shell", &json!({ "command": 42 })));
        assert!(!is_reply_shaped("dev__shell", &json!({ "command": null })));
        assert!(!is_reply_shaped("dev__shell", &json!({})));
        assert!(!is_reply_shaped("dev__shell", &json!("not an object")));
    }

    /// A9 regression: `reasoning_details` contributes real bytes to
    /// `estimated_bytes` (see `types.rs::HistoryItem::size_with`), so a
    /// history item carrying a large opaque reasoning array must actually
    /// drive `truncate_history` eviction — not be silently invisible to the
    /// sizing gate that decides what survives a turn.
    #[test]
    fn truncate_history_evicts_oldest_turn_with_reasoning_details() {
        let big_reasoning = json!([{ "type": "reasoning.text", "text": "x".repeat(400) }]);
        let mut history = vec![
            HistoryItem::User("first question".into()),
            HistoryItem::Assistant {
                text: "first answer".into(),
                tool_calls: vec![],
                reasoning_details: Some(big_reasoning),
            },
            HistoryItem::User("second question".into()),
            HistoryItem::Assistant {
                text: "second answer".into(),
                tool_calls: vec![],
                reasoning_details: None,
            },
        ];

        let total_before: usize = history.iter().map(HistoryItem::estimated_bytes).sum();
        // Budget below the total but above the second (smaller) turn alone,
        // so only the oldest user+assistant pair — the one carrying
        // reasoning_details — must be dropped.
        let max_bytes = total_before - 100;
        assert!(
            max_bytes > 0,
            "test fixture must leave room to evict only one turn"
        );

        truncate_history(&mut history, max_bytes);

        assert_eq!(
            history.len(),
            2,
            "the oldest user+assistant turn (with reasoning_details) must be evicted"
        );
        assert!(matches!(&history[0], HistoryItem::User(s) if s == "second question"));
        assert!(
            matches!(&history[1], HistoryItem::Assistant { text, .. } if text == "second answer")
        );
        let total_after: usize = history.iter().map(HistoryItem::estimated_bytes).sum();
        assert!(total_after <= max_bytes);
    }

    #[test]
    fn unsupported_images_become_recoverable_tool_errors() {
        let mut history = vec![
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    provider_id: "call-image".into(),
                    name: "dev__view_image".into(),
                    arguments: json!({ "source": "spec.png" }),
                    provider_extra: Default::default(),
                }],
                reasoning_details: None,
            },
            HistoryItem::ToolResult(ToolResult {
                provider_id: "call-image".into(),
                content: vec![
                    ToolResultContent::Text("10x10 image from spec.png".into()),
                    ToolResultContent::Image {
                        data: "aW1n".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
            }),
        ];

        assert_eq!(replace_unsupported_images(&mut history), 1);
        let HistoryItem::ToolResult(result) = &history[1] else {
            panic!("tool result must stay paired with the assistant tool call");
        };
        assert_eq!(result.provider_id, "call-image");
        assert!(result.is_error);
        assert!(result
            .content
            .iter()
            .all(|content| !matches!(content, ToolResultContent::Image { .. })));
        assert!(result.text().contains("does not support image input"));
        assert!(result.text().contains("10x10 image from spec.png"));
        assert_eq!(replace_unsupported_images(&mut history), 0);
    }

    #[test]
    fn truncate_history_noop_when_under_budget() {
        let mut history = vec![
            HistoryItem::User("hi".into()),
            HistoryItem::Assistant {
                text: "hello".into(),
                tool_calls: vec![],
                reasoning_details: None,
            },
        ];
        let original_len = history.len();
        truncate_history(&mut history, 1_000_000);
        assert_eq!(
            history.len(),
            original_len,
            "under budget must not evict anything"
        );
    }

    // ── fold_pricing_identity: turn discipline ────────────────────────────────

    fn pi(authority: &str, model: &str) -> PricingIdentity {
        PricingIdentity {
            authority: authority.to_string(),
            model: model.to_string(),
            cache_class: None,
        }
    }

    /// Case 1: two usage-bearing rounds with different proven identities in one
    /// turn must poison the accumulator.  The wire payload omits `pricingIdentity`
    /// when `Some(None)`.
    #[test]
    fn fold_pricing_identity_mismatch_poisons() {
        let round_a = Some(pi("api.anthropic.com", "claude-opus-4-5"));
        let round_b = Some(pi("api.openai.com", "gpt-4o"));

        // Start: unseen.
        let acc = None;
        // After round A: consistent — Some(Some(claude-opus-4-5)).
        let acc = fold_pricing_identity(acc, round_a);
        assert!(
            matches!(acc, Some(Some(_))),
            "after one round must be consistent"
        );
        // After round B (different authority + model): poisoned.
        let acc = fold_pricing_identity(acc, round_b);
        assert_eq!(
            acc,
            Some(None),
            "different proven identities in one turn must poison"
        );
    }

    /// Case 2: proven identity followed by a usage-bearing round with no proven
    /// identity (request_model absent or non-allowlisted endpoint) must poison.
    /// An unpaired cumulative snapshot also produces round=None (no model known)
    /// and hits this same path — case 4 collapses into case 2.
    #[test]
    fn fold_pricing_identity_unproven_round_poisons() {
        let round_a = Some(pi("api.anthropic.com", "claude-3-7-sonnet"));
        let round_unproven: Option<PricingIdentity> = None; // absent request_model or custom endpoint

        let acc = None;
        let acc = fold_pricing_identity(acc, round_a);
        assert!(
            matches!(acc, Some(Some(_))),
            "after one proven round must be consistent"
        );
        let acc = fold_pricing_identity(acc, round_unproven);
        assert_eq!(
            acc,
            Some(None),
            "an unproven round after a proven round must poison (no-model / custom-endpoint path)"
        );
    }

    /// Case 3: a poisoned accumulator must not heal, even if a later round
    /// carries an identity matching the original.
    #[test]
    fn fold_pricing_identity_poisoned_never_heals() {
        let round_a = Some(pi("api.openai.com", "gpt-4o"));
        let round_unproven: Option<PricingIdentity> = None;
        let round_a_again = Some(pi("api.openai.com", "gpt-4o")); // identical to round_a

        let acc = None;
        let acc = fold_pricing_identity(acc, round_a);
        let acc = fold_pricing_identity(acc, round_unproven); // poisons
        assert_eq!(acc, Some(None), "must be poisoned before heal attempt");
        let acc = fold_pricing_identity(acc, round_a_again); // must not heal
        assert_eq!(
            acc,
            Some(None),
            "poisoned accumulator must stay poisoned even when the next round matches the original identity"
        );
    }

    /// Baseline: a turn where every round carries the same proven identity
    /// stays consistent and emits the identity on the wire.
    #[test]
    fn fold_pricing_identity_consistent_rounds_stay_proven() {
        let identity = pi("api.openrouter.ai", "meta-llama/llama-4-scout");

        let acc = None;
        let acc = fold_pricing_identity(acc, Some(identity.clone()));
        let acc = fold_pricing_identity(acc, Some(identity.clone()));
        let acc = fold_pricing_identity(acc, Some(identity.clone()));
        assert_eq!(
            acc,
            Some(Some(identity)),
            "three identical rounds must remain consistently proven"
        );
    }
}
