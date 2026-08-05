//! The macOS application menu.
//!
//! Buzz never called `Builder::menu()`, so Tauri installed `Menu::default()`
//! for us (`tauri::app::Builder::build`, macOS arm). That default puts a
//! `close_window` item in both the File and Window submenus, and muda gives
//! that item a Cmd+W key equivalent bound to `performClose:`.
//!
//! An earlier revision of this module removed both `close_window` items,
//! because macOS resolves a menu key equivalent before the webview receives
//! any key event, so Buzz Term could never bind Cmd+W to "close this
//! terminal tab" while the accelerator was claimed here. That cure traded a
//! terminal-local conflict for an app-wide regression: Cmd+W is the standard
//! macOS chord for "close the focused window", and with the item gone it did
//! nothing anywhere else in the app. For a tray-resident app "close" means
//! hide-to-tray (the `CloseRequested` interception in `lib.rs`), exactly the
//! Cmd+W behavior of other tray-resident chat apps.
//!
//! So the menu now restores File > Close Window as a *custom* item
//! (predefined items cannot be toggled after creation) and Buzz Term
//! disables it for exactly as long as it owns the keyboard, via the
//! `set_close_window_menu_enabled` command: a disabled menu item does not
//! consume its key equivalent, so Cmd+W falls through to the webview and the
//! terminal's close-tab chord (`matchTabChord` in `terminalState.ts`) runs.
//! Everything else still mirrors `Menu::default()` deliberately; the Window
//! submenu's duplicate close item is not restored because File is the
//! chord's canonical home.
//!
//! The app submenu additionally carries the standard Settings… (Cmd+,) and
//! Check for Updates… items in their HIG positions; both forward to the
//! webview, where the settings and updater surfaces live.

#[cfg(target_os = "macos")]
use tauri::menu::{
    AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Emitter, Manager};
use tauri::{Builder, Runtime};

/// Menu id for File > Close Window.
#[cfg(target_os = "macos")]
const CLOSE_WINDOW_ID: &str = "close-window";

/// Menu id for the app submenu's Settings… (Cmd+,) item.
#[cfg(target_os = "macos")]
const SETTINGS_ID: &str = "settings";

/// Menu id for the app submenu's Check for Updates… item.
#[cfg(target_os = "macos")]
const CHECK_FOR_UPDATES_ID: &str = "check-for-updates";

/// Handle to the File > Close Window item, managed so
/// `set_close_window_menu_enabled` can toggle it after the menu is built.
#[cfg(target_os = "macos")]
struct CloseWindowMenuItem<R: Runtime>(MenuItem<R>);

/// Installs Buzz's menu, replacing the `Menu::default()` Tauri would otherwise
/// auto-install. A no-op off macOS, where that default is never created and
/// the Cmd+W accelerator does not exist.
pub fn install<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(build)
        .on_menu_event(|app, event| match event.id().as_ref() {
            CLOSE_WINDOW_ID => close_focused_window(app),
            SETTINGS_ID => forward_menu_action(app, "menu-open-settings"),
            CHECK_FOR_UPDATES_ID => forward_menu_action(app, "menu-check-for-updates"),
            _ => {}
        });
    builder
}

/// Shows the main window and forwards a menu action to its webview.
///
/// Settings-flavored items stay clickable while the window is hidden to the
/// tray (the menu bar is reachable whenever the app is active), so the
/// window is re-presented first — acting invisibly would look like the item
/// did nothing.
#[cfg(target_os = "macos")]
fn forward_menu_action<R: Runtime>(app: &AppHandle<R>, event: &str) {
    crate::tray_menu::show_main_window(app);
    if let Err(error) = app.emit_to("main", event, ()) {
        eprintln!("buzz-desktop: failed to forward menu action {event}: {error}");
    }
}

/// Closes the focused window, falling back to the main window.
///
/// `close()` goes through `CloseRequested`, so the main window takes the
/// hide-to-tray path in `lib.rs` and huddle windows keep their
/// drawer-restore behavior — the same outcome as clicking the native close
/// button.
#[cfg(target_os = "macos")]
fn close_focused_window<R: Runtime>(app: &AppHandle<R>) {
    let windows = app.webview_windows();
    let target = windows
        .values()
        .find(|window| window.is_focused().unwrap_or(false))
        .or_else(|| windows.get("main"));
    let Some(window) = target else {
        return;
    };
    if let Err(error) = window.close() {
        eprintln!("buzz-desktop: failed to close window from menu: {error}");
    }
}

/// Enables or disables File > Close Window (Cmd+W).
///
/// Buzz Term claims Cmd+W to close terminal tabs while it owns the keyboard.
/// macOS resolves menu key equivalents before the webview sees any key
/// event, so the item must be disabled for the chord to reach the terminal
/// at all — a disabled item does not consume its key equivalent. A no-op off
/// macOS, where this menu is never installed.
#[tauri::command]
pub fn set_close_window_menu_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let Some(item) = app.try_state::<CloseWindowMenuItem<R>>() else {
            return Ok(());
        };
        let item = item.0.clone();
        app.run_on_main_thread(move || {
            if let Err(error) = item.set_enabled(enabled) {
                eprintln!("buzz-desktop: failed to set Close Window enabled={enabled}: {error}");
            }
        })
        .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, enabled);
        Ok(())
    }
}

/// Mirrors `Menu::default()` with File > Close Window as a toggleable custom
/// item, the Window submenu's duplicate close item omitted, and Settings… /
/// Check for Updates… added to the app submenu.
///
/// The Window and Help submenus keep Tauri's well-known ids: `init_app_menu`
/// looks them up by id to call `set_as_windows_menu_for_nsapp` and
/// `set_as_help_menu_for_nsapp`, and a plain `with_items` submenu would skip
/// both silently -- no error, just a Window menu AppKit no longer manages.
#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let close_window = MenuItem::with_id(
        app,
        CLOSE_WINDOW_ID,
        "Close Window",
        true,
        Some("CmdOrCtrl+W"),
    )?;
    app.manage(CloseWindowMenuItem(close_window.clone()));

    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                pkg_info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about_metadata))?,
                    &PredefinedMenuItem::separator(app)?,
                    // Standard app-submenu items macOS users expect between
                    // About and Services; both forward to the webview (the
                    // settings UI lives there).
                    &MenuItem::with_id(
                        app,
                        CHECK_FOR_UPDATES_ID,
                        "Check for Updates…",
                        true,
                        None::<&str>,
                    )?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("CmdOrCtrl+,"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            // `Menu::default()`'s File submenu holds exactly one item on
            // macOS -- close_window -- restored here as the custom item so
            // Buzz Term can release the accelerator while it owns Cmd+W.
            &Submenu::with_items(app, "File", true, &[&close_window])?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &Submenu::with_id_and_items(
                app,
                WINDOW_SUBMENU_ID,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                ],
            )?,
            // Empty upstream too on macOS: About lives in the app submenu.
            &Submenu::with_id_and_items(app, HELP_SUBMENU_ID, "Help", true, &[])?,
        ],
    )
}
