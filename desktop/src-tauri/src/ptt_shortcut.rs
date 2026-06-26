use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::app_state::AppState;
use crate::huddle::{HuddlePhase, VoiceInputMode};

const SETTINGS_FILE_NAME: &str = "ptt-shortcut.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PttShortcutSettings {
    pub enabled: bool,
    pub shortcut: String,
    pub display: String,
    pub registered: bool,
    pub error: Option<String>,
}

impl Default for PttShortcutSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            shortcut: "Ctrl+Space".to_string(),
            display: "Ctrl+Space".to_string(),
            registered: false,
            error: None,
        }
    }
}

#[derive(Debug)]
pub struct PttShortcutRuntimeState {
    pub enabled: AtomicBool,
    pub registered: AtomicBool,
    pub error: Mutex<Option<String>>,
}

impl Default for PttShortcutRuntimeState {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(PttShortcutSettings::default().enabled),
            registered: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }
}

fn ptt_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Space)
}

fn settings_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("app config dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app config dir: {e}"))?;
    Ok(dir.join(SETTINGS_FILE_NAME))
}

fn read_enabled(app: &AppHandle) -> Result<bool, String> {
    let path = settings_path(app)?;
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(PttShortcutSettings::default().enabled);
    };
    if raw.trim().is_empty() {
        return Ok(PttShortcutSettings::default().enabled);
    }
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("read push-to-talk shortcut settings: {e}"))?;
    Ok(parsed
        .get("enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(PttShortcutSettings::default().enabled))
}

fn write_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let path = settings_path(app)?;
    let settings = serde_json::json!({ "enabled": enabled });
    let payload = serde_json::to_vec_pretty(&settings)
        .map_err(|e| format!("encode push-to-talk shortcut settings: {e}"))?;
    let mut file = AtomicWriteFile::open(&path)
        .map_err(|e| format!("open push-to-talk shortcut settings: {e}"))?;
    file.write_all(&payload)
        .map_err(|e| format!("write push-to-talk shortcut settings: {e}"))?;
    file.commit()
        .map_err(|e| format!("commit push-to-talk shortcut settings: {e}"))
}

pub fn initialize(app: &AppHandle, state: &AppState) {
    match read_enabled(app) {
        Ok(enabled) => state.ptt_shortcut.enabled.store(enabled, Ordering::Release),
        Err(e) => {
            state
                .ptt_shortcut
                .error
                .lock()
                .map(|mut guard| *guard = Some(e.clone()))
                .ok();
            eprintln!("buzz-desktop: failed to load PTT shortcut settings: {e}");
        }
    }
}

pub fn settings_snapshot(state: &AppState) -> PttShortcutSettings {
    PttShortcutSettings {
        enabled: state.ptt_shortcut.enabled.load(Ordering::Acquire),
        registered: state.ptt_shortcut.registered.load(Ordering::Acquire),
        error: state
            .ptt_shortcut
            .error
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| Some("Push-to-talk shortcut state is unavailable".to_string())),
        ..PttShortcutSettings::default()
    }
}

fn emit_settings_changed(app: &AppHandle, state: &AppState) {
    use tauri::Emitter;
    let _ = app.emit("ptt-shortcut-settings-changed", settings_snapshot(state));
}

fn set_ptt_inactive(app: &AppHandle, state: &AppState) {
    if let Ok(hs) = state.huddle_state.lock() {
        hs.ptt_active.store(false, Ordering::Release);
    }
    use tauri::Emitter;
    let _ = app.emit("ptt-state", false);
}

fn should_register(state: &AppState) -> bool {
    if !state.ptt_shortcut.enabled.load(Ordering::Acquire) {
        return false;
    }
    let Ok(hs) = state.huddle_state.lock() else {
        return false;
    };
    hs.voice_input_mode == VoiceInputMode::PushToTalk
        && matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
}

pub fn refresh_registration(app: &AppHandle, state: &AppState) {
    let should_be_registered = should_register(state);
    let is_registered = state.ptt_shortcut.registered.load(Ordering::Acquire);

    if should_be_registered == is_registered {
        if !should_be_registered {
            let cleared_error = state
                .ptt_shortcut
                .error
                .lock()
                .map(|mut error| {
                    let had_error = error.is_some();
                    *error = None;
                    had_error
                })
                .unwrap_or(false);
            if cleared_error {
                emit_settings_changed(app, state);
            }
        }
        return;
    }

    let shortcut = ptt_shortcut();
    if should_be_registered {
        match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                state.ptt_shortcut.registered.store(true, Ordering::Release);
                if let Ok(mut error) = state.ptt_shortcut.error.lock() {
                    *error = None;
                }
                emit_settings_changed(app, state);
            }
            Err(e) => {
                let message = format!("Could not register Ctrl+Space: {e}");
                state
                    .ptt_shortcut
                    .registered
                    .store(false, Ordering::Release);
                if let Ok(mut error) = state.ptt_shortcut.error.lock() {
                    *error = Some(message.clone());
                }
                emit_settings_changed(app, state);
                eprintln!("buzz-desktop: {message}");
            }
        }
    } else {
        match app.global_shortcut().unregister(shortcut) {
            Ok(()) => {
                state
                    .ptt_shortcut
                    .registered
                    .store(false, Ordering::Release);
                set_ptt_inactive(app, state);
                if let Ok(mut error) = state.ptt_shortcut.error.lock() {
                    *error = None;
                }
                emit_settings_changed(app, state);
            }
            Err(e) => {
                let message = format!("Could not unregister Ctrl+Space: {e}");
                if let Ok(mut error) = state.ptt_shortcut.error.lock() {
                    *error = Some(message.clone());
                }
                emit_settings_changed(app, state);
                eprintln!("buzz-desktop: {message}");
            }
        }
    }
}

pub fn refresh_registration_from_state(state: &AppState) {
    let app = match state.app_handle.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    if let Some(app) = app {
        refresh_registration(&app, state);
    }
}

#[tauri::command]
pub fn get_ptt_shortcut_settings(
    state: State<'_, AppState>,
) -> Result<PttShortcutSettings, String> {
    Ok(settings_snapshot(&state))
}

#[tauri::command]
pub fn set_ptt_shortcut_enabled(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PttShortcutSettings, String> {
    write_enabled(&app, enabled)?;
    state.ptt_shortcut.enabled.store(enabled, Ordering::Release);
    refresh_registration(&app, &state);
    emit_settings_changed(&app, &state);
    Ok(settings_snapshot(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(enabled: bool, mode: VoiceInputMode, phase: HuddlePhase) -> AppState {
        let state = crate::app_state::build_app_state();
        state.ptt_shortcut.enabled.store(enabled, Ordering::Release);
        {
            let mut hs = state.huddle_state.lock().unwrap();
            hs.voice_input_mode = mode;
            hs.phase = phase;
        }
        state
    }

    #[test]
    fn registers_only_when_enabled_active_and_ptt() {
        assert!(should_register(&state_with(
            true,
            VoiceInputMode::PushToTalk,
            HuddlePhase::Connected,
        )));
        assert!(should_register(&state_with(
            true,
            VoiceInputMode::PushToTalk,
            HuddlePhase::Active,
        )));

        assert!(!should_register(&state_with(
            false,
            VoiceInputMode::PushToTalk,
            HuddlePhase::Active,
        )));
        assert!(!should_register(&state_with(
            true,
            VoiceInputMode::VoiceActivity,
            HuddlePhase::Active,
        )));
        assert!(!should_register(&state_with(
            true,
            VoiceInputMode::PushToTalk,
            HuddlePhase::Idle,
        )));
    }
}
