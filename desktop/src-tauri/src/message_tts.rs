//! App-scoped read-aloud playback for ordinary chat messages.
//!
//! Mental model:
//!
//! ```text
//! frontend: Play on a delivered agent message
//!   → speak_chat_message(message_id, text)
//!       1. Reject while a huddle is live (one audio owner at a time)
//!       2. Reject when the Pocket TTS model is not ready (kick off download)
//!       3. Preprocess + truncate (MAX_MESSAGE_TTS_CHARS, disclosed to the UI)
//!       4. Supersede any active playback (bump generation, cancel, emit)
//!       5. Lazily build a dedicated TtsPipeline (NOT the huddle one)
//!       6. Queue text, emit "preparing", spawn a watcher task
//!   → watcher polls the pipeline's active flag
//!       active rises  → emit "playing"
//!       active falls  → emit "completed"
//!       generation changed → exit silently (a newer request owns the UI)
//! frontend: Stop → stop_chat_message_speech → cancel + emit "stopped"
//! ```
//!
//! This module deliberately does NOT touch `HuddleState`. It owns its own
//! `TtsPipeline` instance with fresh `active`/`cancel` flags, so message
//! read-aloud carries none of the huddle's STT echo-gating, barge-in, or
//! phase lifecycle. The only huddle coupling is exclusion: `speak` refuses
//! while a huddle is non-idle, and huddle start/join calls
//! [`stop_message_playback`] so two pipelines never fight one output device.
//!
//! State machine (message-keyed, generation-guarded):
//!
//! ```text
//! preparing → playing → completed
//!     │          │
//!     ├── stopped (explicit stop / huddle start)
//!     ├── superseded (newer play request)
//!     └── failed (engine/audio/queue error)
//! ```
//!
//! Every transition is emitted as a `message-tts-state-changed` Tauri event.
//! Stale worker callbacks can never overwrite newer state: each play bumps
//! `generation`, and both the watcher and terminal emits check it first.

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::app_state::AppState;
use crate::huddle::preprocessing::preprocess_for_tts;
use crate::huddle::tts::TtsPipeline;
use crate::huddle::{models, HuddlePhase};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum characters synthesized per message (mirrors the huddle cap).
/// ~2000 chars ≈ 1–2 minutes of speech. The UI is told via `truncated`
/// so the cut is disclosed rather than silent.
pub(crate) const MAX_MESSAGE_TTS_CHARS: usize = 2000;

/// Spoken suffix appended when a message is truncated, so the listener also
/// hears the disclosure.
pub(crate) const TRUNCATION_SUFFIX: &str = "... message truncated.";

/// How often the watcher polls the pipeline's active flag.
const WATCH_TICK: Duration = Duration::from_millis(25);

/// How long the watcher waits for audio to start before declaring failure.
/// First-chunk synthesis is a single sentence (~1s warm); 30s covers a cold
/// engine plus pathological first sentences with a wide margin.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `speak` waits for the worker to consume a cancel before deciding
/// the pipeline is wedged and rebuilding it. The worker's recv loop observes
/// the flag within ~100 ms.
const CANCEL_CONSUME_TIMEOUT: Duration = Duration::from_secs(2);

/// Tauri event name for playback state transitions.
pub(crate) const STATE_EVENT: &str = "message-tts-state-changed";

// ── State ─────────────────────────────────────────────────────────────────────

/// App-scoped message read-aloud state. Lives on [`AppState`].
pub struct MessageTtsState {
    /// Pipeline + current playback bookkeeping. Never held across `.await`.
    inner: Mutex<MessageTtsInner>,
    /// Monotonic play-request counter. Bumped by every `speak` and `stop`;
    /// watchers capture it at spawn and exit when it moves on.
    generation: Arc<AtomicU64>,
    /// True while audio for the current request is queued or playing.
    /// Shared with the pipeline worker thread (same contract as huddle TTS,
    /// but nothing else reads it — no STT gating here).
    active: Arc<AtomicBool>,
    /// Cancel flag consumed by the pipeline worker: drains the queue and
    /// silences the player.
    cancel: Arc<AtomicBool>,
    /// TOCTOU sentinel for pipeline construction (mirrors `tts_starting`).
    starting: Arc<AtomicBool>,
}

struct MessageTtsInner {
    pipeline: Option<Arc<TtsPipeline>>,
    /// The message whose audio currently owns the player, if any.
    current_message_id: Option<String>,
}

impl Default for MessageTtsState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(MessageTtsInner {
                pipeline: None,
                current_message_id: None,
            }),
            generation: Arc::new(AtomicU64::new(0)),
            active: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            starting: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl MessageTtsState {
    fn lock(&self) -> std::sync::MutexGuard<'_, MessageTtsInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ── Frontend event payload ────────────────────────────────────────────────────

/// Payload of every `message-tts-state-changed` emission.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PlaybackEvent {
    pub message_id: String,
    /// "preparing" | "playing" | "completed" | "stopped" | "superseded" | "failed"
    pub status: &'static str,
    /// True when the spoken text was cut at [`MAX_MESSAGE_TTS_CHARS`].
    pub truncated: bool,
    pub error: Option<String>,
}

fn emit_playback_event(state: &AppState, event: &PlaybackEvent) {
    use tauri::Emitter;
    let app = match state.app_handle.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };
    if let Some(app) = app {
        let _ = app.emit(STATE_EVENT, event);
    }
}

// ── Pure helpers (unit-tested in message_tts_tests.rs) ────────────────────────

/// Preprocess and cap the raw message body for synthesis.
///
/// Returns the speakable text plus whether it was truncated, or `Err` when
/// preprocessing leaves nothing to say (emoji-only, code-only, etc.).
/// Truncation counts chars (not bytes) so multi-byte UTF-8 never panics.
pub(crate) fn prepare_speech_text(raw: &str) -> Result<(String, bool), String> {
    let truncated = raw.chars().count() > MAX_MESSAGE_TTS_CHARS;
    let capped: String = if truncated {
        let mut t: String = raw.chars().take(MAX_MESSAGE_TTS_CHARS).collect();
        t.push_str(TRUNCATION_SUFFIX);
        t
    } else {
        raw.to_string()
    };

    let speakable = preprocess_for_tts(&capped);
    if speakable.is_empty() {
        return Err("message has no speakable text".to_string());
    }
    Ok((speakable, truncated))
}

/// Whether a watcher spawned at `spawned_gen` still owns the UI state.
/// Split out so the supersede contract is directly testable.
pub(crate) fn watcher_owns_state(generation: &AtomicU64, spawned_gen: u64) -> bool {
    generation.load(Ordering::Acquire) == spawned_gen
}

// ── Playback control ──────────────────────────────────────────────────────────

/// Stop any active message playback. Sync + lock-only, safe from any thread.
/// Used by `stop_chat_message_speech` and by huddle start/join (the huddle
/// takes ownership of the audio device).
///
/// `status` is "stopped" for both paths — from the UI's perspective the
/// playback ended before completion at the user's request either way.
pub(crate) fn stop_message_playback(state: &AppState) {
    let mtts = &state.message_tts;
    let previous = {
        let mut inner = mtts.lock();
        inner.current_message_id.take()
    };
    // Invalidate watchers FIRST so a falling active flag can't emit
    // "completed" concurrently with our "stopped".
    mtts.generation.fetch_add(1, Ordering::AcqRel);
    mtts.cancel.store(true, Ordering::Release);

    if let Some(message_id) = previous {
        emit_playback_event(
            state,
            &PlaybackEvent {
                message_id,
                status: "stopped",
                truncated: false,
                error: None,
            },
        );
    }
}

/// Ensure a live dedicated pipeline exists, building one if needed.
/// Never called with the inner lock held across the construction await.
async fn ensure_pipeline(state: &AppState) -> Result<Arc<TtsPipeline>, String> {
    // Fast path: existing live pipeline.
    {
        let inner = state.message_tts.lock();
        if let Some(ref p) = inner.pipeline {
            if !p.is_finished() {
                return Ok(Arc::clone(p));
            }
        }
    }

    let model_dir = models::tts_model_dir().ok_or_else(|| {
        // Kick the download so a retry can succeed without a huddle first.
        if let Some(mgr) = models::global_model_manager() {
            mgr.start_tts_download(state.http_client.clone());
        }
        "voice model is not downloaded yet — download started, try again shortly".to_string()
    })?;

    // Claim the construction slot (TOCTOU guard, mirrors huddle tts_starting).
    if state.message_tts.starting.swap(true, Ordering::AcqRel) {
        return Err("read-aloud is still starting up — try again in a moment".to_string());
    }

    let active = Arc::clone(&state.message_tts.active);
    let cancel = Arc::clone(&state.message_tts.cancel);
    let output_device = state
        .audio_output_device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let constructed = tokio::task::spawn_blocking(move || {
        TtsPipeline::new(model_dir, active, cancel, output_device)
    })
    .await;

    state.message_tts.starting.store(false, Ordering::Release);

    let pipeline = match constructed {
        Ok(Ok(p)) => Arc::new(p),
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(format!("spawn_blocking failed: {e}")),
    };

    let mut inner = state.message_tts.lock();
    // A dead pipeline may still be stored; replace it. (A concurrent build is
    // impossible — we held the `starting` sentinel.)
    inner.pipeline = Some(Arc::clone(&pipeline));
    Ok(pipeline)
}

/// Wait for the worker to consume the cancel flag so newly queued text is not
/// swallowed by the post-cancel queue drain. Returns `false` on timeout
/// (wedged worker — caller rebuilds the pipeline).
async fn wait_cancel_consumed(cancel: &AtomicBool) -> bool {
    let deadline = std::time::Instant::now() + CANCEL_CONSUME_TIMEOUT;
    while cancel.load(Ordering::Acquire) {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(WATCH_TICK).await;
    }
    true
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Read a delivered chat message aloud. One playback app-wide: a newer call
/// supersedes the previous one.
#[tauri::command]
pub async fn speak_chat_message(
    message_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Pre-flight failures (before any state is claimed) still emit a "failed"
    // event so the frontend has one uniform error surface. They never touch
    // generation/current_message, so an unrelated active playback is unharmed
    // (the frontend reducer keys on message_id).
    let preflight_fail = |err: String, state: &AppState| {
        emit_playback_event(
            state,
            &PlaybackEvent {
                message_id: message_id.clone(),
                status: "failed",
                truncated: false,
                error: Some(err.clone()),
            },
        );
        Err::<(), String>(err)
    };

    // Huddles own the audio device while live — refuse, don't mix.
    {
        let hs = state.huddle()?;
        if hs.phase != HuddlePhase::Idle {
            drop(hs);
            return preflight_fail(
                "read-aloud is unavailable during a huddle".to_string(),
                &state,
            );
        }
    }

    let (speech_text, truncated) = match prepare_speech_text(&text) {
        Ok(v) => v,
        Err(e) => return preflight_fail(e, &state),
    };

    // Supersede any current playback and claim the new generation.
    let superseded = {
        let mut inner = state.message_tts.lock();
        inner.current_message_id.replace(message_id.clone())
    };
    let gen = state.message_tts.generation.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(old_id) = superseded {
        // The same message re-played also passes through supersede: the old
        // audio stops and a fresh request starts — deterministic either way.
        state.message_tts.cancel.store(true, Ordering::Release);
        emit_playback_event(
            &state,
            &PlaybackEvent {
                message_id: old_id,
                status: "superseded",
                truncated: false,
                error: None,
            },
        );
    }

    emit_playback_event(
        &state,
        &PlaybackEvent {
            message_id: message_id.clone(),
            status: "preparing",
            truncated,
            error: None,
        },
    );

    let fail = |err: String, state: &AppState| {
        // Only report failure if no newer request took over meanwhile.
        if watcher_owns_state(&state.message_tts.generation, gen) {
            let mut inner = state.message_tts.lock();
            if inner.current_message_id.as_deref() == Some(message_id.as_str()) {
                inner.current_message_id = None;
            }
            drop(inner);
            emit_playback_event(
                state,
                &PlaybackEvent {
                    message_id: message_id.clone(),
                    status: "failed",
                    truncated,
                    error: Some(err.clone()),
                },
            );
        }
        Err::<(), String>(err)
    };

    let mut pipeline = match ensure_pipeline(&state).await {
        Ok(p) => p,
        Err(e) => return fail(e, &state),
    };

    // If we cancelled a previous playback, wait for the worker to consume the
    // flag — otherwise its post-cancel queue drain would swallow our text.
    if state.message_tts.cancel.load(Ordering::Acquire)
        && !wait_cancel_consumed(&state.message_tts.cancel).await
    {
        // Worker wedged: drop it and rebuild once. Old pipeline drops (and
        // joins its thread) outside the inner lock.
        let old = {
            let mut inner = state.message_tts.lock();
            inner.pipeline.take()
        };
        if let Some(p) = old {
            p.shutdown();
            drop(p);
        }
        state.message_tts.cancel.store(false, Ordering::Release);
        pipeline = match ensure_pipeline(&state).await {
            Ok(p) => p,
            Err(e) => return fail(e, &state),
        };
    }

    // A stop/supersede may have landed while we were constructing/waiting.
    if !watcher_owns_state(&state.message_tts.generation, gen) {
        return Ok(());
    }

    if let Err(e) = pipeline.speak(speech_text) {
        return fail(e, &state);
    }

    spawn_playback_watcher(&state, message_id, gen, truncated);
    Ok(())
}

/// Stop the active message playback, if any. Always succeeds.
#[tauri::command]
pub fn stop_chat_message_speech(state: State<'_, AppState>) -> Result<(), String> {
    stop_message_playback(&state);
    Ok(())
}

// ── Watcher ───────────────────────────────────────────────────────────────────

/// Watch the pipeline's active flag and emit playing/completed/failed for
/// `message_id`. Exits silently the moment `generation` moves past
/// `spawned_gen` — the superseding/stopping call already emitted for us.
fn spawn_playback_watcher(state: &AppState, message_id: String, spawned_gen: u64, truncated: bool) {
    let generation = Arc::clone(&state.message_tts.generation);
    let active = Arc::clone(&state.message_tts.active);
    let app = match state.app_handle.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    let Some(app) = app else { return };

    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let emit = |status: &'static str, error: Option<String>| {
            let _ = app.emit(
                STATE_EVENT,
                PlaybackEvent {
                    message_id: message_id.clone(),
                    status,
                    truncated,
                    error,
                },
            );
        };

        // Phase 1: wait for audio to start.
        let start_deadline = std::time::Instant::now() + START_TIMEOUT;
        loop {
            if !watcher_owns_state(&generation, spawned_gen) {
                return;
            }
            if active.load(Ordering::Acquire) {
                break;
            }
            if std::time::Instant::now() >= start_deadline {
                if watcher_owns_state(&generation, spawned_gen) {
                    emit(
                        "failed",
                        Some("audio did not start — the voice engine may be unavailable".into()),
                    );
                }
                return;
            }
            tokio::time::sleep(WATCH_TICK).await;
        }
        emit("playing", None);

        // Phase 2: wait for the player to drain.
        loop {
            if !watcher_owns_state(&generation, spawned_gen) {
                return;
            }
            if !active.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(WATCH_TICK).await;
        }
        if watcher_owns_state(&generation, spawned_gen) {
            emit("completed", None);
        }
    });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "message_tts_tests.rs"]
mod tests;
