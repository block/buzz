//! System tray icon and close-to-tray window handling.
//!
//! When the "Keep Buzz running in the tray" setting is on, closing the main
//! window hides it instead of quitting. The tray icon is the escape hatch:
//! left-click (or "Show Buzz") reopens the window, and "Quit Buzz" exits.
//!
//! The tray icon exists only while the setting is enabled — it is created when
//! the frontend pushes `set_close_to_tray(true)` and removed on `false`, so
//! users who never opt in never see a tray icon.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, WindowEvent};

use crate::app_state::AppState;

/// Stable id for the close-to-tray icon, used to create and remove it.
const TRAY_ID: &str = "main-tray";

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
        if window.label() != "main" {
            return;
        }
        let state = window.state::<AppState>();
        if state.close_to_tray.load(Ordering::SeqCst) && !state.quitting.load(Ordering::SeqCst) {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

/// Build the system tray icon with a "Show Buzz" / "Quit Buzz" menu. Left-click
/// reopens the window; "Quit Buzz" sets the `quitting` flag (so the close
/// handler does not re-hide the window) and exits the app. Idempotent — a
/// no-op if the tray icon already exists.
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
                if let Some(state) = app.try_state::<AppState>() {
                    state.quitting.store(true, Ordering::SeqCst);
                }
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
pub fn remove_tray_icon(app: &AppHandle) {
    app.remove_tray_by_id(TRAY_ID);
}
