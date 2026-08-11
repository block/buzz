//! Runtime macOS window vibrancy (blur-behind) toggle.
//!
//! Vibrancy applies an `NSVisualEffectView` behind the webview so the desktop
//! (and windows behind Buzz) blur through wherever the app's CSS is
//! transparent. It is a native, macOS-only effect: there is no "intensity"
//! setting at the OS level, only a set of material presets. The frontend tunes
//! perceived intensity by changing CSS surface opacity while this command
//! handles the native material.
//!
//! This is fully reversible at runtime: enabling applies the chosen material,
//! disabling clears it. On non-macOS platforms the command is a no-op so the
//! shared frontend can call it unconditionally.

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
            // Restore opaque backing first: a failure of clear_vibrancy leaves
            // the window with an opaque canvas and a blur layer hidden behind
            // it, not a see-through webview. Either mixed state self-corrects
            // on the next toggle. `None` resets both the NSWindow backing and
            // the WKWebView canvas to their platform defaults.
            window
                .set_background_color(None)
                .map_err(|e| e.to_string())?;
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

        // Install the blur layer first: a failure of set_background_color leaves
        // the window opaque with vibrancy behind it, not a see-through webview.
        // Either mixed state self-corrects on the next toggle.
        apply_vibrancy(&window, material, None, None).map_err(|e| e.to_string())?;

        // Make the WKWebView canvas and NSWindow backing transparent so native
        // vibrancy shows through. Must follow apply_vibrancy so the blur layer
        // is present before the canvas becomes see-through.
        window
            .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}
