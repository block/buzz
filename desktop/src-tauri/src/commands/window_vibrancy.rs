//! Runtime macOS window vibrancy (blur-behind) toggle.
//!
//! **Invariant:** the main window is created opaque (`tauri.conf.json`
//! `transparent: false`) and the NSWindow is never made transparent at runtime.
//! Behind-window vibrancy renders correctly inside opaque windows — this is
//! exactly how Finder and Notes render vibrant sidebars — so glass only requires
//! runtime webview-canvas transparency, which this command sets on the enable
//! path. When glass is disabled the webview canvas may remain non-drawing
//! (wry's `drawsBackground` flag is one-way at runtime), but that is harmless:
//! glass-off CSS paints the full background opaque and the always-opaque NSWindow
//! is beneath it.
//!
//! Why not `transparent: true`? A creation-time transparent window causes tao to
//! call `NSWindow.setOpaque(false)` and `setBackgroundColor(clearColor)`. The
//! runtime `Window::set_background_color(None)` then resolves `None` to
//! `clearColor` instead of the opaque system default — and there is no runtime
//! `setOpaque(true)` path through tauri — leaving the compositor blending the
//! whole window even with glass off.
//!
//! Vibrancy applies an `NSVisualEffectView` behind the webview so the desktop
//! (and windows behind Buzz) blurs through wherever the WKWebView canvas is
//! transparent. It is a native, macOS-only effect: there is no "intensity"
//! setting at the OS level, only a set of material presets. The frontend tunes
//! perceived intensity by adjusting CSS surface opacity while this command
//! handles the native material. On non-macOS platforms the command is a no-op
//! so the shared frontend can call it unconditionally.

#[cfg(target_os = "macos")]
use tauri::Manager;

/// Apply or clear macOS window vibrancy for the main window.
///
/// `material` accepts the common `NSVisualEffectMaterial` names
/// (`sidebar`, `hud-window`, `under-window-background`, `fullscreen-ui`,
/// `header-view`, `popover`, `menu`, `titlebar`). Unknown values fall back to
/// `sidebar`.
#[tauri::command]
pub fn set_window_vibrancy(
    #[allow(unused_variables)] enabled: bool,
    #[allow(unused_variables)] material: Option<String>,
    #[allow(unused_variables)] app_handle: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial};

        let window = app_handle
            .get_webview_window("main")
            .ok_or_else(|| "main window not found".to_string())?;

        if !enabled {
            // The NSWindow layer is permanently opaque, so no window-layer
            // reset is needed here. Skipping `set_background_color(None)` at
            // the webview layer also avoids tauri mapping `None` to opaque
            // white, which would still force `drawsBackground=false` on the
            // WKWebView (counterproductive). After a glass session the webview
            // canvas may stay non-drawing — wry's `drawsBackground` flag is
            // one-way at runtime — but that is harmless: glass-off CSS paints
            // the full background opaque and the always-opaque NSWindow is
            // beneath it. If `clear_vibrancy` fails, the opaque CSS already
            // covers everything, so no see-through state is reachable.
            clear_vibrancy(&window).map_err(|e| e.to_string())?;
            return Ok(());
        }

        let material = match material.as_deref() {
            Some("hud-window") => NSVisualEffectMaterial::HudWindow,
            Some("under-window-background") => NSVisualEffectMaterial::UnderWindowBackground,
            Some("fullscreen-ui") => NSVisualEffectMaterial::FullScreenUI,
            Some("header-view") => NSVisualEffectMaterial::HeaderView,
            Some("popover") => NSVisualEffectMaterial::Popover,
            Some("menu") => NSVisualEffectMaterial::Menu,
            Some("titlebar") => NSVisualEffectMaterial::Titlebar,
            _ => NSVisualEffectMaterial::Sidebar,
        };

        // `apply_vibrancy` appends a new tagged `NSVisualEffectView` each call,
        // while `clear_vibrancy` only removes one. Repeated enables (theme
        // switches, follow-system flips) would otherwise stack blur views and
        // leave a stale one behind on the next non-Buzz theme. Clear any
        // existing view first so exactly one material is ever installed. The
        // clear is a no-op (returns `false`) when none is present.
        let _ = clear_vibrancy(&window);

        // Install the blur layer first: a failure of the canvas write leaves
        // the window with vibrancy behind an opaque webview, not a see-through
        // one. Either mixed state self-corrects on the next toggle.
        apply_vibrancy(&window, material, None, None).map_err(|e| e.to_string())?;

        // Make only the WKWebView canvas transparent so native vibrancy shows
        // through; the NSWindow layer stays opaque by design. Targeting the
        // webview layer directly (via `AsRef<Webview>`) avoids the
        // `WebviewWindow::set_background_color` path, which also writes the
        // NSWindow layer. Must follow `apply_vibrancy` so the blur layer is
        // present before the canvas becomes see-through.
        let webview: &tauri::Webview<_> = window.as_ref();
        webview
            .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}
