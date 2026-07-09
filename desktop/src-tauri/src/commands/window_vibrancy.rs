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

        apply_vibrancy(&window, material, None, None).map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}
