//! Sanitize AppImage-bundled environment for child processes.
//!
//! When Buzz Desktop runs from an AppImage, AppRun sets `LD_LIBRARY_PATH`,
//! `PYTHONHOME`, and related vars to the mount dir. System tools spawned from
//! Buzz (git, curl, python3) then link against older bundled libs and crash.
//! Restore the host environment for those children.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Env keys AppRun sets to a single bundled directory, replacing whatever the
/// user had. Unset them on children when they still reference the mount (unless
/// an `ORIGINAL_*` restore applies).
///
/// Keys AppRun *prepends* to an existing value belong in `PATH_LIKE_KEYS`
/// instead — removing those wholesale would discard the user's own entries.
const APPDIR_SCOPED_KEYS: &[&str] = &[
    "PYTHONHOME",
    "PERLLIB",
    "GTK_PATH",
    "QT_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
];

/// Colon-separated search paths that mix bundled and user entries, so they are
/// filtered per entry rather than unset.
///
/// `PYTHONPATH` is here because `AppRun.wrapped` builds it as
/// `PYTHONPATH=%s/usr/share/pyshared/:%s` — the bundled entry is *prepended* to
/// whatever the user already had, so unsetting the variable would throw the
/// user's own entries away. `PYTHONHOME` uses a single `%s` and stays above.
const PATH_LIKE_KEYS: &[&str] = &["LD_LIBRARY_PATH", "PATH", "PYTHONPATH"];

/// Apply a host-safe environment to `cmd` when running under an AppImage.
///
/// No-op when `APPDIR` is unset (DMG / native installs). Prefer `ORIGINAL_*`
/// values when a launcher supplies them; the current AppRun does not save them.
/// Otherwise strip this app's mount entries from path-like variables.
pub(crate) fn sanitize_appimage_env_for_child(cmd: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR").map(PathBuf::from) else {
        return;
    };

    for key in PATH_LIKE_KEYS {
        apply_path_like(cmd, key, &format!("ORIGINAL_{key}"), &appdir);
    }

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
        .filter(|entry| !is_under_app_mount(entry, appdir))
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

fn value_references_appdir(value: &str, appdir: &Path) -> bool {
    let appdir_str = appdir.to_string_lossy();
    value.contains(appdir_str.as_ref())
        || std::env::split_paths(value).any(|entry| is_under_app_mount(&entry, appdir))
}

/// The mount root and shared filename prefix of this app's AppImage mounts,
/// derived from `$APPDIR` rather than a hardcoded `/tmp` — the AppImage runtime
/// honours `$TMPDIR`, so the mount root is not guaranteed.
///
/// `/tmp/.mount_Buzz.AmOPMHe` yields `("/tmp", ".mount_Buzz.")`, which also
/// matches `/tmp/.mount_Buzz.AkBFKAC`.
fn sibling_mount_pattern(appdir: &Path) -> Option<(&Path, &str)> {
    let root = appdir.parent()?;
    let name = appdir.file_name()?.to_str()?;
    // `rfind`, not `find`: the random suffix is appended after the last dot.
    // Require a non-zero index so a plain dotfile name cannot degrade the
    // prefix to "." and match every hidden directory under the root.
    let app_name = name.strip_prefix(".mount_")?;
    let (app_name, suffix) = app_name.rsplit_once('.')?;
    if app_name.is_empty()
        || suffix.is_empty()
        || !suffix.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let dot = name.rfind('.')?;
    Some((root, &name[..=dot]))
}

/// True when `path` sits under the current `$APPDIR` **or** under a sibling
/// mount of the same AppImage.
///
/// Filtering only against the current `$APPDIR` leaves entries from a
/// concurrently-mounted older build in place, which is the same
/// link-against-stale-bundled-libs failure this module exists to prevent:
/// `AppRun.wrapped` prepends the new mount to the inherited value, so relaunching
/// while an older instance still holds its mount accumulates both.
fn is_under_app_mount(path: &Path, appdir: &Path) -> bool {
    if is_under_or_equal(path, appdir) {
        return true;
    }
    let Some((root, prefix)) = sibling_mount_pattern(appdir) else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .is_some_and(|first| first.starts_with(prefix))
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

    #[test]
    fn pythonpath_is_filtered_per_entry_not_unset() {
        // AppRun builds PYTHONPATH as `<appdir>/usr/share/pyshared/:<inherited>`,
        // so it must be filtered per entry or the user's own entries are lost.
        assert!(PATH_LIKE_KEYS.contains(&"PYTHONPATH"));
        assert!(!APPDIR_SCOPED_KEYS.contains(&"PYTHONPATH"));
        // PYTHONHOME uses a single `%s` and is always just the bundled home.
        assert!(APPDIR_SCOPED_KEYS.contains(&"PYTHONHOME"));
    }

    #[test]
    fn filter_drops_sibling_mount_entries() {
        // A second, older AppImage still mounted: its entries are not under the
        // current $APPDIR but must still be dropped.
        let appdir = PathBuf::from("/tmp/.mount_Buzz.AmOPMHe");
        let input = std::env::join_paths([
            PathBuf::from("/tmp/.mount_Buzz.AmOPMHe/usr/lib"),
            PathBuf::from("/tmp/.mount_Buzz.AkBFKAC/usr/lib"),
            PathBuf::from("/usr/lib/x86_64-linux-gnu"),
        ])
        .expect("join");
        let cleaned = filter_appdir_entries(&input.to_string_lossy(), &appdir);
        let cleaned = cleaned.to_string_lossy().into_owned();
        assert!(cleaned.contains("/usr/lib/x86_64-linux-gnu"), "{cleaned}");
        assert!(
            !cleaned.contains(".mount_Buzz."),
            "sibling mount entries must be stripped: {cleaned}"
        );
    }

    #[test]
    fn filter_keeps_unrelated_entries_under_the_mount_root() {
        // Only sibling mounts of *this* AppImage are dropped — the mount root
        // itself is an ordinary temp dir the user may legitimately use.
        let appdir = PathBuf::from("/tmp/.mount_Buzz.AmOPMHe");
        let input = std::env::join_paths([
            PathBuf::from("/tmp/my-own-libs"),
            PathBuf::from("/tmp/.mount_OtherApp.QQQQQQ/usr/lib"),
        ])
        .expect("join");
        let cleaned = filter_appdir_entries(&input.to_string_lossy(), &appdir);
        let cleaned = cleaned.to_string_lossy().into_owned();
        assert!(cleaned.contains("/tmp/my-own-libs"), "{cleaned}");
        assert!(cleaned.contains(".mount_OtherApp."), "{cleaned}");
    }

    #[test]
    fn sibling_mount_pattern_derives_root_and_prefix_from_appdir() {
        let appdir = PathBuf::from("/tmp/.mount_Buzz.AmOPMHe");
        let (root, prefix) = sibling_mount_pattern(&appdir).expect("pattern");
        assert_eq!(root, Path::new("/tmp"));
        assert_eq!(prefix, ".mount_Buzz.");

        // Honours $TMPDIR rather than assuming /tmp.
        let elsewhere = PathBuf::from("/run/user/1000/.mount_Buzz.ZZZZZZ");
        let (root, prefix) = sibling_mount_pattern(&elsewhere).expect("pattern");
        assert_eq!(root, Path::new("/run/user/1000"));
        assert_eq!(prefix, ".mount_Buzz.");
    }

    #[test]
    fn sibling_mount_pattern_rejects_names_that_would_over_match() {
        // No dot: nothing to derive, fall back to current-$APPDIR-only.
        assert!(sibling_mount_pattern(Path::new("/tmp/appdir")).is_none());
        // Leading dot only: a "." prefix would match every hidden directory.
        assert!(sibling_mount_pattern(Path::new("/tmp/.mount_Buzz_x")).is_none());
        assert!(sibling_mount_pattern(Path::new("/opt/buzz.AppDir")).is_none());
        assert!(sibling_mount_pattern(Path::new("/tmp/.mount_.ABC123")).is_none());
        assert!(sibling_mount_pattern(Path::new("/tmp/.mount_Buzz.")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn spawned_child_keeps_user_pythonpath_and_excludes_both_mounts() {
        let status = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "tests::sanitized_child_environment_probe", "--nocapture"])
            .env_clear()
            .env("BUZZ_APPIMAGE_SANITIZER_TEST", "1")
            .env("APPDIR", "/run/user/1000/.mount_Buzz.ABC123")
            .env("PATH", "/usr/bin:/bin")
            .env("PYTHONHOME", "/run/user/1000/.mount_Buzz.ABC123/usr")
            .env("PYTHONPATH", "/run/user/1000/.mount_Buzz.ABC123/python:/run/user/1000/.mount_Buzz.XYZ789/python:/opt/user-python")
            .env("LD_LIBRARY_PATH", "/run/user/1000/.mount_Buzz.ABC123/lib:/run/user/1000/.mount_Buzz.XYZ789/lib:/opt/user-libs")
            .status()
            .expect("run isolated environment probe");
        assert!(status.success());
    }

    #[cfg(unix)]
    #[test]
    fn sanitized_child_environment_probe() {
        if std::env::var_os("BUZZ_APPIMAGE_SANITIZER_TEST").is_none() {
            return;
        }
        let mut child = Command::new("/usr/bin/env");
        sanitize_appimage_env_for_child(&mut child);
        let output = child.output().expect("spawn sanitized child");
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).expect("controlled environment is UTF-8");
        let lines: Vec<_> = text.lines().collect();
        assert!(lines.contains(&"PYTHONPATH=/opt/user-python"));
        assert!(lines.contains(&"LD_LIBRARY_PATH=/opt/user-libs"));
        assert!(!lines.iter().any(|line| line.starts_with("PYTHONHOME=")));
    }
}
