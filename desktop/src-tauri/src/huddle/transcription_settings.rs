use tauri::{AppHandle, State};

use crate::app_state::AppState;

use super::tts_settings::{
    current_settings, ensure_settings_writable, save_to_path, settings_path, TtsSettings,
};

pub(crate) fn normalize_transcription_language(
    language: Option<&str>,
) -> Result<Option<String>, String> {
    let language = language.unwrap_or_default().trim().to_ascii_lowercase();
    if language.is_empty() || language == "auto" {
        return Ok(None);
    }
    if language.len() == 2 && language.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Ok(Some(language));
    }
    Err("Transcription language must be Auto or a two-letter language code".to_string())
}

#[tauri::command]
pub async fn set_transcription_language(
    language: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsSettings, String> {
    let transition = state.huddle_audio.tts_transition.lock().await;
    ensure_settings_writable(&state)?;
    let mut settings = current_settings(&state)?;
    settings.transcription_language = normalize_transcription_language(language.as_deref())?;
    save_to_path(&settings_path(&app)?, &settings)?;
    *state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))? =
        settings.clone();
    drop(transition);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::huddle::tts_settings::load_from_path;

    #[test]
    fn accepts_auto_and_iso_639_1_codes() {
        assert_eq!(normalize_transcription_language(None).unwrap(), None);
        assert_eq!(normalize_transcription_language(Some("")).unwrap(), None);
        assert_eq!(
            normalize_transcription_language(Some("AUTO")).unwrap(),
            None
        );
        assert_eq!(
            normalize_transcription_language(Some("TR")).unwrap(),
            Some("tr".to_string())
        );
        assert!(normalize_transcription_language(Some("tur"))
            .expect_err("three-letter language code should fail")
            .contains("two-letter"));
    }

    #[test]
    fn existing_settings_without_language_load_as_auto() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tts-settings.json");
        std::fs::write(
            &path,
            r#"{"version":1,"agentTextToSpeech":true,"voicePreferences":["pocket:mary"]}"#,
        )
        .expect("fixture write");

        assert_eq!(
            load_from_path(&path)
                .expect("legacy settings")
                .transcription_language,
            None
        );
    }
}
