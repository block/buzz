//! Open external HTTP(S) URLs on Linux, especially from AppImage builds.
//!
//! Tauri's opener plugin can hand GNOME a fully percent-encoded URI when the
//! AppImage's bundled `xdg-open` and GLib/GIO environment are in play. Delegate
//! to the host `/usr/bin/xdg-open` with a cleaned environment instead.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HOST_XDG_OPEN: &str = "/usr/bin/xdg-open";

/// Env keys that pin GLib/GIO to the AppImage mount and break host handlers.
const APPIMAGE_GLIB_GIO_KEYS: &[&str] = &[
    "GSETTINGS_SCHEMA_DIR",
    "GIO_MODULE_DIR",
    "GIO_EXTRA_MODULES",
    "GI_TYPELIB_PATH",
    "XDG_DATA_DIRS",
    "GTK_PATH",
    "QT_PLUGIN_PATH",
];

/// Keys AppRun commonly points into `$APPDIR` for child processes.
const APPDIR_SCOPED_KEYS: &[&str] = &[
    "PYTHONHOME",
    "PYTHONPATH",
    "PERLLIB",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
];

/// Open an external URL using the platform opener.
///
/// On Linux AppImage builds this uses host `xdg-open` with a sanitized env.
/// Everywhere else it delegates to Tauri's opener plugin.
pub fn open_external_url_for_app(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    let normalized = normalize_external_url(url)?;
    if cfg!(target_os = "linux") && should_use_host_xdg_open() {
        open_with_host_xdg_open(&normalized)
    } else {
        app.opener()
            .open_url(normalized.as_str(), None::<&str>)
            .map_err(|error| format!("could not open external URL: {error}"))
    }
}

fn should_use_host_xdg_open() -> bool {
    std::env::var_os("APPIMAGE").is_some() && Path::new(HOST_XDG_OPEN).is_file()
}

/// Decode a fully percent-encoded HTTP(S) URI once so `xdg-open` receives a
/// normal URL instead of `https%3A%2F%2F...`.
pub fn normalize_external_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("external URL is empty".to_owned());
    }

    if looks_fully_percent_encoded_http_url(trimmed) {
        let decoded = percent_encoding::percent_decode_str(trimmed)
            .decode_utf8()
            .map_err(|error| format!("external URL is not valid UTF-8 after decoding: {error}"))?;
        return Ok(decoded.into_owned());
    }

    Ok(trimmed.to_owned())
}

fn looks_fully_percent_encoded_http_url(url: &str) -> bool {
    (url.starts_with("https%3A") || url.starts_with("http%3A"))
        && !url.starts_with("http://")
        && !url.starts_with("https://")
}

fn open_with_host_xdg_open(url: &str) -> Result<(), String> {
    let mut command = Command::new(HOST_XDG_OPEN);
    command.arg(url).stdin(Stdio::null()).stdout(Stdio::null());

    if let Some(appdir) = std::env::var_os("APPDIR").map(PathBuf::from) {
        sanitize_appimage_env_for_child(&mut command, &appdir);
    }

    command
        .status()
        .map_err(|error| format!("could not launch host xdg-open: {error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "host xdg-open exited with a failure status".to_owned())
}

fn sanitize_appimage_env_for_child(command: &mut Command, appdir: &Path) {
    apply_path_like(command, "LD_LIBRARY_PATH", "ORIGINAL_LD_LIBRARY_PATH", appdir);
    apply_path_like(command, "PATH", "ORIGINAL_PATH", appdir);

    for key in APPDIR_SCOPED_KEYS {
        restore_or_strip_appdir_scoped_key(command, key, appdir);
    }

    for key in APPIMAGE_GLIB_GIO_KEYS {
        restore_or_strip_appdir_scoped_key(command, key, appdir);
    }
}

fn restore_or_strip_appdir_scoped_key(command: &mut Command, key: &str, appdir: &Path) {
    let original = format!("ORIGINAL_{key}");
    if let Some(restored) = std::env::var_os(&original) {
        if restored.is_empty() {
            command.env_remove(key);
        } else {
            command.env(key, restored);
        }
        return;
    }
    if let Ok(value) = std::env::var(key) {
        if value_references_appdir(&value, appdir) {
            command.env_remove(key);
        }
    }
}

fn apply_path_like(command: &mut Command, key: &str, original_key: &str, appdir: &Path) {
    if let Some(restored) = std::env::var_os(original_key) {
        if restored.is_empty() {
            command.env_remove(key);
        } else {
            command.env(key, restored);
        }
        return;
    }
    let Ok(current) = std::env::var(key) else {
        return;
    };
    let cleaned = filter_appdir_entries(&current, appdir);
    if cleaned.is_empty() {
        command.env_remove(key);
    } else {
        command.env(key, cleaned);
    }
}

fn filter_appdir_entries(value: &str, appdir: &Path) -> std::ffi::OsString {
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
    path == root || path.starts_with(root)
}

/// Tauri command wrapper shared by the frontend opener helper.
#[tauri::command]
pub fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    open_external_url_for_app(&app, &url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_decodes_fully_percent_encoded_https_once() {
        let encoded = "https%3A%2F%2Fapp.builderlab.xyz%2Flogin%3Fstate%3Dabc";
        assert_eq!(
            normalize_external_url(encoded).expect("decode"),
            "https://app.builderlab.xyz/login?state=abc"
        );
    }

    #[test]
    fn normalize_leaves_regular_urls_untouched() {
        let url = "https://example.com/path?q=1";
        assert_eq!(normalize_external_url(url).expect("passthrough"), url);
    }

    #[test]
    fn looks_fully_percent_encoded_rejects_plain_https() {
        assert!(!looks_fully_percent_encoded_http_url("https://example.com"));
        assert!(looks_fully_percent_encoded_http_url(
            "https%3A%2F%2Fexample.com"
        ));
    }

    #[test]
    fn filter_drops_appdir_entries_keeps_system_paths() {
        let appdir = PathBuf::from("/tmp/.mount_Buzz_abc/usr");
        let input = std::env::join_paths([
            PathBuf::from("/tmp/.mount_Buzz_abc/usr/lib"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/bin"),
        ])
        .expect("join");
        let cleaned = filter_appdir_entries(&input.to_string_lossy(), &appdir);
        let cleaned = cleaned.to_string_lossy().into_owned();
        assert!(cleaned.contains("/usr/lib"), "{cleaned}");
        assert!(cleaned.contains("/bin"), "{cleaned}");
        assert!(!cleaned.contains(".mount_Buzz"), "{cleaned}");
    }
}
