//! Linux-only: enable media capture (`getUserMedia`) in the WebKitGTK webview.
//!
//! On macOS (WKWebView) and Windows (WebView2) the media-permission prompt is
//! routed to the OS automatically, so microphone/camera capture "just works".
//! WebKitGTK is different on two counts, and both must be handled or capture
//! fails on Linux only:
//!
//! * `enable-media-stream` is **off by default**, so `navigator.mediaDevices`
//!   never exposes a working `getUserMedia`; and
//! * the default `permission-request` handler **denies every request**, so even
//!   with media-stream on, the call rejects with `NotAllowedError`.
//!
//! This module reaches the underlying `webkit2gtk::WebView` via
//! [`tauri::Webview::with_webview`], enables media-stream, and installs a
//! `permission-request` handler that is **deny-by-default**: a `UserMedia`
//! request is allowed only when it comes from a trusted app origin and asks for
//! an audio and/or video device. Tauri does not restrict navigation by default,
//! so without the origin check any document that ended up in this webview would
//! inherit silent mic/camera access for the process lifetime.
//!
//! Buzz's AppImage pins `GDK_BACKEND=x11` (see [`crate::webkit_rendering`]),
//! which is the backend WebKitGTK media capture is reliable on.

/// The origin Tauri serves the packaged app from on Linux.
const PROD_ORIGIN: &str = "tauri://localhost";

/// The Vite dev-server origin (`devUrl` in `tauri.conf.json`, `strictPort`
/// 1420 in `vite.config.ts`). Only trusted in debug builds.
#[cfg(debug_assertions)]
const DEV_ORIGIN: &str = "http://localhost:1420";

/// Whether `uri` (the webview's current document URI) is a trusted app origin
/// allowed to use mic/camera. Matches the origin exactly or as a path prefix so
/// `tauri://localhost.evil.com` and `http://localhost:14200` do not slip
/// through. Pure and platform-independent so it can be unit-tested everywhere.
fn is_trusted_media_origin(uri: &str) -> bool {
    fn matches(uri: &str, origin: &str) -> bool {
        uri == origin
            || uri
                .strip_prefix(origin)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    if matches(uri, PROD_ORIGIN) {
        return true;
    }
    #[cfg(debug_assertions)]
    if matches(uri, DEV_ORIGIN) {
        return true;
    }
    false
}

/// Enable microphone/camera capture for `webview` if it is running on
/// WebKitGTK. A no-op on every non-Linux target, so callers can invoke it
/// unconditionally from shared startup code.
#[cfg(target_os = "linux")]
pub fn enable_media_capture<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    use webkit2gtk::{
        glib::prelude::Cast, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        UserMediaPermissionRequestExt, WebViewExt,
    };

    // `with_webview` runs the closure on the UI thread, which GTK calls
    // require. It errors only if the platform webview is unavailable.
    let result = webview.with_webview(|platform_webview| {
        // On Linux this is the underlying `webkit2gtk::WebView`.
        let webview = platform_webview.inner();

        if let Some(settings) = WebViewExt::settings(&webview) {
            settings.set_enable_media_stream(true);
        }

        // Deny-by-default: allow only mic/camera requests from a trusted app
        // origin; deny everything else (still returning `true` so WebKit's
        // auto-deny default does not also run). Non-`UserMedia` requests return
        // `false` and keep their default handling.
        webview.connect_permission_request(|wv, request| {
            let Some(request) = request.downcast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };

            let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
            let for_device = request.is_for_audio_device() || request.is_for_video_device();

            if for_device && is_trusted_media_origin(&uri) {
                request.allow();
            } else {
                request.deny();
            }
            true
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

#[cfg(test)]
mod tests {
    use super::is_trusted_media_origin;

    #[test]
    fn allows_production_app_origin() {
        assert!(is_trusted_media_origin("tauri://localhost"));
        assert!(is_trusted_media_origin(
            "tauri://localhost/channels/general"
        ));
    }

    #[test]
    fn denies_untrusted_origins() {
        assert!(!is_trusted_media_origin(""));
        assert!(!is_trusted_media_origin("https://evil.example.com"));
        // Prefix look-alikes must not slip through.
        assert!(!is_trusted_media_origin("tauri://localhost.evil.com"));
        assert!(!is_trusted_media_origin("tauri://localhostfoo"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn allows_dev_origin_in_debug_only() {
        assert!(is_trusted_media_origin("http://localhost:1420"));
        assert!(is_trusted_media_origin("http://localhost:1420/"));
        // A different localhost port is still untrusted.
        assert!(!is_trusted_media_origin("http://localhost:14200"));
        assert!(!is_trusted_media_origin("http://localhost:3000"));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn denies_dev_origin_in_release() {
        assert!(!is_trusted_media_origin("http://localhost:1420"));
    }
}
