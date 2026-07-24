//! Sanitize AppImage-bundled environment for child processes.
//!
//! When Buzz Desktop runs from an AppImage, AppRun sets `LD_LIBRARY_PATH`,
//! `PYTHONHOME`, and related vars to the mount dir. System tools spawned from
//! Buzz (git, curl, python3) then link against older bundled libs and crash.
//! Restore the host environment for those children.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Env keys AppRun commonly points into `$APPDIR`. Unset them on children when
/// they still reference the mount (unless an `ORIGINAL_*` restore applies).
const APPDIR_SCOPED_KEYS: &[&str] = &[
    "PYTHONHOME",
    "PYTHONPATH",
    "PERLLIB",
    "GTK_PATH",
    "QT_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
];

/// Apply a host-safe environment to `cmd` when running under an AppImage.
///
/// No-op when `APPDIR` is unset (DMG / native installs). Prefer `ORIGINAL_*`
/// values saved by AppRun when present; otherwise strip `$APPDIR` entries from
/// path-like variables.
pub(crate) fn sanitize_appimage_env_for_child(cmd: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR").map(PathBuf::from) else {
        return;
    };

    apply_path_like(cmd, "LD_LIBRARY_PATH", "ORIGINAL_LD_LIBRARY_PATH", &appdir);
    apply_path_like(cmd, "PATH", "ORIGINAL_PATH", &appdir);

    for key in APPDIR_SCOPED_KEYS {
        let original = format!("ORIGINAL_{key}");
        if let Some(restored) = std::env::var_os(&original) {
            if restored.is_empty() {
                cmd.env_remove(key);
            } else {
                cmd.env(key, restored);
            }
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            if value_references_appdir(&value, &appdir) {
                cmd.env_remove(key);
            }
        }
    }
}

fn apply_path_like(cmd: &mut Command, key: &str, original_key: &str, appdir: &Path) {
    if let Some(restored) = std::env::var_os(original_key) {
        if restored.is_empty() {
            cmd.env_remove(key);
        } else {
            cmd.env(key, restored);
        }
        return;
    }
    let Ok(current) = std::env::var(key) else {
        return;
    };
    let cleaned = filter_appdir_entries(&current, appdir);
    if cleaned.is_empty() {
        cmd.env_remove(key);
    } else {
        cmd.env(key, cleaned);
    }
}

fn filter_appdir_entries(value: &str, appdir: &Path) -> OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(value)
        .filter(|entry| !is_under_or_equal(entry, appdir))
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

fn value_references_appdir(value: &str, appdir: &Path) -> bool {
    let appdir_str = appdir.to_string_lossy();
    value.contains(appdir_str.as_ref())
        || std::env::split_paths(value).any(|entry| is_under_or_equal(&entry, appdir))
}

fn is_under_or_equal(path: &Path, root: &Path) -> bool {
    if path == root {
        return true;
    }
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn filter_drops_appdir_entries_keeps_system() {
        let appdir = PathBuf::from("/tmp/.mount_Buzz_abc/usr");
        let input = std::env::join_paths([
            PathBuf::from("/tmp/.mount_Buzz_abc/usr/lib"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/tmp/.mount_Buzz_abc/usr/bin"),
            PathBuf::from("/bin"),
        ])
        .expect("join");
        let cleaned = filter_appdir_entries(&input.to_string_lossy(), &appdir);
        let cleaned = cleaned.to_string_lossy().into_owned();
        assert!(cleaned.contains("/usr/lib"), "{cleaned}");
        assert!(cleaned.contains("/bin"), "{cleaned}");
        assert!(
            !cleaned.contains(".mount_Buzz"),
            "appdir entries must be stripped: {cleaned}"
        );
    }

    #[test]
    fn value_references_detects_appdir_substring() {
        let appdir = PathBuf::from("/tmp/.mount_Buzz_x");
        assert!(value_references_appdir(
            "/tmp/.mount_Buzz_x/usr/lib/python3",
            &appdir
        ));
        assert!(!value_references_appdir("/usr/lib/python3", &appdir));
    }
}
