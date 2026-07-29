//! Linux WebKit permission integration.
//!
//! WebKitGTK delegates `getUserMedia` decisions to its embedding application.
//! An unanswered `WebKitUserMediaPermissionRequest` is denied, so huddles cannot
//! acquire a microphone unless Buzz handles the `permission-request` signal.

use tauri::Webview;
use webkit2gtk::{
    glib::ObjectExt, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest, WebViewExt,
};

/// Origins served by Buzz itself.
///
/// Release builds use Tauri's custom protocol. Development builds use the Vite
/// URL declared in `tauri.conf.json`. Keep this allowlist narrow so navigating a
/// webview away from Buzz cannot inherit microphone or camera access.
fn is_buzz_origin(uri: &str) -> bool {
    ["tauri://localhost", "http://localhost:1420"]
        .iter()
        .any(|origin| {
            uri == *origin
                || uri
                    .strip_prefix(origin)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// Enable media capture and grant only user-media requests from Buzz's own UI.
pub fn install(webview: &Webview) -> tauri::Result<()> {
    webview.with_webview(|platform_webview| {
        let webview = platform_webview.inner();

        if let Some(settings) = webview.settings() {
            settings.set_enable_media_stream(true);
        }

        webview.connect_permission_request(|webview, request| {
            let is_user_media = request.is::<UserMediaPermissionRequest>();
            let is_trusted_uri = webview.uri().as_deref().is_some_and(is_buzz_origin);

            if is_user_media && is_trusted_uri {
                request.allow();
                true
            } else {
                false
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use super::is_buzz_origin;

    #[test]
    fn accepts_release_and_development_origins() {
        assert!(is_buzz_origin("tauri://localhost"));
        assert!(is_buzz_origin("tauri://localhost/"));
        assert!(is_buzz_origin("tauri://localhost/index.html"));
        assert!(is_buzz_origin("http://localhost:1420/"));
    }

    #[test]
    fn rejects_lookalike_and_remote_origins() {
        assert!(!is_buzz_origin("https://example.com/"));
        assert!(!is_buzz_origin("tauri://localhost.evil.example/"));
        assert!(!is_buzz_origin("https://tauri.localhost/"));
        assert!(!is_buzz_origin("http://localhost:14200/"));
        assert!(!is_buzz_origin("http://localhost:1420.evil.example/"));
    }
}
