/// Set whether closing the main window keeps Buzz running in the background
/// instead of quitting. The frontend owns the persisted preference
/// (`localStorage`) and pushes it here on launch and whenever it changes.
///
/// Windows and Linux need a tray icon as the reopen/quit affordance. macOS
/// keeps the standard Dock icon, and `RunEvent::Reopen` restores the window.
#[tauri::command]
pub fn set_close_to_tray(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        if enabled {
            crate::tray::build_tray_icon(&app)
                .map_err(|error| format!("failed to create tray icon: {error}"))?;
        } else {
            crate::tray::remove_tray_icon(&app);
        }
    }
    // macOS keeps the Dock icon instead of a tray icon, so `app` is unused.
    #[cfg(target_os = "macos")]
    let _ = &app;
    crate::tray::set_close_to_tray_enabled(enabled);
    Ok(())
}
