use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
    sync::Once,
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "desktop-experiments.json";
const THREAD_SCOPE: u8 = 0;
const CHANNEL_SCOPE: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AcpSessionScope {
    Thread,
    Channel,
}

impl AcpSessionScope {
    fn atomic_value(self) -> u8 {
        match self {
            Self::Thread => THREAD_SCOPE,
            Self::Channel => CHANNEL_SCOPE,
        }
    }
}

/// Process-local setting, lazily hydrated from the Rust-owned store on first
/// read. Thread scope is the durable default when no explicit choice exists.
static ACP_SESSION_SCOPE: AtomicU8 = AtomicU8::new(THREAD_SCOPE);
static HYDRATE: Once = Once::new();

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct DesktopSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    acp_session_scope: Option<AcpSessionScope>,
    // Migration from the preview experiment. Remove after existing installs
    // have had a release cycle to persist `acp_session_scope`.
    #[serde(skip_serializing)]
    acp_top_level_sessions: Option<bool>,
    // Fields owned by other features must survive our writes untouched.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl DesktopSettings {
    fn session_scope(&self) -> AcpSessionScope {
        self.acp_session_scope.unwrap_or_else(|| {
            self.acp_top_level_sessions
                .map(|enabled| {
                    if enabled {
                        AcpSessionScope::Thread
                    } else {
                        AcpSessionScope::Channel
                    }
                })
                .unwrap_or(AcpSessionScope::Thread)
        })
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data dir: {error}"))?
        .join(SETTINGS_FILE))
}

fn load_settings(path: &Path) -> Result<DesktopSettings, String> {
    if !path.exists() {
        return Ok(DesktopSettings::default());
    }
    let payload =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn save_settings(path: &Path, settings: &DesktopSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("failed to serialize settings: {error}"))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("open {} for atomic write: {error}", path.display()))?;
    use std::io::Write;
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", path.display()))
}

pub(crate) fn acp_session_scope(app: &AppHandle) -> AcpSessionScope {
    HYDRATE.call_once(
        || match settings_path(app).and_then(|path| hydrate_scope(&path)) {
            Ok(scope) => ACP_SESSION_SCOPE.store(scope.atomic_value(), Ordering::Release),
            Err(error) => {
                eprintln!("buzz-desktop: failed to hydrate desktop settings: {error}");
            }
        },
    );
    match ACP_SESSION_SCOPE.load(Ordering::Acquire) {
        CHANNEL_SCOPE => AcpSessionScope::Channel,
        _ => AcpSessionScope::Thread,
    }
}

/// Load the persisted scope, materializing an explicit legacy override into
/// the durable `acp_session_scope` field so the compatibility reader can be
/// removed without flipping untouched legacy installs to the default.
///
/// The ordinary no-field default is deliberately NOT written: unset must stay
/// distinguishable from a user/legacy override. A failed materialization
/// write keeps the translated scope in memory and leaves the legacy field on
/// disk, so the next launch retries — no divergence, no corruption.
fn hydrate_scope(path: &Path) -> Result<AcpSessionScope, String> {
    let mut settings = load_settings(path)?;
    let scope = settings.session_scope();
    if settings.acp_session_scope.is_none() && settings.acp_top_level_sessions.is_some() {
        settings.acp_session_scope = Some(scope);
        if let Err(error) = save_settings(path, &settings) {
            eprintln!(
                "buzz-desktop: failed to materialize legacy ACP session scope \
                 (kept in memory; legacy field retained on disk for retry): {error}"
            );
        }
    }
    Ok(scope)
}

#[tauri::command]
pub fn get_acp_session_scope(app: AppHandle) -> AcpSessionScope {
    acp_session_scope(&app)
}

#[tauri::command]
pub fn set_acp_session_scope(scope: AcpSessionScope, app: AppHandle) -> Result<(), String> {
    let path = settings_path(&app)?;
    let mut settings = load_settings(&path)?;
    settings.acp_session_scope = Some(scope);
    settings.acp_top_level_sessions = None;
    save_settings(&path, &settings)?;
    ACP_SESSION_SCOPE.store(scope.atomic_value(), Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{hydrate_scope, load_settings, save_settings, AcpSessionScope, DesktopSettings};

    #[test]
    fn missing_store_defaults_to_thread_scope() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_settings(&dir.path().join("missing.json")).unwrap();
        assert_eq!(loaded.session_scope(), AcpSessionScope::Thread);
    }

    #[test]
    fn explicit_channel_scope_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        save_settings(
            &path,
            &DesktopSettings {
                acp_session_scope: Some(AcpSessionScope::Channel),
                acp_top_level_sessions: None,
                extra: Default::default(),
            },
        )
        .unwrap();
        assert_eq!(
            load_settings(&path).unwrap().session_scope(),
            AcpSessionScope::Channel
        );
    }

    #[test]
    fn migrates_explicit_preview_opt_out_to_channel_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, br#"{"acp_top_level_sessions":false}"#).unwrap();
        assert_eq!(
            load_settings(&path).unwrap().session_scope(),
            AcpSessionScope::Channel
        );
    }

    #[test]
    fn malformed_store_returns_error_and_keeps_process_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_settings(&path).is_err());
    }

    #[test]
    fn hydration_materializes_legacy_opt_out_as_channel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, br#"{"acp_top_level_sessions":false}"#).unwrap();
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["acp_session_scope"], "channel");
        assert!(persisted.get("acp_top_level_sessions").is_none());
        // Restart: the durable field alone must reproduce the override.
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
    }

    #[test]
    fn hydration_materializes_legacy_opt_in_as_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, br#"{"acp_top_level_sessions":true}"#).unwrap();
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Thread);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["acp_session_scope"], "thread");
        assert!(persisted.get("acp_top_level_sessions").is_none());
    }

    #[test]
    fn hydration_never_materializes_the_bare_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, br#"{"other_feature":1}"#).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Thread);
        // Unset must remain distinguishable from an override: no write.
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // Nor is a missing store created.
        let missing = dir.path().join("missing.json");
        assert_eq!(hydrate_scope(&missing).unwrap(), AcpSessionScope::Thread);
        assert!(!missing.exists());
    }

    #[test]
    fn explicit_new_field_wins_over_legacy_and_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(
            &path,
            br#"{"acp_session_scope":"channel","acp_top_level_sessions":true}"#,
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn materialization_preserves_unrelated_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(
            &path,
            br#"{"acp_top_level_sessions":false,"other_feature":{"nested":true},"count":3}"#,
        )
        .unwrap();
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["acp_session_scope"], "channel");
        assert_eq!(persisted["other_feature"]["nested"], true);
        assert_eq!(persisted["count"], 3);
    }

    #[test]
    fn save_preserves_unrelated_fields_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(
            &path,
            br#"{"acp_session_scope":"thread","other_feature":"keep-me"}"#,
        )
        .unwrap();
        let mut settings = load_settings(&path).unwrap();
        settings.acp_session_scope = Some(AcpSessionScope::Channel);
        save_settings(&path, &settings).unwrap();
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["acp_session_scope"], "channel");
        assert_eq!(persisted["other_feature"], "keep-me");
    }

    #[cfg(unix)]
    #[test]
    fn failed_materialization_keeps_translated_scope_and_legacy_field() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        let legacy = br#"{"acp_top_level_sessions":false}"#;
        std::fs::write(&path, legacy).unwrap();
        // Read-only directory: the atomic temp-file write must fail.
        let readonly = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(dir.path(), readonly).unwrap();
        // In memory: translated override. On disk: legacy field untouched,
        // so the next launch retries the materialization. No divergence.
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), legacy);
        assert_eq!(hydrate_scope(&path).unwrap(), AcpSessionScope::Channel);
    }
}
