//! Audio-only huddle reconnection after an unexpected relay disconnect.

use std::sync::atomic::Ordering;

use tauri::State;

use crate::app_state::AppState;

use super::{relay_api, HuddlePhase};

/// Re-establish only the audio relay WebSocket after an unexpected owner/pod
/// disconnect. Huddle membership, mic capture, STT/TTS, and frontend state stay
/// live, so a successful reconnect is a short audio blip rather than a leave.
///
/// The session generation and channel id are re-checked after the network dial:
/// an intentional leave/end racing this command wins and the newly-opened audio
/// pipeline is cancelled instead of resurrecting a terminal huddle.
#[tauri::command]
pub async fn reconnect_huddle_audio(state: State<'_, AppState>) -> Result<(), String> {
    let (ephemeral_channel_id, parent_channel_id, session_generation) = {
        let hs = state.huddle()?;
        if matches!(hs.phase, HuddlePhase::Idle | HuddlePhase::Leaving) {
            return Err("huddle is no longer active".into());
        }
        (
            hs.ephemeral_channel_id
                .clone()
                .ok_or("active huddle has no channel id")?,
            hs.parent_channel_id.clone(),
            hs.session_generation.load(Ordering::Acquire),
        )
    };

    let (cancel, pcm_tx) =
        relay_api::connect_audio_relay(&ephemeral_channel_id, parent_channel_id.as_deref(), &state)
            .await?;

    let mut hs = state.huddle()?;
    let still_current = !matches!(hs.phase, HuddlePhase::Idle | HuddlePhase::Leaving)
        && hs.ephemeral_channel_id.as_deref() == Some(ephemeral_channel_id.as_str())
        && hs.session_generation.load(Ordering::Acquire) == session_generation;
    if !still_current {
        cancel.cancel();
        return Err("huddle ended while audio was reconnecting".into());
    }

    if let Some(old_cancel) = hs.audio_ws_cancel.replace(cancel) {
        old_cancel.cancel();
    }
    hs.audio_relay_pcm_tx = Some(pcm_tx);
    Ok(())
}
