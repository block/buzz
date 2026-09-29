use tauri::State;

use crate::app_state::AppState;

use super::{models, pipeline::maybe_start_stt_pipeline};

/// Start the STT pipeline for the active huddle.
///
/// Delegates to `maybe_start_stt_pipeline` — returns `Err` if models are not
/// ready or no huddle is active. Safe to call multiple times: replaces the
/// existing pipeline if already running.
#[tauri::command]
pub async fn start_stt_pipeline(state: State<'_, AppState>) -> Result<(), String> {
    let ephemeral_channel_id = {
        let mut hs = state.huddle()?;
        let ephemeral_channel_id = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no active huddle — start or join a huddle first")?;
        hs.set_transcription_enabled_by_user(true);
        ephemeral_channel_id
    };

    match maybe_start_stt_pipeline(&state, &ephemeral_channel_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err("STT model not ready".to_string()),
        Err(e) => Err(e),
    }
}

/// Enable or disable huddle transcript posting.
///
/// Disabling tears down STT immediately and invalidates any in-flight transcript
/// task before it can post another segment. Enabling starts STT if models are
/// ready; otherwise the hot-start loop will begin transcribing once the model
/// download finishes.
#[tauri::command]
pub async fn set_huddle_transcription_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (ephemeral_channel_id, old_stt) = {
        let mut hs = state.huddle()?;
        let ephemeral_channel_id = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no active huddle — start or join a huddle first")?;
        hs.set_transcription_enabled_by_user(enabled);

        if enabled {
            (ephemeral_channel_id, None)
        } else {
            hs.invalidate_transcription_pipeline();
            (ephemeral_channel_id, hs.take_stt_pipeline())
        }
    };

    if let Some(ref pipeline) = old_stt {
        pipeline.shutdown();
    }
    drop(old_stt);

    if enabled {
        if let Some(manager) = models::global_model_manager() {
            manager.start_stt_download(state.http_client.clone());
        }
        if let Err(e) = maybe_start_stt_pipeline(&state, &ephemeral_channel_id).await {
            eprintln!("buzz-desktop: STT transcript start failed: {e}");
        }
    }

    state.emit_huddle_state_changed();
    Ok(())
}

/// Preset activation keywords for the huddle STT wake dropdown.
#[tauri::command]
pub fn list_huddle_activation_keywords() -> Vec<String> {
    super::stt_wake::ACTIVATION_KEYWORD_PRESETS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Current spoken activation keyword for the active huddle (defaults to "hey").
#[tauri::command]
pub fn get_huddle_activation_keyword(state: State<'_, AppState>) -> Result<String, String> {
    let hs = state.huddle()?;
    let keyword = hs
        .activation_keyword
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(keyword)
}

/// Set the spoken activation keyword used by STT wake matching.
///
/// Accepts only the short preset allow-list (hey / at / agent / bot / robo /
/// ok / yo / okay). Updates the live Arc so an in-flight transcription task
/// picks up the new keyword on the next STT final.
#[tauri::command]
pub fn set_huddle_activation_keyword(
    keyword: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let normalized = super::stt_wake::normalize_activation_keyword(&keyword).ok_or_else(|| {
        format!(
            "unsupported activation keyword {keyword:?}; choose one of: {}",
            super::stt_wake::ACTIVATION_KEYWORD_PRESETS.join(", ")
        )
    })?;
    {
        let hs = state.huddle()?;
        let mut guard = hs
            .activation_keyword
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = normalized.clone();
    }
    state.emit_huddle_state_changed();
    Ok(normalized)
}
