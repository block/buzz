//! Linux-only: enable media capture (`getUserMedia`) in the WebKitGTK webview.
//!
//! On macOS (WKWebView) and Windows (WebView2) the media-permission prompt is
//! routed to the OS automatically, so microphone/camera capture "just works".
//! WebKitGTK is different on two counts, and both must be fixed or capture
//! fails on Linux only:
//!
//! * `enable-media-stream` is **off by default**, so `navigator.mediaDevices`
//!   never exposes a working `getUserMedia`; and
//! * the default `permission-request` handler **denies every request**, so even
//!   with media-stream on, the call rejects with `NotAllowedError`.
//!
//! This module reaches the underlying `webkit2gtk::WebView` via
//! [`tauri::Webview::with_webview`], enables the media-stream settings, and
//! installs a `permission-request` handler that allows microphone/camera
//! requests while leaving every other permission kind to WebKit's default.
//!
//! Buzz's AppImage pins `GDK_BACKEND=x11` (see [`crate::webkit_rendering`]),
//! which is the backend WebKitGTK media capture is reliable on.

/// Enable microphone/camera capture for `webview` if it is running on
/// WebKitGTK. A no-op on every non-Linux target, so callers can invoke it
/// unconditionally from shared startup code.
#[cfg(target_os = "linux")]
pub fn enable_media_capture<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    use webkit2gtk::{
        glib::prelude::Cast, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        WebViewExt,
    };

    // `with_webview` runs the closure on the UI thread, which GTK calls
    // require. It errors only if the platform webview is unavailable.
    let result = webview.with_webview(|platform_webview| {
        // On Linux this is the underlying `webkit2gtk::WebView`.
        let webview = platform_webview.inner();

        if let Some(settings) = WebViewExt::settings(&webview) {
            // The setting that actually makes `getUserMedia` reachable.
            settings.set_enable_media_stream(true);
            // Lets captured/remote streams play back via MediaSource.
            settings.set_enable_mediasource(true);
            // Capture is a deliberate user action; no gesture gate needed.
            settings.set_media_playback_requires_user_gesture(false);

            // WebRTC peer connections and getDisplayMedia need a WebKitGTK
            // built with -DENABLE_WEB_RTC=ON and the crate's `v2_38` API.
            // Gated so the build still works against stock WebKitGTK.
            #[cfg(feature = "webkit_webrtc")]
            settings.set_enable_webrtc(true);
        }

        // Replace WebKit's auto-deny default: allow microphone/camera prompts,
        // and return `false` for anything else so geolocation, notifications,
        // etc. keep their default (denied) handling.
        webview.connect_permission_request(|_webview, request| {
            if request
                .downcast_ref::<UserMediaPermissionRequest>()
                .is_some()
            {
                request.allow();
                true
            } else {
                false
            }
        });
    });

    if let Err(error) = result {
        eprintln!("buzz-desktop: could not enable WebKitGTK media capture: {error}");
    }
}

/// No-op stub so shared startup code can call [`enable_media_capture`] on every
/// platform. macOS and Windows route media permissions through the OS.
#[cfg(not(target_os = "linux"))]
pub fn enable_media_capture<R: tauri::Runtime>(_webview: &tauri::Webview<R>) {}
