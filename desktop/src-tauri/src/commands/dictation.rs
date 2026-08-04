//! Composer dictation — local speech-to-text for the message composer.
//!
//! Mental model:
//!
//! ```text
//! AudioWorklet (48 kHz f32 PCM, /worklet.js)
//!   → push_dictation_pcm (Tauri cmd, raw binary)
//!   → SttPipeline::push_audio           [reused from huddle::stt]
//!       rubato 48→16 kHz, earshot VAD, Parakeet TDT-CTC 110M
//!   → text_rx
//!   → forwarding task → app.emit("dictation-text", …)
//!   → useDictation inserts at the composer caret
//! ```
//!
//! This reuses the huddle STT stack wholesale (model download, resampling,
//! VAD, inference — see [`crate::huddle::stt`]) and changes only the sink:
//! huddle publishes kind:9 events to the relay, dictation emits text back to
//! the webview. Nothing leaves the machine.
//!
//! Dictation is deliberately independent of [`crate::huddle::HuddleState`] —
//! dictating a message is the common case and must not require an active
//! huddle. The two pipelines can run concurrently; each owns its own
//! `SttPipeline` and its own audio feed command.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::{AppHandle, Emitter, State};

use crate::huddle::{models, stt::SttPipeline};

/// Event name carrying a finalized transcript segment to the webview.
pub const DICTATION_TEXT_EVENT: &str = "dictation-text";

/// A 100 ms batch at 48 kHz mono f32 is ~19 KB; 100 KB allows headroom
/// without letting a malformed IPC call allocate unbounded memory.
/// Mirrors the huddle audio-batch ceiling.
const MAX_AUDIO_BATCH_BYTES: usize = 100 * 1024;

/// Active dictation session.
///
/// Registered as its own Tauri managed state (`.manage()` in `lib.rs`) rather
/// than as an `AppState` field: dictation shares no data with the rest of the
/// app, so a separate lock keeps it off the contended `AppState` mutexes.
///
/// `cancel` is owned by the session rather than derived from a generation
/// counter: stopping sets it, which lets the forwarding task exit on its next
/// wakeup even though `text_rx` may still be open.
#[derive(Default)]
pub struct DictationState(Mutex<DictationSession>);

impl DictationState {
    /// Lock the session, converting a poisoned-lock error to a String.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, DictationSession>, String> {
        self.0.lock().map_err(|e| e.to_string())
    }
}

#[derive(Default)]
struct DictationSession {
    pipeline: Option<SttPipeline>,
    cancel: Option<Arc<AtomicBool>>,
}

impl DictationSession {
    /// Signal the current session (if any) to stop and release its pipeline.
    ///
    /// Returns the pipeline so the caller can drop it *outside* the state
    /// lock — `SttPipeline::drop` joins the worker thread (~200 ms) and must
    /// never block under the mutex.
    fn take_session(&mut self) -> Option<SttPipeline> {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        let pipeline = self.pipeline.take();
        if let Some(ref p) = pipeline {
            p.shutdown();
        }
        pipeline
    }
}

/// Whether dictation can start right now.
///
/// False while the Parakeet model is still downloading in the background
/// (see [`crate::huddle::models`]); the frontend uses this to disable the mic
/// button with an explanatory tooltip rather than failing on click.
#[tauri::command]
pub fn is_dictation_available() -> bool {
    models::is_stt_ready()
}

/// Begin a dictation session.
///
/// Idempotent in effect: an existing session is torn down first, so a
/// double-click cannot leak a pipeline or leave two workers feeding the same
/// event stream.
#[tauri::command]
pub fn start_dictation(app: AppHandle, state: State<'_, DictationState>) -> Result<(), String> {
    if !models::is_stt_ready() {
        return Err("speech model is still downloading".to_string());
    }
    let model_dir = models::stt_model_dir().ok_or("STT model directory not found")?;

    // Tear down any prior session before constructing the new one. The old
    // pipeline is dropped after the lock is released.
    let old = {
        let mut session = state.lock()?;
        session.take_session()
    };
    drop(old);

    // Dictation has no TTS to duck against and no push-to-talk gating: the
    // button *is* the gate. `tts_active` is a private always-false flag so the
    // worker's echo-suppression branches stay inert.
    let (pipeline, mut text_rx) =
        SttPipeline::new(model_dir, Arc::new(AtomicBool::new(false)), None, None)?;

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = Arc::clone(&cancel);

    tauri::async_runtime::spawn(async move {
        while let Some(text) = text_rx.recv().await {
            if cancel_task.load(Ordering::Acquire) {
                break;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Emit failure means the window is gone — stop rather than spin.
            if app.emit(DICTATION_TEXT_EVENT, trimmed).is_err() {
                break;
            }
        }
    });

    let mut session = state.lock()?;
    session.pipeline = Some(pipeline);
    session.cancel = Some(cancel);
    Ok(())
}

/// End the dictation session and release the worker thread.
///
/// Safe to call when no session is active.
#[tauri::command]
pub fn stop_dictation(state: State<'_, DictationState>) -> Result<(), String> {
    let old = {
        let mut session = state.lock()?;
        session.take_session()
    };
    // Dropped here, outside the lock — Drop joins the STT worker thread.
    drop(old);
    Ok(())
}

/// Receive raw PCM audio from the AudioWorklet and feed the dictation pipeline.
///
/// Expects a raw binary body of f32 LE samples at 48 kHz mono. If no session
/// is active the bytes are silently discarded — in-flight worklet batches can
/// arrive after `stop_dictation` and are not an error.
#[tauri::command]
pub fn push_dictation_pcm(
    request: tauri::ipc::Request<'_>,
    state: State<'_, DictationState>,
) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            if bytes.len() > MAX_AUDIO_BATCH_BYTES {
                return Err(format!(
                    "audio batch too large: {} bytes (max {})",
                    bytes.len(),
                    MAX_AUDIO_BATCH_BYTES
                ));
            }
            let session = state.lock()?;
            if let Some(ref pipeline) = session.pipeline {
                pipeline.push_audio(bytes.to_vec())?;
            }
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}

#[cfg(test)]
#[path = "dictation_tests.rs"]
mod tests;
