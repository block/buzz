//! Refuse Drive/snapshot/record when the playground WKWebView is parked/hidden
//! or laid out at an unusable size. Drive auto-unparks (emit + native show);
//! Observe snapshot refuses without auto-unpark.

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use super::grant::{BrowserAgentGrant, BrowserAgentMode};
use super::{playground_surface_id_from_label, set_webview_hidden, webview_is_hidden, BrowserAgentState};
use crate::playground_webview;

/// Minimum logical CSS edge for click/snapshot/record to be trustworthy.
pub const MIN_USABLE_VIEWPORT_EDGE: f64 = 64.0;

pub fn viewport_unavailable_message(hidden: bool, width: f64, height: f64) -> String {
    if hidden {
        "Browser card is hidden/parked — open the Playground browser (Browsers → Open or the card) so Drive/snapshot/record can use a real viewport.".to_string()
    } else {
        format!(
            "Browser viewport is unusable ({width:.0}×{height:.0}). Open the Playground browser card so the page can lay out at a real size, then retry."
        )
    }
}

pub fn webview_layout_size(app: &AppHandle, label: &str) -> Option<(f64, f64)> {
    let webview = app.get_webview(label)?;
    playground_webview::webview_logical_size(&webview)
}

pub fn layout_is_usable(width: f64, height: f64) -> bool {
    width.is_finite()
        && height.is_finite()
        && width >= MIN_USABLE_VIEWPORT_EDGE
        && height >= MIN_USABLE_VIEWPORT_EDGE
}

/// Ask the UI to mount/show the playground card and best-effort `show()` the
/// native WKWebView with last full bounds.
pub fn request_ensure_visible(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    reason: &str,
) {
    let surface_id = state
        .grants
        .get(label)
        .map(|g| g.surface_id.clone())
        .or_else(|| playground_surface_id_from_label(label));
    let Some(surface_id) = surface_id else {
        return;
    };
    let _ = app.emit(
        "browser-agent-ensure-visible",
        json!({
            "surfaceId": surface_id,
            "webviewLabel": label,
            "reason": reason,
        }),
    );
    if let Err(e) = playground_webview::ensure_playground_webview_shown(app, label) {
        eprintln!("buzz-desktop: ensure playground visible failed ({label}): {e}");
        return;
    }
    set_webview_hidden(app, state, label, false);
}

fn viewport_ready(app: &AppHandle, state: &BrowserAgentState, label: &str) -> bool {
    if webview_is_hidden(state, label) {
        return false;
    }
    match webview_layout_size(app, label) {
        Some((w, h)) => layout_is_usable(w, h),
        None => false,
    }
}

/// Drive path: auto-unpark, briefly wait for a usable layout, else clear error.
pub async fn ensure_drive_viewport_usable(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<(), String> {
    if viewport_ready(app, state, label) {
        return Ok(());
    }
    request_ensure_visible(app, state, label, "drive");
    for _ in 0..10 {
        if viewport_ready(app, state, label) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let hidden = webview_is_hidden(state, label);
    let (w, h) = webview_layout_size(app, label).unwrap_or((0.0, 0.0));
    Err(viewport_unavailable_message(hidden, w, h))
}

/// Observe snapshot / any path that must not surprise-unpark: refuse when bad.
pub fn require_usable_viewport(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<(), String> {
    if viewport_ready(app, state, label) {
        return Ok(());
    }
    let hidden = webview_is_hidden(state, label);
    let (w, h) = webview_layout_size(app, label).unwrap_or((0.0, 0.0));
    Err(viewport_unavailable_message(hidden, w, h))
}

/// On Drive grant: ask UI to show the card (theater). Never fails the grant.
pub fn maybe_ensure_visible_on_drive_grant(
    app: &AppHandle,
    state: &BrowserAgentState,
    grant: &BrowserAgentGrant,
) {
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return;
    }
    if viewport_ready(app, state, &grant.webview_label) {
        return;
    }
    request_ensure_visible(app, state, &grant.webview_label, "drive-grant");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_layout_requires_min_edge() {
        assert!(layout_is_usable(1280.0, 800.0));
        assert!(layout_is_usable(64.0, 64.0));
        assert!(!layout_is_usable(63.0, 800.0));
        assert!(!layout_is_usable(800.0, 0.0));
        assert!(!layout_is_usable(f64::NAN, 800.0));
    }

    #[test]
    fn unavailable_message_mentions_park_and_size() {
        let hidden = viewport_unavailable_message(true, 0.0, 0.0);
        assert!(hidden.contains("hidden/parked"), "{hidden}");
        assert!(hidden.contains("Open"), "{hidden}");
        let sized = viewport_unavailable_message(false, 0.0, 0.0);
        assert!(sized.contains("0×0"), "{sized}");
    }
}
