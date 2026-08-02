//! Per-listener preferences for translated huddle captions.
//!
//! Split out of `tts_settings` (which owns the `TtsSettings` struct and its
//! persistence) to keep both files under the desktop file-size ratchet.
//! Unlike voice/enable changes, neither setting here touches the TTS
//! runtime — they only affect which captions `useTtsSubscription` chooses to
//! speak client-side (see the field docs on `TtsSettings`).

use tauri::{AppHandle, State};

use crate::app_state::AppState;

use super::tts_settings::{
    current_settings, ensure_settings_writable, save_to_path, settings_path, TtsSettings,
};

fn settings_with_caption_language(
    settings: TtsSettings,
    language: &str,
) -> Result<TtsSettings, String> {
    let trimmed = language.trim();
    if trimmed.is_empty() {
        return Err("Caption language cannot be empty".to_string());
    }
    Ok(TtsSettings {
        caption_language: trimmed.to_lowercase(),
        ..settings
    })
}

fn settings_with_speak_captions(settings: TtsSettings, enabled: bool) -> TtsSettings {
    TtsSettings {
        speak_captions: enabled,
        ..settings
    }
}

/// Preferred language (lowercase ISO 639-1) for translated huddle captions.
#[tauri::command]
pub fn set_caption_language(
    language: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsSettings, String> {
    ensure_settings_writable(&state)?;
    let settings = settings_with_caption_language(current_settings(&state)?, &language)?;
    save_to_path(&settings_path(&app)?, &settings)?;
    *state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))? =
        settings.clone();
    Ok(settings)
}

/// Whether captions matching `caption_language` are spoken aloud. Captions
/// still render as text regardless — see the field doc on `TtsSettings`.
#[tauri::command]
pub fn set_speak_captions(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsSettings, String> {
    ensure_settings_writable(&state)?;
    let settings = settings_with_speak_captions(current_settings(&state)?, enabled);
    save_to_path(&settings_path(&app)?, &settings)?;
    *state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))? =
        settings.clone();
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_language_is_trimmed_and_lowercased() {
        let updated = settings_with_caption_language(TtsSettings::default(), "  ES  ")
            .expect("non-empty language");
        assert_eq!(updated.caption_language, "es");
        // Unrelated fields are preserved.
        assert_eq!(
            updated.voice_preferences,
            TtsSettings::default().voice_preferences
        );
    }

    #[test]
    fn empty_caption_language_is_rejected() {
        let error = settings_with_caption_language(TtsSettings::default(), "   ")
            .expect_err("blank language should be rejected");
        assert!(error.contains("cannot be empty"));
    }

    #[test]
    fn speak_captions_toggle_preserves_other_fields() {
        let current = TtsSettings {
            caption_language: "es".to_string(),
            ..TtsSettings::default()
        };
        let updated = settings_with_speak_captions(current, false);
        assert!(!updated.speak_captions);
        assert_eq!(updated.caption_language, "es");
    }
}
