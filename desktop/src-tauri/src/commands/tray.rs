use std::sync::atomic::Ordering;

use crate::app_state::AppState;

/// Set whether closing the main window hides it to the system tray instead of
/// quitting. The frontend owns the persisted preference (localStorage) and
/// pushes the current value here on launch and whenever the user toggles it.
///
/// The tray icon is created/removed to match the setting so it exists only
/// while the feature is on. Enabling fails closed: if the tray icon cannot be
/// built we leave `close_to_tray` off and return an error, so the window-close
/// button never hides the window with no way to get it back.
#[tauri::command]
pub fn set_close_to_tray(
    enabled: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if enabled {
        crate::tray::build_tray_icon(&app)
            .map_err(|error| format!("failed to create tray icon: {error}"))?;
    } else {
        crate::tray::remove_tray_icon(&app);
    }
    state.close_to_tray.store(enabled, Ordering::SeqCst);
    Ok(())
}
