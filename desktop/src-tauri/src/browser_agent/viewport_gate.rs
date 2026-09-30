//! Drive/snapshot/record viewport readiness.
//!
//! Hidden/parked WKWebViews do not paint (`takeSnapshot` blanks/times out) and
//! can lay out near 0×0. Drive keeps a **background paint** surface: show the
//! native child at last usable size **offscreen** (no Stage mount, no window
//! focus). Observe snapshot still refuses without auto-unpark. Cold / missing
//! native views still refuse.

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use super::grant::{BrowserAgentGrant, BrowserAgentMode};
use super::{
    playground_surface_id_from_label, set_webview_hidden, webview_is_hidden, BrowserAgentState,
};
use crate::playground_webview;

/// Minimum logical CSS edge for click/snapshot/record to be trustworthy.
pub const MIN_USABLE_VIEWPORT_EDGE: f64 = 64.0;

/// Far off the content view so a shown WKWebView can paint without covering UI.
pub const BACKGROUND_CAPTURE_ORIGIN: f64 = -20_000.0;

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

/// Offscreen x/y/w/h for background paint (unit-tested; applied via playground).
pub fn background_capture_xywh(width: f64, height: f64) -> (f64, f64, f64, f64) {
    let w = width.max(MIN_USABLE_VIEWPORT_EDGE);
    let h = height.max(MIN_USABLE_VIEWPORT_EDGE);
    (
        BACKGROUND_CAPTURE_ORIGIN - w.max(0.0),
        BACKGROUND_CAPTURE_ORIGIN - h.max(0.0),
        w,
        h,
    )
}

pub fn is_background_paint(state: &BrowserAgentState, label: &str) -> bool {
    state
        .background_paint
        .lock()
        .map(|set| set.contains(label))
        .unwrap_or(false)
}

pub fn set_background_paint(state: &BrowserAgentState, label: &str, active: bool) {
    let Ok(mut set) = state.background_paint.lock() else {
        return;
    };
    if active {
        set.insert(label.to_string());
    } else {
        set.remove(label);
    }
}

/// Clear background-paint hold when the Stage actually shows this label.
pub fn clear_background_paint(state: &BrowserAgentState, label: &str) {
    set_background_paint(state, label, false);
}

/// Drive should keep a paint-ready surface while Stage is parked.
pub fn should_preserve_drive_paint(state: &BrowserAgentState, label: &str) -> bool {
    if state.drive_records.is_recording(label) {
        return true;
    }
    state
        .grants
        .get(label)
        .is_some_and(|g| matches!(g.mode, BrowserAgentMode::Drive))
}

fn resolve_usable_size(app: &AppHandle, label: &str) -> Result<(f64, f64), String> {
    if let Some((w, h)) = webview_layout_size(app, label) {
        if layout_is_usable(w, h) {
            return Ok((w, h));
        }
    }
    let sid = playground_surface_id_from_label(label)
        .ok_or_else(|| format!("webview {label} is not a playground label"))?;
    if let Some(b) = playground_webview::last_usable_bounds_for_sid(app, &sid) {
        if layout_is_usable(b.width, b.height) {
            return Ok((b.width, b.height));
        }
    }
    let (w, h) = webview_layout_size(app, label).unwrap_or((0.0, 0.0));
    Err(viewport_unavailable_message(true, w, h))
}

/// Ask the UI to mount/show the playground card and best-effort `show()` the
/// native WKWebView with last full bounds. Prefer
/// [`ensure_drive_viewport_usable`] / background paint for Drive tools — this
/// steals focus via Stage mount and is kept for explicit theater unpark only.
#[allow(dead_code)]
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
    clear_background_paint(state, label);
    set_webview_hidden(app, state, label, false);
}

fn drive_viewport_ready(app: &AppHandle, state: &BrowserAgentState, label: &str) -> bool {
    let Some((w, h)) = webview_layout_size(app, label) else {
        return false;
    };
    if !layout_is_usable(w, h) {
        return false;
    }
    // Stage visible, or background offscreen paint hold.
    !webview_is_hidden(state, label) || is_background_paint(state, label)
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

/// Show the playground WKWebView offscreen at a usable size so Drive
/// snapshot/record/actions can paint without mounting Stage or focusing.
///
/// When the Stage is already user-visible, only heals on-screen bounds (no
/// offscreen move). Leaves `webviewHidden` unchanged so parked grants stay
/// parked in the agent mirror.
pub fn prepare_background_drive_viewport(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<(), String> {
    if app.get_webview(label).is_none() {
        return Err(viewport_unavailable_message(true, 0.0, 0.0));
    }
    let (width, height) = resolve_usable_size(app, label)?;

    if !webview_is_hidden(state, label) {
        // Stage is up — heal last full bounds in place; no offscreen move.
        let _ = playground_webview::ensure_playground_webview_shown(app, label);
        clear_background_paint(state, label);
        return Ok(());
    }

    playground_webview::apply_background_capture_bounds(app, label, width, height)?;
    let webview = app
        .get_webview(label)
        .ok_or_else(|| viewport_unavailable_message(true, 0.0, 0.0))?;
    webview.show().map_err(|error| error.to_string())?;
    set_background_paint(state, label, true);
    Ok(())
}

/// Drive path: prepare offscreen paint (no Stage / focus), wait for usable
/// layout, else clear error. Does **not** emit `browser-agent-ensure-visible`.
pub async fn ensure_drive_viewport_usable(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<(), String> {
    if drive_viewport_ready(app, state, label) {
        return Ok(());
    }
    if let Err(e) = prepare_background_drive_viewport(app, state, label) {
        return Err(e);
    }
    for _ in 0..10 {
        if drive_viewport_ready(app, state, label) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let hidden = webview_is_hidden(state, label) && !is_background_paint(state, label);
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

/// On Drive grant: warm an offscreen paint surface (no Stage focus steal).
pub fn maybe_ensure_visible_on_drive_grant(
    app: &AppHandle,
    state: &BrowserAgentState,
    grant: &BrowserAgentGrant,
) {
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return;
    }
    if drive_viewport_ready(app, state, &grant.webview_label) {
        return;
    }
    if let Err(e) = prepare_background_drive_viewport(app, state, &grant.webview_label) {
        eprintln!(
            "buzz-desktop: background Drive viewport warm failed ({}): {e}",
            grant.webview_label
        );
    }
}

/// Park / hide path: for Drive grants + active recordings, keep paint alive
/// offscreen instead of `hide()` (which breaks `takeSnapshot`).
pub fn park_preserving_drive_paint(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<(), String> {
    if !should_preserve_drive_paint(state, label) {
        let webview = app
            .get_webview(label)
            .ok_or_else(|| format!("webview {label} is not open"))?;
        webview.hide().map_err(|error| error.to_string())?;
        clear_background_paint(state, label);
        set_webview_hidden(app, state, label, true);
        return Ok(());
    }

    // Mark parked first so prepare takes the offscreen path.
    set_webview_hidden(app, state, label, true);
    prepare_background_drive_viewport(app, state, label)?;
    set_background_paint(state, label, true);
    Ok(())
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

    #[test]
    fn background_capture_xywh_keeps_size_offscreen() {
        let (x, y, w, h) = background_capture_xywh(1280.0, 800.0);
        assert_eq!(w, 1280.0);
        assert_eq!(h, 800.0);
        assert!(x <= BACKGROUND_CAPTURE_ORIGIN);
        assert!(y <= BACKGROUND_CAPTURE_ORIGIN);
        assert!(x < -64.0 && y < -64.0);
        // Must not look like keeper 64×64 park.
        assert!(w > 64.0 && h > 64.0);
    }

    #[test]
    fn background_paint_set_tracks_labels() {
        let state = BrowserAgentState::default();
        assert!(!is_background_paint(&state, "playground-a"));
        set_background_paint(&state, "playground-a", true);
        assert!(is_background_paint(&state, "playground-a"));
        clear_background_paint(&state, "playground-a");
        assert!(!is_background_paint(&state, "playground-a"));
    }

    #[test]
    fn should_preserve_drive_paint_for_drive_grant_only() {
        let state = BrowserAgentState::default();
        assert!(!should_preserve_drive_paint(&state, "playground-a"));
        let grant = BrowserAgentGrant {
            webview_label: "playground-a".into(),
            surface: super::super::grant::BrowserAgentSurface::Playground,
            surface_id: "a".into(),
            agent_id: "agent".into(),
            agent_pubkey: "aa".into(),
            channel_id: "ch".into(),
            thread_root: None,
            mode: BrowserAgentMode::Observe,
            user_has_control: false,
            created_at_ms: 1,
        };
        state.grants.set(grant.clone(), false).unwrap();
        assert!(!should_preserve_drive_paint(&state, "playground-a"));
        let mut drive = grant;
        drive.mode = BrowserAgentMode::Drive;
        state.grants.set(drive, true).unwrap();
        assert!(should_preserve_drive_paint(&state, "playground-a"));
    }
}
