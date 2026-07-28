//! User-selectable Huddle speech settings.

use tauri::State;

use crate::app_state::AppState;

use super::{pipeline::maybe_start_stt_pipeline, HuddlePhase, TranscriptionLanguage};

/// Set the language used by the local multilingual Whisper recognizer.
///
/// The recognizer captures its language at construction time, so changing the
/// selection during an active transcribed huddle restarts only the STT pipeline.
#[tauri::command]
pub async fn set_transcription_language(
    language: TranscriptionLanguage,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let needs_restart = {
        let mut hs = state.huddle()?;
        let old_language = hs.transcription_language;
        hs.transcription_language = language;
        old_language != language
            && matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
            && hs.stt_pipeline.is_some()
            && hs.transcription_enabled
    };

    let restart_result = if needs_restart {
        let eph_id = {
            let hs = state.huddle()?;
            hs.ephemeral_channel_id.clone()
        };
        if let Some(eph_id) = eph_id {
            maybe_start_stt_pipeline(&state, &eph_id)
                .await
                .map(|_| ())
                .map_err(|e| format!("restart transcription for language change: {e}"))
        } else {
            Ok(())
        }
    } else {
        Ok(())
    };

    state.emit_huddle_state_changed();
    restart_result
}

/// Return the language used by the local multilingual Whisper recognizer.
#[tauri::command]
pub fn get_transcription_language(
    state: State<'_, AppState>,
) -> Result<TranscriptionLanguage, String> {
    let hs = state.huddle()?;
    Ok(hs.transcription_language)
}
