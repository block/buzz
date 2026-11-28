//! Persisted-login state for the bridge's Chromium profile.
//!
//! The bridge never handles Muse credentials itself. It launches Chromium
//! with a persistent `--user-data-dir`; the user signs in to Muse once in
//! that browser context (headed, on first run) and the login cookies persist
//! in the profile afterwards. This module tracks only a marker file that
//! records "a login was observed", which is what `muse-acp auth status`
//! (the catalog's `auth_probe_args`) checks.
//!
//! The marker is a heuristic, not proof: cookies can expire while the
//! marker exists. The driver re-checks for a signed-in composer on every
//! turn and re-enters the sign-in wait (clearing the marker) if login is
//! gone, so a stale marker self-heals instead of stranding turns.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::BridgeError;

/// Environment override for the bridge profile directory (also the
/// Chromium `--user-data-dir`). Primarily for tests and operators who want
/// the bridge profile somewhere other than the default.
pub const PROFILE_ENV: &str = "MUSE_ACP_PROFILE";

const MARKER_FILE: &str = "session.json";
const MARKER_VERSION: u64 = 1;

/// Resolve the bridge profile directory, or `None` when no home directory
/// can be determined.
pub fn profile_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(PROFILE_ENV) {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(
            || match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
                (Some(drive), Some(path)) => {
                    let mut home = PathBuf::from(drive);
                    home.push(path);
                    Some(home)
                }
                _ => None,
            },
        )?;
    Some(home.join(".muse-acp"))
}

fn marker_path() -> Option<PathBuf> {
    profile_dir().map(|dir| dir.join(MARKER_FILE))
}

/// Whether a previous successful Muse sign-in has been recorded.
///
/// Fast and side-effect free: this is what the desktop auth probe runs, so
/// it must never launch a browser.
pub fn has_saved_session() -> bool {
    let Some(path) = marker_path() else {
        return false;
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    value.get("version").and_then(|v| v.as_u64()) == Some(MARKER_VERSION)
}

/// Record that a signed-in Muse composer was observed. Creates the profile
/// directory if needed.
pub fn mark_signed_in() -> Result<(), BridgeError> {
    let Some(dir) = profile_dir() else {
        return Err(BridgeError::NotSignedIn(
            "cannot determine a profile directory for the session marker".into(),
        ));
    };
    fs::create_dir_all(&dir)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let marker = json!({ "version": MARKER_VERSION, "logged_in_at_unix": now });
    let text = serde_json::to_string_pretty(&marker)?;
    fs::write(dir.join(MARKER_FILE), text)?;
    Ok(())
}

/// Best-effort removal of the session marker (used when a turn discovers
/// the saved login is gone). Failures are ignored: the next login rewrites
/// the marker, and a stale file only costs one extra sign-in wait.
pub fn clear_saved_session() {
    if let Some(path) = marker_path() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes the env-mutating tests: they share `MUSE_ACP_PROFILE`,
    /// so parallel execution could point one test at another's tempdir.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn marker_round_trip() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(PROFILE_ENV, dir.path());
        assert!(!has_saved_session());
        mark_signed_in().unwrap();
        assert!(has_saved_session());
        clear_saved_session();
        assert!(!has_saved_session());
        std::env::remove_var(PROFILE_ENV);
    }

    #[test]
    fn malformed_marker_is_not_a_session() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(PROFILE_ENV, dir.path());
        fs::write(dir.path().join(MARKER_FILE), "not json").unwrap();
        assert!(!has_saved_session());
        std::env::remove_var(PROFILE_ENV);
    }
}
