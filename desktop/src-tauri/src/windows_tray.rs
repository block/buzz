//! Windows notification-area icon.
//!
//! Closing the main window on Windows hides it instead of tearing the process
//! down (see the `CloseRequested` arm in `lib.rs`), so in-flight agent turns
//! keep running. This icon is the way back: left click reopens Buzz, right
//! click offers Open and Quit. It is also the only quit path once the window
//! is hidden, because Alt+F4 and the taskbar's Close both raise the same
//! `CloseRequested` event.
//!
//! macOS has its own richer status-item menu in `tray_menu`; the two are
//! mutually exclusive and share no code.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

const TRAY_ID: &str = "buzz-windows-tray";
const OPEN_BUZZ_ID: &str = "windows-tray-open-buzz";
const QUIT_ID: &str = "windows-tray-quit";

#[derive(Debug, PartialEq, Eq)]
enum TrayCommand {
    Open,
    Quit,
}

fn command_for_menu_id(id: &str) -> Option<TrayCommand> {
    match id {
        OPEN_BUZZ_ID => Some(TrayCommand::Open),
        QUIT_ID => Some(TrayCommand::Quit),
        _ => None,
    }
}

/// Whether a tray icon event is the "reopen Buzz" gesture.
///
/// Windows reports both mouse buttons as `Click`, and each press produces a
/// `Down` and an `Up`. Only a left-button release reopens the window: right
/// click belongs to the context menu, and acting on `Down` would fire twice.
fn is_open_click(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

/// Restores the main window from the tray. The window can be hidden, minimized
/// or both, so every step runs even if an earlier one fails.
pub(crate) fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to show main window from tray: {error}");
    }
    if let Err(error) = window.unminimize() {
        eprintln!("buzz-desktop: failed to restore main window from tray: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window from tray: {error}");
    }
}

/// Installs the persistent Buzz tray icon.
pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        OPEN_BUZZ_ID,
        "Open Buzz",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        QUIT_ID,
        "Quit Buzz",
        true,
        None::<&str>,
    )?)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Buzz")
        // Left click reopens Buzz instead of dropping the menu; right click
        // still opens it, which is the Windows convention.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match command_for_menu_id(event.id.as_ref()) {
            Some(TrayCommand::Open) => show_main_window(app),
            Some(TrayCommand::Quit) => app.exit(0),
            None => {}
        })
        .on_tray_icon_event(|tray, event| {
            if is_open_click(&event) {
                show_main_window(tray.app_handle());
            }
        });

    // The macOS status item uses a monochrome template bee; Windows tints
    // nothing, so a fully transparent-black icon would vanish into the
    // taskbar. Use the app icon, which is already sized for this.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{command_for_menu_id, is_open_click, TrayCommand, OPEN_BUZZ_ID, QUIT_ID, TRAY_ID};
    use tauri::{
        tray::{MouseButton, MouseButtonState, TrayIconEvent, TrayIconId},
        PhysicalPosition, Rect,
    };

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: TrayIconId::new(TRAY_ID),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
            button_state,
        }
    }

    #[test]
    fn menu_ids_map_to_their_commands() {
        assert_eq!(command_for_menu_id(OPEN_BUZZ_ID), Some(TrayCommand::Open));
        assert_eq!(command_for_menu_id(QUIT_ID), Some(TrayCommand::Quit));
    }

    #[test]
    fn unknown_menu_ids_are_ignored() {
        assert_eq!(command_for_menu_id("tray-open-buzz"), None);
        assert_eq!(command_for_menu_id(""), None);
    }

    #[test]
    fn left_button_release_reopens_buzz() {
        assert!(is_open_click(&click(
            MouseButton::Left,
            MouseButtonState::Up
        )));
    }

    #[test]
    fn right_click_is_left_to_the_context_menu() {
        assert!(!is_open_click(&click(
            MouseButton::Right,
            MouseButtonState::Up
        )));
    }

    #[test]
    fn button_press_does_not_reopen_buzz() {
        // Otherwise a single left click would fire on both Down and Up.
        assert!(!is_open_click(&click(
            MouseButton::Left,
            MouseButtonState::Down
        )));
    }

    #[test]
    fn hover_does_not_reopen_buzz() {
        assert!(!is_open_click(&TrayIconEvent::Enter {
            id: TrayIconId::new(TRAY_ID),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
        }));
    }
}
