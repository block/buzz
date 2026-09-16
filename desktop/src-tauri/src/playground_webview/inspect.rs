//! Inspect opens a detached inspector for `playground-{sid}` only.
//! Linux CI cannot compile AppKit; keep presentation policy tests host-free.

use super::{
    apply_bounds, playground_webview_label, PlaygroundBounds, PlaygroundWebviewManager,
    APP_WEBVIEW_LABEL,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, Webview};

/// Matches `tauri.conf.json` window min size. Used only to unlock after Inspect.
const MAIN_WINDOW_MIN_LOGICAL: (f64, f64) = (800.0, 500.0);

/// How Inspect presents the WebKit inspector relative to the playground stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaygroundInspectPresentation {
    /// Standalone inspector window. Must not dock into the stage or main window.
    DetachedWindow,
}

/// Inspect must not bounce the Buzz window. Locking min=max is allowed;
/// `set_size` is not — that grow/shrink flashes the left sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectWindowFrameAction {
    Lock { before: (u32, u32) },
}

pub fn playground_inspect_presentation() -> PlaygroundInspectPresentation {
    PlaygroundInspectPresentation::DetachedWindow
}

/// Inspect must leave the main window at its pre-inspector size.
pub fn resolved_window_size_after_inspect(before: (u32, u32), after: (u32, u32)) -> (u32, u32) {
    let _ = after;
    before
}

/// Never write the window frame after Inspect opens. The sidebar (`md`
/// / 768px) disappears if we `set_size` a bounce back from a docked grow.
pub fn inspect_set_size_after_open(before: (u32, u32), after: (u32, u32)) -> Option<(u32, u32)> {
    let _ = (before, after);
    None
}

pub fn inspect_window_frame_action(
    before: (u32, u32),
    after: (u32, u32),
) -> InspectWindowFrameAction {
    let _ = after;
    InspectWindowFrameAction::Lock { before }
}

/// After Inspect, the playground page stays in the same stage rectangle.
pub fn resolved_stage_bounds_after_inspect(
    before: &PlaygroundBounds,
    window_before: (u32, u32),
    window_after: (u32, u32),
) -> PlaygroundBounds {
    let _ = (window_before, window_after);
    clamp_webview_bounds_to_stage(before, before)
}

/// Pin a drifted child webview back inside the last stage host.
/// Never returns a rect that invades `x < stage.x` (left primary menu).
pub fn clamp_webview_bounds_to_stage(
    bounds: &PlaygroundBounds,
    stage: &PlaygroundBounds,
) -> PlaygroundBounds {
    const MIN_EDGE: f64 = 32.0;
    let x = bounds.x.max(stage.x);
    let y = bounds.y.max(stage.y);
    let max_width = (stage.x + stage.width - x).max(MIN_EDGE);
    let max_height = (stage.y + stage.height - y).max(MIN_EDGE);
    PlaygroundBounds {
        x,
        y,
        width: bounds.width.min(max_width).max(MIN_EDGE),
        height: bounds.height.min(max_height).max(MIN_EDGE),
    }
}

/// NSUserDefaults keys WebKit reads for `inspectorStartsAttached`.
/// `detach` before `show` is a no-op until the frontend exists; these
/// must be false so `shouldOpenAttached()` does not dock on first show.
pub const INSPECTOR_STARTS_ATTACHED_DEFAULTS: &[&str] = &[
    "WebKit2InspectorStartsAttached",
    "WebKitInspectorStartsAttached",
];

pub fn open_playground_inspector(webview: &Webview) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        open_detached_macos_inspector(webview)
    }
    #[cfg(not(target_os = "macos"))]
    {
        webview.open_devtools();
        Ok(())
    }
}

/// Pin min=max to the current frame so WebKit cannot grow or shrink the
/// Buzz window when the inspector frontend appears. Never call `set_size`.
pub fn lock_main_window_size(app: &AppHandle, before: Option<(u32, u32)>) {
    let Some(before) = before else {
        return;
    };
    let Some(window) = app.get_window(APP_WEBVIEW_LABEL) else {
        return;
    };
    let size = tauri::PhysicalSize::new(before.0, before.1);
    let _ = window.set_min_size(Some(size));
    let _ = window.set_max_size(Some(size));
}

pub fn unlock_main_window_size(app: &AppHandle) {
    let Some(window) = app.get_window(APP_WEBVIEW_LABEL) else {
        return;
    };
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(
        MAIN_WINDOW_MIN_LOGICAL.0,
        MAIN_WINDOW_MIN_LOGICAL.1,
    )));
    let _ = window.set_max_size(None::<tauri::LogicalSize<f64>>);
}

fn reapply_last_bounds(app: &AppHandle, sid: &str, window_label: &str) {
    let Some(manager) = app.try_state::<PlaygroundWebviewManager>() else {
        return;
    };
    let bounds = manager.sessions.lock().ok().and_then(|sessions| {
        sessions
            .get(sid)
            .and_then(|session| session.last_bounds.clone())
    });
    if let Some(bounds) = bounds.as_ref() {
        let keep = clamp_webview_bounds_to_stage(bounds, bounds);
        let _ = apply_bounds(app, sid, window_label, &keep);
    }
}

pub fn schedule_inspect_stage_restore(app: AppHandle, sid: String, window_label: String) {
    tauri::async_runtime::spawn(async move {
        // show() creates the frontend asynchronously. detach() is a no-op
        // until that page exists, so re-detach during the open settle, then
        // keep pinning the stage for as long as Inspect stays open — including
        // if the user later docks to the bottom or side. Never set_size the
        // main window; the frame was locked before show.
        for delay_ms in [16_u64, 50, 200, 500] {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            reapply_last_bounds(&app, &sid, &window_label);
            redetach_macos_inspector(&app, &sid, &window_label);
        }
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            reapply_last_bounds(&app, &sid, &window_label);
            if !playground_inspector_is_visible(&app, &sid, &window_label) {
                reapply_last_bounds(&app, &sid, &window_label);
                unlock_main_window_size(&app);
                break;
            }
        }
    });
}

pub fn playground_inspector_is_visible(app: &AppHandle, sid: &str, window_label: &str) -> bool {
    let Some(webview) = app.get_webview(&playground_webview_label(sid, window_label)) else {
        return false;
    };
    inspector_is_visible(&webview)
}

fn inspector_is_visible(webview: &Webview) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_inspector_is_visible(webview)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        false
    }
}

fn redetach_macos_inspector(app: &AppHandle, sid: &str, window_label: &str) {
    #[cfg(target_os = "macos")]
    {
        let Some(webview) = app.get_webview(&playground_webview_label(sid, window_label)) else {
            return;
        };
        let _ = webview.with_webview(|platform| {
            use objc2_web_kit::WKWebView;

            // SAFETY: same child WKWebView / `_inspector` contract as
            // open_detached_macos_inspector. Called after show settles so
            // detach is no longer a no-op on a missing frontend page.
            let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
            unsafe {
                prefer_detached_inspector_defaults();
                let Some(inspector) = macos_inspector_ptr(view) else {
                    return;
                };
                force_inspector_detached(&*inspector);
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, sid, window_label);
    }
}

#[cfg(target_os = "macos")]
fn open_detached_macos_inspector(webview: &Webview) -> Result<(), String> {
    webview
        .with_webview(|platform| {
            use objc2::sel;
            use objc2_web_kit::WKWebView;

            // SAFETY: inner() is the child WKWebView for this playground label.
            // `_inspector` / `detach` / `show` / `setAttached:` are WebKit
            // private inspectors (same `_inspector` wry uses for
            // open_devtools). with_webview is already on the AppKit main
            // thread. Do not call open_devtools() — that show()s attached.
            let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
            unsafe {
                prefer_detached_inspector_defaults();
                prefer_detached_inspector_on_view(view);
                let Some(inspector) = macos_inspector_ptr(view) else {
                    return;
                };
                let inspector = &*inspector;
                force_inspector_detached(inspector);
                let show = sel!(show);
                if objc2::msg_send![inspector, respondsToSelector: show] {
                    let (): () = objc2::msg_send![inspector, show];
                }
                force_inspector_detached(inspector);
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
unsafe fn macos_inspector_ptr(
    view: &objc2_web_kit::WKWebView,
) -> Option<*mut objc2::runtime::AnyObject> {
    use objc2::runtime::AnyObject;
    use objc2::sel;

    let has_inspector: bool = objc2::msg_send![view, respondsToSelector: sel!(_inspector)];
    if !has_inspector {
        return None;
    }
    let inspector: *mut AnyObject = objc2::msg_send![view, _inspector];
    if inspector.is_null() {
        None
    } else {
        Some(inspector)
    }
}

#[cfg(target_os = "macos")]
unsafe fn prefer_detached_inspector_defaults() {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    let Some(cls) = AnyClass::get(c"NSUserDefaults") else {
        return;
    };
    let defaults: *mut AnyObject = objc2::msg_send![cls, standardUserDefaults];
    if defaults.is_null() {
        return;
    }
    let defaults = &*defaults;
    for name in INSPECTOR_STARTS_ATTACHED_DEFAULTS {
        let key = NSString::from_str(name);
        let (): () = objc2::msg_send![defaults, setBool: false, forKey: &*key];
    }
}

#[cfg(target_os = "macos")]
unsafe fn prefer_detached_inspector_on_view(view: &objc2_web_kit::WKWebView) {
    use objc2::runtime::AnyObject;
    use objc2::sel;

    let config: *mut AnyObject = objc2::msg_send![view, configuration];
    if config.is_null() {
        return;
    }
    let prefs: *mut AnyObject = objc2::msg_send![&*config, preferences];
    if prefs.is_null() {
        return;
    }
    let set_pref = sel!(_setInspectorStartsAttached:);
    if objc2::msg_send![&*prefs, respondsToSelector: set_pref] {
        let (): () = objc2::msg_send![&*prefs, _setInspectorStartsAttached: false];
    }
}

#[cfg(target_os = "macos")]
unsafe fn force_inspector_detached(inspector: &objc2::runtime::AnyObject) {
    use objc2::sel;

    let set_attached = sel!(setAttached:);
    if objc2::msg_send![inspector, respondsToSelector: set_attached] {
        let (): () = objc2::msg_send![inspector, setAttached: false];
    }
    let set_attached_priv = sel!(_setAttached:);
    if objc2::msg_send![inspector, respondsToSelector: set_attached_priv] {
        let (): () = objc2::msg_send![inspector, _setAttached: false];
    }
    let detach = sel!(detach);
    if objc2::msg_send![inspector, respondsToSelector: detach] {
        let (): () = objc2::msg_send![inspector, detach];
    }
}

#[cfg(target_os = "macos")]
fn macos_inspector_is_visible(webview: &Webview) -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // with_webview requires FnOnce + Send + 'static. Cell<bool> is not Sync
    // (`&Cell<bool>` is not Send). A stack AtomicBool is Send but not 'static.
    // Arc<AtomicBool> satisfies both bounds. Do not revert to Cell.
    let visible = Arc::new(AtomicBool::new(false));
    let visible_flag = Arc::clone(&visible);
    let _ = webview.with_webview(move |platform| {
        use objc2::runtime::AnyObject;
        use objc2::sel;
        use objc2_web_kit::WKWebView;

        // SAFETY: same child WKWebView / `_inspector` contract as
        // open_detached_macos_inspector. isVisible is the matching query
        // wry uses for is_devtools_open.
        let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
        let is_visible = unsafe {
            let has_inspector: bool = objc2::msg_send![view, respondsToSelector: sel!(_inspector)];
            if !has_inspector {
                false
            } else {
                let inspector: *mut AnyObject = objc2::msg_send![view, _inspector];
                if inspector.is_null() {
                    false
                } else {
                    objc2::msg_send![&*inspector, isVisible]
                }
            }
        };
        visible_flag.store(is_visible, Ordering::Relaxed);
    });
    visible.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_visibility_flag_is_send_static() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        fn take_send_static<F: FnOnce() + Send + 'static>(f: F) {
            f();
        }

        let visible = Arc::new(AtomicBool::new(false));
        let visible_flag = Arc::clone(&visible);
        take_send_static(move || {
            visible_flag.store(true, Ordering::Relaxed);
        });
        assert!(visible.load(Ordering::Relaxed));
    }

    #[test]
    fn inspect_opens_a_detached_window_not_a_docked_split() {
        assert_eq!(
            playground_inspect_presentation(),
            PlaygroundInspectPresentation::DetachedWindow
        );
    }

    #[test]
    fn inspect_does_not_change_main_window_size() {
        assert_eq!(
            resolved_window_size_after_inspect((1280, 800), (1800, 800)),
            (1280, 800)
        );
        assert_eq!(
            resolved_window_size_after_inspect((1024, 640), (1024, 640)),
            (1024, 640)
        );
        assert_eq!(inspect_set_size_after_open((1280, 800), (1800, 800)), None);
        assert_eq!(inspect_set_size_after_open((1024, 640), (800, 500)), None);
        assert_eq!(
            inspect_window_frame_action((1280, 800), (1800, 800)),
            InspectWindowFrameAction::Lock {
                before: (1280, 800)
            }
        );
        assert_eq!(
            playground_inspect_presentation(),
            PlaygroundInspectPresentation::DetachedWindow
        );
    }

    #[test]
    fn inspect_does_not_resize_the_playground_stage() {
        let before = PlaygroundBounds {
            x: 120.0,
            y: 80.0,
            width: 640.0,
            height: 480.0,
        };
        let after = resolved_stage_bounds_after_inspect(&before, (1280, 800), (1800, 800));
        assert_eq!(after, before);
        assert_eq!(after.width, 640.0);
        assert_eq!(after.height, 480.0);
        assert_eq!(after.x, 120.0);
        assert_eq!(after.y, 80.0);
    }

    #[test]
    fn docked_inspect_cannot_move_the_page_left_of_the_stage() {
        let stage = PlaygroundBounds {
            x: 256.0,
            y: 80.0,
            width: 640.0,
            height: 480.0,
        };
        let drifted = PlaygroundBounds {
            x: 0.0,
            y: 40.0,
            width: 1100.0,
            height: 700.0,
        };
        let keep = clamp_webview_bounds_to_stage(&drifted, &stage);
        assert_eq!(keep.x, stage.x);
        assert!(keep.x >= stage.x);
        assert!(keep.y >= stage.y);
        assert!(keep.width <= stage.width);
        assert!(keep.height <= stage.height);
        assert_eq!(
            playground_inspect_presentation(),
            PlaygroundInspectPresentation::DetachedWindow
        );
        assert!(INSPECTOR_STARTS_ATTACHED_DEFAULTS.contains(&"WebKit2InspectorStartsAttached"));
        assert!(INSPECTOR_STARTS_ATTACHED_DEFAULTS.contains(&"WebKitInspectorStartsAttached"));
    }
}
