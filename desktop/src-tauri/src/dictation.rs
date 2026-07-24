//! Standalone composer dictation — streaming STT without a huddle.
//!
//! Reuses the huddle `SttPipeline` in live-transcript mode: the in-progress
//! phrase is re-decoded every ~300 ms and emitted as a partial (`final:
//! false`), so words appear in the composer as they're spoken; each natural
//! pause commits the phrase as a final (`final: true`). The frontend captures
//! mic audio with `getUserMedia` + the existing AudioWorklet and pushes PCM
//! via `push_dictation_pcm`. Transcripts are emitted to the webview as
//! `dictation-transcript` events instead of being posted to the relay.
//!
//! Session model: `start_dictation` marks the session active;
//! `stop_dictation` marks it inactive and pushes ~1 s of silence into the
//! pipeline — the mic stops with the key release, so without it the VAD would
//! never see the silence that closes (and decodes) the trailing phrase.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::Emitter;

use crate::app_state::AppState;
use crate::huddle::{models, stt::SttPipeline, HuddlePhase};

/// Payload for `dictation-transcript` events. Partials (`final: false`)
/// replace the previous partial of the same phrase; a final commits it.
#[derive(Clone, serde::Serialize)]
struct TranscriptEvent {
    text: String,
    #[serde(rename = "final")]
    is_final: bool,
}

/// Same cap as the huddle audio ingest (huddle/mod.rs).
const MAX_AUDIO_BATCH_BYTES: usize = 100 * 1024;

/// 1 s of 48 kHz f32-LE silence — comfortably past the worker's ~608 ms
/// live-mode silence-flush threshold (stt.rs LIVE_SILENCE_FLUSH_FRAMES)
/// after resampling. The margin matters: the threshold counts *consecutive*
/// silence frames, and the flush also unlocks the phrase's punctuation
/// (the model only commits it with ≥600 ms of trailing silence).
const FLUSH_SILENCE_BYTES: usize = 4 * 48_000;

/// Managed Tauri state for dictation.
///
/// ponytail: the pipeline stays alive after the first start (sherpa-onnx
/// model init takes seconds; ~110 MB resident). Add idle teardown if that
/// memory ever matters.
#[derive(Default)]
pub struct DictationState {
    pipeline: Mutex<Option<SttPipeline>>,
    /// True while a session is live — blocks a second concurrent session and
    /// gates `push_dictation_pcm` so late batches can't dirty the VAD buffer.
    active: AtomicBool,
}

/// Start (or resume) a dictation session.
///
/// Errors if the speech model isn't downloaded yet or a session is already
/// active (prevents two composers opening two mic streams into one pipeline).
#[tauri::command]
pub fn start_dictation(
    app: tauri::AppHandle,
    state: tauri::State<'_, DictationState>,
    app_state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state.active.load(Ordering::Acquire) {
        return Err("dictation already active".to_string());
    }
    // The huddle owns the mic (and Ctrl+Space) while it's up.
    if app_state
        .huddle()
        .is_ok_and(|hs| !matches!(hs.phase, HuddlePhase::Idle))
    {
        return Err("dictation is unavailable during a huddle".to_string());
    }

    let mut guard = state
        .pipeline
        .lock()
        .map_err(|_| "dictation state poisoned".to_string())?;

    // Clear a dead pipeline (init failure or crash) so retry works.
    if guard.as_ref().is_some_and(|p| p.is_finished()) {
        *guard = None;
    }

    if guard.is_none() {
        let model_dir = models::stt_model_dir()
            .ok_or("speech model still downloading — try again in a moment".to_string())?;
        let (live_tx, mut live_rx) = tokio::sync::mpsc::channel::<(String, bool)>(64);
        // In live mode all transcripts arrive on live_rx; the pipeline's own
        // text receiver stays silent and is dropped here.
        let (pipeline, _text_rx) = SttPipeline::new(
            model_dir,
            models::stt_model_family(),
            Arc::new(AtomicBool::new(false)), // no TTS during dictation
            None,
            None, // continuous VAD segments phrases at natural pauses
            Some(live_tx),
        )?;
        *guard = Some(pipeline);

        // Forward transcripts to the webview until the pipeline is dropped.
        tauri::async_runtime::spawn(async move {
            while let Some((text, is_final)) = live_rx.recv().await {
                let event = TranscriptEvent { text, is_final };
                if let Err(e) = app.emit("dictation-transcript", event) {
                    eprintln!("buzz-desktop: dictation transcript emit failed: {e}");
                }
            }
        });
    }

    state.active.store(true, Ordering::Release);
    Ok(())
}

/// End the session. Pushes silence so the VAD worker closes and decodes the
/// phrase in flight — that final transcript arrives as an event shortly
/// after. Idempotent.
#[tauri::command]
pub fn stop_dictation(state: tauri::State<'_, DictationState>) {
    if !state.active.swap(false, Ordering::AcqRel) {
        return;
    }
    if let Ok(guard) = state.pipeline.lock() {
        if let Some(ref pipeline) = *guard {
            let _ = pipeline.push_audio(vec![0u8; FLUSH_SILENCE_BYTES]);
        }
    }
}

/// Receive raw PCM (f32 LE, 48 kHz mono) from the AudioWorklet.
/// Mirrors `push_audio_pcm` but feeds the dictation pipeline.
#[tauri::command]
pub fn push_dictation_pcm(
    request: tauri::ipc::Request<'_>,
    state: tauri::State<'_, DictationState>,
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
            // Drop late batches after stop — the flush silence must be the
            // last audio the VAD sees for a session.
            if !state.active.load(Ordering::Acquire) {
                return Ok(());
            }
            let guard = state
                .pipeline
                .lock()
                .map_err(|_| "dictation state poisoned".to_string())?;
            if let Some(ref pipeline) = *guard {
                pipeline.push_audio(bytes.to_vec())?;
            }
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}
