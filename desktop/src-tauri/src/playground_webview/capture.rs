//! Snapshot the playground child webview itself. Never display-capture.

#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::time::Duration;

use tauri::{LogicalSize, Webview};

use super::{inspect_target_is_safe, playground_label, PlaygroundScreenshotResult};

/// Capture backend name. Tests assert this is not `screencapture`.
pub const PLAYGROUND_CAPTURE_BACKEND: &str = "webview-snapshot";

/// Cap full-document snapshot height (CSS px) so huge pages do not explode memory.
pub const FULL_PAGE_MAX_HEIGHT_CSS_PX: f64 = 6000.0;

/// Optional bitmap width when callers request `full_page` (not used for Browsers
/// list thumbs — those use default viewport-native snapshots).
pub const FULL_PAGE_SNAPSHOT_WIDTH_PX: f64 = 280.0;

/// JS that returns JSON `{w,h}` for the scrollable document (CSS px).
pub const DOCUMENT_CONTENT_SIZE_JS: &str = r#"(function(){
  try {
    var b = document.body, e = document.documentElement;
    var w = Math.max(
      (b && b.scrollWidth) || 0,
      (e && e.scrollWidth) || 0,
      (e && e.clientWidth) || 0,
      (window.innerWidth) || 0
    );
    var h = Math.max(
      (b && b.scrollHeight) || 0,
      (e && e.scrollHeight) || 0,
      (e && e.clientHeight) || 0,
      (window.innerHeight) || 0
    );
    return JSON.stringify({w: w || 0, h: h || 0});
  } catch (err) {
    return JSON.stringify({w: 0, h: 0});
  }
})()"#;

/// JS probe for Drive screen settle: document complete + network quiet + size.
pub const PAGE_READY_PROBE_JS: &str = r#"(function(){
  try {
    var ready = document.readyState || '';
    var now = performance.now();
    var resources = performance.getEntriesByType('resource') || [];
    var recent = 0;
    for (var i = 0; i < resources.length; i++) {
      var end = resources[i].responseEnd || 0;
      if (end > 0 && (now - end) < 500) recent++;
    }
    var body = document.body;
    var textLen = body ? String(body.innerText || '').trim().length : 0;
    var e = document.documentElement;
    var w = Math.max(
      (e && e.clientWidth) || 0,
      (window.innerWidth) || 0
    );
    var h = Math.max(
      (e && e.scrollHeight) || 0,
      (body && body.scrollHeight) || 0,
      (window.innerHeight) || 0
    );
    return JSON.stringify({
      ready: ready,
      recent: recent,
      textLen: textLen,
      w: w || 0,
      h: h || 0,
      href: String(location.href || '')
    });
  } catch (err) {
    return JSON.stringify({
      ready: 'error',
      recent: 99,
      textLen: 0,
      w: 0,
      h: 0,
      href: ''
    });
  }
})()"#;

/// Scroll to origin before a Drive viewport snapshot (full visible frame).
pub const PREPARE_VIEWPORT_JS: &str = r#"(function(){
  try {
    window.scrollTo(0, 0);
    if (document.documentElement) document.documentElement.scrollTop = 0;
    if (document.body) document.body.scrollTop = 0;
    return 'ok';
  } catch (err) {
    return String(err);
  }
})()"#;

/// Parsed [`PAGE_READY_PROBE_JS`] payload.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageReadyProbe {
    pub ready: String,
    pub recent: u64,
    pub text_len: u64,
    pub w: f64,
    pub h: f64,
    pub href: String,
}

impl PageReadyProbe {
    /// Document complete, no recent network, and a usable layout size.
    pub fn is_settled(&self) -> bool {
        self.ready == "complete"
            && self.recent == 0
            && self.w.is_finite()
            && self.h.is_finite()
            && self.w >= 32.0
            && self.h >= 32.0
            && !self.href.is_empty()
            && self.href != "about:blank"
    }
}

/// Parse [`PAGE_READY_PROBE_JS`] JSON (unit-tested).
pub fn parse_page_ready_probe(raw: &str) -> Result<PageReadyProbe, String> {
    serde_json::from_str(raw.trim())
        .map_err(|error| format!("page ready probe parse failed: {error}"))
}

/// Clamp document height for full-page thumbs: at least the viewport, at most the cap.
pub fn clamp_full_page_capture_height(content_height: f64, viewport_height: f64) -> f64 {
    content_height
        .max(viewport_height)
        .min(FULL_PAGE_MAX_HEIGHT_CSS_PX)
        .max(1.0)
}

/// Target WKWebView logical size for a full-document thumb capture.
/// Keeps viewport **width** (avoids width-driven reflow) and expands **height**
/// to the capped scroll height.
pub fn full_page_capture_logical_size(
    viewport_width: f64,
    viewport_height: f64,
    content_height: f64,
) -> (f64, f64) {
    (
        viewport_width.max(1.0),
        clamp_full_page_capture_height(content_height, viewport_height),
    )
}

pub fn playground_screenshot_target(sid: &str) -> Result<String, String> {
    if sid.is_empty() || sid == "main" {
        return Err("screenshot must target the playground webview".into());
    }
    let label = playground_label(sid);
    if !inspect_target_is_safe(&label) {
        return Err("screenshot must target the playground webview".into());
    }
    Ok(label)
}

pub fn capture_playground_png(
    webview: &Webview,
    label: &str,
    full_page: bool,
) -> Result<PlaygroundScreenshotResult, String> {
    if !inspect_target_is_safe(label) {
        return Err("screenshot must target the playground webview".into());
    }
    if webview.label() != label {
        return Err("screenshot must target the playground webview".into());
    }
    let bytes = if full_page {
        snapshot_playground_webview_full_page(webview)?
    } else {
        snapshot_playground_webview(webview)?
    };
    let sid = label
        .strip_prefix("playground-")
        .unwrap_or(label)
        .to_string();
    Ok(PlaygroundScreenshotResult {
        bytes,
        mime: "image/png".into(),
        filename: format!("playground-{sid}.png"),
    })
}

pub fn snapshot_child_webview_png(webview: &Webview) -> Result<Vec<u8>, String> {
    snapshot_playground_webview(webview)
}

/// Drive chat screenshots: reset page zoom + scroll, then full WKWebView bounds.
pub fn snapshot_viewport_png_for_drive(webview: &Webview) -> Result<Vec<u8>, String> {
    let _ = prepare_viewport_for_capture(webview);
    snapshot_playground_webview(webview)
}

fn snapshot_playground_webview(webview: &Webview) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        return snapshot_wkwebview(webview, SnapshotMode::Viewport);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        empty_png()
    }
}

fn snapshot_playground_webview_full_page(webview: &Webview) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        return snapshot_wkwebview(webview, SnapshotMode::FullPage);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        empty_png()
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum SnapshotMode {
    /// Visible WKWebView bounds at native pixel size (Browsers list thumbs,
    /// agent / chrome screenshot). Default WKSnapshotConfiguration — no
    /// snapshotWidth override.
    Viewport,
    /// Optional full scrollable document, height-capped, scaled via snapshotWidth.
    FullPage,
}

#[cfg(target_os = "macos")]
fn snapshot_wkwebview(webview: &Webview, mode: SnapshotMode) -> Result<Vec<u8>, String> {
    match mode {
        SnapshotMode::Viewport => take_wk_snapshot(webview, None),
        SnapshotMode::FullPage => snapshot_wkwebview_full_page(webview),
    }
}

#[cfg(target_os = "macos")]
fn snapshot_wkwebview_full_page(webview: &Webview) -> Result<Vec<u8>, String> {
    let (content_w, content_h) = match document_content_size(webview) {
        Ok(size) => size,
        Err(_) => {
            // Soft fallback: viewport snapshot still beats a hard failure for thumbs.
            return take_wk_snapshot(webview, Some(FULL_PAGE_SNAPSHOT_WIDTH_PX));
        }
    };
    let _ = content_w;

    let scale = webview
        .window()
        .scale_factor()
        .unwrap_or(1.0)
        .max(0.5);
    let physical = webview
        .size()
        .map_err(|error| error.to_string())?;
    let viewport_w = (physical.width as f64 / scale).max(1.0);
    let viewport_h = (physical.height as f64 / scale).max(1.0);
    let (target_w, target_h) =
        full_page_capture_logical_size(viewport_w, viewport_h, content_h);

    let resized = (target_h - viewport_h).abs() > 0.5 || (target_w - viewport_w).abs() > 0.5;
    if resized {
        webview
            .set_size(LogicalSize::new(target_w, target_h))
            .map_err(|error| error.to_string())?;
    }

    let result = take_wk_snapshot(webview, Some(FULL_PAGE_SNAPSHOT_WIDTH_PX));

    if resized {
        let _ = webview.set_size(LogicalSize::new(viewport_w, viewport_h));
    }

    result
}

#[cfg(target_os = "macos")]
fn document_content_size(webview: &Webview) -> Result<(f64, f64), String> {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKWebView;

    let (tx, rx) = mpsc::channel();
    webview
        .with_webview(move |platform| {
            // SAFETY: same as takeSnapshot — inner is this child WKWebView.
            let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
            let script = NSString::from_str(DOCUMENT_CONTENT_SIZE_JS);
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                if !error.is_null() {
                    let _ = tx.send(Err("document content size eval failed".into()));
                    return;
                }
                if result.is_null() {
                    let _ = tx.send(Err("document content size eval returned null".into()));
                    return;
                }
                let _ = tx.send(parse_content_size_json(unsafe { &*result }));
            });
            unsafe {
                view.evaluateJavaScript_completionHandler(&script, Some(&*block));
            }
            std::mem::forget(block);
        })
        .map_err(|error| error.to_string())?;

    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(result) => result,
        Err(_) => Err("document content size eval timed out".into()),
    }
}

#[cfg(target_os = "macos")]
fn parse_content_size_json(result: &objc2::runtime::AnyObject) -> Result<(f64, f64), String> {
    use objc2_foundation::NSString;

    let Some(ns) = result.downcast_ref::<NSString>() else {
        return Err("document content size was not a string".into());
    };
    parse_content_size_payload(&ns.to_string())
}

/// Parse `{w,h}` JSON from [`DOCUMENT_CONTENT_SIZE_JS`] (unit-tested).
pub fn parse_content_size_payload(raw: &str) -> Result<(f64, f64), String> {
    #[derive(serde::Deserialize)]
    struct Size {
        w: f64,
        h: f64,
    }
    let size: Size = serde_json::from_str(raw.trim())
        .map_err(|error| format!("document content size parse failed: {error}"))?;
    if !size.w.is_finite() || !size.h.is_finite() || size.w < 0.0 || size.h < 0.0 {
        return Err("document content size out of range".into());
    }
    Ok((size.w, size.h))
}

#[cfg(target_os = "macos")]
fn eval_js_string(webview: &Webview, js: &str) -> Result<String, String> {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKWebView;

    let script_owned = js.to_string();
    let (tx, rx) = mpsc::channel();
    webview
        .with_webview(move |platform| {
            let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
            let script = NSString::from_str(&script_owned);
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                if !error.is_null() {
                    let _ = tx.send(Err("webview eval failed".into()));
                    return;
                }
                if result.is_null() {
                    let _ = tx.send(Ok(String::new()));
                    return;
                }
                let raw = unsafe { &*result };
                let Some(ns) = raw.downcast_ref::<NSString>() else {
                    let _ = tx.send(Err("webview eval result was not a string".into()));
                    return;
                };
                let _ = tx.send(Ok(ns.to_string()));
            });
            unsafe {
                view.evaluateJavaScript_completionHandler(&script, Some(&*block));
            }
            std::mem::forget(block);
        })
        .map_err(|error| error.to_string())?;

    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(result) => result,
        Err(_) => Err("webview eval timed out".into()),
    }
}

/// Probe document readiness for Drive screen settle (macOS WKWebView).
pub fn probe_page_ready(webview: &Webview) -> Result<PageReadyProbe, String> {
    #[cfg(target_os = "macos")]
    {
        let raw = eval_js_string(webview, PAGE_READY_PROBE_JS)?;
        return parse_page_ready_probe(&raw);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        Ok(PageReadyProbe {
            ready: "complete".into(),
            recent: 0,
            text_len: 1,
            w: 1280.0,
            h: 800.0,
            href: "https://example.invalid/".into(),
        })
    }
}

/// Reset pageZoom to 1 and scroll to (0,0) so the snapshot is the full frame.
pub fn prepare_viewport_for_capture(webview: &Webview) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_web_kit::WKWebView;
        webview
            .with_webview(|platform| {
                let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
                unsafe {
                    view.setPageZoom(1.0);
                }
            })
            .map_err(|error| error.to_string())?;
        let _ = eval_js_string(webview, PREPARE_VIEWPORT_JS)?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn take_wk_snapshot(webview: &Webview, snapshot_width: Option<f64>) -> Result<Vec<u8>, String> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSError, NSNumber};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

    let (tx, rx) = mpsc::channel();
    webview
        .with_webview(move |platform| {
            // SAFETY: Tauri's macOS PlatformWebview::inner is the WKWebView
            // pointer for this child view. takeSnapshot captures that view
            // only — not the host window or the display. WKWebView is
            // MainThreadOnly; with_webview runs on the AppKit main thread, so
            // the marker is taken from that live object (not constructed
            // unchecked).
            let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
            let mtm = MainThreadMarker::from(view);
            // Default config (null rect) = current WKWebView bounds. For
            // full-page thumbs the view was temporarily expanded to the
            // capped document height before this call; snapshotWidth keeps
            // the PNG small for list rows.
            let config = unsafe { WKSnapshotConfiguration::new(mtm) };
            // Full WKWebView bounds (null rect). Wait for paint so Drive
            // screenshots are not mid-frame crops.
            unsafe {
                config.setAfterScreenUpdates(true);
            }
            if let Some(width) = snapshot_width {
                unsafe {
                    config.setSnapshotWidth(Some(&NSNumber::numberWithDouble(width)));
                }
            }
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() {
                    let _ = tx.send(Err("playground webview snapshot failed".into()));
                    return;
                }
                if image.is_null() {
                    let _ = tx.send(empty_png());
                    return;
                }
                let image = unsafe { &*image };
                let _ = tx.send(ns_image_png(image));
            });
            unsafe {
                view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
            // WKWebView retains the completion handler until it fires.
            std::mem::forget(block);
        })
        .map_err(|error| error.to_string())?;

    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(result) => result,
        Err(_) => Err("playground webview snapshot timed out".into()),
    }
}

#[cfg(target_os = "macos")]
fn ns_image_png(image: &objc2_app_kit::NSImage) -> Result<Vec<u8>, String> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::NSDictionary;

    let tiff = image
        .TIFFRepresentation()
        .ok_or_else(|| "playground snapshot produced no image data".to_string())?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)
        .ok_or_else(|| "playground snapshot image decode failed".to_string())?;
    // SAFETY: empty property dict is AppKit's default for PNG encode.
    // objc2 0.3.2 takes `&NSDictionary`, not `Option`, and the method is unsafe.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or_else(|| "playground snapshot png encode failed".to_string())?;
    Ok(png.to_vec())
}

pub fn empty_png() -> Result<Vec<u8>, String> {
    // 1×1 transparent PNG so draft staging still has a real image payload
    // when a platform capture is unavailable (tests, headless).
    Ok(vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_is_webview_snapshot_not_screencapture() {
        assert_eq!(PLAYGROUND_CAPTURE_BACKEND, "webview-snapshot");
        assert_ne!(PLAYGROUND_CAPTURE_BACKEND, "screencapture");
        assert_eq!(
            playground_screenshot_target("abc").expect("label"),
            "playground-abc"
        );
        assert!(inspect_target_is_safe(
            &playground_screenshot_target("abc").expect("label")
        ));
        assert!(playground_screenshot_target("").is_err());
    }

    #[test]
    fn full_page_height_is_capped_and_at_least_viewport() {
        assert_eq!(clamp_full_page_capture_height(500.0, 800.0), 800.0);
        assert_eq!(clamp_full_page_capture_height(1200.0, 800.0), 1200.0);
        assert_eq!(
            clamp_full_page_capture_height(50_000.0, 800.0),
            FULL_PAGE_MAX_HEIGHT_CSS_PX
        );
        assert!(FULL_PAGE_MAX_HEIGHT_CSS_PX >= 4000.0);
        assert!(FULL_PAGE_MAX_HEIGHT_CSS_PX <= 8000.0);
        assert!(FULL_PAGE_SNAPSHOT_WIDTH_PX >= 220.0);
        assert!(FULL_PAGE_SNAPSHOT_WIDTH_PX <= 360.0);
    }

    #[test]
    fn full_page_logical_size_keeps_width_expands_height() {
        let (w, h) = full_page_capture_logical_size(1280.0, 800.0, 4200.0);
        assert_eq!(w, 1280.0);
        assert_eq!(h, 4200.0);
        let (w2, h2) = full_page_capture_logical_size(390.0, 844.0, 20_000.0);
        assert_eq!(w2, 390.0);
        assert_eq!(h2, FULL_PAGE_MAX_HEIGHT_CSS_PX);
    }

    #[test]
    fn content_size_js_and_payload_parse() {
        assert!(DOCUMENT_CONTENT_SIZE_JS.contains("scrollHeight"));
        assert!(DOCUMENT_CONTENT_SIZE_JS.contains("scrollWidth"));
        assert!(DOCUMENT_CONTENT_SIZE_JS.contains("JSON.stringify"));
        assert_eq!(
            parse_content_size_payload(r#"{"w":1280,"h":5400}"#).expect("parse"),
            (1280.0, 5400.0)
        );
        assert!(parse_content_size_payload("nope").is_err());
        assert!(parse_content_size_payload(r#"{"w":-1,"h":10}"#).is_err());
    }

    #[test]
    fn page_ready_probe_settled_requires_complete_and_quiet() {
        assert!(PAGE_READY_PROBE_JS.contains("readyState"));
        assert!(PAGE_READY_PROBE_JS.contains("responseEnd"));
        assert!(PREPARE_VIEWPORT_JS.contains("scrollTo"));
        let settled = parse_page_ready_probe(
            r#"{"ready":"complete","recent":0,"textLen":40,"w":1280,"h":800,"href":"https://ex.test/"}"#,
        )
        .expect("parse");
        assert!(settled.is_settled());
        let loading = parse_page_ready_probe(
            r#"{"ready":"interactive","recent":0,"textLen":40,"w":1280,"h":800,"href":"https://ex.test/"}"#,
        )
        .expect("parse");
        assert!(!loading.is_settled());
        let busy = parse_page_ready_probe(
            r#"{"ready":"complete","recent":3,"textLen":40,"w":1280,"h":800,"href":"https://ex.test/"}"#,
        )
        .expect("parse");
        assert!(!busy.is_settled());
        let blank = parse_page_ready_probe(
            r#"{"ready":"complete","recent":0,"textLen":0,"w":1280,"h":800,"href":"about:blank"}"#,
        )
        .expect("parse");
        assert!(!blank.is_settled());
    }
}
