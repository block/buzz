//! Native companion windows for popped-out conversations and playgrounds.

use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const POPOUT_WINDOWS_CHANGED: &str = "popout-windows-changed";

#[derive(Serialize)]
pub struct PopoutWindowInfo {
    pub label: String,
    pub title: String,
}

/// Bring a companion to the front.
///
/// Tao's `set_focus` is a no-op while the window is miniaturized or not
/// visible, and overlay/hidden-title companions often stay behind the main
/// window that just handled the sidebar click. Unminimize + show first, then
/// pulse always-on-top around `set_focus`. On macOS also activate the app and
/// order the NSWindow front so a windowed (non-fullscreen) pop-out on another
/// Space or behind the main session actually rises.
fn show_and_focus(window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    activate_ns_app();

    let _ = window.unminimize();
    window.show().map_err(|error| error.to_string())?;

    #[cfg(target_os = "macos")]
    raise_nswindow(&window);

    let _ = window.set_always_on_top(true);
    let focus_result = window.set_focus();
    let _ = window.set_always_on_top(false);
    focus_result.map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn activate_ns_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    NSApplication::activate(&app);
}

/// Overlay titlebar windows sometimes ignore `makeKeyAndOrderFront` from Tao
/// while the main window still owns the click. `orderFrontRegardless` plus
/// MoveToActiveSpace covers minimized, behind, and other-Space companions.
#[cfg(target_os = "macos")]
fn raise_nswindow(window: &tauri::WebviewWindow) {
    use objc2::runtime::AnyObject;

    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    let Ok(ptr) = window.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    let ns_window = ptr as *mut AnyObject;
    unsafe {
        // NSWindowCollectionBehaviorMoveToActiveSpace = 1 << 1
        let current: usize = objc2::msg_send![ns_window, collectionBehavior];
        let behavior = current | (1 << 1);
        let _: () = objc2::msg_send![ns_window, setCollectionBehavior: behavior];
        let _: () = objc2::msg_send![ns_window, deminiaturize: std::ptr::null::<AnyObject>()];
        let _: () =
            objc2::msg_send![ns_window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        let _: () = objc2::msg_send![ns_window, orderFrontRegardless];
    }
}

pub fn emit_popout_windows_changed(app: &tauri::AppHandle) {
    if let Err(error) = app.emit(POPOUT_WINDOWS_CHANGED, ()) {
        eprintln!("buzz-desktop: failed to emit {POPOUT_WINDOWS_CHANGED}: {error}");
    }
}

#[tauri::command]
pub fn list_popout_windows(app: tauri::AppHandle) -> Vec<PopoutWindowInfo> {
    let mut rows: Vec<PopoutWindowInfo> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("popout-"))
        .map(|(label, window)| PopoutWindowInfo {
            label,
            title: window.title().unwrap_or_default(),
        })
        .collect();
    rows.sort_by(|a, b| a.title.cmp(&b.title).then(a.label.cmp(&b.label)));
    rows
}

#[tauri::command]
pub fn focus_popout_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if !label.starts_with("popout-") {
        return Err("invalid popout window label".into());
    }
    let Some(window) = app.get_webview_window(&label) else {
        return Err("popout window not found".into());
    };
    show_and_focus(window)
}

#[tauri::command]
pub async fn open_popout_window(
    app: tauri::AppHandle,
    label: String,
    title: String,
    fullscreen: Option<bool>,
) -> Result<(), String> {
    if !label.starts_with("popout-") {
        return Err("invalid popout window label".into());
    }
    if let Some(window) = app.get_webview_window(&label) {
        return show_and_focus(window);
    }

    let start_fullscreen = fullscreen.unwrap_or(false);

    let builder =
        WebviewWindowBuilder::new(&app, label.as_str(), WebviewUrl::App("index.html".into()))
            .title(title)
            .inner_size(1100.0, 780.0)
            .min_inner_size(720.0, 520.0);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(16.0, 25.0));

    #[cfg(target_os = "macos")]
    let builder = if start_fullscreen {
        builder.fullscreen(true)
    } else {
        builder
    };

    #[cfg(not(target_os = "macos"))]
    let builder = if start_fullscreen {
        builder.maximized(true)
    } else {
        builder
    };

    match builder.build() {
        Ok(_) => {
            emit_popout_windows_changed(&app);
            Ok(())
        }
        Err(error) => {
            if let Some(window) = app.get_webview_window(&label) {
                return show_and_focus(window);
            }
            Err(error.to_string())
        }
    }
}
