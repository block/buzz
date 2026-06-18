//! Agent enrollment for huddles.
//!
//! Mental model:
//!   add_agent_to_huddle → kind:9000 to ephemeral channel
//!                       → kind:9000 to parent channel (best-effort)
//!
//! ACP spawning is NOT needed here: the running agent process auto-subscribes
//! when it receives the kind:9000 membership notification. Huddle-specific
//! env vars (interrupt mode, custom system prompt) are a post-MVP enhancement.

use serde::Serialize;
use uuid::Uuid;

use crate::{app_state::AppState, events, relay::submit_event};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Voice-mode guidelines posted as kind:48106 (huddle guidelines) to the
/// ephemeral channel at huddle start. Agents see them via EOSE replay.
/// Instructs agents on voice-mode etiquette: TTS constraints, brevity,
/// self-selection, and sentence-at-a-time delivery.
///
/// Why sentence-at-a-time: the desktop speaks each agent message as it
/// arrives (queued, in order), so an agent that sends its first sentence
/// immediately — then the rest as separate messages — cuts time-to-first-
/// audio from "full reply generated" to "first sentence generated". This is
/// the prompt-level equivalent of token streaming, with no harness changes.
///
/// Build voice-mode guidelines with the parent channel ID so agents know
/// where "the main channel" is.
pub fn voice_mode_guidelines(parent_channel_id: &str) -> String {
    format!(
        "\
You are in a live voice huddle attached to channel {parent_channel_id}.
Your text is read aloud via TTS, message by message, in the order sent.

Latency matters most: reply IMMEDIATELY — do not compose your full reply
before sending anything. The moment your first sentence is formed, send it
as its own `buzz messages send` tool call: it is what breaks the silence.
Then send each following sentence the same way — one sentence per separate
`buzz messages send` call. Never hold a finished sentence back to bundle it
with the next one.

- If not addressed or relevant: do nothing. Do not respond.
- Keep the whole reply short — a few sentences at most. Start with the answer, no preamble.
- No markdown, code blocks, lists, or structured data — say it naturally.
- To share code or detailed data: say \"I'll post that in the main channel\" and do so.
- When you need a tool, say one short sentence first (e.g. \"Let me check.\"), then run it, then summarize the key finding verbally.
- If a new human message arrives mid-reply, you were interrupted: drop your unsent sentences and respond to the new message instead.
- In multi-agent huddles, identify yourself only when needed.
- Use your Buzz tools proactively when asked."
    )
}

// ── Agent enrollment ──────────────────────────────────────────────────────────

/// Result of adding an agent to a huddle.
///
/// **Invariant:** `ephemeral_added` is always `true` on success — the function
/// returns `Err` before constructing this struct if the ephemeral add fails.
/// The field exists for forward compatibility with future batch-add operations
/// where partial success may be meaningful.
///
/// `parent_added` reflects whether the parent-channel add succeeded;
/// `parent_error` carries the error string when it didn't.
#[derive(Debug, Serialize)]
pub struct AgentAddResult {
    /// Always `true` — invariant guaranteed by [`add_agent_to_huddle`].
    pub ephemeral_added: bool,
    /// Whether the agent was also added to the parent channel (best-effort).
    pub parent_added: bool,
    /// Error from the parent-channel add, if it failed.
    pub parent_error: Option<String>,
}

/// Add an agent to both the ephemeral and parent huddle channels.
///
/// Returns `Err` only if the ephemeral-channel add fails (policy rejection or
/// network error). The parent-channel add is best-effort: failure is captured
/// in `AgentAddResult::parent_error` rather than propagated.
///
/// The running ACP process for this agent will auto-subscribe to the new
/// channel when it receives the kind:9000 membership notification.
pub async fn add_agent_to_huddle(
    ephemeral_channel_id: Uuid,
    parent_channel_id: Uuid,
    agent_pubkey: &str,
    state: &AppState,
) -> Result<AgentAddResult, String> {
    // 1. Add agent to ephemeral channel (required — fail hard on rejection).
    let add_eph = events::build_add_member(ephemeral_channel_id, agent_pubkey, Some("bot"))?;
    submit_event(add_eph, state).await?;

    // 2. Add agent to parent channel — so agent has full context.
    //    Best-effort: capture the error but don't propagate it.
    let (parent_added, parent_error) = {
        let add_parent = events::build_add_member(parent_channel_id, agent_pubkey, Some("bot"))?;
        match submit_event(add_parent, state).await {
            Ok(_) => (true, None),
            Err(e) => {
                eprintln!(
                    "buzz-desktop: add agent to parent channel failed (may already be member): {e}"
                );
                (false, Some(e))
            }
        }
    };

    Ok(AgentAddResult {
        ephemeral_added: true,
        parent_added,
        parent_error,
    })
}
