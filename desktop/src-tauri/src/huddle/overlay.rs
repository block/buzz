use serde::Deserialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const VOICE_OVERLAY_LABEL: &str = "voice-overlay";

#[derive(Debug, PartialEq, Eq)]
enum OpenVoiceOverlayEffect {
    Create,
    Reveal,
}

fn open_voice_overlay_effect(window_exists: bool) -> OpenVoiceOverlayEffect {
    if window_exists {
        OpenVoiceOverlayEffect::Reveal
    } else {
        OpenVoiceOverlayEffect::Create
    }
}

fn reveal_window(window: &WebviewWindow, context: &str) -> Result<(), String> {
    if let Err(error) = window.unminimize() {
        eprintln!("buzz-desktop: failed to restore {context}: {error}");
    }
    if let Err(error) = window.set_always_on_top(true) {
        eprintln!("buzz-desktop: failed to keep {context} on top: {error}");
    }
    window
        .show()
        .map_err(|error| format!("failed to show {context}: {error}"))?;
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus {context}: {error}");
    }
    Ok(())
}

fn open_voice_overlay(app: &AppHandle) -> Result<(), String> {
    match open_voice_overlay_effect(app.get_webview_window(VOICE_OVERLAY_LABEL).is_some()) {
        OpenVoiceOverlayEffect::Reveal => {
            let window = app
                .get_webview_window(VOICE_OVERLAY_LABEL)
                .ok_or_else(|| "voice overlay disappeared while opening".to_owned())?;
            reveal_window(&window, "voice overlay")
        }
        OpenVoiceOverlayEffect::Create => WebviewWindowBuilder::new(
            app,
            VOICE_OVERLAY_LABEL,
            WebviewUrl::App("index.html#/voice-overlay".into()),
        )
        .title("Buzz Voice")
        .inner_size(376.0, 148.0)
        .min_inner_size(320.0, 148.0)
        .max_inner_size(520.0, 220.0)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .shadow(true)
        .center()
        .build()
        .map(|_| ())
        .or_else(|error| {
            if let Some(window) = app.get_webview_window(VOICE_OVERLAY_LABEL) {
                reveal_window(&window, "voice overlay")
            } else {
                Err(format!("failed to create voice overlay: {error}"))
            }
        }),
    }
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is unavailable".to_owned())?;
    window
        .unminimize()
        .map_err(|error| format!("failed to restore main window: {error}"))?;
    window
        .show()
        .map_err(|error| format!("failed to show main window: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("failed to focus main window: {error}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceOverlayWindowAction {
    Open,
    Ready,
    ShowMain,
}

#[tauri::command]
pub async fn voice_overlay_window(
    app: AppHandle,
    window: WebviewWindow,
    action: VoiceOverlayWindowAction,
) -> Result<(), String> {
    match action {
        VoiceOverlayWindowAction::Open => open_voice_overlay(&app),
        VoiceOverlayWindowAction::Ready => {
            if window.label() != VOICE_OVERLAY_LABEL {
                return Err("voice overlay ready called from an unexpected window".to_owned());
            }
            reveal_window(&window, "voice overlay")
        }
        VoiceOverlayWindowAction::ShowMain => show_main_window(&app),
    }
}

#[cfg(test)]
mod tests {
    use super::{open_voice_overlay_effect, OpenVoiceOverlayEffect};

    #[test]
    fn opening_without_an_existing_overlay_creates_one() {
        assert_eq!(
            open_voice_overlay_effect(false),
            OpenVoiceOverlayEffect::Create
        );
    }

    #[test]
    fn repeated_open_reveals_the_single_existing_overlay() {
        assert_eq!(
            open_voice_overlay_effect(true),
            OpenVoiceOverlayEffect::Reveal
        );
    }
}
