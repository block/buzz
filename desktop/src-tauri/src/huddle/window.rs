//! Native companion-window lifecycle for an active Huddle.

use tauri::{
    utils::config::{BackgroundThrottlingPolicy, WindowConfig},
    Emitter, Manager, State, WebviewWindowBuilder,
};

use crate::app_state::AppState;

fn companion_config(label: String) -> WindowConfig {
    WindowConfig {
        label,
        title: "Huddle".into(),
        width: 960.0,
        height: 720.0,
        min_width: Some(720.0),
        min_height: Some(520.0),
        // Match the capture owner: hidden macOS rooms must not suspend live media.
        background_throttling: Some(BackgroundThrottlingPolicy::Disabled),
        ..WindowConfig::default()
    }
}

/// Close the companion belonging to an ended huddle. The native lifecycle is
/// authoritative here because a webview can be suspended while it is closing.
pub(super) fn close_huddle_window(app: &tauri::AppHandle, ephemeral_channel_id: &str) {
    if ephemeral_channel_id.is_empty() {
        return;
    }
    let label = format!("huddle-{ephemeral_channel_id}");
    if let Some(window) = app.get_webview_window(&label) {
        if let Err(error) = window.close() {
            eprintln!("buzz-desktop: failed to close huddle companion: {error}");
        }
    }
}

/// Close the active companion without leaving the huddle. The main window uses
/// this to restore its drawer presentation while retaining the audio session.
#[tauri::command]
pub fn close_huddle_companion(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ephemeral_channel_id = state
        .huddle()?
        .ephemeral_channel_id
        .clone()
        .ok_or("no active huddle")?;
    close_huddle_window(&app, &ephemeral_channel_id);
    app.emit("huddle-companion-returned", ())
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Open the active huddle's ephemeral channel in a focused companion window.
/// The main window remains the owner of microphone capture; closing this room
/// must never leave the shared huddle session.
#[tauri::command]
pub async fn open_huddle_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ephemeral_channel_id = state
        .huddle()?
        .ephemeral_channel_id
        .clone()
        .ok_or("no active huddle")?;
    let label = format!("huddle-{ephemeral_channel_id}");

    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    WebviewWindowBuilder::from_config(&app, &companion_config(label))
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_companions_keep_the_owners_background_media_policy() {
        let config = companion_config("huddle-latency-regression".into());
        assert_eq!(
            config.background_throttling,
            Some(BackgroundThrottlingPolicy::Disabled)
        );
        assert_eq!(config.url, tauri::WebviewUrl::App("index.html".into()));
        assert_eq!(config.label, "huddle-latency-regression");
    }
}
