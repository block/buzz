//! Pure navigation and load-recovery rules for pin webviews.
//!
//! These helpers are unit-tested without a WKWebView: reuse must recover from
//! a dead first load, a blank document URL, a changed start URL, or 1×1 bounds.
//! Load failure is decided from the navigation URL (or a failed `navigate`),
//! never from inspecting page HTML.

use url::Url;

/// Minimum logical edge before a child webview is created.
/// A 1×1 first layout must not become the permanent page.
pub const MIN_PIN_WEBVIEW_EDGE: f64 = 32.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinLoadVerdict {
    Ok,
    Failed {
        message: String,
        status: Option<u16>,
    },
}

pub fn parse_optional_url(raw: Option<&str>) -> Option<Url> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    Url::parse(raw).ok()
}

pub fn is_unusable_document_url(url: Option<&Url>) -> bool {
    let Some(url) = url else {
        return true;
    };
    if url.as_str().trim().is_empty() {
        return true;
    }
    matches!(url.scheme(), "about" | "data")
}

/// Reuse an existing `pin-{id}` webview whenever it already shows a live
/// document for the same start URL, even if the site moved to another origin
/// (SSO / IdP redirects, app hosts such as Amazon WorkSpaces). Re-showing a
/// pin must resume the live page, never reset it to `start_url` — that reset
/// tore down logged-in sessions. Navigate only on a failed / blank load, an
/// explicit new start (deep link), or a lost session with a different URL.
pub fn should_navigate_existing(
    current: Option<&Url>,
    start_url: &Url,
    previous_start: Option<&Url>,
    last_load_failed: bool,
) -> bool {
    if last_load_failed {
        return true;
    }
    if is_unusable_document_url(current) {
        return true;
    }
    // Explicit new start (deep link / home change) must navigate even on-origin.
    if previous_start.is_some_and(|previous| previous != start_url) {
        return true;
    }
    // Session lost but webview still alive: honor requested start when the
    // document is not already there (deep link after manager drop).
    if previous_start.is_none() {
        return match current {
            Some(url) => url != start_url,
            None => true,
        };
    }
    current.is_none()
}

/// True for Tauri's duplicate-label error from `add_child`; the show path
/// then reuses the existing webview instead of failing the open.
pub fn is_label_exists_error(message: &str) -> bool {
    message.contains("already exists") && message.contains("label")
}

/// Invisible marker the injected change watcher appends to `document.title`.
pub const CHANGE_MARK: char = '\u{2063}';

/// Decide whether a title change is a "page changed" signal for a pin.
/// Returns `(changed, real_title_to_remember)`. A marked title is the
/// watcher's DOM ping; an unmarked title that differs from the last real
/// title (e.g. "(3) Inbox") is a change too. The first title is a baseline.
pub fn classify_title_change(last_real: Option<&str>, title: &str) -> (bool, Option<String>) {
    if title.contains(CHANGE_MARK) {
        return (true, None);
    }
    match last_real {
        None => (false, Some(title.to_string())),
        Some(previous) if previous == title => (false, None),
        Some(_) => (true, Some(title.to_string())),
    }
}

pub fn bounds_are_usable(width: f64, height: f64) -> bool {
    width >= MIN_PIN_WEBVIEW_EDGE && height >= MIN_PIN_WEBVIEW_EDGE
}

/// Classify a pin load from the navigation URL only. Do not inspect HTML.
pub fn classify_pin_load(current_url: Option<&Url>) -> PinLoadVerdict {
    if is_unusable_document_url(current_url) {
        return PinLoadVerdict::Failed {
            message: "The page did not load.".into(),
            status: None,
        };
    }
    PinLoadVerdict::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("url")
    }

    #[test]
    fn reuse_navigates_when_current_url_is_blank_or_about_blank() {
        let start = url("https://wayfinder.huladesk.com/");
        assert!(should_navigate_existing(None, &start, Some(&start), false));
        assert!(should_navigate_existing(
            Some(&url("about:blank")),
            &start,
            Some(&start),
            false,
        ));
        assert!(should_navigate_existing(
            parse_optional_url(Some("")).as_ref(),
            &start,
            Some(&start),
            false,
        ));
    }

    #[test]
    fn reuse_keeps_live_page_on_another_origin() {
        // SSO / WorkSpaces style: logged-in page lives off the start origin.
        let start = url("https://wayfinder.huladesk.com/");
        assert!(!should_navigate_existing(
            Some(&url("https://example.com/")),
            &start,
            Some(&start),
            false,
        ));
    }

    #[test]
    fn reuse_navigates_when_start_url_changed() {
        let previous = url("https://wayfinder.huladesk.com/");
        let next = url("https://wayfinder.huladesk.com/app");
        assert!(should_navigate_existing(
            Some(&previous),
            &next,
            Some(&previous),
            false,
        ));
    }

    #[test]
    fn dead_first_load_then_show_navigates_again() {
        let start = url("https://wayfinder.huladesk.com/");
        assert!(should_navigate_existing(
            Some(&start),
            &start,
            Some(&start),
            true,
        ));
    }

    #[test]
    fn reuse_keeps_in_origin_history_when_healthy() {
        let start = url("https://wayfinder.huladesk.com/");
        let current = url("https://wayfinder.huladesk.com/path");
        assert!(!should_navigate_existing(
            Some(&current),
            &start,
            Some(&start),
            false,
        ));
    }

    #[test]
    fn reuse_navigates_deep_link_when_previous_start_missing() {
        let start = url("https://wayfinder.huladesk.com/docs/guide");
        let current = url("https://wayfinder.huladesk.com/");
        assert!(should_navigate_existing(
            Some(&current),
            &start,
            None,
            false,
        ));
    }

    #[test]
    fn duplicate_label_error_is_recognised_for_reuse() {
        assert!(is_label_exists_error(
            "a webview with label `pin-wayfinder` already exists"
        ));
        assert!(!is_label_exists_error("window not found"));
    }

    #[test]
    fn title_signals_are_binary_and_ignore_the_watcher_restore() {
        // First title is the baseline.
        assert_eq!(classify_title_change(None, "Kalshi"), (false, Some("Kalshi".into())));
        // Watcher ping (marked) → changed; baseline unchanged.
        assert_eq!(classify_title_change(Some("Kalshi"), "Kalshi\u{2063}"), (true, None));
        // Watcher restoring the plain title is not a change.
        assert_eq!(classify_title_change(Some("Kalshi"), "Kalshi"), (false, None));
        // Real title change (badge count) is a change.
        assert_eq!(
            classify_title_change(Some("Inbox"), "(3) Inbox"),
            (true, Some("(3) Inbox".into()))
        );
    }

    #[test]
    fn tiny_first_bounds_are_not_usable() {
        assert!(!bounds_are_usable(1.0, 1.0));
        assert!(!bounds_are_usable(16.0, 400.0));
        assert!(bounds_are_usable(320.0, 240.0));
        assert!(bounds_are_usable(32.0, 32.0));
    }

    #[test]
    fn about_blank_is_a_visible_failure_and_https_is_ok() {
        assert_eq!(
            classify_pin_load(Some(&url("about:blank"))),
            PinLoadVerdict::Failed {
                message: "The page did not load.".into(),
                status: None,
            }
        );
        assert_eq!(
            classify_pin_load(None),
            PinLoadVerdict::Failed {
                message: "The page did not load.".into(),
                status: None,
            }
        );
        assert_eq!(
            classify_pin_load(Some(&url("https://wayfinder.huladesk.com/"))),
            PinLoadVerdict::Ok
        );
    }
}
