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
//! In debug builds the app is served from the Vite dev server rather than
//! `tauri://localhost`. The just-based entrypoints derive the dev port per
//! worktree (`scripts/instance-env.sh`: `10000 + sha256(worktree) % 55000`) and
//! export it as `VITE_PORT`, so the trusted dev origin is derived from that
//! variable at webview startup; it falls back to Vite's default port (1420)
//! when the variable is missing or invalid (e.g. a raw `pnpm tauri dev` without
//! instance-env).
//!
//! Buzz's AppImage pins `GDK_BACKEND=x11` (see [`crate::webkit_rendering`]),
//! which is the backend WebKitGTK media capture is reliable on.

/// The origin Tauri serves the packaged app from on Linux.
/// Consumed only by linux-gated [`enable_media_capture`]; kept compiling on all
/// platforms so the unit tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const PROD_ORIGIN: &str = "tauri://localhost";

/// Vite's default dev-server port (the `vite.config.ts` fallback), used as the
/// trusted dev origin when `VITE_PORT` is missing or invalid.
#[cfg(debug_assertions)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const DEV_DEFAULT_ORIGIN: &str = "http://localhost:1420";

/// Build the trusted dev-server origin for debug builds from the configured
/// Vite port. Pure over its input so it can be unit-tested without env
/// interference; [`dev_media_origin`] supplies the real value.
#[cfg(debug_assertions)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn dev_origin_for_port(vite_port: Option<&str>) -> String {
    match vite_port {
        // The port must be 1–5 ASCII digits that parse to a non-zero u16,
        // which rejects empty input, `0`, values above 65535, and anything
        // non-numeric.
        Some(port)
            if (1..=5).contains(&port.len())
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|p| p != 0) =>
        {
            format!("http://localhost:{port}")
        }
        _ => DEV_DEFAULT_ORIGIN.to_string(),
    }
}

/// The Vite dev-server origin to trust in this debug build. Derived from
/// `VITE_PORT` (exported by `scripts/instance-env.sh` for the just-based
/// entrypoints) so the per-worktree dev port is trusted, not only the
/// hardcoded Vite default.
#[cfg(debug_assertions)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn dev_media_origin() -> String {
    dev_origin_for_port(std::env::var("VITE_PORT").ok().as_deref())
}

/// Whether `uri` (the webview's current document URI) is a trusted app origin
/// allowed to use mic/camera. Matches the origin exactly or as a path prefix so
/// `tauri://localhost.evil.com` and dev-port look-alikes do not slip through.
/// `dev_origin` is `Some` only in debug builds, where the app is served from
/// the dev server. Pure and platform-independent so it can be unit-tested
/// everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_trusted_media_origin(uri: &str, dev_origin: Option<&str>) -> bool {
    fn matches(uri: &str, origin: &str) -> bool {
        uri == origin
            || uri
                .strip_prefix(origin)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    if matches(uri, PROD_ORIGIN) {
        return true;
    }
    if let Some(origin) = dev_origin {
        if matches(uri, origin) {
            return true;
        }
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

    // Debug builds are served from the Vite dev server, whose port is derived
    // per worktree by `scripts/instance-env.sh`; packaged builds are only
    // reachable from `tauri://localhost`.
    #[cfg(debug_assertions)]
    let dev_origin = Some(dev_media_origin());
    #[cfg(not(debug_assertions))]
    let dev_origin: Option<String> = None;

    // `with_webview` runs the closure on the UI thread, which GTK calls
    // require. It errors only if the platform webview is unavailable.
    let result = webview.with_webview(move |platform_webview| {
        // On Linux this is the underlying `webkit2gtk::WebView`.
        let webview = platform_webview.inner();

        if let Some(settings) = WebViewExt::settings(&webview) {
            settings.set_enable_media_stream(true);
        }

        // Deny-by-default: allow only mic/camera requests from a trusted app
        // origin; deny everything else (still returning `true` so WebKit's
        // auto-deny default does not also run). Non-`UserMedia` requests return
        // `false` and keep their default handling. The signal handler must be
        // `'static`, so it moves its own clone of the trusted origin.
        let trusted_dev = dev_origin.clone();
        webview.connect_permission_request(move |wv, request| {
            let Some(request) = request.downcast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };

            let uri = wv.uri().map(|u| u.to_string()).unwrap_or_default();
            let for_device = request.is_for_audio_device() || request.is_for_video_device();

            if for_device && is_trusted_media_origin(&uri, trusted_dev.as_deref()) {
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
        assert!(is_trusted_media_origin("tauri://localhost", None));
        assert!(is_trusted_media_origin(
            "tauri://localhost/channels/general",
            None
        ));
    }

    #[test]
    fn denies_untrusted_origins() {
        assert!(!is_trusted_media_origin("", None));
        assert!(!is_trusted_media_origin("https://evil.example.com", None));
        // Prefix look-alikes must not slip through.
        assert!(!is_trusted_media_origin("tauri://localhost.evil.com", None));
        assert!(!is_trusted_media_origin("tauri://localhostfoo", None));
    }

    #[test]
    fn allows_trusted_dev_origin_and_denies_lookalikes() {
        let dev = Some("http://localhost:43210");
        assert!(is_trusted_media_origin("http://localhost:43210", dev));
        assert!(is_trusted_media_origin("http://localhost:43210/", dev));
        // A different localhost port is still untrusted.
        assert!(!is_trusted_media_origin("http://localhost:432100", dev));
        assert!(!is_trusted_media_origin("http://localhost:1420", dev));
        // The production origin is trusted regardless of the dev origin.
        assert!(is_trusted_media_origin("tauri://localhost", dev));
    }

    #[test]
    fn without_dev_origin_only_production_is_trusted() {
        assert!(!is_trusted_media_origin("http://localhost:1420", None));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn denies_dev_origin_in_release() {
        assert!(!is_trusted_media_origin("http://localhost:1420", None));
    }

    #[cfg(debug_assertions)]
    mod dev_origin_tests {
        use super::super::dev_origin_for_port;

        #[test]
        fn uses_configured_port() {
            assert_eq!(dev_origin_for_port(Some("43210")), "http://localhost:43210");
            assert_eq!(dev_origin_for_port(Some("1420")), "http://localhost:1420");
        }

        #[test]
        fn falls_back_to_vite_default_port() {
            // Missing, empty, non-numeric, zero, and out-of-range values must
            // not produce a spoofable origin.
            assert_eq!(dev_origin_for_port(None), "http://localhost:1420");
            assert_eq!(dev_origin_for_port(Some("")), "http://localhost:1420");
            assert_eq!(
                dev_origin_for_port(Some("not-a-port")),
                "http://localhost:1420"
            );
            assert_eq!(dev_origin_for_port(Some("0")), "http://localhost:1420");
            assert_eq!(dev_origin_for_port(Some("99999")), "http://localhost:1420");
            assert_eq!(dev_origin_for_port(Some("-1")), "http://localhost:1420");
            assert_eq!(dev_origin_for_port(Some("142000")), "http://localhost:1420");
        }
    }
}
