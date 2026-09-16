//! Native companion-window lifecycle for an active Huddle.

use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app_state::AppState;

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

    WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into()))
        .title("Huddle")
        .inner_size(960.0, 720.0)
        .min_inner_size(720.0, 520.0)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}
