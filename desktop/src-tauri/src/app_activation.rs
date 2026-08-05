//! Restores the main window when Buzz becomes active with nothing visible.
//!
//! macOS surfaces Dock-icon clicks as `applicationShouldHandleReopen:` (tao
//! forwards it as `RunEvent::Reopen`), but Cmd+Tab and other app-switcher
//! activations only post `NSApplicationDidBecomeActiveNotification`.
//! Without an observer for it, switching back to a hidden-to-tray Buzz
//! brings up just the menu bar and no window, where every standard macOS
//! app re-presents one. The observer shows the main window whenever the
//! app becomes active with no visible webview window, gated on
//! `initial_window::INITIAL_REVEAL_DONE` so launch-time activation cannot
//! preempt the deliberate geometry-settled first reveal.

use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use crate::initial_window::INITIAL_REVEAL_DONE;
use crate::tray_menu::show_main_window;

/// Whether an app activation should re-present the main window.
///
/// Minimized-only counts as "nothing visible" (`NSWindow.isVisible` is
/// false while miniaturized), matching how Cmd+Tab into a standard app
/// with only minimized windows de-miniaturizes one.
fn should_restore(initial_reveal_done: bool, any_window_visible: bool) -> bool {
    initial_reveal_done && !any_window_visible
}

pub fn init<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    use block2::RcBlock;
    use objc2_app_kit::NSApplicationDidBecomeActiveNotification;
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use tauri::Manager;

    let app = app_handle.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let any_visible = app
            .webview_windows()
            .values()
            .any(|window| window.is_visible().unwrap_or(false));
        if should_restore(INITIAL_REVEAL_DONE.load(Ordering::Acquire), any_visible) {
            show_main_window(&app);
        }
    });

    // SAFETY: reading the notification-name static is sound — AppKit
    // exports it for the process lifetime. For the registration: a `None`
    // object matches any poster, a `None` queue delivers on the posting
    // thread (the main thread for application lifecycle notifications),
    // and the block is sendable — it captures only a cloned `AppHandle`,
    // which is `Send + Sync`. The observer token is deliberately leaked:
    // the observer must live for the whole app lifetime.
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidBecomeActiveNotification),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(token);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_only_after_reveal_and_only_when_nothing_is_visible() {
        assert!(should_restore(true, false));
        assert!(!should_restore(true, true));
        assert!(!should_restore(false, false));
        assert!(!should_restore(false, true));
    }
}
