//! Small Huddle controls that mutate an active session.

use std::sync::{atomic::Ordering, Arc};

use tauri::State;
use uuid::Uuid;

use crate::{app_state::AppState, events, relay::submit_event};

use super::pipeline::start_auto_enabled_transcription;
use super::relay_api::MAX_HUDDLE_AGENTS;
use super::{agents, relay_api::validate_pubkey_hex, HuddlePhase};

/// Update the clickable microphone control independently from the PTT shortcut.
#[tauri::command]
pub fn set_huddle_manual_mic_unmuted(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let huddle = state.huddle()?;
    if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
        return Err("no active huddle".to_string());
    }
    huddle.manual_mic_unmuted.store(enabled, Ordering::Release);
    Ok(())
}

/// Immediately interrupt the agent utterance that is currently speaking.
#[tauri::command]
pub fn interrupt_huddle_speech(
    agent_pubkey: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_pubkey_hex(&agent_pubkey)?;
    let tts_pipeline = {
        let huddle = state.huddle()?;
        if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Err("no active huddle".to_string());
        }
        huddle.tts_pipeline.as_ref().map(Arc::clone)
    };
    if let Some(tts_pipeline) = tts_pipeline {
        tts_pipeline.cancel_active_speaker(&agent_pubkey);
    }
    Ok(())
}

/// Remove an agent from the active huddle without removing its parent-channel
/// membership. Keeping the parent membership intact means it remains available
/// to rejoin this huddle from the agent picker.
#[tauri::command]
pub async fn remove_agent_from_huddle(
    agent_pubkey: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_pubkey_hex(&agent_pubkey)?;

    let (ephemeral_channel_id, huddle_generation) = {
        let huddle = state.huddle()?;
        if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Err("no active huddle".to_string());
        }

        let is_huddle_agent = huddle
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|pubkey| pubkey.eq_ignore_ascii_case(&agent_pubkey));
        if !is_huddle_agent {
            return Err("agent is not in this huddle".to_string());
        }

        (
            huddle
                .ephemeral_channel_id
                .clone()
                .ok_or("no ephemeral channel")?,
            huddle.huddle_generation,
        )
    };

    let ephemeral_channel_uuid =
        Uuid::parse_str(&ephemeral_channel_id).map_err(|error| error.to_string())?;
    submit_event(
        events::build_remove_member(ephemeral_channel_uuid, &agent_pubkey)?,
        &state,
    )
    .await?;

    let (roster_changed, tts_pipeline) = {
        let mut huddle = state.huddle()?;
        if !huddle.is_current_huddle(&ephemeral_channel_id, huddle_generation) {
            (false, None)
        } else {
            let mut agent_pubkeys = huddle
                .agent_pubkeys
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let initial_count = agent_pubkeys.len();
            agent_pubkeys.retain(|pubkey| !pubkey.eq_ignore_ascii_case(&agent_pubkey));
            let changed = agent_pubkeys.len() != initial_count;
            drop(agent_pubkeys);

            if changed {
                huddle
                    .participants
                    .retain(|pubkey| !pubkey.eq_ignore_ascii_case(&agent_pubkey));
                if let Some(settings_pubkey) = huddle
                    .agent_voice_settings
                    .keys()
                    .find(|pubkey| pubkey.eq_ignore_ascii_case(&agent_pubkey))
                    .cloned()
                {
                    huddle.agent_voice_settings.remove(&settings_pubkey);
                }
            }
            let tts_pipeline = changed
                .then_some(huddle.tts_pipeline.as_ref())
                .flatten()
                .map(Arc::clone);
            (changed, tts_pipeline)
        }
    };

    if let Some(tts_pipeline) = tts_pipeline {
        tts_pipeline.cancel_speaker(&agent_pubkey);
    }
    if roster_changed {
        state.emit_huddle_state_changed();
    }

    Ok(())
}

/// Add an agent to the active huddle.
///
/// Steps:
/// 1. Validates the huddle is in the Connected or Active phase.
/// 2. Adds the agent to both the ephemeral and parent channels (kind:9000).
/// 3. Only appends the agent pubkey to `agent_pubkeys` if the ephemeral add
///    succeeded — failed adds (policy rejection) are NOT p-tagged.
///
/// Returns a structured `AgentAddResult` so the frontend can surface
/// parent-channel errors without treating them as hard failures.
///
/// The running ACP process for this agent auto-subscribes when it receives
/// the kind:9000 membership notification — no separate process spawn needed.
#[tauri::command]
pub async fn add_agent_to_huddle(
    agent_pubkey: String,
    state: State<'_, AppState>,
) -> Result<agents::AgentAddResult, String> {
    validate_pubkey_hex(&agent_pubkey)?;

    let (eph_id, parent_id, huddle_generation) = {
        let hs = state.huddle()?;
        if !matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Err("no active huddle".to_string());
        }

        // Enforce agent cap on incremental adds too.
        let current_agent_count = hs
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        if current_agent_count >= MAX_HUDDLE_AGENTS {
            return Err(format!(
                "agent limit reached: {} (max {})",
                current_agent_count, MAX_HUDDLE_AGENTS
            ));
        }

        let eph = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no ephemeral channel")?;
        let parent = hs.parent_channel_id.clone().ok_or("no parent channel")?;
        (eph, parent, hs.huddle_generation)
    };

    let eph_uuid = Uuid::parse_str(&eph_id).map_err(|e| e.to_string())?;
    let parent_uuid = Uuid::parse_str(&parent_id).map_err(|e| e.to_string())?;

    // Returns Err only if the ephemeral add fails — parent failure is in the result.
    let result = agents::add_agent_to_huddle(eph_uuid, parent_uuid, &agent_pubkey, &state).await?;

    // Ephemeral add succeeded — register it only if this is still the huddle
    // that initiated the relay operation.
    let transcription_auto_enabled = {
        let mut hs = state.huddle()?;
        if !hs.is_current_huddle(&eph_id, huddle_generation) {
            return Ok(result);
        }
        let mut pubkeys = hs.agent_pubkeys.lock().unwrap_or_else(|e| e.into_inner());
        if !pubkeys.contains(&agent_pubkey) {
            pubkeys.push(agent_pubkey.clone());
        }
        drop(pubkeys);
        if !hs.participants.contains(&agent_pubkey) {
            hs.participants.push(agent_pubkey.clone());
        }
        hs.maybe_auto_enable_transcription_for_agents()
    };

    // No guidelines re-post needed — the agent sees the original kind:48106
    // guidelines via EOSE replay when it subscribes to the ephemeral channel.
    if transcription_auto_enabled {
        start_auto_enabled_transcription(&state, &eph_id).await;
    } else {
        state.emit_huddle_state_changed();
    }

    Ok(result)
}
