//! Crash-loop recovery for Linux WebKitGTK startup.
//!
//! Some WebKitGTK/GPU combinations abort or segfault while opening a persisted
//! `WebKitCache`. The process dies after Tauri setup starts but before the main
//! page finishes loading, so the next launch can detect that incomplete startup.
//! Only the disposable WebKit cache is quarantined; identity and user data are
//! never removed.

use std::path::{Path, PathBuf};

const STARTUP_PENDING: &str = ".webkit-startup-pending";
const WEBKIT_CACHE: &str = "WebKitCache";
const RECOVERY_TRASH: &str = ".WebKitCache.startup-recovery";

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PrepareOutcome {
    pub quarantined_cache: bool,
}

fn marker_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(STARTUP_PENDING)
}

fn cache_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(WEBKIT_CACHE)
}

fn trash_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(RECOVERY_TRASH)
}

/// Mark this startup as pending and quarantine WebKit's cache when the previous
/// launch never reached a finished main-page load.
pub(crate) fn prepare(app_data_dir: &Path) -> Result<PrepareOutcome, String> {
    std::fs::create_dir_all(app_data_dir)
        .map_err(|error| format!("create app data directory: {error}"))?;

    let marker = marker_path(app_data_dir);
    let mut outcome = PrepareOutcome::default();
    if marker.exists() {
        let cache = cache_path(app_data_dir);
        let trash = trash_path(app_data_dir);
        if trash.exists() {
            std::fs::remove_dir_all(&trash).map_err(|error| {
                format!(
                    "remove prior WebKit recovery trash {}: {error}",
                    trash.display()
                )
            })?;
        }
        if cache.exists() {
            std::fs::rename(&cache, &trash).map_err(|error| {
                format!(
                    "quarantine WebKit cache {} to {}: {error}",
                    cache.display(),
                    trash.display()
                )
            })?;
            outcome.quarantined_cache = true;
        }
    }

    // Write after recovery so another crash during/after setup remains visible.
    std::fs::write(&marker, b"pending\n")
        .map_err(|error| format!("write WebKit startup marker {}: {error}", marker.display()))?;
    Ok(outcome)
}

/// A finished main-page load proves WebKit startup succeeded. Remove the marker
/// and any quarantined cache from the prior failed launch.
pub(crate) fn mark_ready(app_data_dir: &Path) -> Result<(), String> {
    let marker = marker_path(app_data_dir);
    match std::fs::remove_file(&marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "remove WebKit startup marker {}: {error}",
                marker.display()
            ));
        }
    }

    let trash = trash_path(app_data_dir);
    match std::fs::remove_dir_all(&trash) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove WebKit recovery trash {}: {error}",
            trash.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_marks_pending_without_touching_data() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("identity.migrated"), b"1").unwrap();

        let outcome = prepare(&app).unwrap();

        assert_eq!(outcome, PrepareOutcome::default());
        assert!(marker_path(&app).is_file());
        assert_eq!(std::fs::read(app.join("identity.migrated")).unwrap(), b"1");
    }

    #[test]
    fn incomplete_start_quarantines_only_webkit_cache() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        let cache = cache_path(&app);
        std::fs::create_dir_all(cache.join("Version 17")).unwrap();
        std::fs::write(cache.join("Version 17/salt"), b"stale").unwrap();
        std::fs::write(app.join("identity.migrated"), b"1").unwrap();
        std::fs::write(marker_path(&app), b"pending\n").unwrap();

        let outcome = prepare(&app).unwrap();

        assert!(outcome.quarantined_cache);
        assert!(!cache.exists());
        assert_eq!(
            std::fs::read(trash_path(&app).join("Version 17/salt")).unwrap(),
            b"stale"
        );
        assert_eq!(std::fs::read(app.join("identity.migrated")).unwrap(), b"1");
        assert!(marker_path(&app).is_file());
    }

    #[test]
    fn ready_clears_marker_and_recovery_trash() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir_all(trash_path(&app)).unwrap();
        std::fs::write(trash_path(&app).join("salt"), b"old").unwrap();
        std::fs::write(marker_path(&app), b"pending\n").unwrap();

        mark_ready(&app).unwrap();

        assert!(!marker_path(&app).exists());
        assert!(!trash_path(&app).exists());
    }
}
