//! buzz-agent's loop decisions, as goose `Operation`s.
//!
//! goose's `StateMachine` runs an ordered list of operations and re-runs the
//! whole list from the top after each one applies. Expressing buzz's decisions
//! as operations makes their precedence *list order* rather than control flow,
//! and makes each one testable without a provider.
//!
//! goose's own concrete operations are `pub(super)`, so these are buzz's. That
//! is the intent rather than a workaround: buzz's rules differ (no recipes, no
//! slash commands, no tool approval, no retry), and the traits are public
//! precisely so an embedder can bring its own.
//!
//! # Signalling a stop reason
//!
//! `ConversationEffect` can append messages and `yielded()` can end the turn, but
//! neither carries *why*. buzz's caller has to answer `session/prompt` with a
//! specific ACP `stopReason`, so operations record it in a shared [`Outcome`]
//! cell that the driving loop reads after the step. This is the one piece of
//! state the state machine does not model for us.

use std::sync::{Arc, Mutex};

use crate::mcp::McpRegistry;
use crate::turn_state::TurnSession;
use anyhow::Result;
use async_trait::async_trait;
use goose_agent::machine::{StateMachine, Step};
use goose_agent::operation::{
    applied, assistant_turn_count, messages_since_kickoff, not_applicable, yielded,
    ConversationEffect, Emitter, Operation, OperationResult,
};
use goose_provider_types::conversation::message::{
    Message, MessageContent, MessageMetadata, ToolRequest,
};
use goose_provider_types::conversation::{merge_consecutive_messages, Conversation};

use crate::types::StopReason;

/// Why the turn stopped, as decided by whichever operation applied.
///
/// Shared with the loop because `StepResult` says *that* a step yielded, not
/// which one or with what reason.
#[derive(Clone, Default)]
pub struct Outcome(Arc<Mutex<Option<StopReason>>>);

impl Outcome {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the reason the turn ended. First writer wins: the machine stops
    /// at the first operation that applies, so a later write would describe a
    /// decision that never took effect.
    fn set(&self, reason: StopReason) {
        let mut slot = match self.0.lock() {
            Ok(slot) => slot,
            // A poisoned lock means another thread panicked mid-write. The
            // turn still has to answer with *some* stop reason, so recover the
            // guard rather than propagating a panic into the loop.
            Err(poisoned) => poisoned.into_inner(),
        };
        if slot.is_none() {
            *slot = Some(reason);
        }
    }

    /// Take the recorded reason, if any.
    pub fn take(&self) -> Option<StopReason> {
        match self.0.lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        }
    }
}

/// Ends the turn once the model has had its budget of provider round-trips.
///
/// buzz-agent's loop was previously bounded by goose's `max_turns`; owning the
/// loop means owning the bound, and an unbounded loop against a model that
/// tool-calls forever is a runaway spend.
///
/// Deliberately *not* goose's `MaxTurnsOperation`, which appends an assistant
/// message ("Would you like me to continue?") before yielding. buzz-acp
/// surfaces the exhausted budget through the `max_turn_requests` stop reason
/// instead, and injecting a chat message the model never produced would put
/// words in the agent's mouth in a Buzz channel.
pub struct BuzzMaxRoundsOperation {
    max_rounds: u32,
    outcome: Outcome,
}

impl BuzzMaxRoundsOperation {
    pub fn new(max_rounds: u32, outcome: Outcome) -> Self {
        Self {
            max_rounds,
            outcome,
        }
    }
}

#[async_trait]
impl Operation<TurnSession, ConversationEffect> for BuzzMaxRoundsOperation {
    fn name(&self) -> &'static str {
        "buzz_max_rounds"
    }

    async fn run(
        &self,
        _session: &TurnSession,
        conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<ConversationEffect>> {
        // Counted from the kickoff message, so the budget is per *turn* and
        // does not leak across prompts in a long-lived session. Counting
        // assistant turns rather than loop iterations also means the bound
        // survives a restart, which a loop-local counter did not.
        let Ok(messages) = messages_since_kickoff(conversation) else {
            // No kickoff message means no turn is in progress, so there is no
            // budget to exceed. goose errors here; buzz declines instead --
            // an operation that cannot decide must not end the turn.
            return not_applicable();
        };
        let rounds = assistant_turn_count(messages);
        if rounds < self.max_rounds {
            return not_applicable();
        }

        tracing::warn!(rounds, "max rounds reached; ending turn");

        self.outcome.set(StopReason::MaxTurnRequests);
        yielded()
    }
}

/// Whether the last message ends the turn: an assistant message with no error
/// and no outstanding tool request.
///
/// goose has this as `ends_turn` in its private `operation` module
/// (`operation.rs:63`); only the names re-exported from `state_machine` escape
/// the crate. Reimplemented to match, with one deliberate narrowing: buzz has
/// no frontend tools and no approval flow, so `FrontendToolRequest` and
/// `ActionRequired` cannot appear in a buzz conversation. Matching on
/// `ToolRequest` alone is the whole of buzz's turn shape.
fn ends_turn(messages: &[Message]) -> bool {
    messages.last().is_some_and(|last| {
        last.role == rmcp::model::Role::Assistant
            && last.error_kind().is_none()
            && !last.content.iter().any(|content| {
                matches!(
                    content,
                    goose_provider_types::conversation::message::MessageContent::ToolRequest(_)
                )
            })
    })
}

/// Lets the `_Stop` hook object to a turn that wants to end.
///
/// When the agent has finished, `_Stop` is asked whether it may stop. An
/// objection is appended as an agent-visible `[Stop]` message and the turn
/// continues; silence lets it end.
///
/// # How this differs from goose's `StopHookOperation`
///
/// goose drives its own `HookManager` plugins and emits a user-facing
/// notification for each denial. buzz's hook is an **MCP tool** (`_Stop` on
/// the dev-MCP extension), and buzz stays silent: `buzz-acp` publishes
/// assistant messages into a channel, so a "hook blocked ending this turn"
/// notification would be posted to the humans in that channel every time an
/// agent's todo list was incomplete. The objection reaches the model, not the
/// room. Same reasoning as `BuzzMaxRoundsOperation` not appending goose's
/// "Would you like me to continue?".
///
/// # Why the block count lives on the messages
///
/// The cap was a loop-local `u32` in `run_turn`. Counting prior objections
/// from their own metadata instead makes the operation a pure function of the
/// conversation, so a pipeline rebuilt from persisted messages reaches the
/// same decision — the same reason goose tags its denials.
///
/// **This is not a behaviour change.** `run_turn` is per `session/prompt`, so
/// the old counter was already turn-scoped, and `messages_since_kickoff`
/// spans exactly that same turn. Three objections still end the turn; a new
/// prompt still starts from zero.
pub struct BuzzStopVetoOperation {
    mcp: Arc<McpRegistry>,
    extension: String,
    block_cap: u32,
}

impl BuzzStopVetoOperation {
    /// `extension` is the MCP extension serving `_Stop`; without one there is
    /// no hook to ask, and the operation never applies.
    pub fn new(mcp: Arc<McpRegistry>, extension: String, block_cap: u32) -> Self {
        Self {
            mcp,
            extension,
            block_cap,
        }
    }
}

/// Metadata key marking a message as a `_Stop` objection, so later rounds can
/// count them without a side channel.
const OBJECTED: &str = "objected";

#[async_trait]
impl Operation<TurnSession, ConversationEffect> for BuzzStopVetoOperation {
    fn name(&self) -> &'static str {
        "buzz_stop_veto"
    }

    async fn run(
        &self,
        _session: &TurnSession,
        conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<ConversationEffect>> {
        let Ok(messages) = messages_since_kickoff(conversation) else {
            return not_applicable();
        };
        // Only a turn that is trying to end can be vetoed.
        if !ends_turn(messages) {
            return not_applicable();
        }

        let blocks = messages
            .iter()
            .filter(|message| self.message_meta(message, OBJECTED).is_some())
            .count() as u32;
        if blocks >= self.block_cap {
            tracing::warn!(blocks, "_Stop veto cap reached; ending turn");
            return not_applicable();
        }

        // A broken or absent hook must never trap a turn -- `stop_objection`
        // maps every failure to `None`, which is "no objection".
        let Some(objection) = crate::hooks::stop_objection(&self.mcp, &self.extension).await else {
            return not_applicable();
        };

        tracing::info!(blocks = blocks + 1, "_Stop hook vetoed end of turn");

        let mut message = Message::user()
            .with_text(format!("[Stop] {objection}"))
            .with_visibility(false, true);
        self.set_message_meta(&mut message, OBJECTED, serde_json::json!(true));
        applied([message.into()])
    }
}

/// Reminds the model to publish when a turn is about to end with nothing
/// posted to Buzz.
///
/// Ported from main's reply guard (`agent.rs`, pre-goose), which this branch
/// dropped. Desktop still enables it by default for shared-compute/mesh agents
/// (`relay_mesh.rs`), so without this the flag is set and silently ignored.
///
/// Advisory: at most [`MAX_REPLY_NAGS`] reminders, then the turn ends whether
/// or not anything was published. The guard catches accidental omission; it
/// does not compel speech.
pub struct BuzzReplyGuardOperation {
    max_nags: u32,
    available_tools: std::collections::HashSet<String>,
}

/// Reminder text. Kept byte-identical to main's.
///
/// Explicitly licenses silence: the base prompt tells agents publishing is
/// optional and "silence is usually correct", and a reminder that argued
/// otherwise would fight that instruction and make agents chattier.
const REPLY_GUARD_NAG: &str = "You are about to end this turn without calling `buzz messages send`. \
Your assistant text and reasoning are never shown to anyone — if you did work, found an answer, \
or hit a blocker that someone is waiting on, it exists only if you publish it. \
If you already posted, or if silence is genuinely correct for this turn, ignore this and end your turn.";

/// Maximum reminders per turn, as on main.
pub const MAX_REPLY_NAGS: u32 = 2;

/// Metadata key marking a reminder, so the budget survives in the
/// conversation rather than in a loop-local counter.
const NAGGED: &str = "nagged";

impl BuzzReplyGuardOperation {
    pub fn new(max_nags: u32, available_tools: impl IntoIterator<Item = String>) -> Self {
        Self {
            max_nags,
            available_tools: available_tools.into_iter().collect(),
        }
    }

    /// Whether a tool request is a recognised attempt to publish to Buzz.
    ///
    /// An *attempt*, not a success: a failed send already returns a non-zero
    /// exit and error JSON to the model, which is louder than this reminder.
    ///
    /// The available-tool check is load-bearing: provider output can contain a
    /// hallucinated request that Goose later rejects at dispatch. Such a name
    /// must not disarm the guard merely because it looks like a shell tool.
    fn is_reply_shaped(&self, request: &ToolRequest) -> bool {
        let Ok(call) = &request.tool_call else {
            return false;
        };
        if !self.available_tools.contains(call.name.as_ref()) || !call.name.ends_with("__shell") {
            return false;
        }
        call.arguments
            .as_ref()
            .and_then(|args| args.get("command"))
            .and_then(|command| command.as_str())
            // Coarse substring test against the structured `command` field, as
            // on main: `messages send` also covers `send-diff`, and `reactions
            // add` counts because the base prompt tells agents to react rather
            // than post a bare acknowledgement. Missing a real post is the
            // expensive direction, so the forgiving match is the right one.
            .is_some_and(|cmd| cmd.contains("messages send") || cmd.contains("reactions add"))
    }
}

#[async_trait]
impl Operation<TurnSession, ConversationEffect> for BuzzReplyGuardOperation {
    fn name(&self) -> &'static str {
        "buzz_reply_guard"
    }

    async fn run(
        &self,
        _session: &TurnSession,
        conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<ConversationEffect>> {
        let Ok(messages) = messages_since_kickoff(conversation) else {
            return not_applicable();
        };
        if !ends_turn(messages) {
            return not_applicable();
        }

        let nags = messages
            .iter()
            .filter(|message| self.message_meta(message, NAGGED).is_some())
            .count() as u32;
        if nags >= self.max_nags {
            return not_applicable();
        }

        // Any publish-shaped call this turn disarms the guard for the rest of
        // it, matching main's `buzz_reply_call_seen` latch.
        let published = messages.iter().any(|message| {
            message.content.iter().any(|content| {
                matches!(
                    content,
                    goose_provider_types::conversation::message::MessageContent::ToolRequest(
                        request,
                    ) if self.is_reply_shaped(request)
                )
            })
        });
        if published {
            return not_applicable();
        }

        tracing::info!(nags = nags + 1, "reply guard reminded the model to publish");

        let mut message = Message::user()
            .with_text(REPLY_GUARD_NAG)
            .with_visibility(false, true);
        self.set_message_meta(&mut message, NAGGED, serde_json::json!(true));
        applied([message.into()])
    }
}

/// Drains `session/steer` messages into the conversation at the round
/// boundary.
///
/// Steering is buzz's own: goose has an equivalent queue on `Agent`, but
/// `drain_pending_steers` is `pub(crate)` — only `Agent::reply` can consume
/// it, and buzz owns the loop. Expressing the drain as an operation puts it
/// under the same gate as the other loop decisions instead of beside them.
///
/// Semantics are unchanged from the inline drain: messages land at the round
/// boundary and never mid-inference, where they would race a partially
/// streamed response.
pub struct BuzzSteerOperation {
    steers: crate::steer::SteerQueue,
}

impl BuzzSteerOperation {
    pub fn new(steers: crate::steer::SteerQueue) -> Self {
        Self { steers }
    }
}

#[async_trait]
impl Operation<TurnSession, ConversationEffect> for BuzzSteerOperation {
    fn name(&self) -> &'static str {
        "buzz_steer"
    }

    async fn run(
        &self,
        _session: &TurnSession,
        _conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<ConversationEffect>> {
        let messages = self.steers.drain().await;
        if messages.is_empty() {
            return not_applicable();
        }
        tracing::info!(
            count = messages.len(),
            "steer messages drained into the turn"
        );
        applied(messages.into_iter().map(ConversationEffect::AppendMessage))
    }
}

fn auto_compact_threshold() -> f64 {
    std::env::var("GOOSE_AUTO_COMPACT_THRESHOLD")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(goose_context_management::DEFAULT_COMPACTION_THRESHOLD)
}

fn context_limit_override() -> Result<Option<usize>> {
    std::env::var("GOOSE_CONTEXT_LIMIT")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|error| anyhow::anyhow!("invalid GOOSE_CONTEXT_LIMIT {value:?}: {error}"))
        })
        .transpose()
}

fn needs_compaction(
    provider_manages_context: bool,
    total_tokens: Option<i32>,
    context_limit: usize,
    threshold: f64,
) -> bool {
    if provider_manages_context || threshold <= 0.0 || threshold >= 1.0 {
        return false;
    }
    context_limit > 0
        && total_tokens.is_some_and(|tokens| {
            let current_tokens = tokens.max(0) as f64;
            current_tokens / context_limit as f64 > threshold
        })
}

fn compacted_conversation(conversation: &Conversation, summary: Message) -> Conversation {
    const CONTINUATION: &str = "Your context was compacted. The previous message contains a summary of the conversation so far.\nDo not mention that you read a summary or that conversation summarization occurred.\nJust continue the conversation naturally based on the summarized context.";
    const TOOL_CONTINUATION: &str = "Your context was compacted. The previous message contains a summary of the conversation so far.\nDo not mention that you read a summary or that conversation summarization occurred.\nContinue calling tools as necessary to complete the task.";

    let messages = conversation.messages();
    let preserved = messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            if !message.is_agent_visible()
                || message.is_turn_context()
                || message.role != rmcp::model::Role::User
            {
                return None;
            }
            let projected = message.agent_visible_content();
            let has_text = projected
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::Text(_)));
            let has_tool_content = projected.content.iter().any(|content| {
                matches!(
                    content,
                    MessageContent::ToolRequest(_) | MessageContent::ToolResponse(_)
                )
            });
            if !has_text || has_tool_content {
                return None;
            }
            let message = projected
                .content
                .into_iter()
                .filter(|content| matches!(content, MessageContent::Text(_)))
                .fold(
                    Message::user().with_metadata(MessageMetadata::agent_only()),
                    Message::with_content,
                );
            Some((index, message))
        });

    let is_most_recent = preserved
        .as_ref()
        .is_some_and(|(index, _)| messages[*index + 1..].iter().all(Message::is_turn_context));
    let continuation_text = if is_most_recent {
        CONTINUATION
    } else {
        TOOL_CONTINUATION
    };

    let mut compacted = messages
        .iter()
        .cloned()
        .map(|message| {
            let metadata = message.metadata.clone().with_agent_invisible();
            message.with_metadata(metadata)
        })
        .collect::<Vec<_>>();
    let summary = summary.with_metadata(MessageMetadata::agent_only());
    let continuation = Message::assistant()
        .with_text(continuation_text)
        .with_metadata(MessageMetadata::agent_only());
    let continuation_created = continuation.created;
    let (continuation, _) = merge_consecutive_messages(vec![summary, continuation]);
    compacted.extend(continuation);

    if let Some((index, mut message)) = preserved {
        message.created = continuation_created;
        compacted.push(message);
        if let Some(turn_context) = messages[index + 1..]
            .iter()
            .rev()
            .find(|message| message.is_turn_context() && message.is_agent_visible())
        {
            let mut carried = turn_context.clone();
            carried.id = None;
            if let Some(latest) = compacted.iter().map(|message| message.created).max() {
                carried.created = carried.created.max(latest);
            }
            compacted.push(carried);
        }
    }

    Conversation::new_unvalidated(compacted)
}

/// Compacts the conversation when it approaches the context limit.
///
/// Summarization comes from the standalone Goose context-management crate.
/// This operation owns the threshold policy and the Buzz-specific
/// `_PostCompact` hook that re-injects buzz-dev-mcp's todo state.
///
/// Not goose's `CompactionOperation`: that one yields to the client and emits
/// its own user-facing notification. In buzz a yield ends the turn and the
/// notification would post into the channel, so a long conversation would stop
/// mid-work and announce its own housekeeping to the humans watching.
pub struct BuzzCompactionOperation {
    model: crate::model::SessionModel,
    mcp: Arc<McpRegistry>,
    hook_extension: Option<String>,
}

impl BuzzCompactionOperation {
    pub fn new(
        model: crate::model::SessionModel,
        mcp: Arc<McpRegistry>,
        hook_extension: Option<String>,
    ) -> Self {
        Self {
            model,
            mcp,
            hook_extension,
        }
    }
}

#[async_trait]
impl Operation<TurnSession, ConversationEffect> for BuzzCompactionOperation {
    fn name(&self) -> &'static str {
        "buzz_compaction"
    }

    async fn run(
        &self,
        session: &TurnSession,
        conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<ConversationEffect>> {
        let (provider, model_config, _model_id) = self.model.snapshot().await;
        let threshold = auto_compact_threshold();
        let context_limit_override = context_limit_override()?;
        let context_limit = provider
            .get_context_limit(&model_config.model_name, context_limit_override)
            .await;
        if !needs_compaction(
            provider.manages_own_context(),
            session.total_tokens,
            context_limit,
            threshold,
        ) {
            return not_applicable();
        }

        let visible_messages = conversation
            .messages()
            .iter()
            .filter(|message| message.is_agent_visible() && !message.is_turn_context())
            .cloned()
            .collect::<Vec<_>>();
        let model = goose_context_management::ProviderModel::new(provider, model_config);
        let summary = goose_context_management::summarize(
            &model,
            None,
            &goose_context_management::Templates::default(),
            &visible_messages,
        )
        .await?;
        let compacted = compacted_conversation(conversation, summary.message);

        tracing::info!(target: "buzz_agent::compaction", "history compacted");

        // `ConversationEffect::ReplaceConversation` carries no usage figure, so
        // the running total is reset by the driving loop's `apply_effects`
        // instead: the old total described a conversation that no longer
        // exists, and carrying it forward would re-trigger compaction at once.
        let mut effects = vec![ConversationEffect::ReplaceConversation(compacted)];

        if let Some(extension) = &self.hook_extension {
            if let Some(reported) = crate::hooks::post_compact_state(&self.mcp, extension).await {
                // `[PostCompact]` prefix preserved from the inline version:
                // it is how the model tells re-injected state apart from a
                // user turn, and dropping it would change what it reads.
                effects.push(ConversationEffect::AppendMessage(
                    Message::user()
                        .with_text(format!("[PostCompact] {reported}"))
                        .with_visibility(false, true),
                ));
            }
        }

        applied(effects)
    }
}

/// The operations that gate the start of a round.
///
/// One entry today. As `PLANS/BUZZ_OPERATIONS_MIGRATION.md` proceeds, the
/// steer drain, compaction, the `_Stop` veto and `[Reflect]` join it, and
/// their precedence becomes the order of this list.
pub fn round_gate(
    max_rounds: u32,
    outcome: Outcome,
    cancel: tokio_util::sync::CancellationToken,
    stop_veto: Option<(Arc<McpRegistry>, String)>,
    reply_guard_tools: Option<Vec<String>>,
) -> StateMachine<'static, TurnSession, ConversationEffect> {
    // Order is precedence. The round budget is checked first: once it is
    // spent the turn ends, and asking `_Stop` to veto a turn we are ending
    // anyway would dispatch a tool call whose answer cannot be honoured.
    let mut steps: Vec<Step<TurnSession, ConversationEffect>> = vec![Step::Operation(Arc::new(
        BuzzMaxRoundsOperation::new(max_rounds, outcome),
    ))];
    if let Some((mcp, extension)) = stop_veto {
        steps.push(Step::Operation(Arc::new(BuzzStopVetoOperation::new(
            mcp,
            extension,
            crate::hooks::stop_block_cap(),
        ))));
    }
    // After the `_Stop` veto: on main both objections rode the same gate, and
    // a hook objection already keeps the turn alive, so reminding as well
    // would spend two rounds where main spent one.
    if let Some(available_tools) = reply_guard_tools {
        steps.push(Step::Operation(Arc::new(BuzzReplyGuardOperation::new(
            MAX_REPLY_NAGS,
            available_tools,
        ))));
    }
    StateMachine::new(steps, cancel)
}

/// The operations that run at the start of a round, before inference.
///
/// A separate machine from [`round_gate`] because these two sets run at
/// different points and must not be confused: `round_gate` also runs when a
/// turn wants to *end*, and draining steers or compacting there would change
/// when they happen. `StateMachine::step` stops at the first operation that
/// applies, so one combined machine could not express "do both".
pub fn round_start(
    steers: crate::steer::SteerQueue,
    compaction: BuzzCompactionOperation,
    cancel: tokio_util::sync::CancellationToken,
) -> StateMachine<'static, TurnSession, ConversationEffect> {
    // Steer before compaction: a steer that arrives just as the window fills
    // should be part of what gets summarised, not appended to a conversation
    // that was compacted a moment earlier without it.
    StateMachine::new(
        vec![
            Step::Operation(Arc::new(BuzzSteerOperation::new(steers))),
            Step::Operation(Arc::new(compaction)),
        ],
        cancel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// A bare agent with no extensions: enough to dispatch a tool call that
    /// will fail to resolve, which is the "hook unavailable" path.
    async fn agent() -> Arc<McpRegistry> {
        Arc::new(McpRegistry::empty())
    }

    fn emitter() -> Emitter {
        let (tx, rx) = mpsc::channel(16);
        // Keep the receiver alive for the duration of the test: a dropped
        // receiver makes every emit fail silently, which would mask a real
        // regression in an operation that emits.
        std::mem::forget(rx);
        Emitter::new(tx, CancellationToken::new())
    }

    /// A turn in progress: one user kickoff, then `assistant_turns` rounds of
    /// assistant work.
    ///
    /// Tool responses separate the assistant messages because consecutive
    /// assistant messages count as a *single* turn -- which is the real shape
    /// anyway, since a round that continues does so by calling a tool.
    fn conversation(assistant_turns: usize) -> Conversation {
        let mut messages = vec![Message::user().with_text("kickoff")];
        for i in 0..assistant_turns {
            messages.push(Message::assistant().with_text(format!("turn {i}")));
            if i + 1 < assistant_turns {
                messages.push(tool_response());
            }
        }
        Conversation::new_unvalidated(messages)
    }

    /// A user-role message that does *not* reset the kickoff, standing in for
    /// a tool result.
    fn tool_response() -> Message {
        Message::user()
            .with_text("tool output")
            .with_visibility(false, true)
    }

    #[test]
    fn compaction_threshold_requires_reported_occupancy_above_the_boundary() {
        assert!(!needs_compaction(false, None, 100_000, 0.8));
        assert!(!needs_compaction(false, Some(80_000), 100_000, 0.8));
        assert!(needs_compaction(false, Some(80_001), 100_000, 0.8));
        assert!(!needs_compaction(true, Some(90_000), 100_000, 0.8));
        assert!(!needs_compaction(false, Some(90_000), 100_000, 0.0));
        assert!(!needs_compaction(false, Some(90_000), 100_000, 1.0));
        assert!(!needs_compaction(false, Some(1), 0, 0.8));
    }

    #[test]
    fn compaction_preserves_the_latest_prompt_and_turn_context() {
        let first = Message::user().with_text("old prompt");
        let answer = Message::assistant().with_text("old answer");
        let latest = Message::user().with_text("latest prompt");
        let mut turn_context = Message::user()
            .with_text("current context")
            .with_visibility(false, true);
        turn_context.metadata.turn_context = true;
        let compacted = compacted_conversation(
            &Conversation::new_unvalidated(vec![first, answer, latest, turn_context]),
            Message::assistant().with_text("summary"),
        );
        let visible = compacted
            .messages()
            .iter()
            .filter(|message| message.is_agent_visible())
            .collect::<Vec<_>>();

        assert!(visible
            .iter()
            .any(|message| message.as_concat_text().contains("summary")));
        assert!(visible
            .iter()
            .any(|message| message.as_concat_text() == "latest prompt"));
        assert!(visible.iter().any(|message| message.is_turn_context()));
        assert!(!compacted.messages()[0].is_agent_visible());
        assert!(!compacted.messages()[1].is_agent_visible());
    }

    #[tokio::test]
    async fn under_budget_does_not_apply() {
        let outcome = Outcome::new();
        let op = BuzzMaxRoundsOperation::new(3, outcome.clone());
        let result = op
            .run(&TurnSession::default(), &conversation(2), &emitter())
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
        assert!(outcome.take().is_none());
    }

    #[tokio::test]
    async fn at_budget_ends_the_turn_with_max_turn_requests() {
        let outcome = Outcome::new();
        let op = BuzzMaxRoundsOperation::new(3, outcome.clone());
        let result = op
            .run(&TurnSession::default(), &conversation(3), &emitter())
            .await
            .unwrap();
        match result {
            OperationResult::Applied(step) => assert!(step.yield_to_client),
            OperationResult::NotApplicable => panic!("budget was reached but nothing applied"),
        }
        assert!(matches!(outcome.take(), Some(StopReason::MaxTurnRequests)));
    }

    /// An assistant message with an outstanding tool request: the model is
    /// still working, so the turn is not trying to end.
    fn tool_calling_conversation() -> Conversation {
        Conversation::new_unvalidated(vec![
            Message::user().with_text("kickoff"),
            Message::assistant().with_tool_request(
                "call_1",
                Ok(rmcp::model::CallToolRequestParams::new("developer__shell")),
            ),
        ])
    }

    #[tokio::test]
    async fn the_veto_does_not_apply_while_tools_are_outstanding() {
        // The hook is only asked when the turn is trying to end. Asking
        // mid-tool-call would dispatch `_Stop` on every round.
        let op = BuzzStopVetoOperation::new(agent().await, "buzz-dev-mcp".to_string(), 3);
        let result = op
            .run(
                &TurnSession::default(),
                &tool_calling_conversation(),
                &emitter(),
            )
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
    }

    #[tokio::test]
    async fn a_missing_hook_extension_never_traps_the_turn() {
        // No such extension -> `stop_objection` yields None -> the turn ends.
        // A broken hook must not be able to hold a turn open.
        let op = BuzzStopVetoOperation::new(agent().await, "no-such-extension".to_string(), 3);
        let result = op
            .run(&TurnSession::default(), &conversation(1), &emitter())
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
    }

    #[tokio::test]
    async fn the_cap_counts_objections_already_in_the_conversation() {
        // Three prior objections is the cap, so the operation must decline
        // rather than dispatch a fourth `_Stop` call. Proven by pointing it at
        // an extension that would otherwise be consulted: the result is the
        // same NotApplicable either way, so instead assert the counting
        // helper directly against tagged messages.
        let op = BuzzStopVetoOperation::new(agent().await, "buzz-dev-mcp".to_string(), 3);

        let mut objection = Message::user()
            .with_text("[Stop] finish your todos")
            .with_visibility(false, true);
        op.set_message_meta(&mut objection, OBJECTED, serde_json::json!(true));

        let mut messages = vec![Message::user().with_text("kickoff")];
        for _ in 0..3 {
            messages.push(objection.clone());
            messages.push(Message::assistant().with_text("done"));
        }
        let conversation = Conversation::new_unvalidated(messages);

        let counted = messages_since_kickoff(&conversation)
            .unwrap()
            .iter()
            .filter(|message| op.message_meta(message, OBJECTED).is_some())
            .count();
        assert_eq!(counted, 3, "each objection must be countable from history");

        // At the cap the turn is allowed to end.
        let result = op
            .run(&TurnSession::default(), &conversation, &emitter())
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
    }

    fn shell_request(command: &str) -> Message {
        let mut args = serde_json::Map::new();
        args.insert("command".into(), serde_json::json!(command));
        Message::assistant().with_tool_request(
            "call_1",
            Ok(rmcp::model::CallToolRequestParams::new("developer__shell").with_arguments(args)),
        )
    }

    /// A turn that ran a tool and then answered.
    fn turn_with(request: Message) -> Conversation {
        Conversation::new_unvalidated(vec![
            Message::user().with_text("kickoff"),
            request,
            tool_response(),
            Message::assistant().with_text("done"),
        ])
    }

    #[tokio::test]
    async fn the_reply_guard_reminds_when_nothing_was_published() {
        let op = BuzzReplyGuardOperation::new(MAX_REPLY_NAGS, ["developer__shell".to_string()]);
        let result = op
            .run(&TurnSession::default(), &conversation(1), &emitter())
            .await
            .unwrap();
        match result {
            OperationResult::Applied(step) => {
                assert!(!step.yield_to_client, "a reminder must not end the turn");
                assert_eq!(step.effects.len(), 1);
            }
            OperationResult::NotApplicable => panic!("expected a reminder"),
        }
    }

    #[tokio::test]
    async fn a_publish_attempt_disarms_the_guard() {
        let op = BuzzReplyGuardOperation::new(MAX_REPLY_NAGS, ["developer__shell".to_string()]);
        for command in [
            "buzz messages send --channel x --content hi",
            "buzz messages send-diff --channel x",
            "buzz reactions add --event x --emoji +1",
        ] {
            let result = op
                .run(
                    &TurnSession::default(),
                    &turn_with(shell_request(command)),
                    &emitter(),
                )
                .await
                .unwrap();
            assert!(
                matches!(result, OperationResult::NotApplicable),
                "`{command}` should disarm the guard"
            );
        }
    }

    #[tokio::test]
    async fn an_unrelated_tool_call_does_not_disarm_the_guard() {
        let op = BuzzReplyGuardOperation::new(MAX_REPLY_NAGS, ["developer__shell".to_string()]);
        let result = op
            .run(
                &TurnSession::default(),
                &turn_with(shell_request("ls -la")),
                &emitter(),
            )
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::Applied(_)));
    }

    #[tokio::test]
    async fn a_hallucinated_shell_does_not_disarm_the_guard() {
        let op = BuzzReplyGuardOperation::new(MAX_REPLY_NAGS, ["real__shell".to_string()]);
        let result = op
            .run(
                &TurnSession::default(),
                &turn_with(shell_request("buzz messages send --channel x --content hi")),
                &emitter(),
            )
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::Applied(_)));
    }

    #[tokio::test]
    async fn the_guard_stops_after_its_budget() {
        // Advisory, not compulsion: after MAX_REPLY_NAGS the turn ends whether
        // or not anything was published.
        let op = BuzzReplyGuardOperation::new(MAX_REPLY_NAGS, ["developer__shell".to_string()]);
        let mut nag = Message::user()
            .with_text(REPLY_GUARD_NAG)
            .with_visibility(false, true);
        op.set_message_meta(&mut nag, NAGGED, serde_json::json!(true));

        let mut messages = vec![Message::user().with_text("kickoff")];
        for _ in 0..MAX_REPLY_NAGS {
            messages.push(nag.clone());
            messages.push(Message::assistant().with_text("done"));
        }
        let result = op
            .run(
                &TurnSession::default(),
                &Conversation::new_unvalidated(messages),
                &emitter(),
            )
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
    }

    #[tokio::test]
    async fn steering_appends_queued_messages_and_drains_them() {
        let steers = crate::steer::SteerQueue::new();
        steers
            .push(Message::user().with_text("actually, do X"))
            .await;
        let op = BuzzSteerOperation::new(steers.clone());

        match op
            .run(&TurnSession::default(), &conversation(1), &emitter())
            .await
            .unwrap()
        {
            OperationResult::Applied(step) => {
                assert_eq!(step.effects.len(), 1);
                assert!(!step.yield_to_client, "a steer must not end the turn");
            }
            OperationResult::NotApplicable => panic!("expected the steer to apply"),
        }

        // Drained, not peeked: a steer delivered twice would repeat the
        // instruction to the model on the next round.
        assert!(matches!(
            op.run(&TurnSession::default(), &conversation(1), &emitter())
                .await
                .unwrap(),
            OperationResult::NotApplicable
        ));
    }

    #[tokio::test]
    async fn an_empty_steer_queue_does_not_apply() {
        // Must be NotApplicable rather than an empty Applied: the loop treats
        // "applied but changed nothing" as a reason to look again, and an
        // always-applying operation would spin it.
        let op = BuzzSteerOperation::new(crate::steer::SteerQueue::new());
        assert!(matches!(
            op.run(&TurnSession::default(), &conversation(1), &emitter())
                .await
                .unwrap(),
            OperationResult::NotApplicable
        ));
    }

    #[tokio::test]
    async fn a_zero_cap_disables_the_veto_entirely() {
        // Documented behaviour on main: BUZZ_AGENT_STOP_MAX_REJECTIONS=0 turns
        // `_Stop` off. An operator with a misbehaving hook depends on it.
        let op = BuzzStopVetoOperation::new(agent().await, "buzz-dev-mcp".to_string(), 0);
        let result = op
            .run(&TurnSession::default(), &conversation(1), &emitter())
            .await
            .unwrap();
        assert!(matches!(result, OperationResult::NotApplicable));
    }

    #[tokio::test]
    async fn ends_turn_matches_the_shape_of_a_finished_answer() {
        assert!(ends_turn(&[Message::assistant().with_text("done")]));
        assert!(!ends_turn(&[Message::user().with_text("hi")]));
        assert!(!ends_turn(tool_calling_conversation().messages()));
        assert!(!ends_turn(&[]));
    }

    #[tokio::test]
    async fn the_budget_appends_no_message() {
        // goose's MaxTurnsOperation asks "Would you like me to continue?".
        // buzz must not: buzz-acp publishes assistant messages to a channel,
        // so a synthesised one would be indistinguishable from the agent
        // actually saying it.
        let op = BuzzMaxRoundsOperation::new(1, Outcome::new());
        let result = op
            .run(&TurnSession::default(), &conversation(1), &emitter())
            .await
            .unwrap();
        match result {
            OperationResult::Applied(step) => assert!(step.effects.is_empty()),
            OperationResult::NotApplicable => panic!("budget was reached but nothing applied"),
        }
    }

    #[test]
    fn the_first_recorded_reason_wins() {
        // The machine stops at the first operation that applies, so a second
        // write would name a decision that never took effect.
        let outcome = Outcome::new();
        outcome.set(StopReason::MaxTurnRequests);
        outcome.set(StopReason::Cancelled);
        assert!(matches!(outcome.take(), Some(StopReason::MaxTurnRequests)));
    }

    #[tokio::test]
    async fn a_mid_turn_steer_restarts_the_budget() {
        // A steer is a user-visible message, so it becomes the new kickoff and
        // the budget counts from there. That is the behaviour we want -- the
        // user gave fresh direction, so the agent gets a fresh allowance --
        // but it is emergent rather than chosen, so pin it: the alternative
        // (a steer silently inheriting an almost-exhausted budget) would cut
        // the agent off mid-instruction.
        let outcome = Outcome::new();
        let op = BuzzMaxRoundsOperation::new(2, outcome.clone());

        let mut messages = conversation(2).messages().to_vec();
        messages.push(
            Message::user()
                .with_text("actually, do this instead")
                .with_steer(),
        );
        messages.push(Message::assistant().with_text("ok"));
        let steered = Conversation::new_unvalidated(messages);

        let result = op
            .run(&TurnSession::default(), &steered, &emitter())
            .await
            .unwrap();
        assert!(
            matches!(result, OperationResult::NotApplicable),
            "the steer should have restarted the turn budget"
        );
        assert!(outcome.take().is_none());
    }
}
