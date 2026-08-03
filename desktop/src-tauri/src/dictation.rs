//! Local dictation pipeline — uses the Parakeet STT engine for offline
//! speech-to-text in the message composer.
//!
//! Unlike the huddle STT pipeline (which posts kind:9 events to the relay),
//! dictation emits transcribed text back to the frontend via Tauri events
//! so the composer can display it in real-time.
//!
//! Key differences from huddle STT:
//! - No TTS barge-in / echo gating (no agent voice in composer context)
//! - No shared huddle PTT gate; the composer controls its own session
//! - Slightly longer silence threshold for more coherent sentences
//! - Text goes to the frontend, not to the relay

use std::sync::{Arc, LazyLock, Mutex};

use tauri::{Emitter, State};

use crate::app_state::AppState;
use crate::huddle::{models, stt::SttPipeline};

/// Tauri event name emitted when a dictation transcript segment is ready.
const DICTATION_TRANSCRIPT_EVENT: &str = "dictation-transcript";

/// Tauri event name emitted when dictation state changes (started/stopped).
const DICTATION_STATE_EVENT: &str = "dictation-state";

/// State for the active dictation session.
///
/// Stored app-wide behind a `Mutex`. Only one dictation session can be active
/// at a time (starting a new one stops the previous).
pub(crate) struct DictationState {
    /// The running STT engine, if dictation is active.
    engine: Option<Arc<SttPipeline>>,
    /// Monotonically increasing session counter. Included in all emitted events
    /// so the frontend can ignore stale transcripts from a previous session's
    /// forwarder that arrive after a new session has started.
    session_id: u64,
}

impl DictationState {
    pub fn new() -> Self {
        Self {
            engine: None,
            session_id: 0,
        }
    }
}

static DICTATION_STATE: LazyLock<Mutex<DictationState>> =
    LazyLock::new(|| Mutex::new(DictationState::new()));

/// `start_dictation` — begin local STT dictation.
///
/// Starts the Parakeet STT engine and spawns a task that emits
/// `dictation-transcript` events to the frontend as text is recognized.
/// Returns an error if models are not downloaded yet.
#[tauri::command]
pub async fn start_dictation(state: State<'_, AppState>) -> Result<u64, String> {
    // Check if models are ready.
    if !models::is_stt_ready() {
        // Kick off download if not already in progress.
        if let Some(mgr) = models::global_model_manager() {
            mgr.start_stt_download(state.http_client.clone());
        }
        return Err("STT model not ready — download in progress".to_string());
    }

    let model_dir = models::stt_model_dir().ok_or("STT model directory not found")?;

    // Stop any existing dictation session first.
    stop_dictation_inner(None);

    let (engine, text_rx) = SttPipeline::new_dictation(model_dir)?;
    let engine = Arc::new(engine);

    // Store the engine in state and increment the session counter.
    let session_id = {
        let mut ds = DICTATION_STATE.lock().unwrap_or_else(|e| e.into_inner());
        ds.engine = Some(Arc::clone(&engine));
        ds.session_id += 1;
        ds.session_id
    };

    // Spawn a task that forwards transcribed text to the frontend.
    let app_handle = state
        .app_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    if let Some(handle) = app_handle {
        let _ = handle.emit(
            DICTATION_STATE_EVENT,
            serde_json::json!({ "state": "started", "session": session_id }),
        );
        spawn_dictation_forwarder(text_rx, handle, session_id);
    }

    Ok(session_id)
}

/// `stop_dictation` — stop the active dictation session.
///
/// The final transcript (if any) is emitted asynchronously by the forwarder
/// task. The `dictation-state: stopped` event is emitted by the forwarder
/// after all pending transcripts have been forwarded, ensuring the frontend
/// receives the final text before the stopped signal.
///
/// `session` scopes the stop to a specific session: the engine is only torn
/// down when the currently-stored `session_id` matches. This prevents a
/// delayed/fire-and-forget stop from an old session (e.g. one deferred behind
/// a final audio flush) from killing a *newer* session the user started in the
/// meantime. Pass `None` for an unconditional stop (used on cancel/unmount).
#[tauri::command]
pub fn stop_dictation(session: Option<u64>) -> Result<(), String> {
    stop_dictation_inner(session);
    // Note: `stopped` is emitted by the forwarder task after draining all
    // pending transcripts — not here. This avoids a race where the frontend
    // sees `stopped` before the final transcript arrives.
    Ok(())
}

/// `push_dictation_audio` — feed raw PCM bytes into the dictation pipeline.
///
/// Expects a raw binary body: an 8-byte little-endian `u64` session header
/// followed by f32 LE samples at 48 kHz mono. The header scopes the push to a
/// specific session — bytes are only fed to the engine when the header matches
/// the currently-stored `session_id`. This prevents late audio from a
/// just-stopped session (whose final `flushAudioBatch()` chunks are still
/// arriving) from being accepted by a *newer* session the user started in the
/// meantime and transcribed into the new draft.
///
/// If no dictation session is active, or the session header doesn't match the
/// active session, the bytes are silently discarded.
#[tauri::command]
pub fn push_dictation_audio(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    /// Size of the leading little-endian `u64` session header.
    const SESSION_HEADER_BYTES: usize = 8;
    /// Maximum IPC audio batch size (audio payload only, excluding the header): 100 KB.
    const MAX_AUDIO_BATCH_BYTES: usize = 100 * 1024;

    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            if bytes.len() < SESSION_HEADER_BYTES {
                return Err(format!(
                    "audio batch too small: {} bytes (need at least {} for session header)",
                    bytes.len(),
                    SESSION_HEADER_BYTES
                ));
            }
            let (header, audio) = bytes.split_at(SESSION_HEADER_BYTES);
            if audio.len() > MAX_AUDIO_BATCH_BYTES {
                return Err(format!(
                    "audio batch too large: {} bytes (max {})",
                    audio.len(),
                    MAX_AUDIO_BATCH_BYTES
                ));
            }
            // `split_at` guarantees `header` is exactly `SESSION_HEADER_BYTES` long.
            let session = u64::from_le_bytes(
                header
                    .try_into()
                    .map_err(|_| "invalid session header".to_string())?,
            );
            let ds = DICTATION_STATE.lock().unwrap_or_else(|e| e.into_inner());
            // Only feed audio tagged with the currently-active session. Late
            // chunks from an old session are silently dropped.
            if session == ds.session_id {
                if let Some(ref engine) = ds.engine {
                    engine.push_audio(audio.to_vec())?;
                }
            }
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}

/// `get_dictation_status` — check if local dictation is available and/or active.
#[tauri::command]
pub fn get_dictation_status() -> DictationStatus {
    let model_ready = models::is_stt_ready();
    let is_active = DICTATION_STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .engine
        .is_some();

    DictationStatus {
        available: model_ready,
        active: is_active,
    }
}

/// Response for `get_dictation_status`.
#[derive(serde::Serialize, Clone)]
pub struct DictationStatus {
    /// Whether the local STT model is downloaded and ready.
    pub available: bool,
    /// Whether a dictation session is currently active.
    pub active: bool,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn stop_dictation_inner(session: Option<u64>) {
    let old_engine = {
        let mut ds = DICTATION_STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Session-scoped stop: only tear down when the requested session matches
        // the one currently stored. A `None` session stops unconditionally.
        match session {
            Some(requested) if requested != ds.session_id => None,
            _ => ds.engine.take(),
        }
    };
    if let Some(engine) = old_engine {
        engine.shutdown();
        // Drop outside the lock — thread join may block briefly.
        drop(engine);
    }
}

/// Spawn an async task that reads transcribed text and emits Tauri events.
///
/// Each event includes the `session` ID so the frontend can ignore stale
/// transcripts from a previous session's forwarder. When the channel closes
/// (engine stopped), the forwarder emits `dictation-state: stopped`.
fn spawn_dictation_forwarder(
    mut text_rx: tokio::sync::mpsc::Receiver<String>,
    app_handle: tauri::AppHandle,
    session_id: u64,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(text) = text_rx.recv().await {
            if text.is_empty() {
                continue;
            }
            let payload = serde_json::json!({ "text": text, "session": session_id });
            if app_handle
                .emit(DICTATION_TRANSCRIPT_EVENT, payload)
                .is_err()
            {
                break; // App window closed.
            }
        }
        // All transcripts forwarded — signal the frontend that dictation is done.
        let _ = app_handle.emit(
            DICTATION_STATE_EVENT,
            serde_json::json!({ "state": "stopped", "session": session_id }),
        );
    });
}
