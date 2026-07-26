use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;

use super::{pipeline::maybe_start_tts_pipeline, siri_tts, HuddlePhase};

const SETTINGS_FILE: &str = "tts-settings.json";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsBackend {
    #[default]
    Pocket,
    Siri,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TtsSettings {
    pub backend: TtsBackend,
    pub siri_voice: Option<String>,
    pub siri_language: Option<String>,
    pub siri_rate: f32,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            backend: TtsBackend::Pocket,
            siri_voice: None,
            siri_language: None,
            siri_rate: 1.0,
        }
    }
}

impl TtsSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.5..=2.0).contains(&self.siri_rate) {
            return Err("Siri speech rate must be between 0.5 and 2.0".into());
        }
        if self.backend == TtsBackend::Siri {
            let name = self
                .siri_voice
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Choose a downloaded Siri voice first".to_string())?;
            let language = self
                .siri_language
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "The Siri voice language is missing".to_string())?;
            if !siri_tts::is_voice_installed(name, language)? {
                return Err(format!(
                    "Siri voice '{name}' ({language}) is not downloaded and usable"
                ));
            }
        }
        Ok(())
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("could not resolve TTS settings directory: {error}"))
}

pub fn load_settings(app: &AppHandle) -> TtsSettings {
    let Ok(path) = settings_path(app) else {
        return TtsSettings::default();
    };
    load_settings_from(&path).unwrap_or_else(|error| {
        eprintln!("buzz-desktop: could not load TTS settings: {error}");
        TtsSettings::default()
    })
}

fn load_settings_from(path: &Path) -> Result<TtsSettings, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TtsSettings::default()),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}

fn save_settings(app: &AppHandle, settings: &TtsSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("serialize TTS settings: {error}"))?;
    crate::managed_agents::storage::atomic_write_json(&path, &payload)
}

#[tauri::command]
pub fn get_tts_settings(state: State<'_, AppState>) -> Result<TtsSettings, String> {
    state
        .tts_settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_siri_tts_voices(
    language_prefix: Option<String>,
) -> Result<Vec<siri_tts::SiriVoice>, String> {
    let prefix = language_prefix.unwrap_or_else(|| "en".into());
    tokio::task::spawn_blocking(move || siri_tts::list_voices(&prefix))
        .await
        .map_err(|error| format!("Siri voice discovery task failed: {error}"))?
}

#[tauri::command]
pub async fn download_siri_tts_voice(
    name: String,
    language: String,
) -> Result<siri_tts::SiriVoice, String> {
    let trigger_name = name.clone();
    let trigger_language = language.clone();
    tokio::task::spawn_blocking(move || {
        siri_tts::trigger_download(&trigger_name, &trigger_language)
    })
    .await
    .map_err(|error| format!("Siri download task failed: {error}"))??;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    while tokio::time::Instant::now() < deadline {
        let check_name = name.clone();
        let check_language = language.clone();
        if tokio::task::spawn_blocking(move || {
            siri_tts::is_voice_installed(&check_name, &check_language)
        })
        .await
        .map_err(|error| format!("Siri validation task failed: {error}"))??
        {
            let catalog_language = language.clone();
            let voices =
                tokio::task::spawn_blocking(move || siri_tts::list_voices(&catalog_language))
                    .await
                    .map_err(|error| format!("Siri catalog task failed: {error}"))??;
            return voices
                .into_iter()
                .find(|voice| {
                    voice.name.eq_ignore_ascii_case(&name)
                        && voice
                            .language
                            .replace('_', "-")
                            .eq_ignore_ascii_case(&language.replace('_', "-"))
                })
                .ok_or_else(|| "Downloaded Siri voice disappeared from the catalog".into());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(format!(
        "Timed out waiting for Siri voice '{name}' ({language}) to finish downloading"
    ))
}

#[tauri::command]
pub async fn set_tts_settings(
    settings: TtsSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let candidate = settings.clone();
    tokio::task::spawn_blocking(move || candidate.validate())
        .await
        .map_err(|error| format!("TTS settings validation task failed: {error}"))??;
    save_settings(&app, &settings)?;

    let (old_pipeline, should_restart) = {
        let mut guard = state
            .tts_settings
            .lock()
            .map_err(|error| error.to_string())?;
        *guard = settings;
        drop(guard);

        let mut huddle = state.huddle()?;
        let should_restart = huddle.tts_enabled
            && matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active);
        (huddle.tts_pipeline.take(), should_restart)
    };
    if let Some(pipeline) = old_pipeline {
        pipeline.shutdown();
        drop(pipeline);
    }
    if should_restart {
        maybe_start_tts_pipeline(&state).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_pocket() {
        assert_eq!(TtsSettings::default().backend, TtsBackend::Pocket);
    }

    #[test]
    fn rejects_invalid_rate_before_voice_lookup() {
        let settings = TtsSettings {
            backend: TtsBackend::Pocket,
            siri_rate: 2.1,
            ..TtsSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn loads_missing_file_as_default() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            load_settings_from(&directory.path().join("missing.json")).unwrap(),
            TtsSettings::default()
        );
    }
}
