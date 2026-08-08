//! macOS close-to-tray behavior preference (#4024).
//!
//! Closing Buzz's main window keeps the process and local agents running but
//! hides the only main window. That suits background agent work, but a user on
//! a window-switcher that excludes hidden windows (e.g. BetterTouchTool) loses
//! Buzz from the switcher even though it is still running — the app becomes
//! "windowless" with no way back through the normal switching workflow.
//!
//! This module persists a per-installation preference controlling what the
//! macOS close handler does. The default stays `KeepRunning` (current
//! behavior), preserving active agent work; `QuitWhenClosed` gives users who
//! prefer conventional visible-window app switching an explicit opt-out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{app_state::AppState, managed_agents::storage::atomic_write_json_restricted};

const SETTINGS_FILE: &str = "close-to-tray.json";
const CURRENT_VERSION: u32 = 1;

/// What the macOS handler does when the main window is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseToTrayBehavior {
    /// Keep Buzz running in the menu bar with the window hidden (default,
    /// preserves active agent sessions). Selected on the initial install and
    /// whenever the setting file is missing or unreadable.
    #[serde(rename = "keepRunning")]
    KeepRunning,
    /// Minimize the window to the Dock instead of hiding it, keeping Buzz
    /// visible to window switchers that exclude hidden windows.
    #[serde(rename = "minimizeToTray")]
    MinimizeToTray,
    /// Quit Buzz when the last main window closes, for users who do not want
    /// a windowless background process.
    #[serde(rename = "quitWhenClosed")]
    QuitWhenClosed,
}

impl Default for CloseToTrayBehavior {
    /// Default to the current unconditional behavior (`keepRunning`) so an
    /// upgrade without the setting behaves identically to before.
    fn default() -> Self {
        Self::KeepRunning
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseToTraySettings {
    version: u32,
    behavior: CloseToTrayBehavior,
}

pub(crate) fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(SETTINGS_FILE))
        .map_err(|error| format!("could not locate Buzz settings storage: {error}"))
}

pub(crate) fn load_from_path(path: &Path) -> Result<CloseToTraySettings, String> {
    if !path.exists() {
        return Ok(CloseToTraySettings {
            version: CURRENT_VERSION,
            behavior: CloseToTrayBehavior::default(),
        });
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read close-to-tray settings: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("close-to-tray settings are not valid JSON: {error}"))?;

    // Unversioned settings are incompatible with the V1 schema — use explicit
    // defaults rather than interpret ambiguous fields.
    let version = match value.get("version").and_then(serde_json::Value::as_u64) {
        None => {
            return Ok(CloseToTraySettings {
                version: CURRENT_VERSION,
                behavior: CloseToTrayBehavior::default(),
            });
        }
        Some(v) => v,
    };
    if version > u64::from(CURRENT_VERSION) {
        return Err(format!(
            "close-to-tray settings version {version} is newer than this Buzz build supports"
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("close-to-tray settings are invalid: {error}"))
}

pub(crate) fn save_to_path(path: &Path, settings: &CloseToTraySettings) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("could not encode close-to-tray settings: {error}"))?;
    atomic_write_json_restricted(path, &payload)
        .map_err(|error| format!("could not save close-to-tray settings: {error}"))
}

pub fn load_for_app(app: &AppHandle) -> CloseToTrayBehavior {
    match settings_path(app).and_then(|path| load_from_path(&path)) {
        Ok(settings) => settings.behavior,
        Err(error) => {
            eprintln!("buzz-desktop: {error}; keeping windows hidden on close for this session");
            CloseToTrayBehavior::default()
        }
    }
}

#[tauri::command]
pub fn get_close_to_tray_behavior(state: State<'_, AppState>) -> Result<String, String> {
    let behavior = *state
        .close_to_tray_behavior
        .lock()
        .map_err(|lock_error| format!("close-to-tray settings lock poisoned: {lock_error}"))?;
    Ok(match behavior {
        CloseToTrayBehavior::KeepRunning => "keepRunning",
        CloseToTrayBehavior::MinimizeToTray => "minimizeToTray",
        CloseToTrayBehavior::QuitWhenClosed => "quitWhenClosed",
    }
    .to_string())
}

#[tauri::command]
pub fn set_close_to_tray_behavior(
    app: AppHandle,
    state: State<'_, AppState>,
    behavior: String,
) -> Result<(), String> {
    let behavior = match behavior.as_str() {
        "keepRunning" => CloseToTrayBehavior::KeepRunning,
        "minimizeToTray" => CloseToTrayBehavior::MinimizeToTray,
        "quitWhenClosed" => CloseToTrayBehavior::QuitWhenClosed,
        other => return Err(format!("unsupported close-to-tray behavior: {other}")),
    };
    {
        let mut stored = state
            .close_to_tray_behavior
            .lock()
            .map_err(|lock_error| format!("close-to-tray settings lock poisoned: {lock_error}"))?;
        *stored = behavior;
    }
    let settings = CloseToTraySettings {
        version: CURRENT_VERSION,
        behavior,
    };
    let path = settings_path(&app)?;
    save_to_path(&path, &settings)
        .map_err(|error| format!("could not persist close-to-tray settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn default_when_missing() {
        let dir = temp_dir();
        let path = dir.path().join(SETTINGS_FILE);
        let settings = load_from_path(&path).expect("missing file must load defaults");
        assert_eq!(settings.behavior, CloseToTrayBehavior::KeepRunning);
        assert_eq!(settings.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip_persists_behavior() {
        let dir = temp_dir();
        let path = dir.path().join(SETTINGS_FILE);
        let settings = CloseToTraySettings {
            version: CURRENT_VERSION,
            behavior: CloseToTrayBehavior::QuitWhenClosed,
        };
        save_to_path(&path, &settings).expect("save");
        let loaded = load_from_path(&path).expect("load");
        assert_eq!(loaded.behavior, CloseToTrayBehavior::QuitWhenClosed);
    }

    #[test]
    fn unversioned_file_falls_back_to_default() {
        let dir = temp_dir();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, br##"{"behavior":"quitWhenClosed"}"##).unwrap();
        let loaded = load_from_path(&path).expect("unversioned must load default");
        assert_eq!(loaded.behavior, CloseToTrayBehavior::KeepRunning);
    }

    #[test]
    fn newer_version_is_an_error() {
        let dir = temp_dir();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            br##"{"version":999,"behavior":"keepRunning"}"##,
        )
        .unwrap();
        assert!(load_from_path(&path).is_err());
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = temp_dir();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_from_path(&path).is_err());
    }
}
