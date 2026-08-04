//! Small Huddle controls that mutate an active session.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::State;
use uuid::Uuid;

use crate::{app_state::AppState, events, relay::submit_event};

use super::{relay_api::validate_pubkey_hex, HuddlePhase};

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

/// Immediately interrupt the agent's current speech and discard queued TTS.
///
/// This shares the barge-in cancellation path used when a participant starts
/// talking, so it stops both audio already handed to the player and response
/// text waiting to be synthesized.
#[tauri::command]
pub fn interrupt_huddle_speech(state: State<'_, AppState>) -> Result<(), String> {
    let (active, cancel) = {
        let huddle = state.huddle()?;
        if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Err("no active huddle".to_string());
        }
        (huddle.tts_active.clone(), huddle.tts_cancel.clone())
    };
    request_speech_interrupt(&active, &cancel);
    Ok(())
}

fn request_speech_interrupt(active: &AtomicBool, cancel: &AtomicBool) -> bool {
    if !active.load(Ordering::Acquire) {
        return false;
    }
    cancel.store(true, Ordering::Release);
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_stop_does_not_cancel_the_next_utterance() {
        let active = AtomicBool::new(false);
        let cancel = AtomicBool::new(false);

        assert!(!request_speech_interrupt(&active, &cancel));
        active.store(true, Ordering::Release);

        assert!(!cancel.load(Ordering::Acquire));
    }

    #[test]
    fn stop_cancels_active_speech() {
        let active = AtomicBool::new(true);
        let cancel = AtomicBool::new(false);

        assert!(request_speech_interrupt(&active, &cancel));
        assert!(cancel.load(Ordering::Acquire));
    }
}
