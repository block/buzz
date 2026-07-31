//! The turn driver: Goose's agent loop, translated onto buzz-agent's ACP wire.
//!
//! This replaces roughly 8,000 lines of `crates/buzz-agent`:
//!
//! | buzz-agent file | lines | who owns it now |
//! |---|---:|---|
//! | `llm.rs`      | 3846 | `goose::providers` (superset of the 4 providers) |
//! | `mcp.rs`      | 1139 | `goose::agents::extension_manager` |
//! | `auth.rs`     |  845 | `goose::providers` (incl. Databricks OAuth) |
//! | `agent.rs`    |  746 | `goose::agents::Agent::reply` |
//! | `hints.rs`    |  726 | kept -- goose's loader can't replace it (below) |
//! | `catalog.rs`  |  631 | `goose::providers::init` model discovery |
//! | `builtin.rs`  |  575 | kept -- served as a goose *frontend* tool (below) |
//! | `handoff.rs`  |  430 | `goose::context_mgmt` (auto-compaction) |
//!
//! Two entries above are NOT goose substitutions; the old buzz-agent modules
//! are kept and wired into goose instead, because goose's own equivalents
//! don't cover them:
//!
//! * **`builtin.rs` / `load_skill`.** `Agent::with_config` loads zero
//!   extensions and `build_agent` only adds the `mcpServers` the harness
//!   declares, so goose's `skills` platform extension is never loaded. Instead
//!   `build_agent` registers `load_skill` as a goose *platform* extension
//!   backed by an `McpClientTrait` (see [`crate::builtin_client`]), so goose
//!   dispatches it on its ordinary tool path.
//! * **`AGENTS.md` hints / skill index.** Goose's loader keys off
//!   `GOOSE_HINTS_FILENAME` (`.goosehints`); the old code walked the directory
//!   chain for `AGENTS.md` plus `~/AGENTS.md`. Every repo here ships the
//!   latter shape, so `hints.rs` is kept and its output injected via
//!   `extend_system_prompt` at session build.
//!
//! What this file keeps is the part Goose does *not* know about: the mapping
//! from `AgentEvent` onto the exact `session/update` notifications `buzz-acp`
//! consumes (`acp.rs:1528-1627`), and the `keepalive` ticker that resets the
//! harness idle clock.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use goose::agents::{Agent, AgentEvent, SessionConfig};
use goose_provider_types::conversation::message::{Message, MessageContent};

use crate::types::{AgentError, ContentBlock, StopReason};
use crate::wire::{self, WireSender};

/// How often to emit `keepalive` while waiting on the provider.
///
/// NOT part of ACP. `buzz-acp` runs an idle-timeout clock that is reset on
/// every line of valid JSON (`acp.rs:1197-1199`); a silent agent is killed.
/// buzz-agent emitted this from inside its provider `select!`
/// (`agent.rs:122-127`) and `buzz-acp` treats it as a no-op that only resets
/// the clock (`acp.rs:1623`).
///
/// Goose streams, so during generation the chunks themselves keep the clock
/// alive — but there is no traffic during a long *pre-first-token* wait (big
/// prompt processing, provider queueing, reasoning models). So the ticker
/// stays.
const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Appended by buzz-agent to every failed tool result (`agent.rs:21-22`) so
/// the model diagnoses a failure instead of blindly retrying it.
///
/// buzz-agent mutated the tool result itself. Goose gives no interception
/// point for that: its `PostToolUseFailure` hook is fire-and-forget and its
/// output is discarded (`agent.rs:589-620`). So we deliver the same text as a
/// steer instead — goose drains pending steers at the round boundary
/// (`agent.rs:1951-1974`), which is exactly when the model would next act on
/// the failed result. Agent-visible, user-invisible.
const ERROR_REFLECTION: &str =
    "[Reflect] Before retrying, identify the cause and change your approach.";

/// Cap on reflections per turn, so a tool failing in a loop cannot flood the
/// conversation.
const MAX_REFLECTIONS: usize = 8;

/// How long to let goose unwind after `session/cancel` before giving up.
///
/// Cancellation is cooperative: goose has to notice the token, send
/// `notifications/cancelled` to each in-flight MCP request
/// (`mcp_client.rs:687-690`), emit the resulting tool responses, and end the
/// stream. Dropping the stream instead skips all of that. This bounds the
/// wait so a wedged MCP child cannot hold the turn open indefinitely.
const CANCEL_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-turn token accounting, mirroring buzz-agent's contract.
#[derive(Debug, Default, Clone, Copy)]
pub struct TurnTokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    /// Cache reads+writes. A *subset* of `input`, not an addition to it --
    /// goose documents `cache_read_input_tokens`/`cache_write_input_tokens` as
    /// already counted in `input_tokens` (`token_usage.rs:72-78`). buzz-acp
    /// reads this as `accumulatedCachedInputTokens` for pricing (#3463).
    pub cached_input: Option<u64>,
    /// Provider-reported total, when it reports one. buzz-acp treats a missing
    /// total as unknown rather than zero (#3593).
    pub total: Option<u64>,
}

impl TurnTokens {
    pub fn observed(&self) -> bool {
        self.input.is_some() || self.output.is_some()
    }
}

/// Flatten ACP prompt blocks into the text Goose's `Message` carries.
pub fn prompt_to_text(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            ContentBlock::Text { text } => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
            ContentBlock::ResourceLink { uri } => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(uri);
            }
            ContentBlock::Unsupported => {}
        }
    }
    out
}

/// Drive one `session/prompt` turn to completion.
///
/// Returns the ACP stop reason plus the turn's token counts. The caller is
/// responsible for emitting `usage_update` *before* the `session/prompt`
/// response — that ordering is load-bearing for kind-44200 metrics
/// (`buzz-agent/src/lib.rs:701-706`).
///
/// `hook_extension` names the MCP extension carrying `_Stop`/`_PostCompact`
/// (buzz-dev-mcp). When set, the turn is not allowed to end while `_Stop`
/// objects — see [`crate::hooks`].
#[allow(clippy::too_many_arguments)]
pub async fn run_turn(
    agent: Arc<Agent>,
    session_id: &str,
    prompt: Vec<ContentBlock>,
    max_rounds: Option<u32>,
    wire_tx: &WireSender,
    cancel: CancellationToken,
    hook_extension: Option<&str>,
) -> (Result<StopReason, AgentError>, TurnTokens) {
    let mut tokens = TurnTokens::default();
    let mut next_message = Message::user().with_text(prompt_to_text(&prompt));
    let mut stop_blocks: u32 = 0;

    // Outer loop exists solely for the `_Stop` veto: goose's `reply()` stream
    // ends when the model stops, so continuing means re-entering `reply()`
    // with the objection appended. Goose's own Stop hook does the equivalent
    // internally (`agent.rs:2891-2917`); we do it here because its
    // `hook_manager` is private.
    loop {
        let stop = match drive_stream(
            &agent,
            session_id,
            next_message,
            max_rounds,
            wire_tx,
            &cancel,
            &mut tokens,
            hook_extension,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => return (Err(e), tokens),
        };

        // Only a clean end-of-turn is vetoable. Cancellation and refusals pass
        // through untouched.
        if !matches!(stop, StopReason::EndTurn) || cancel.is_cancelled() {
            return (Ok(stop), tokens);
        }

        let Some(extension) = hook_extension else {
            return (Ok(stop), tokens);
        };

        if stop_blocks >= crate::hooks::MAX_STOP_BLOCKS {
            tracing::warn!(
                blocks = stop_blocks,
                "_Stop veto cap reached; ending turn anyway"
            );
            return (Ok(stop), tokens);
        }

        let Some(session) = current_session(session_id).await else {
            return (Ok(stop), tokens);
        };
        let Some(objection) = crate::hooks::stop_objection(&agent, &session, extension).await
        else {
            return (Ok(stop), tokens);
        };

        stop_blocks += 1;
        tracing::info!(blocks = stop_blocks, "_Stop hook vetoed end of turn");

        // Agent-visible, user-invisible — the objection steers the model
        // without appearing in the channel, matching both buzz-agent and
        // goose's own Deny handling.
        next_message = Message::user()
            .with_text(format!("[Stop] {objection}"))
            .with_visibility(false, true);
    }
}

/// Look up the goose `Session` record needed to dispatch a hook tool.
async fn current_session(session_id: &str) -> Option<goose::session::session_manager::Session> {
    goose::session::session_manager::SessionManager::instance()
        .get_session(session_id, false)
        .await
        .ok()
}

/// Drive a single `reply()` stream to completion.
#[allow(clippy::too_many_arguments)]
async fn drive_stream(
    agent: &Arc<Agent>,
    session_id: &str,
    message: Message,
    max_rounds: Option<u32>,
    wire_tx: &WireSender,
    cancel: &CancellationToken,
    tokens: &mut TurnTokens,
    hook_extension: Option<&str>,
) -> Result<StopReason, AgentError> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: max_rounds,
        retry_config: None,
    };

    let mut stream = match agent
        .reply(message, session_config, Some(cancel.clone()))
        .await
    {
        Ok(s) => s,
        Err(e) => return Err(AgentError::Llm(e.to_string())),
    };

    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    keepalive.tick().await; // first tick is immediate; discard it

    let mut stop = StopReason::EndTurn;
    let mut compacted = false;
    let mut reflections = 0usize;

    // Set once cancellation is observed; bounds how long we let goose unwind.
    let mut drain: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;

    // Tool calls announced to the harness, minus those that reached a terminal
    // state. Anything still here when the stream ends gets a synthetic
    // terminal update — see the cancel arm below.
    let mut open_tool_calls: Vec<String> = Vec::new();

    loop {
        tokio::select! {
            // Keep the harness idle clock alive during provider silence.
            _ = keepalive.tick() => {
                wire::send(
                    wire_tx,
                    wire::session_update(session_id, json!({ "sessionUpdate": "keepalive" })),
                )
                .await;
            }

            // Cancellation is a cooperative DRAIN, not an abort.
            //
            // Breaking here drops `stream`, which drops the futures goose is
            // awaiting — so `mcp_client.rs:688` never reaches its
            // `cancel_token.cancelled()` arm and never sends
            // `notifications/cancelled`. The MCP child keeps running its tool
            // after the turn is over, and any announced `tool_call` never
            // reaches a terminal state, leaving the desktop UI spinning
            // (the invariant buzz-agent held at `agent.rs:470-477`).
            //
            // So we keep polling the stream and let goose unwind: it emits the
            // tool responses, sends the MCP cancellations, and ends the stream
            // itself. `drain` only bounds how long we are willing to wait.
            _ = cancel.cancelled(), if drain.is_none() => {
                stop = StopReason::Cancelled;
                drain = Some(Box::pin(tokio::time::sleep(CANCEL_DRAIN_TIMEOUT)));
            }

            // Goose did not unwind in time. Give up and synthesise terminal
            // states, because a stuck spinner is worse than a wrong status.
            _ = async { drain.as_mut().unwrap().await }, if drain.is_some() => {
                tracing::warn!("cancel drain timed out; forcing terminal tool states");
                break;
            }

            next = stream.next() => {
                let Some(event) = next else { break };
                match event {
                    Ok(ev) => {
                        if let Some(reason) = handle_event(
                            ev,
                            agent,
                            session_id,
                            wire_tx,
                            tokens,
                            &mut compacted,
                            &mut reflections,
                            &mut open_tool_calls,
                        )
                        .await
                        {
                            stop = reason;
                            break;
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        // Preserve buzz-agent's error taxonomy so the harness's
                        // JSON-RPC code mapping stays meaningful.
                        let err = if msg.contains("auth") || msg.contains("401") {
                            AgentError::LlmAuth(msg)
                        } else if msg.contains("model") && msg.contains("not found") {
                            AgentError::LlmModelNotFound(msg)
                        } else {
                            AgentError::Llm(msg)
                        };
                        return Err(err);
                    }
                }
            }
        }
    }

    // Anything announced but never resolved gets a synthetic terminal update.
    // buzz-agent guaranteed this invariant (`agent.rs:470-477`) because the
    // desktop renders an unresolved `tool_call` as a spinner forever. Normally
    // the drain above means this list is already empty; it only fires when
    // goose failed to unwind inside CANCEL_DRAIN_TIMEOUT.
    for tool_call_id in open_tool_calls.drain(..) {
        wire::send(
            wire_tx,
            wire::session_update(
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": tool_call_id,
                    "status": "failed",
                }),
            ),
        )
        .await;
    }

    // Goose compacted history mid-turn. Re-inject the todo list so it survives
    // the truncation — this is what buzz-agent's `_PostCompact` existed for
    // (`handoff.rs:73-81`). Steering is the right channel: goose drains it at
    // the next round boundary (`agent.rs:1951-1974`).
    if compacted && !cancel.is_cancelled() {
        if let Some(extension) = hook_extension {
            if let Some(session) = current_session(session_id).await {
                if let Some(state) =
                    crate::hooks::post_compact_state(agent, &session, extension).await
                {
                    agent
                        .steer(
                            session_id,
                            Message::user()
                                .with_text(format!("[PostCompact] {state}"))
                                .with_visibility(false, true),
                        )
                        .await;
                }
            }
        }
    }

    if cancel.is_cancelled() {
        stop = StopReason::Cancelled;
    }

    Ok(stop)
}

/// Translate one `AgentEvent` into ACP notifications.
///
/// Returns `Some(StopReason)` if the event terminates the turn.
#[allow(clippy::too_many_arguments)]
async fn handle_event(
    event: AgentEvent,
    agent: &Arc<Agent>,
    session_id: &str,
    wire_tx: &WireSender,
    tokens: &mut TurnTokens,
    compacted: &mut bool,
    reflections: &mut usize,
    open_tool_calls: &mut Vec<String>,
) -> Option<StopReason> {
    match event {
        AgentEvent::Message(msg) => {
            for content in &msg.content {
                emit_content(content, session_id, wire_tx).await;

                if let MessageContent::ToolRequest(req) = content {
                    open_tool_calls.push(req.id.clone());
                }

                if let MessageContent::ToolResponse(resp) = content {
                    open_tool_calls.retain(|id| id != &resp.id);

                    let failed = match &resp.tool_result {
                        Ok(r) => r.is_error.unwrap_or(false),
                        Err(_) => true,
                    };
                    if failed && *reflections < MAX_REFLECTIONS {
                        *reflections += 1;
                        agent
                            .steer(
                                session_id,
                                Message::user()
                                    .with_text(ERROR_REFLECTION)
                                    .with_visibility(false, true),
                            )
                            .await;
                    }
                }
            }
            None
        }

        // Usage arrives per provider chunk, already cost-enriched. Accumulate
        // rather than overwrite: `MessageUsage` is suppressed when the last
        // assistant message has no user-visible content (goose
        // `agent.rs:299`), so it is not a reliable sole source.
        AgentEvent::Usage(usage) => {
            let u = &usage.usage;
            if let Some(i) = u.input_tokens {
                tokens.input = Some(tokens.input.unwrap_or(0) + i.max(0) as u64);
            }
            if let Some(o) = u.output_tokens {
                tokens.output = Some(tokens.output.unwrap_or(0) + o.max(0) as u64);
            }
            let cached = u.cache_read_input_tokens.unwrap_or(0).max(0) as u64
                + u.cache_write_input_tokens.unwrap_or(0).max(0) as u64;
            if u.cache_read_input_tokens.is_some() || u.cache_write_input_tokens.is_some() {
                tokens.cached_input = Some(tokens.cached_input.unwrap_or(0) + cached);
            }
            if let Some(t) = u.total_tokens {
                tokens.total = Some(tokens.total.unwrap_or(0) + t.max(0) as u64);
            }
            None
        }

        // Compaction happened. buzz-agent surfaced this as a `[Context
        // Handoff]` history rewrite; Goose has already done the rewrite, so we
        // only flag it — the caller then asks `_PostCompact` for state to
        // re-inject once the stream settles.
        AgentEvent::HistoryReplaced(_) => {
            tracing::info!(target: "buzz_agent::compaction", "history compacted");
            *compacted = true;
            None
        }

        _ => None,
    }
}

/// Map message content onto the nine `sessionUpdate` variants `buzz-acp`
/// recognises (`acp.rs:1528-1627`). Anything else it debug-logs and drops.
async fn emit_content(content: &MessageContent, session_id: &str, wire_tx: &WireSender) {
    match content {
        MessageContent::Text(t) if !t.text.is_empty() => {
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": t.text },
                    }),
                ),
            )
            .await;
        }

        MessageContent::Thinking(t) if !t.thinking.is_empty() => {
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "agent_thought_chunk",
                        "content": { "type": "text", "text": t.thinking },
                    }),
                ),
            )
            .await;
        }

        // Tool lifecycle. buzz-agent guaranteed every announced tool reached a
        // terminal state, or the desktop UI shows a stuck spinner
        // (`agent.rs:470-477`). Goose emits request and response as separate
        // messages, so the pairing is by tool-call id.
        MessageContent::ToolRequest(req) => {
            let (name, raw) = match &req.tool_call {
                Ok(call) => (
                    call.name.to_string(),
                    serde_json::to_value(&call.arguments).unwrap_or(Value::Null),
                ),
                Err(e) => (String::from("unknown"), json!({ "error": e.to_string() })),
            };
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": req.id,
                        "title": name,
                        "kind": "other",
                        "status": "in_progress",
                        "rawInput": raw,
                    }),
                ),
            )
            .await;
        }

        // Frontend tool calls (`load_skill`) are stripped out of the normal
        // `ToolRequest` flow by goose and arrive as this variant instead. The
        // desktop still needs the announcement: its response comes back as a
        // plain `ToolResponse`, and an update for a never-announced id would
        // break the announce→terminal pairing.
        MessageContent::FrontendToolRequest(req) => {
            let (name, raw) = match &req.tool_call {
                Ok(call) => (
                    call.name.to_string(),
                    serde_json::to_value(&call.arguments).unwrap_or(Value::Null),
                ),
                Err(e) => (String::from("unknown"), json!({ "error": e.to_string() })),
            };
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": req.id,
                        "title": name,
                        "kind": "other",
                        "status": "in_progress",
                        "rawInput": raw,
                    }),
                ),
            )
            .await;
        }

        MessageContent::ToolResponse(resp) => {
            let failed = match &resp.tool_result {
                Ok(r) => r.is_error.unwrap_or(false),
                Err(_) => true,
            };
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": resp.id,
                        "status": if failed { "failed" } else { "completed" },
                    }),
                ),
            )
            .await;
        }

        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_flattens_text_blocks() {
        let blocks = vec![
            ContentBlock::Text {
                text: "hello".into(),
            },
            ContentBlock::Text {
                text: "world".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "hello\nworld");
    }

    #[test]
    fn prompt_includes_resource_links() {
        let blocks = vec![
            ContentBlock::Text { text: "see".into() },
            ContentBlock::ResourceLink {
                uri: "file:///a".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "see\nfile:///a");
    }

    #[test]
    fn prompt_skips_unsupported() {
        let blocks = vec![
            ContentBlock::Unsupported,
            ContentBlock::Text {
                text: "only".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "only");
    }

    #[test]
    fn turn_tokens_observed_requires_a_count() {
        assert!(!TurnTokens::default().observed());
        assert!(TurnTokens {
            input: Some(1),
            ..Default::default()
        }
        .observed());
        assert!(TurnTokens {
            output: Some(1),
            ..Default::default()
        }
        .observed());
    }
}
