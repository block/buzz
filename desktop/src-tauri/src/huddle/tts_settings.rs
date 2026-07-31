//! Installation-global text-to-speech preferences and the local voice registry.
//!
//! Voice keys are backend-qualified (`pocket:mary`, `siri:aaron`) and
//! preferences are ordered. A client resolves the first compatible entry for
//! its one active playback backend. The same [`VoicePreferences`] value can be
//! embedded in installation-global settings or future agent identity without a
//! schema change. Availability is intentionally client-local.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{app_state::AppState, managed_agents::storage::atomic_write_json_restricted};

use super::{
    models,
    pocket::DEFAULT_VOICE,
    tts_voice_registry::{source_url, MARY_VOICE_KEY, POCKET_VOICES},
    HuddlePhase, HuddleState,
};

const SETTINGS_FILE: &str = "tts-settings.json";
const CURRENT_VERSION: u32 = 1;
const VOICE_CHANGE_ACK_TIMEOUT: Duration = Duration::from_secs(5);
pub const POCKET_BACKEND_ID: &str = "pocket";

type VoiceChangeWait = (
    Arc<super::tts::TtsPipeline>,
    tokio::sync::oneshot::Receiver<()>,
);

const VOICE_AVAILABILITY_BUNDLED: &str = "bundled";
const VOICE_AVAILABILITY_INSTALLED: &str = "installed";

/// Installation-global huddle audio and speech preferences.
#[derive(Default)]
pub struct HuddleAudioSettingsState {
    pub tts: Mutex<TtsSettings>,
    pub tts_load_error: Mutex<Option<String>>,
    pub tts_transition: tokio::sync::Mutex<()>,
    /// Selected huddle output device. `None` uses the system default.
    pub output_device: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRegistryEntry {
    /// Stable identity, never derived from or merged by the display name.
    ///
    /// Built-ins use `backend:slug`. Future imports use
    /// `pocket:imported:<audio-content-sha256>` so two clips with the same
    /// editable label remain distinct.
    pub key: String,
    pub display_name: String,
    pub backend: String,
    pub backend_name: String,
    /// Client-local state: bundled, installed, downloadable, or unavailable.
    pub availability: String,
    pub fallback_key: Option<String>,
    pub reference_file: Option<String>,
    pub provenance: VoiceProvenance,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProvenance {
    pub source: String,
    pub content_hash: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
}

/// Ordered, backend-qualified preferences shared by global and agent settings.
///
/// Unknown but well-formed keys remain persisted because a different client
/// may have that backend installed. Resolution is always local.
pub type VoicePreferences = Vec<String>;

/// Bundled Pocket voices available without local imports.
pub fn bundled_voice_registry() -> Vec<VoiceRegistryEntry> {
    POCKET_VOICES
        .iter()
        .map(|voice| VoiceRegistryEntry {
            key: voice.key.to_string(),
            display_name: voice.display_name.to_string(),
            backend: POCKET_BACKEND_ID.to_string(),
            backend_name: "Pocket TTS".to_string(),
            availability: VOICE_AVAILABILITY_BUNDLED.to_string(),
            fallback_key: (voice.key != MARY_VOICE_KEY).then(|| MARY_VOICE_KEY.to_string()),
            reference_file: Some(voice.reference_file.to_string()),
            provenance: VoiceProvenance {
                source: "bundled".to_string(),
                content_hash: Some(voice.sha256.to_string()),
                license: Some("CC-BY-4.0".to_string()),
                source_url: Some(source_url(voice)),
            },
        })
        .collect()
}

/// Cross-backend registry of bundled and locally installed voices.
pub fn voice_registry(app: &AppHandle) -> Vec<VoiceRegistryEntry> {
    let mut registry = bundled_voice_registry();
    match super::tts_voice_import::load_registry(app) {
        Ok(imported) => registry.extend(imported.into_iter().map(|voice| VoiceRegistryEntry {
            key: voice.key,
            display_name: voice.display_name,
            backend: POCKET_BACKEND_ID.to_string(),
            backend_name: "Pocket TTS".to_string(),
            availability: VOICE_AVAILABILITY_INSTALLED.to_string(),
            fallback_key: Some(MARY_VOICE_KEY.to_string()),
            reference_file: Some(voice.file_name),
            provenance: VoiceProvenance {
                source: "local import".to_string(),
                content_hash: Some(voice.content_hash),
                license: None,
                source_url: None,
            },
        })),
        Err(error) => {
            eprintln!(
                "buzz-desktop: {error}; imported Pocket voices are unavailable for this session"
            );
        }
    }
    registry
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettings {
    pub version: u32,
    pub agent_text_to_speech: bool,
    pub voice_preferences: VoicePreferences,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            agent_text_to_speech: true,
            voice_preferences: vec![MARY_VOICE_KEY.to_string()],
        }
    }
}

pub fn voice_by_key(app: &AppHandle, key: &str) -> Option<VoiceRegistryEntry> {
    voice_registry(app)
        .into_iter()
        .find(|voice| voice.key == key)
}

fn is_qualified_voice_key(key: &str) -> bool {
    key.split_once(':')
        .is_some_and(|(backend, voice)| !backend.is_empty() && !voice.is_empty())
}

fn is_locally_available(availability: &str) -> bool {
    matches!(
        availability,
        VOICE_AVAILABILITY_BUNDLED | VOICE_AVAILABILITY_INSTALLED
    )
}

#[cfg(test)]
pub fn resolve_voice_for_backend(
    preferences: &[String],
    backend: &str,
) -> Result<VoiceRegistryEntry, String> {
    resolve_voice_for_backend_in_registry(preferences, backend, &bundled_voice_registry())
}

fn resolve_voice_for_backend_in_registry(
    preferences: &[String],
    backend: &str,
    registry: &[VoiceRegistryEntry],
) -> Result<VoiceRegistryEntry, String> {
    preferences
        .iter()
        .filter_map(|key| registry.iter().find(|voice| voice.key == *key))
        .find(|voice| voice.backend == backend && is_locally_available(voice.availability.as_str()))
        .or_else(|| {
            registry.iter().find(|voice| {
                voice.backend == backend
                    && voice.fallback_key.is_none()
                    && is_locally_available(voice.availability.as_str())
            })
        })
        .cloned()
        .ok_or_else(|| format!("No locally available fallback voice for backend {backend}"))
}

pub fn bundled_pocket_voice_reference(preferences: &[String]) -> String {
    resolve_voice_for_backend_in_registry(preferences, POCKET_BACKEND_ID, &bundled_voice_registry())
        .ok()
        .and_then(|voice| voice.reference_file)
        .and_then(|file| file.strip_suffix(".wav").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_VOICE.to_string())
}

pub fn pocket_voice_reference(app: &AppHandle, preferences: &[String]) -> Result<String, String> {
    let registry = voice_registry(app);
    let voice = resolve_voice_for_backend_in_registry(preferences, POCKET_BACKEND_ID, &registry)?;
    if voice.key.starts_with("pocket:imported:") {
        let imported = super::tts_voice_import::load_registry(app)?
            .into_iter()
            .find(|candidate| candidate.key == voice.key)
            .ok_or_else(|| format!("Imported voice {} is unavailable", voice.display_name))?;
        return super::tts_voice_import::resolve_file(app, &imported)
            .map(|path| path.to_string_lossy().into_owned());
    }
    Ok(voice
        .reference_file
        .and_then(|file| file.strip_suffix(".wav").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_VOICE.to_string()))
}

pub(crate) fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(SETTINGS_FILE))
        .map_err(|error| format!("could not locate Buzz settings storage: {error}"))
}

pub(crate) fn load_from_path(path: &Path) -> Result<TtsSettings, String> {
    if !path.exists() {
        return Ok(TtsSettings::default());
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read text-to-speech settings: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("text-to-speech settings are not valid JSON: {error}"))?;

    // Unversioned settings are incompatible with the V1 schema. Use
    // deterministic V1 defaults rather than interpreting ambiguous fields.
    if value.get("version").is_none() {
        return Ok(TtsSettings::default());
    }

    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("text-to-speech settings version is invalid")?;
    if version > u64::from(CURRENT_VERSION) {
        return Err(format!(
            "text-to-speech settings version {version} is newer than this Buzz build supports"
        ));
    }

    // Legacy V1 settings may contain one bare Pocket `voiceId`. Preserve the
    // toggle and qualify it into the ordered cross-backend preference schema.
    if value.get("voicePreferences").is_none() {
        let legacy_voice = value
            .get("voiceId")
            .or_else(|| value.get("voice_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("mary");
        let voice_key = if is_qualified_voice_key(legacy_voice) {
            legacy_voice.to_string()
        } else {
            format!("{POCKET_BACKEND_ID}:{legacy_voice}")
        };
        return Ok(TtsSettings {
            version: CURRENT_VERSION,
            agent_text_to_speech: value
                .get("agentTextToSpeech")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            voice_preferences: vec![voice_key],
        });
    }

    let mut settings: TtsSettings = serde_json::from_value(value)
        .map_err(|error| format!("text-to-speech settings are invalid: {error}"))?;
    settings.version = CURRENT_VERSION;
    if settings.voice_preferences.is_empty()
        || settings
            .voice_preferences
            .iter()
            .any(|key| !is_qualified_voice_key(key))
    {
        settings.voice_preferences = TtsSettings::default().voice_preferences;
    }
    Ok(settings)
}

pub(crate) fn save_to_path(path: &Path, settings: &TtsSettings) -> Result<(), String> {
    if settings.voice_preferences.is_empty() {
        return Err("At least one voice preference is required".to_string());
    }
    if let Some(key) = settings
        .voice_preferences
        .iter()
        .find(|key| !is_qualified_voice_key(key))
    {
        return Err(format!(
            "Voice preference keys must be backend-qualified: {key}"
        ));
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not encode text-to-speech settings: {error}"))?;
    atomic_write_json_restricted(path, &payload)
        .map_err(|error| format!("could not save text-to-speech settings: {error}"))
}

pub fn load_for_app(app: &AppHandle) -> (TtsSettings, Option<String>) {
    let result = settings_path(app).and_then(|path| load_from_path(&path));
    match result {
        Ok(settings) => (settings, None),
        Err(error) => {
            eprintln!("buzz-desktop: {error}; preserving the file and using Mary for this session");
            (TtsSettings::default(), Some(error))
        }
    }
}

#[tauri::command]
pub fn get_tts_settings(state: State<'_, AppState>) -> Result<TtsSettings, String> {
    if let Some(error) = state
        .huddle_audio
        .tts_load_error
        .lock()
        .map_err(|lock_error| format!("text-to-speech settings lock poisoned: {lock_error}"))?
        .clone()
    {
        return Err(format!(
            "Voice settings could not be loaded and were left unchanged: {error}"
        ));
    }
    state
        .huddle_audio
        .tts
        .lock()
        .map(|settings| settings.clone())
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))
}

#[tauri::command]
pub fn list_voice_registry(app: AppHandle) -> Vec<VoiceRegistryEntry> {
    voice_registry(&app)
}

fn ensure_settings_writable(state: &AppState) -> Result<(), String> {
    if let Some(error) = state
        .huddle_audio
        .tts_load_error
        .lock()
        .map_err(|lock_error| format!("text-to-speech settings lock poisoned: {lock_error}"))?
        .as_ref()
    {
        return Err(format!(
            "Voice settings were not saved because the existing file could not be loaded: {error}"
        ));
    }
    Ok(())
}

fn cancel_huddle_speech(
    huddle: &mut super::HuddleState,
) -> Option<std::sync::Arc<super::tts::TtsPipeline>> {
    huddle.tts_enabled = false;
    huddle
        .tts_cancel
        .store(true, std::sync::atomic::Ordering::Release);
    huddle.tts_pipeline.take()
}

fn disable_tts_runtime(state: &AppState) -> Result<(), String> {
    let old_pipeline = {
        let mut huddle = state.huddle()?;
        cancel_huddle_speech(&mut huddle)
    };
    if let Some(ref pipeline) = old_pipeline {
        pipeline.shutdown();
    }
    drop(old_pipeline);
    state.emit_huddle_state_changed();
    Ok(())
}

fn commit_effective_off(state: &AppState) -> Result<(), String> {
    state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))?
        .agent_text_to_speech = false;
    Ok(())
}

fn enable_tts_runtime(huddle: &mut HuddleState, voice: &str) -> Option<VoiceChangeWait> {
    huddle.tts_enabled = true;
    // OFF removes the pipeline. Clear a prior cancellation only when enabling
    // a fresh pipeline; an idempotent ON write must not erase a voice
    // transition that the existing worker still needs to drain.
    prepare_enable_cancel(&huddle.tts_cancel, huddle.tts_pipeline.is_some());
    huddle.tts_pipeline.as_ref().and_then(|pipeline| {
        pipeline
            .select_voice(voice)
            .map(|acknowledged| (Arc::clone(pipeline), acknowledged))
    })
}

fn prepare_enable_cancel(cancel: &std::sync::atomic::AtomicBool, has_pipeline: bool) {
    if !has_pipeline {
        cancel.store(false, std::sync::atomic::Ordering::Release);
    }
}

async fn apply_tts_settings(
    settings: TtsSettings,
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<VoiceChangeWait>, String> {
    if settings.version != CURRENT_VERSION {
        return Err(format!(
            "Unsupported text-to-speech settings version: {}",
            settings.version
        ));
    }

    // OFF is safety-sensitive: stop current and queued speech before any disk
    // I/O, and never resume it merely because persistence fails.
    if !settings.agent_text_to_speech {
        disable_tts_runtime(state)?;
        commit_effective_off(state)?;
    }

    ensure_settings_writable(state)?;
    save_to_path(&settings_path(app)?, &settings)?;

    *state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))? =
        settings.clone();

    let mut voice_change_wait = None;
    if settings.agent_text_to_speech {
        let (active, voice_change_ack) = {
            let mut huddle = state.huddle()?;
            let voice_reference = pocket_voice_reference(app, &settings.voice_preferences)?;
            let voice_change_ack = enable_tts_runtime(&mut huddle, &voice_reference);
            (
                matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active),
                voice_change_ack,
            )
        };
        voice_change_wait = voice_change_ack;
        if active {
            if let Err(error) = super::pipeline::maybe_start_tts_pipeline(state).await {
                eprintln!("buzz-desktop: could not hot-start text to speech: {error}");
            }
        }
        state.emit_huddle_state_changed();
    }
    Ok(voice_change_wait)
}

fn current_settings(state: &AppState) -> Result<TtsSettings, String> {
    state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))
        .map(|settings| settings.clone())
}

async fn finish_voice_change(voice_change: Option<VoiceChangeWait>) -> Result<(), String> {
    let Some((pipeline, acknowledged)) = voice_change else {
        return Ok(());
    };
    wait_for_voice_change_ack(acknowledged, VOICE_CHANGE_ACK_TIMEOUT, || {
        pipeline.is_finished()
    })
    .await
}

async fn finish_durable_voice_change(voice_change: Option<VoiceChangeWait>) {
    if let Err(error) = finish_voice_change(voice_change).await {
        eprintln!(
            "buzz-desktop: tts stage=voice_switch status=delayed reason=ack_timeout error={error}"
        );
    }
}

async fn wait_for_voice_change_ack(
    mut acknowledged: tokio::sync::oneshot::Receiver<()>,
    timeout: Duration,
    mut worker_is_finished: impl FnMut() -> bool,
) -> Result<(), String> {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut acknowledged => return Ok(()),
            _ = &mut deadline => {
                return Err(
                    "Pocket TTS is still finishing the previous voice. Turn Agent text to speech off and try again."
                        .to_string(),
                );
            }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {
                if worker_is_finished() {
                    return Ok(());
                }
            }
        }
    }
}

/// Compatibility command for the huddle speaker button. It updates the same
/// installation-global preference as Settings; there is no per-huddle override.
#[tauri::command]
pub async fn set_tts_enabled(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsSettings, String> {
    let transition = state.huddle_audio.tts_transition.lock().await;
    let mut settings = state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))?
        .clone();
    settings.agent_text_to_speech = enabled;
    let voice_change = apply_tts_settings(settings, &app, &state).await?;
    drop(transition);
    finish_voice_change(voice_change).await?;
    current_settings(&state)
}

fn settings_with_pocket_voice(
    settings: TtsSettings,
    voice_key: &str,
    app: &AppHandle,
) -> Result<TtsSettings, String> {
    settings_with_pocket_voice_from_registry(settings, voice_key, &voice_registry(app))
}

fn settings_with_pocket_voice_from_registry(
    mut settings: TtsSettings,
    voice_key: &str,
    registry: &[VoiceRegistryEntry],
) -> Result<TtsSettings, String> {
    let voice = registry
        .iter()
        .find(|voice| voice.key == voice_key)
        .ok_or_else(|| format!("Unknown voice: {voice_key}"))?;
    if voice.backend != POCKET_BACKEND_ID || !is_locally_available(&voice.availability) {
        return Err("The selected Pocket voice is not available on this device".to_string());
    }
    let first_pocket_index = settings
        .voice_preferences
        .iter()
        .position(|key| key.starts_with("pocket:"));
    settings
        .voice_preferences
        .retain(|key| !key.starts_with("pocket:"));
    let insert_at = first_pocket_index
        .unwrap_or(settings.voice_preferences.len())
        .min(settings.voice_preferences.len());
    settings
        .voice_preferences
        .insert(insert_at, voice_key.to_string());
    Ok(settings)
}

#[tauri::command]
pub async fn set_pocket_voice(
    voice_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsSettings, String> {
    let transition = state.huddle_audio.tts_transition.lock().await;
    let settings = state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))?
        .clone();
    let settings = settings_with_pocket_voice(settings, &voice_key, &app)?;
    let voice_change = apply_tts_settings(settings, &app, &state).await?;
    drop(transition);
    finish_durable_voice_change(voice_change).await;
    current_settings(&state)
}

#[tauri::command]
pub async fn preview_pocket_voice(
    voice_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let voice =
        voice_by_key(&app, &voice_key).ok_or_else(|| format!("Unknown voice: {voice_key}"))?;
    if voice.backend != POCKET_BACKEND_ID {
        return Err("Only Pocket voices can be previewed in this build".to_string());
    }
    if !models::is_tts_ready() {
        return Err("Voice files are still downloading. Try preview again shortly.".to_string());
    }
    let model_dir = models::tts_model_dir().ok_or("Pocket voice files are unavailable")?;
    let output_device = state
        .huddle_audio
        .output_device
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let voice_name = pocket_voice_reference(&app, std::slice::from_ref(&voice_key))?;
    tokio::task::spawn_blocking(move || {
        let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pipeline = super::tts::TtsPipeline::new_with_voice(
            model_dir,
            active.clone(),
            cancel,
            &voice_name,
            output_device,
        )?;
        pipeline.speak("Hello! This is how I’ll read agent responses.".to_string())?;
        let started = std::time::Instant::now();
        let mut heard_audio = false;
        while started.elapsed() < std::time::Duration::from_secs(30) {
            let is_active = active.load(std::sync::atomic::Ordering::Acquire);
            heard_audio |= is_active;
            if heard_audio && !is_active {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        Err("Voice preview timed out. Check your audio output and try again.".to_string())
    })
    .await
    .map_err(|error| format!("Voice preview task failed: {error}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceMutation {
    pub settings: TtsSettings,
    pub registry: Vec<VoiceRegistryEntry>,
}

#[tauri::command]
pub async fn import_pocket_voice(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<TtsVoiceMutation>, String> {
    let Some(imported) = super::tts_voice_import::pick_and_import(&app).await? else {
        return Ok(None);
    };
    let transition = state.huddle_audio.tts_transition.lock().await;
    let settings = current_settings(&state)?;
    let settings = settings_with_pocket_voice(settings, &imported.key, &app)?;
    let voice_change = apply_tts_settings(settings, &app, &state).await?;
    drop(transition);
    finish_durable_voice_change(voice_change).await;
    Ok(Some(TtsVoiceMutation {
        settings: current_settings(&state)?,
        registry: voice_registry(&app),
    }))
}

#[tauri::command]
pub async fn delete_pocket_voice(
    voice_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TtsVoiceMutation, String> {
    if !voice_key.starts_with("pocket:imported:") {
        return Err("Bundled voices cannot be deleted".to_string());
    }
    if voice_by_key(&app, &voice_key).is_none() {
        return Err(format!("Unknown imported voice: {voice_key}"));
    }

    let transition = state.huddle_audio.tts_transition.lock().await;
    let current = current_settings(&state)?;
    let selected = resolve_voice_for_backend_in_registry(
        &current.voice_preferences,
        POCKET_BACKEND_ID,
        &voice_registry(&app),
    )
    .is_ok_and(|voice| voice.key == voice_key);
    let voice_change = if selected {
        let fallback = settings_with_pocket_voice(current, MARY_VOICE_KEY, &app)?;
        apply_tts_settings(fallback, &app, &state).await?
    } else {
        None
    };
    drop(transition);
    finish_durable_voice_change(voice_change).await;
    super::tts_voice_import::delete(&app, &voice_key)?;
    Ok(TtsVoiceMutation {
        settings: current_settings(&state)?,
        registry: voice_registry(&app),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const EVE_VOICE_KEY: &str = "pocket:eve";

    #[tokio::test]
    async fn stalled_voice_change_returns_an_actionable_error() {
        let (_keep_pending, acknowledged) = tokio::sync::oneshot::channel();

        let error = wait_for_voice_change_ack(acknowledged, Duration::from_millis(1), || false)
            .await
            .expect_err("stalled worker should time out");

        assert!(error.contains("Turn Agent text to speech off"));
    }

    #[test]
    fn idempotent_enable_preserves_an_existing_pipeline_cancel() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        prepare_enable_cancel(&cancel, true);
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
        prepare_enable_cancel(&cancel, false);
        assert!(!cancel.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn defaults_are_backwards_compatible_and_use_mary() {
        assert_eq!(
            TtsSettings::default(),
            TtsSettings {
                version: 1,
                agent_text_to_speech: true,
                voice_preferences: vec!["pocket:mary".to_string()],
            }
        );
    }

    #[test]
    fn registry_has_all_official_english_vctk_presets() {
        assert_eq!(
            bundled_voice_registry()
                .iter()
                .map(|voice| {
                    (
                        voice.key.as_str(),
                        voice.display_name.as_str(),
                        voice.reference_file.as_deref(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("pocket:anna", "Anna", Some("anna.wav")),
                ("pocket:vera", "Vera", Some("vera.wav")),
                ("pocket:fantine", "Fantine", Some("fantine.wav")),
                ("pocket:charles", "Charles", Some("charles.wav")),
                ("pocket:paul", "Paul", Some("paul.wav")),
                ("pocket:eponine", "Eponine", Some("eponine.wav")),
                ("pocket:azelma", "Azelma", Some("azelma.wav")),
                ("pocket:george", "George", Some("george.wav")),
                ("pocket:mary", "Mary", Some("reference_sample.wav")),
                ("pocket:jane", "Jane", Some("jane.wav")),
                ("pocket:michael", "Michael", Some("michael.wav")),
                ("pocket:eve", "Eve", Some("eve.wav")),
            ]
        );
    }

    #[test]
    fn local_backend_resolution_uses_first_compatible_preference() {
        let preferences = vec![
            "siri:aaron".to_string(),
            EVE_VOICE_KEY.to_string(),
            MARY_VOICE_KEY.to_string(),
            "kokoro:af_heart".to_string(),
        ];
        assert_eq!(
            resolve_voice_for_backend(&preferences, POCKET_BACKEND_ID)
                .expect("Pocket fallback")
                .key,
            EVE_VOICE_KEY
        );
    }

    #[test]
    fn unsupported_or_missing_preferences_fall_back_to_backend_default() {
        let preferences = vec![
            "siri:aaron".to_string(),
            "pocket:imported:deadbeef".to_string(),
        ];
        assert_eq!(
            resolve_voice_for_backend(&preferences, POCKET_BACKEND_ID)
                .expect("Pocket fallback")
                .key,
            MARY_VOICE_KEY
        );
    }

    #[test]
    fn identity_is_qualified_key_not_display_label() {
        assert!(is_qualified_voice_key("pocket:imported:audio-content-hash"));
        assert_ne!(MARY_VOICE_KEY, EVE_VOICE_KEY);
        let mut registry = bundled_voice_registry();
        registry[0].display_name = "Jim".to_string();
        registry[1].display_name = "Jim".to_string();
        assert_eq!(registry[0].display_name, registry[1].display_name);
        assert_ne!(registry[0].key, registry[1].key);
        assert_eq!(
            registry
                .iter()
                .map(|voice| voice.key.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            registry.len()
        );
    }

    #[test]
    fn bundled_vctk_assets_match_the_registry_manifest() {
        for voice in POCKET_VOICES {
            let Some(bytes) = voice.bytes else {
                continue;
            };
            assert_eq!(&bytes[0..4], b"RIFF", "{}", voice.display_name);
            assert_eq!(&bytes[8..12], b"WAVE", "{}", voice.display_name);
            assert_eq!(
                hex::encode(<sha2::Sha256 as sha2::Digest>::digest(bytes)),
                voice.sha256,
                "{}",
                voice.display_name
            );
        }
    }

    #[test]
    fn migrates_unversioned_experiment_settings_to_v1_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, r#"{"voice":"legacy-experiment"}"#).expect("fixture write");
        assert_eq!(
            load_from_path(&path).expect("migration"),
            TtsSettings::default()
        );
    }

    #[test]
    fn migrates_bare_pocket_voice_id_to_qualified_preferences() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"version":1,"agentTextToSpeech":false,"voiceId":"eve"}"#,
        )
        .expect("fixture write");
        assert_eq!(
            load_from_path(&path).expect("migration"),
            TtsSettings {
                version: 1,
                agent_text_to_speech: false,
                voice_preferences: vec![EVE_VOICE_KEY.to_string()],
            }
        );
    }

    #[test]
    fn unknown_qualified_preferences_are_preserved_for_other_clients() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"version":1,"agentTextToSpeech":false,"voicePreferences":["siri:aaron","pocket:imported:abc123"]}"#,
        )
        .expect("fixture write");
        let settings = load_from_path(&path).expect("load");
        assert!(!settings.agent_text_to_speech);
        assert_eq!(
            settings.voice_preferences,
            vec!["siri:aaron", "pocket:imported:abc123"]
        );
    }

    #[test]
    fn rejects_future_schema_versions_clearly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"version":99,"agentTextToSpeech":true,"voicePreferences":["pocket:mary"]}"#,
        )
        .expect("fixture write");
        assert!(load_from_path(&path)
            .expect_err("future version should fail")
            .contains("newer than this Buzz build supports"));
    }

    #[test]
    fn disabling_cancels_runtime_before_persistence_can_fail() {
        let mut huddle = super::super::HuddleState {
            tts_enabled: true,
            ..super::super::HuddleState::default()
        };
        assert!(!huddle.tts_cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(cancel_huddle_speech(&mut huddle).is_none());
        assert!(!huddle.tts_enabled);
        assert!(huddle.tts_cancel.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn pocket_voice_update_preserves_the_latest_toggle_and_other_backends() {
        let current = TtsSettings {
            agent_text_to_speech: false,
            voice_preferences: vec!["siri:aaron".to_string(), MARY_VOICE_KEY.to_string()],
            ..TtsSettings::default()
        };
        let updated = settings_with_pocket_voice_from_registry(
            current,
            EVE_VOICE_KEY,
            &bundled_voice_registry(),
        )
        .expect("available voice");
        assert!(!updated.agent_text_to_speech);
        assert_eq!(updated.voice_preferences, vec!["siri:aaron", EVE_VOICE_KEY]);
    }

    #[test]
    fn failed_off_persistence_cannot_be_undone_by_a_later_voice_update() {
        let state = crate::app_state::build_app_state();
        commit_effective_off(&state).expect("commit effective OFF state");

        // This models the next command after the OFF save fails: it must merge
        // from effective memory state, not the stale last-persisted ON value.
        let current = state.huddle_audio.tts.lock().expect("settings").clone();
        let voice_update = settings_with_pocket_voice_from_registry(
            current,
            EVE_VOICE_KEY,
            &bundled_voice_registry(),
        )
        .expect("available voice");
        assert!(!voice_update.agent_text_to_speech);
    }

    #[test]
    fn failed_disabled_voice_save_does_not_change_the_remembered_voice() {
        let state = crate::app_state::build_app_state();
        state
            .huddle_audio
            .tts
            .lock()
            .expect("settings")
            .agent_text_to_speech = false;
        let current = state.huddle_audio.tts.lock().expect("settings").clone();
        let unsaved = settings_with_pocket_voice_from_registry(
            current,
            EVE_VOICE_KEY,
            &bundled_voice_registry(),
        )
        .expect("available voice");

        // This is the only pre-persistence mutation for an OFF candidate.
        commit_effective_off(&state).expect("commit effective OFF state");
        let remembered = state.huddle_audio.tts.lock().expect("settings").clone();
        assert_eq!(remembered.voice_preferences, vec![MARY_VOICE_KEY]);
        assert_eq!(unsaved.voice_preferences, vec![EVE_VOICE_KEY]);
    }
}
