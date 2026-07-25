//! Background-window lifecycle and system tray handling.
//!
//! When "Keep Buzz running when the window closes" is on, closing the main
//! window hides it instead of quitting. Windows and Linux expose a tray icon:
//! left-click (or "Show Buzz") reopens the window, and "Quit Buzz" exits.
//! macOS uses its normal Dock icon and application Quit command.
//!
//! The non-macOS tray icon exists only while the setting is enabled.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WindowEvent};

// These flags describe process-wide native window lifecycle, not
// community-scoped application data, so they intentionally live beside the
// tray handlers instead of growing the already-oversized AppState.
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Stable id for the close-to-tray icon, used to create and remove it.
#[cfg(not(target_os = "macos"))]
const TRAY_ID: &str = "main-tray";

/// Apply the frontend-owned persisted preference to the native close handler.
pub fn set_close_to_tray_enabled(enabled: bool) {
    CLOSE_TO_TRAY.store(enabled, Ordering::SeqCst);
}

/// Allow window teardown to proceed during a genuine application exit.
pub fn mark_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}

/// Show, unminimize, and focus the main window. Used by the tray menu,
/// left-click, and the macOS dock-reopen handler to surface the window after
/// close-to-tray has hidden it.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Builder `on_window_event` handler implementing close-to-tray: when the user
/// closes the main window and the setting is on, hide the window instead of
/// quitting. A genuine quit (tray "Quit Buzz", or any app exit, which the
/// `RunEvent::ExitRequested` handler marks) sets `quitting` first so the window
/// is allowed to close normally.
pub fn handle_window_event(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if should_hide_window(
            window.label(),
            CLOSE_TO_TRAY.load(Ordering::SeqCst),
            QUITTING.load(Ordering::SeqCst),
        ) {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

fn should_hide_window(label: &str, keep_running: bool, quitting: bool) -> bool {
    label == "main" && keep_running && !quitting
}

/// Build the system tray icon with a "Show Buzz" / "Quit Buzz" menu. Left-click
/// reopens the window; "Quit Buzz" sets the `quitting` flag (so the close
/// handler does not re-hide the window) and exits the app. Idempotent — a
/// no-op if the tray icon already exists.
#[cfg(not(target_os = "macos"))]
pub fn build_tray_icon(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }

    let show_item = MenuItem::with_id(app, "tray-show", "Show Buzz", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "tray-quit", "Quit Buzz", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Buzz")
        .menu(&menu)
        // Linux (StatusNotifierItem) frequently does not deliver left-click
        // activation events, so left-click-to-show is unreliable there; open
        // the menu on left-click instead so "Show Buzz" stays reachable.
        // macOS/Windows keep left-click-to-show with the menu on right-click.
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show_main_window(app),
            "tray-quit" => {
                mark_quitting();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // Reuse the bundled window icon for the tray glyph.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Remove the tray icon. Called when the user turns close-to-tray off so the
/// icon does not linger after the feature is disabled.
#[cfg(not(target_os = "macos"))]
pub fn remove_tray_icon(app: &AppHandle) {
    app.remove_tray_by_id(TRAY_ID);
}

#[cfg(test)]
mod tests {
    use super::should_hide_window;

    #[test]
    fn hides_main_window_when_background_mode_is_enabled() {
        assert!(should_hide_window("main", true, false));
    }

    #[test]
    fn allows_main_window_to_close_during_explicit_quit() {
        assert!(!should_hide_window("main", true, true));
    }

    #[test]
    fn allows_close_when_background_mode_is_disabled() {
        assert!(!should_hide_window("main", false, false));
    }

    #[test]
    fn never_intercepts_auxiliary_windows() {
        assert!(!should_hide_window("settings", true, false));
    }
}
