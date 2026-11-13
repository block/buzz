/// Whether this webview parses `backdrop-filter` but will not paint it.
///
/// WebKitGTK without accelerated compositing still accepts `backdrop-filter`,
/// so CSS `@supports` passes, yet nothing behind the element is blurred. The
/// glass surfaces then show sharp text through a thin tint. WebKit drops
/// compositing on its own on some GPU/driver setups (and `--safe-rendering`
/// drops it on purpose); in both cases the hardware-acceleration policy reads
/// back as `Never`, whatever it was set to. The frontend switches the glass
/// surfaces to solid when this returns `true`.
///
/// Always `false` off Linux: WKWebView and WebView2 paint what they parse.
#[tauri::command]
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub async fn get_backdrop_filter_unpainted(webview: tauri::Webview) -> bool {
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};

        let (tx, rx) = tokio::sync::oneshot::channel();
        // GTK calls must run on the UI thread, which `with_webview` provides.
        let result = webview.with_webview(move |platform_webview| {
            let never = WebViewExt::settings(&platform_webview.inner()).is_some_and(|settings| {
                settings.hardware_acceleration_policy() == HardwareAccelerationPolicy::Never
            });
            let _ = tx.send(never);
        });
        if let Err(error) = result {
            eprintln!("buzz-desktop: could not read WebKitGTK acceleration policy: {error}");
            return false;
        }
        rx.await.unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
