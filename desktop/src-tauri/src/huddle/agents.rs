//! Agent enrollment for huddles.
//!
//! Mental model:
//!   add_agent_to_huddle → kind:9000 to ephemeral channel
//!                       → preserve existing parent membership, or
//!                         kind:9000 to parent channel (best-effort)
//!
//! ACP spawning is NOT needed here: the running agent process auto-subscribes
//! when it receives the kind:9000 membership notification. Huddle-specific
//! env vars (interrupt mode, custom system prompt) are a post-MVP enhancement.

use std::collections::HashSet;

use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    events,
    huddle::relay_api::{
        fetch_channel_members, fetch_channel_members_with_roles, validate_pubkey_hex,
        MAX_HUDDLE_AGENTS,
    },
    relay::submit_event,
};

use super::{pipeline::start_auto_enabled_transcription, HuddlePhase};

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
/// `parent_added` reflects whether the parent already contained the agent or
/// the parent-channel add succeeded; `parent_error` carries the error string
/// when neither condition could be confirmed.
#[derive(Debug, Serialize)]
pub struct AgentAddResult {
    /// Always `true` — invariant guaranteed by [`add_agent_to_huddle`].
    pub ephemeral_added: bool,
    /// Whether the agent was also added to the parent channel (best-effort).
    pub parent_added: bool,
    /// Error from the parent-channel add, if it failed.
    pub parent_error: Option<String>,
}

/// Result of reconciling channel agent additions into the active Huddle.
#[derive(Debug, Serialize)]
pub struct AgentHuddleSyncResult {
    /// Whether `channel_id` belonged to the active Huddle.
    pub matched_active_huddle: bool,
    /// Agents newly enrolled in the Huddle's ephemeral channel.
    pub added: Vec<String>,
}

// Multiple frontend mutation paths can observe the same membership addition
// (for example, the member hook and the mention send flow). Serialize native
// reconciliation so they share the first result instead of racing duplicate
// membership events through a relay read that has not caught up yet.
static AGENT_SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    // 2. Preserve any active parent membership, regardless of role. Rewriting
    //    an existing DM member as `bot` is both unnecessary and forbidden for
    //    non-admins. Otherwise add the agent so it has full context.
    //    Best-effort: capture a real error but don't propagate it.
    let parent_channel_id_string = parent_channel_id.to_string();
    let parent_already_contains_agent =
        fetch_channel_members_with_roles(&parent_channel_id_string, state)
            .await
            .is_ok_and(|members| contains_member(&members, agent_pubkey));

    let (parent_added, parent_error) = if parent_already_contains_agent {
        (true, None)
    } else {
        let add_parent = events::build_add_member(parent_channel_id, agent_pubkey, Some("bot"))?;
        match submit_event(add_parent, state).await {
            Ok(_) => (true, None),
            Err(e) => {
                let active_after_error =
                    fetch_channel_members_with_roles(&parent_channel_id_string, state)
                        .await
                        .is_ok_and(|members| contains_member(&members, agent_pubkey));
                if active_after_error {
                    (true, None)
                } else {
                    eprintln!("buzz-desktop: add agent to parent channel failed: {e}");
                    (false, Some(e))
                }
            }
        }
    };

    Ok(AgentAddResult {
        ephemeral_added: true,
        parent_added,
        parent_error,
    })
}

/// Reconcile explicitly added channel agents into the active Huddle.
///
/// The source channel may be either the Huddle's parent or its ephemeral chat.
/// Existing ephemeral membership is hydrated first so a mention sent from the
/// Huddle chat does not publish a duplicate membership event. Missing agents
/// are added through the same parent + ephemeral path as the Add agent picker.
pub(crate) async fn sync_agents_for_active_huddle(
    channel_id: &str,
    agent_pubkeys: Vec<String>,
    state: &AppState,
) -> Result<AgentHuddleSyncResult, String> {
    let mut seen = HashSet::new();
    let mut requested = Vec::new();
    for pubkey in agent_pubkeys {
        let normalized = pubkey.to_ascii_lowercase();
        validate_pubkey_hex(&normalized)?;
        if seen.insert(normalized.clone()) {
            requested.push(normalized);
        }
    }
    if requested.is_empty() {
        return Ok(AgentHuddleSyncResult {
            matched_active_huddle: false,
            added: Vec::new(),
        });
    }
    let _sync_guard = AGENT_SYNC_LOCK.lock().await;

    let (ephemeral_channel_id, parent_channel_id, huddle_generation, state_agents) = {
        let huddle = state.huddle()?;
        if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Ok(AgentHuddleSyncResult {
                matched_active_huddle: false,
                added: Vec::new(),
            });
        }
        let ephemeral_channel_id = huddle
            .ephemeral_channel_id
            .clone()
            .ok_or("no ephemeral channel")?;
        let parent_channel_id = huddle
            .parent_channel_id
            .clone()
            .ok_or("no parent channel")?;
        if channel_id != ephemeral_channel_id && channel_id != parent_channel_id {
            return Ok(AgentHuddleSyncResult {
                matched_active_huddle: false,
                added: Vec::new(),
            });
        }
        let state_agents = huddle
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        (
            ephemeral_channel_id,
            parent_channel_id,
            huddle.huddle_generation,
            state_agents,
        )
    };

    // Membership reads can lag a just-accepted write, so merge the relay view
    // with local state instead of allowing a stale snapshot to remove agents.
    let fresh_agents = fetch_channel_members(&ephemeral_channel_id, Some("bot"), state)
        .await
        .unwrap_or_default();
    let mut known_agents = HashSet::new();
    let mut merged_agents = Vec::new();
    for pubkey in state_agents.into_iter().chain(fresh_agents) {
        let normalized = pubkey.to_ascii_lowercase();
        if known_agents.insert(normalized.clone()) {
            merged_agents.push(normalized);
        }
    }
    let missing: Vec<String> = requested
        .into_iter()
        .filter(|pubkey| !known_agents.contains(pubkey))
        .collect();
    if known_agents.len() + missing.len() > MAX_HUDDLE_AGENTS {
        return Err(format!(
            "agent limit reached: {} requested with {} already present (max {})",
            missing.len(),
            known_agents.len(),
            MAX_HUDDLE_AGENTS
        ));
    }

    let ephemeral_uuid = Uuid::parse_str(&ephemeral_channel_id).map_err(|e| e.to_string())?;
    let parent_uuid = Uuid::parse_str(&parent_channel_id).map_err(|e| e.to_string())?;
    let mut added = Vec::new();
    for pubkey in missing {
        add_agent_to_huddle(ephemeral_uuid, parent_uuid, &pubkey, state).await?;
        merged_agents.push(pubkey.clone());
        added.push(pubkey);
    }

    let (roster_changed, transcription_auto_enabled) = {
        let mut huddle = state.huddle()?;
        if !huddle.is_current_huddle(&ephemeral_channel_id, huddle_generation) {
            return Ok(AgentHuddleSyncResult {
                matched_active_huddle: true,
                added,
            });
        }
        let mut roster_changed = false;
        {
            let mut current_agents = huddle
                .agent_pubkeys
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *current_agents != merged_agents {
                *current_agents = merged_agents.clone();
                roster_changed = true;
            }
        }
        for pubkey in &merged_agents {
            if !huddle.participants.contains(pubkey) {
                huddle.participants.push(pubkey.clone());
                roster_changed = true;
            }
        }
        (
            roster_changed,
            huddle.maybe_auto_enable_transcription_for_agents(),
        )
    };

    if transcription_auto_enabled {
        start_auto_enabled_transcription(state, &ephemeral_channel_id).await;
    } else if roster_changed {
        state.emit_huddle_state_changed();
    }

    Ok(AgentHuddleSyncResult {
        matched_active_huddle: true,
        added,
    })
}

#[tauri::command]
pub async fn sync_agents_to_active_huddle(
    channel_id: String,
    agent_pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<AgentHuddleSyncResult, String> {
    sync_agents_for_active_huddle(&channel_id, agent_pubkeys, &state).await
}

fn contains_member(members: &[(String, Option<String>)], pubkey: &str) -> bool {
    members
        .iter()
        .any(|(member_pubkey, _)| member_pubkey.eq_ignore_ascii_case(pubkey))
}

#[cfg(test)]
mod tests {
    use super::contains_member;

    #[test]
    fn existing_parent_membership_is_preserved_regardless_of_role() {
        let members = vec![
            ("agent-member".to_owned(), Some("member".to_owned())),
            ("agent-bot".to_owned(), Some("bot".to_owned())),
        ];

        assert!(contains_member(&members, "AGENT-MEMBER"));
        assert!(contains_member(&members, "agent-bot"));
        assert!(!contains_member(&members, "missing"));
    }
}
