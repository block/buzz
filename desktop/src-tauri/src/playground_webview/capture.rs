//! Snapshot the playground child webview itself. Never display-capture.

#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::time::Duration;

use tauri::Webview;

use super::{inspect_target_is_safe, playground_label, PlaygroundScreenshotResult};

/// Capture backend name. Tests assert this is not `screencapture`.
pub const PLAYGROUND_CAPTURE_BACKEND: &str = "webview-snapshot";

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
) -> Result<PlaygroundScreenshotResult, String> {
    if !inspect_target_is_safe(label) {
        return Err("screenshot must target the playground webview".into());
    }
    if webview.label() != label {
        return Err("screenshot must target the playground webview".into());
    }
    let bytes = snapshot_playground_webview(webview)?;
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

fn snapshot_playground_webview(webview: &Webview) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        return snapshot_wkwebview(webview);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        empty_png()
    }
}

#[cfg(target_os = "macos")]
fn snapshot_wkwebview(webview: &Webview) -> Result<Vec<u8>, String> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSError;
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
            let config = unsafe { WKSnapshotConfiguration::new(mtm) };
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
}
