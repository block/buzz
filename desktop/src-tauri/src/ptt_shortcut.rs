//! Push-to-talk global shortcut lifecycle.
//!
//! `Ctrl+Space` is only reserved with the OS while a huddle is connected and
//! the voice input mode is push-to-talk. Reserving it for the whole app
//! lifetime conflicts with IDEs and other apps, so registration is synced to
//! huddle state instead.

use crate::huddle::HuddleState;
#[cfg(not(test))]
use crate::huddle::{HuddlePhase, VoiceInputMode};
use tauri::{Builder, Runtime};

/// Install the global-shortcut plugin and its push-to-talk key handler.
///
/// No-op in test builds: linking the plugin into the lib-test binary makes it
/// fail to load on Windows (STATUS_ENTRYPOINT_NOT_FOUND) before any test runs.
/// `sync_registration` is stubbed out under the same cfg for the same reason.
#[cfg(test)]
pub fn install<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
}

/// Install the global-shortcut plugin and its push-to-talk key handler.
///
/// Registration itself is driven by huddle state through [`sync_registration`];
/// this only installs the plugin the handler runs on.
#[cfg(not(test))]
pub fn install<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    use crate::app_state::AppState;
    use std::sync::Arc;
    use tauri::{Emitter, Manager};
    use tauri_plugin_global_shortcut::ShortcutState;

    // Generation counter for the release delay task. Incremented on
    // every press — a delayed release only fires if the generation
    // hasn't changed (i.e. no new press happened during the delay).
    // This prevents press→release→press within 200 ms from having
    // the first release clobber the second press.
    let ptt_press_gen = Arc::new(std::sync::atomic::AtomicU64::new(0));

    builder.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, _shortcut, event| {
                let state = match app.try_state::<AppState>() {
                    Some(s) => s,
                    None => return,
                };

                // Only act if a huddle is active and mode is PTT.
                let (is_ptt_mode, is_active) = match state.huddle_state.lock() {
                    Ok(hs) => (
                        hs.voice_input_mode == VoiceInputMode::PushToTalk,
                        matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active),
                    ),
                    Err(_) => return,
                };

                if !is_ptt_mode || !is_active {
                    return;
                }

                match event.state {
                    ShortcutState::Pressed => {
                        // Bump generation — invalidates any pending release delay.
                        ptt_press_gen.fetch_add(1, std::sync::atomic::Ordering::Release);

                        if let Ok(hs) = state.huddle_state.lock() {
                            hs.ptt_active
                                .store(true, std::sync::atomic::Ordering::Release);
                            // Only cancel TTS if it's actually playing — avoids
                            // a stale cancel flag that drops the next queued message.
                            if hs.tts_active.load(std::sync::atomic::Ordering::Acquire) {
                                hs.tts_cancel
                                    .store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        // Emit ptt-state=true to the frontend.
                        // The React side plays the press audio cue on this event
                        // (Web Audio API via HuddleContext). Rust-side rodio audio
                        // was considered but rejected: the rodio OutputStream must
                        // outlive the handler and sharing it across the shortcut
                        // closure adds lifecycle complexity for marginal gain.
                        // The React implementation is sufficient and simpler.
                        let _ = app.emit("ptt-state", true);
                    }
                    ShortcutState::Released => {
                        // Capture generation at release time.
                        let gen_at_release =
                            ptt_press_gen.load(std::sync::atomic::Ordering::Acquire);
                        let gen_arc = Arc::clone(&ptt_press_gen);
                        let app_handle = app.clone();
                        // 200 ms release delay — captures the tail of the utterance.
                        // Only applies if no new press happened during the delay.
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            // Check generation — if it changed, a new press arrived.
                            if gen_arc.load(std::sync::atomic::Ordering::Acquire) != gen_at_release
                            {
                                return; // Superseded by a new press.
                            }
                            if let Some(state) = app_handle.try_state::<AppState>() {
                                if let Ok(hs) = state.huddle_state.lock() {
                                    hs.ptt_active
                                        .store(false, std::sync::atomic::Ordering::Release);
                                }
                            }
                            // Emit ptt-state=false — React plays the release audio cue.
                            let _ = app_handle.emit("ptt-state", false);
                        });
                    }
                }
            })
            .build(),
    )
}

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
