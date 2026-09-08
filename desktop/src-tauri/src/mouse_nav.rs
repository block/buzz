//! Native macOS handler for back/forward navigation inputs (mouse X1/X2
//! buttons and horizontal swipe gestures).
//!
//! WKWebView never delivers these inputs to the web content layer, so a DOM
//! listener can't see them (Safari itself handles them natively in the app
//! layer, not in the page). This module installs an NSEvent local monitor
//! and emits a `mouse-nav` Tauri event that `useBackForwardControls` acts on
//! in the frontend. Two event shapes map to navigation:
//!
//! - `otherMouseUp` with button 3/4 — mice whose X1/X2 buttons reach the app
//!   as plain mouse buttons.
//! - `swipe` with a horizontal delta — AppKit's page-swipe gesture
//!   (`swipeWithEvent:`): `deltaX > 0` is back, `deltaX < 0` is forward.
//!   Sent by mouse drivers that synthesize a page-swipe gesture for the
//!   back/forward buttons instead of button-3/4 events (the hardware this
//!   was verified on). Stock Apple trackpad and Magic Mouse swipes arrive
//!   as phased scroll-wheel events instead, which this module does not
//!   handle — that path (`ScrollWheel` + `trackSwipeEventWithOptions:`,
//!   which also needs scroll-edge detection) is a follow-up.
//!
//! Compiled macOS-only (via `tray_menu`). Non-macOS X1/X2 behavior is left
//! to the underlying webview.

/// Maps an `otherMouseUp` button number to a navigation direction.
/// Buttons 3 and 4 are X1 (back) and X2 (forward).
fn direction_for_button(button: isize) -> Option<&'static str> {
    match button {
        3 => Some("back"),
        4 => Some("forward"),
        _ => None,
    }
}

/// Maps a swipe gesture's horizontal delta to a navigation direction,
/// following the AppKit `swipeWithEvent:` convention: positive is back,
/// negative is forward. A swipe arrives as a begin/end pair and only the
/// end event carries the direction, so `deltaX == 0` maps to `None`.
fn direction_for_swipe(delta_x: f64) -> Option<&'static str> {
    if delta_x > 0.0 {
        Some("back")
    } else if delta_x < 0.0 {
        Some("forward")
    } else {
        None
    }
}

pub fn init<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};
    use tauri::Emitter;

    let app = app_handle.clone();
    let block = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: the monitor hands us a valid NSEvent for the matched mask.
        let ev = unsafe { event.as_ref() };

        match ev.r#type() {
            NSEventType::OtherMouseUp => {
                if let Some(direction) = direction_for_button(ev.buttonNumber()) {
                    // Emit to the main window explicitly instead of
                    // broadcasting (`emit`) so navigation stays scoped if
                    // multi-window ever lands. "main" is the default label
                    // for the single configured window (see deep_link.rs).
                    let _ = app.emit_to("main", "mouse-nav", direction);
                    // Swallow the release: nothing downstream should also act
                    // on it. The matching press deliberately passes through:
                    // WKWebView never delivers X1/X2 to the page, so the
                    // unmatched down is inert, and swallowing presses risks
                    // interfering with AppKit behaviors keyed off mouse-down.
                    return std::ptr::null_mut();
                }
            }
            NSEventType::Swipe => {
                if let Some(direction) = direction_for_swipe(ev.deltaX()) {
                    let _ = app.emit_to("main", "mouse-nav", direction);
                }
                // Pass swipes through: nothing else navigates on them, and
                // swallowing mid-gesture events could confuse AppKit's
                // gesture tracking.
            }
            _ => {}
        }

        event.as_ptr()
    });

    // SAFETY: the block returns either null or the pointer it was given, both
    // valid per the monitor contract. The returned monitor token is
    // deliberately leaked: the monitor must live for the whole app lifetime.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::OtherMouseUp | NSEventMask::Swipe,
            &block,
        )
    };

    if let Some(monitor) = monitor {
        std::mem::forget(monitor);
    } else {
        eprintln!("buzz-desktop: mouse-nav: failed to install NSEvent monitor");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_3_is_back() {
        assert_eq!(direction_for_button(3), Some("back"));
    }

    #[test]
    fn button_4_is_forward() {
        assert_eq!(direction_for_button(4), Some("forward"));
    }

    #[test]
    fn other_buttons_do_not_navigate() {
        for button in [0, 1, 2, 5, -1] {
            assert_eq!(direction_for_button(button), None);
        }
    }

    #[test]
    fn positive_swipe_delta_is_back() {
        assert_eq!(direction_for_swipe(1.0), Some("back"));
        assert_eq!(direction_for_swipe(0.5), Some("back"));
    }

    #[test]
    fn negative_swipe_delta_is_forward() {
        assert_eq!(direction_for_swipe(-1.0), Some("forward"));
        assert_eq!(direction_for_swipe(-0.5), Some("forward"));
    }

    #[test]
    fn zero_delta_swipe_begin_event_is_ignored() {
        assert_eq!(direction_for_swipe(0.0), None);
    }
}
