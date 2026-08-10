//! The macOS application menu.
//!
//! Buzz never called `Builder::menu()`, so Tauri installed `Menu::default()`
//! for us (`tauri::app::Builder::build`, macOS arm). That default puts a
//! `close_window` item in both the File and Window submenus, and muda gives
//! that item a Cmd+W key equivalent bound to `performClose:`.
//!
//! Two consequences, both wrong for Buzz:
//!
//! 1. `CloseRequested` on the main window is intercepted in `lib.rs` and turned
//!    into hide-to-tray, so Cmd+W never closed a window -- it hid the whole
//!    app. That is already redundant with Cmd+H (Hide), which stays.
//! 2. macOS resolves a menu key equivalent before the webview receives any key
//!    event, so Buzz Term could never bind Cmd+W to "close this terminal tab"
//!    while the accelerator was claimed here.
//!
//! So this module builds the standard menu minus both `close_window` items.
//! Everything else matches `Menu::default()` deliberately: the goal is to drop
//! one item, not to design a menu.
//!
//! If hide-on-Cmd+W is ever wanted back in Buzz mode, the revisit path is to
//! restore the item and disable it while the terminal owns input (a disabled
//! item does not consume its key equivalent) -- at the cost of an owner->Rust
//! IPC hop this approach does not need.

#[cfg(target_os = "macos")]
use tauri::menu::{
    AboutMetadata, Menu, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
#[cfg(target_os = "macos")]
use tauri::AppHandle;
use tauri::{Builder, Runtime};

/// Installs Buzz's menu, replacing the `Menu::default()` Tauri would otherwise
/// auto-install. A no-op off macOS, where that default is never created and
/// the Cmd+W accelerator does not exist.
pub fn install<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    #[cfg(target_os = "macos")]
    let builder = builder.menu(build);
    builder
}

/// Mirrors `Menu::default()` with every `close_window` item omitted.
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
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            // `Menu::default()`'s File submenu holds exactly one item on macOS
            // -- close_window -- so dropping that item drops the submenu too.
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
