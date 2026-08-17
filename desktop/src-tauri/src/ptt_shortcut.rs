//! Push-to-talk global shortcut lifecycle.
//!
//! `Ctrl+Space` is only reserved with the OS while a huddle is connected and
//! the voice input mode is push-to-talk. Reserving it for the whole app
//! lifetime conflicts with IDEs and other apps, so registration is synced to
//! huddle state instead.

use crate::huddle::HuddleState;
#[cfg(not(test))]
use crate::huddle::{HuddlePhase, VoiceInputMode};

/// Whether the PTT shortcut should currently be reserved with the OS.
#[cfg(not(test))]
fn should_register(hs: &HuddleState) -> bool {
    hs.voice_input_mode == VoiceInputMode::PushToTalk
        && matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
}

/// Register or unregister the PTT shortcut to match the given huddle state.
///
/// Idempotent and best-effort: failures are logged, never fatal — the huddle
/// still works in VAD mode without the shortcut.
#[cfg(not(test))]
pub fn sync_registration(app: &tauri::AppHandle, hs: &HuddleState) {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

    let shortcut = Shortcut::new(Some(Modifiers::CONTROL), Code::Space);
    let manager = app.global_shortcut();
    let want = should_register(hs);
    let is_registered = manager.is_registered(shortcut);

    if want && !is_registered {
        if let Err(e) = manager.register(shortcut) {
            eprintln!("buzz-desktop: failed to register PTT shortcut: {e}");
        }
    } else if !want && is_registered {
        if let Err(e) = manager.unregister(shortcut) {
            eprintln!("buzz-desktop: failed to unregister PTT shortcut: {e}");
        }
    }
}

/// Test builds omit the global-shortcut plugin (see `run()`), so syncing is a
/// no-op — calling the plugin would panic without it installed.
#[cfg(test)]
pub fn sync_registration(_app: &tauri::AppHandle, _hs: &HuddleState) {}
