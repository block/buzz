//! Native child webview for channel website / agent-app tabs.
//!
//! HTML `<iframe>` inside the main WKWebView stays blank on macOS because that
//! webview is transparent (`drawsBackground = false`) for sidebar glass, and
//! WKWebView does not composite cross-origin iframes in that mode. A child
//! webview is a top-level document (same as Flutter's WebView), so the site
//! paints. Chrome web and mobile already worked; this is desktop-only.

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewBuilder, WebviewUrl,
};

const EMBED_LABEL: &str = "channel-embed";

fn main_window<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::Window<R>, String> {
    app.get_window("main")
        .ok_or_else(|| "main window not found".to_string())
}

fn parse_embed_url(raw: &str) -> Result<url::Url, String> {
    let parsed = raw
        .parse::<url::Url>()
        .map_err(|error| error.to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("unsupported embed url".to_string());
    }
    Ok(parsed)
}

fn apply_bounds<R: Runtime>(
    webview: &tauri::Webview<R>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    webview
        .set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    webview
        .set_size(LogicalSize::new(width.max(1.0), height.max(1.0)))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn show_channel_embed(
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle,
) -> Result<(), String> {
    if width < 8.0 || height < 8.0 {
        return Ok(());
    }
    let parsed = parse_embed_url(&url)?;
    if let Some(existing) = app.get_webview(EMBED_LABEL) {
        let current = existing.url().ok();
        if current.as_ref() != Some(&parsed) {
            existing.navigate(parsed).map_err(|e| e.to_string())?;
        }
        apply_bounds(&existing, x, y, width, height)?;
        existing.show().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = main_window(&app)?;
    let builder = WebviewBuilder::new(EMBED_LABEL, WebviewUrl::External(parsed)).transparent(false);
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn set_channel_embed_bounds(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle,
) -> Result<(), String> {
    if width < 8.0 || height < 8.0 {
        return Ok(());
    }
    let Some(existing) = app.get_webview(EMBED_LABEL) else {
        return Ok(());
    };
    apply_bounds(&existing, x, y, width, height)
}

#[tauri::command]
pub async fn hide_channel_embed(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview(EMBED_LABEL) {
        existing.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
