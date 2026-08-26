//! TEMPORARY diagnostic logging for the live-subscription unread bug.
//!
//! Appends relay subscription/frame events to
//! `<app_data_dir>/buzz-relay-debug.log` so we can see, on a packaged Windows
//! build (where console output is swallowed and the relay socket is native and
//! invisible to devtools), whether the app sends background-channel
//! subscriptions and whether the relay pushes events back.
//!
//! REMOVE BEFORE RELEASE. This is a build-time diagnostic aid, not shipped
//! behavior — it must never go out in a tagged release.
use std::io::Write;

use tauri::{AppHandle, Manager};

/// Append one line to `<app_data_dir>/buzz-relay-debug.log`. Best-effort: any
/// failure is swallowed, since this is a diagnostic aid. Callable from Rust
/// (e.g. the Windows toast path) as well as the frontend command below.
pub(crate) fn append_line(app: &AppHandle, line: &str) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("buzz-relay-debug.log");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{ts} {line}");
    }
}

#[tauri::command]
pub(crate) fn debug_append_relay_log(app: AppHandle, line: String) -> Result<(), String> {
    append_line(&app, &line);
    Ok(())
}
