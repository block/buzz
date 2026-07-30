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
//!   (`swipeWithEvent:`): `deltaX > 0` is back, `deltaX < 0` is forward,
//!   the convention Safari follows. Trackpad two-finger swipes arrive this
//!   way, and many mouse drivers synthesize the same gesture for the
//!   back/forward buttons instead of button-3/4 events.
//!
//! Windows/Linux are unaffected: WebView2 and WebKitGTK deliver X1/X2 as
//! ordinary DOM mouse events, which the frontend `mouseup` listener handles.

/// Maps an `otherMouseUp` button number to a navigation direction.
/// Buttons 3 and 4 are X1 (back) and X2 (forward).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn direction_for_swipe(delta_x: f64) -> Option<&'static str> {
    if delta_x > 0.0 {
        Some("back")
    } else if delta_x < 0.0 {
        Some("forward")
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
pub fn init(app_handle: &tauri::AppHandle) {
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
                    let _ = app.emit("mouse-nav", direction);
                    // Swallow the event: nothing downstream should also act on it.
                    return std::ptr::null_mut();
                }
            }
            NSEventType::Swipe => {
                if let Some(direction) = direction_for_swipe(ev.deltaX()) {
                    let _ = app.emit("mouse-nav", direction);
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

#[cfg(not(target_os = "macos"))]
pub fn init(_app_handle: &tauri::AppHandle) {}

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
