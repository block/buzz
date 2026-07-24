//! Text-to-Speech pipeline for huddle agent voice output.
//!
//! Mental model:
//!
//! ```text
//! caller: pipeline.speak("Hello world. How are you?")
//!   → bounded sync_channel (TEXT_QUEUE_DEPTH = 8)
//!   → tts_worker thread (owns 1 Pocket TTS engine + 1 persistent Player)
//!       1. Preprocess text
//!       2. Split into sentences
//!       3. Synthesize each sentence individually → f32 PCM
//!       4. Clamp to full scale + fade out each sentence
//!       5. Append each buffer to the persistent rodio Player (gapless)
//!       6. While audio is draining, keep pulling queued text items and
//!          synthesizing ahead — playback of item N overlaps synthesis of
//!          item N+1
//!   → tts_active = true while audio is queued/playing, false when idle
//!   → cancel flag: a 10 ms barge-in monitor thread silences the player and
//!     releases tts_active on the flag's rising edge (~15 ms flag-to-silence,
//!     even mid-sentence while the worker is blocked in synth_chunk); the
//!     worker then consumes the flag — drain queue + clear + play (un-pause).
//!     Monitor clears and worker player mutations are serialized through the
//!     `player_ops` mutex, with the flag re-checked under the lock — see the
//!     monitor block in `tts_worker` for the race this closes.
//! ```
//!
//! Lookahead pipelining spans *items*, not just sentences within one item:
//! the worker only blocks on the text channel when the player is empty.
//! With sentence-per-message delivery (each agent message ≈ one sentence),
//! a per-item drain barrier would insert a full synth latency of dead air
//! between every pair of sentences — the cross-item overlap is what keeps
//! multi-message replies gapless.
//!
//! `tts_active` is an `Arc<AtomicBool>` shared with the STT pipeline so STT
//! can gate microphone input while the agent is speaking.

use std::{
    num::NonZero,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread,
    time::Duration,
};

use super::pocket::{load_text_to_speech, load_voice_style, SAMPLE_RATE, VOICE_FILE_EXT};
use super::preprocessing::{preprocess_for_tts, split_sentences};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of queued text items.
/// Prevents unbounded accumulation when the agent produces text faster than
/// TTS can play it. Excess items are dropped with a warning.
const TEXT_QUEUE_DEPTH: usize = 8;

/// How long the worker waits on the text channel before checking the shutdown flag.
const RECV_TIMEOUT: Duration = Duration::from_millis(100);

/// Poll interval of the barge-in monitor thread. Bounds flag-to-silence
/// latency: a cancel is noticed within one tick, and rodio's internal
/// `periodic_access` wrapper stops the in-flight source within a further
/// ~5 ms — so playing audio dies ~15 ms after the flag is set, even while
/// the worker is blocked inside `synth_chunk`.
const MONITOR_TICK: Duration = Duration::from_millis(10);

/// Pocket TTS is a one-step consistency model, not diffusion. Kept for API compat.
const SYNTH_STEPS: usize = 1;

/// Fade-out length in samples (8 ms at 24 kHz ≈ 192 samples).
///
/// Applied only at the *end* of each synthesised sentence to eliminate the
/// click that would otherwise occur when a non-zero waveform terminates
/// abruptly. **No fade-in is applied** — see `apply_fade_out` for the
/// rationale and `examples/pocket_onset_probe.rs` for the measurement that
/// motivated removing the leading fade.
const FADE_OUT_SAMPLES: usize = (SAMPLE_RATE as f64 * 0.008) as usize;

/// Length of the zero-sample cushion prepended before each synthesized
/// sentence chunk, so the OS audio device / rodio mixer has a fully-quiet
/// ramp-up window before the real onset hits.
///
/// This used to be applied only before the first sentence of a whole response.
/// That still left later sentence chunks vulnerable to first-syllable clipping
/// when their first phoneme was soft (notably `I'm` / `I've`) and rodio crossed
/// from an explicit silence buffer straight into non-zero speech. 20 ms ≈ 480
/// samples is enough to cover a CoreAudio buffer turnover without being audible
/// as latency. At sentence boundaries this lead-in is budgeted out of the
/// existing inter-sentence pause, so it does not lengthen multi-sentence gaps.
const SENTENCE_LEAD_IN_SAMPLES: usize = (SAMPLE_RATE as f64 * 0.020) as usize;

/// Approximate character budget for one synthesis chunk.
///
/// Upstream pocket-tts groups sentences into chunks of up to
/// `MAX_TOKEN_PER_CHUNK = 50` tokenizer tokens (`default_parameters.py`) —
/// typically multi-sentence chunks — because every `generate()` call is an
/// independent generation with a cold FlowLM start, and each chunk boundary
/// is an exposed prosody seam (kyutai-labs/pocket-tts #151; the Kyutai team
/// names chunk stitching as the reliability lever). Our previous
/// sentence-per-call path created ~2–4× more seams than upstream.
///
/// We don't ship the SentencePiece tokenizer, so 50 tokens is approximated
/// with a character budget. The bundled 4k-entry vocab averages ~4 chars per
/// token, but usage-weighted English text leans on short common tokens, so
/// the effective ratio is ~2–4 chars/token and 200 chars ≈ 60–100 tokens —
/// modestly above upstream's 50, deliberately: erring large means fewer
/// seams, and even ~100 tokens is far below the model's 500-LM-step (~40 s)
/// ceiling. Do not shrink this budget to chase an exact 50-token match.
const MAX_CHUNK_CHARS: usize = 200;

/// Silence inserted between sentences by the TTS pipeline (seconds).
/// Injected as a silent buffer between each synthesized sentence chunk.
const INTER_SENTENCE_SILENCE: f32 = 0.1;

// ── Public pipeline handle ────────────────────────────────────────────────────

/// Handle to the running TTS pipeline.
///
/// Not Clone — wrap in `Arc` to share across threads.
#[derive(Debug)]
pub struct TtsPipeline {
    /// Send preprocessed text into the pipeline.
    text_tx: SyncSender<String>,
    /// `true` while the agent is speaking. Shared with the STT pipeline for gating.
    #[allow(dead_code)]
    pub tts_active: Arc<AtomicBool>,
    /// Signals the worker thread to stop.
    shutdown: Arc<AtomicBool>,
    /// Cancel flag: worker drains the queue and stops current playback.
    /// Kept alive here so the Arc isn't dropped — the worker holds a clone.
    #[allow(dead_code)]
    cancel: Arc<AtomicBool>,
    /// Voice name (e.g. "reference_sample"). Stored for future voice-switching support.
    #[allow(dead_code)]
    voice: String,
    /// Worker thread handle — taken on drop to join cleanly.
    thread: Option<thread::JoinHandle<()>>,
}

impl TtsPipeline {
    /// Spawn the TTS pipeline thread using the default voice.
    ///
    /// `model_dir` must contain the Pocket TTS files declared by `huddle::models`
    /// (the five ONNX sessions, the two JSON tables, and `<voice>.wav`).
    ///
    /// `tts_active` is set to `true` while audio is playing and `false` when idle.
    /// Pass the same `Arc` to the STT pipeline to gate microphone input.
    ///
    /// `cancel` is the shared barge-in flag from `HuddleState.tts_cancel`. Pass the
    /// same `Arc` to the STT pipeline so both sides reference the same flag for the
    /// entire huddle session — no stale references after pipeline restarts.
    pub fn new(
        model_dir: PathBuf,
        tts_active: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        output_device: Option<String>,
    ) -> Result<Self, String> {
        use super::pocket::DEFAULT_VOICE;
        Self::new_with_voice(model_dir, tts_active, cancel, DEFAULT_VOICE, output_device)
    }

    /// Spawn the TTS pipeline thread with a specific voice name. Today only the
    /// bundled default voice (see `pocket::DEFAULT_VOICE`) is shipped; other
    /// names will surface a clear error from `load_voice_style`.
    pub fn new_with_voice(
        model_dir: PathBuf,
        tts_active: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        voice: &str,
        output_device: Option<String>,
    ) -> Result<Self, String> {
        let (text_tx, text_rx) = mpsc::sync_channel::<String>(TEXT_QUEUE_DEPTH);
        let shutdown = Arc::new(AtomicBool::new(false));
        // cancel is passed in from HuddleState.tts_cancel — shared with STT for barge-in.

        let shutdown_worker = Arc::clone(&shutdown);
        let cancel_worker = Arc::clone(&cancel);
        let tts_active_worker = Arc::clone(&tts_active);
        let voice_name = voice.to_string();
        let model_dir_worker = model_dir.clone();

        let handle = thread::Builder::new()
            .name("tts-worker".into())
            .spawn(move || {
                tts_worker(
                    model_dir_worker,
                    voice_name,
                    text_rx,
                    tts_active_worker,
                    shutdown_worker,
                    cancel_worker,
                    output_device,
                )
            })
            .map_err(|e| format!("failed to spawn tts-worker thread: {e}"))?;

        Ok(Self {
            text_tx,
            tts_active,
            shutdown,
            cancel,
            voice: voice.to_string(),
            thread: Some(handle),
        })
    }

    /// Queue `text` for TTS synthesis and playback.
    ///
    /// Non-blocking. Returns `Err` if the queue is full (bounded at
    /// `TEXT_QUEUE_DEPTH`) — caller may log and discard.
    pub fn speak(&self, text: String) -> Result<(), String> {
        self.text_tx.try_send(text).map_err(|e| {
            eprintln!("buzz-desktop: TTS queue saturated, dropping message: {e}");
            format!("TTS queue full, dropping: {e}")
        })
    }

    /// Signal the worker thread to stop.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Returns `true` if the worker thread has exited (init failure, crash, or normal exit).
    /// Used by hot-start to detect dead pipelines and clear them for retry.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|h| h.is_finished())
    }
}

impl Drop for TtsPipeline {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Dropping `text_tx` unblocks the worker's recv_timeout loop.
        // Join to ensure the audio thread exits cleanly.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────────

fn tts_worker(
    model_dir: PathBuf,
    voice_name: String,
    text_rx: mpsc::Receiver<String>,
    tts_active: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    output_device: Option<String>,
) {
    // ── 1. Initialise TTS engine ──────────────────────────────────────────────
    let model_dir_str = model_dir.to_string_lossy().to_string();

    let engine = match load_text_to_speech(&model_dir_str) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "buzz-desktop: TTS engine init failed (model_dir={}): {e}. TTS disabled.",
                model_dir.display()
            );
            drain_until_shutdown(text_rx, &shutdown);
            return;
        }
    };

    // ── 2. Load voice style ───────────────────────────────────────────────────
    let voice_path = model_dir.join(format!("{voice_name}.{VOICE_FILE_EXT}"));
    let style = match load_voice_style(&voice_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "buzz-desktop: TTS voice style load failed ({voice_name}): {e}. TTS disabled."
            );
            drain_until_shutdown(text_rx, &shutdown);
            return;
        }
    };

    // ── 2b. Warmup inference ─────────────────────────────────────────────────
    // The first ONNX inference on any session is significantly slower than
    // subsequent ones — it can trigger native session initialization, memory
    // pool allocation, and graph-specific caches. Run a short dummy synthesis
    // and discard the output so the first real utterance runs at warm-session speed.
    {
        let t = std::time::Instant::now();
        match engine.synth_chunk("warmup", "en", &style, SYNTH_STEPS) {
            Ok(_) => eprintln!(
                "buzz-desktop: TTS warmup completed in {:.0}ms",
                t.elapsed().as_millis()
            ),
            Err(e) => eprintln!(
                "buzz-desktop: TTS warmup failed after {:.0}ms: {e} — first utterance may be slow",
                t.elapsed().as_millis()
            ),
        }
    }

    // ── 3. Initialise rodio output device ─────────────────────────────────────
    use rodio::buffer::SamplesBuffer;
    use rodio::Player;

    let sink_handle = match super::audio_output::open_output_sink_by_name(output_device.as_deref())
    {
        Ok(h) => h,
        Err(e) => {
            eprintln!("buzz-desktop: TTS audio output failed: {e}. TTS disabled.");
            drain_until_shutdown(text_rx, &shutdown);
            return;
        }
    };

    let channels = match NonZero::new(1u16) {
        Some(c) => c,
        None => {
            eprintln!("buzz-desktop: TTS channel count invariant violated");
            return;
        }
    };
    let rate = match NonZero::new(SAMPLE_RATE) {
        Some(r) => r,
        None => {
            eprintln!("buzz-desktop: TTS sample rate invariant violated");
            return;
        }
    };

    // Single persistent Player for the lifetime of the worker — all sentence
    // buffers from all text items append here, and rodio plays them gaplessly.
    // Persistence is what enables cross-item pipelining: the worker never
    // waits for one item to drain before synthesizing the next.
    //
    // Shared (Arc) with the barge-in monitor thread below, which needs to
    // silence it while this thread is blocked inside `synth_chunk`.
    let player = Arc::new(Player::connect_new(sink_handle.mixer()));

    // Prime the audio output stream with a short silent buffer.
    // On macOS, CoreAudio initializes the output device lazily on first use.
    // Without this, the first real append races against device startup and
    // player.empty() returns true before audio has started draining — causing
    // the first TTS message to be truncated after a few words.
    {
        let silence = vec![0.0f32; SAMPLE_RATE as usize / 10]; // 100ms of silence
        player.append(SamplesBuffer::new(channels, rate, silence));
        // Wait for the silent buffer to drain — this ensures the output stream
        // is fully initialized before the first real utterance.
        while !player.empty() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    // ── 3b. Barge-in monitor thread ───────────────────────────────────────────
    //
    // The worker loop only observes `cancel` between sentences — while it is
    // blocked inside `synth_chunk` (hundreds of ms for a long sentence),
    // nothing would silence the audio that is already playing. The monitor
    // closes that gap: every MONITOR_TICK it checks the flag and, while set,
    // silences the player and releases the mic gate. It does NOT consume the
    // flag — the worker still owns that (drain queue, reset lead-in), so the
    // monitor keeps re-clearing until the worker catches up, which also
    // covers a sentence appended in the race window after the worker's own
    // post-synthesis cancel check.
    //
    // `player_ops` closes the converse race (found in review): the monitor
    // loads `cancel == true`, is preempted, the worker consumes the cancel
    // and appends a fresh post-cancel utterance, then the monitor resumes
    // from its stale branch and deletes audio that was meant to play. All
    // worker player mutations (appends and cancel/shutdown clears) hold this
    // lock, and the monitor re-checks `cancel` *while holding it* — so its
    // clear either runs before fresh audio can be appended, or observes
    // `cancel == false` and no-ops. The lock is uncontended except during an
    // actual barge-in, so the hot path is unaffected.
    let player_ops = Arc::new(Mutex::new(()));
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = {
        let player = Arc::clone(&player);
        let cancel = Arc::clone(&cancel);
        let tts_active = Arc::clone(&tts_active);
        let stop = Arc::clone(&monitor_stop);
        let player_ops = Arc::clone(&player_ops);
        thread::Builder::new()
            .name("tts-barge-in-monitor".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    if cancel.load(Ordering::Acquire) {
                        let _ops = lock_player_ops(&player_ops);
                        // Re-check under the lock: the worker may have
                        // consumed this cancel (and appended fresh audio)
                        // between the load above and the lock acquisition.
                        if cancel.load(Ordering::Acquire) {
                            // clear() pauses the persistent player; play()
                            // un-pauses (see handle_cancel_or_shutdown).
                            // Idempotent — safe to repeat every tick until
                            // the worker consumes the flag.
                            player.clear();
                            player.play();
                            tts_active.store(false, Ordering::Release);
                        }
                    }
                    thread::sleep(MONITOR_TICK);
                }
            })
    };
    if let Err(ref e) = monitor {
        // Degraded but functional: barge-in still works between sentences
        // via the worker's own checks, just not mid-synthesis.
        eprintln!("buzz-desktop: TTS barge-in monitor failed to spawn: {e}");
    }

    // ── 4. Main loop ──────────────────────────────────────────────────────────
    //
    // One iteration = one text item. The worker blocks on the channel for at
    // most RECV_TIMEOUT and never waits for playback to drain before taking
    // the next item — synthesis of item N+1 overlaps playback of item N.
    // `tts_active` lifecycle: set on the first append while idle, cleared
    // whenever the player has fully drained — either in the idle timeout
    // arm or on item receipt before synthesis begins.
    let silence_buf_len = (INTER_SENTENCE_SILENCE * SAMPLE_RATE as f32) as usize;
    // `first_append` = "no audio queued since the player last went idle".
    // Flipped by `build_sentence_append_buffer` on the first real append; the
    // idle branch below uses it to decide when to drop `tts_active` and to
    // arm a fresh lead-in cushion for the next utterance.
    let mut first_append = true;

    loop {
        if handle_cancel_or_shutdown(
            &cancel,
            &shutdown,
            &tts_active,
            &text_rx,
            Some((&player, &player_ops)),
        ) {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            // Cancel consumed: queued audio cleared, queue drained. The next
            // append starts a new utterance and needs its own lead-in cushion.
            first_append = true;
            continue;
        }

        let raw_text = match text_rx.recv_timeout(RECV_TIMEOUT) {
            Ok(t) => t,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Nothing queued. If playback has also finished, the agent
                // has gone quiet — release the mic gate and reset the
                // lead-in so the next utterance gets a fresh cushion.
                if player.empty() && !first_append {
                    tts_active.store(false, Ordering::Release);
                    first_append = true;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Check cancel again after unblocking — a cancel may have arrived
        // while we were waiting.
        if handle_cancel_or_shutdown(
            &cancel,
            &shutdown,
            &tts_active,
            &text_rx,
            Some((&player, &player_ops)),
        ) {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            first_append = true;
            continue;
        }

        // If playback already drained while we were waiting for this item,
        // the agent is silent — release the mic gate BEFORE preprocessing/
        // synthesis. Without this, an item arriving inside the recv timeout
        // window would run the whole synthesis pass with `tts_active` stuck
        // true and nothing playing, making STT discard human speech as
        // "echo" during a silent window. (Pipelining is unaffected: when
        // audio is still draining, `player.empty()` is false and the flag
        // stays set across items.)
        if player.empty() && !first_append {
            tts_active.store(false, Ordering::Release);
            first_append = true;
        }

        // Preprocess text.
        let text = preprocess_for_tts(&raw_text);
        if text.is_empty() {
            continue;
        }

        // Split into sentences, then group into synthesis chunks: the first
        // sentence stays alone (fast time-to-first-audio), the rest pack
        // greedily up to MAX_CHUNK_CHARS. Each chunk is one `generate()`
        // call; playback of chunk N overlaps synthesis of chunk N+1
        // (lookahead pipelining). Grouping matches upstream's ~50-token
        // chunking and halves the exposed prosody seams on multi-sentence
        // replies — see MAX_CHUNK_CHARS.
        let sentences: Vec<String> = split_sentences(&text)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        let chunks = group_sentences_into_chunks(&sentences, MAX_CHUNK_CHARS);

        for chunk in &chunks {
            if handle_cancel_or_shutdown(
                &cancel,
                &shutdown,
                &tts_active,
                &text_rx,
                Some((&player, &player_ops)),
            ) {
                first_append = true;
                break;
            }

            let text = chunk.trim();
            if text.is_empty() {
                continue;
            }

            match engine.synth_chunk(text, "en", &style, SYNTH_STEPS) {
                Ok(samples) if !samples.is_empty() => {
                    let mut audio = clamp_to_full_scale(samples);
                    // Fade-out only — fading-in would attenuate the consonant
                    // onset (see `apply_fade_out` docstring + the
                    // 2026-05-18 "first little sound is missing" regression).
                    apply_fade_out(&mut audio);

                    // Build one contiguous buffer per synthesized sentence:
                    // lead-in cushion + audio + trailing gap. Keeping this as
                    // a single rodio source preserves the original queue/drain
                    // semantics (one append per sentence) while still giving
                    // every chunk a quiet device warm-up window.
                    let buf =
                        build_sentence_append_buffer(&mut first_append, audio, silence_buf_len);

                    // Check-and-append under `player_ops`, serialized with
                    // the monitor: a barge-in may have arrived during
                    // synthesis (the blocking window the monitor thread
                    // exists for). Don't append the now-stale sentence — the
                    // human interrupted; speaking it anyway would talk over
                    // them. Holding the lock for the check + append means the
                    // monitor can never clear between our check passing and
                    // the buffer landing. The flag is deliberately NOT
                    // consumed here: the loop-top handle_cancel_or_shutdown
                    // does the full consume (drain queue, reset lead-in) on
                    // the next iteration.
                    let _ops = lock_player_ops(&player_ops);
                    if cancel.load(Ordering::Acquire) {
                        // Nothing appended; the loop-top consume re-arms
                        // `first_append` (the flag is still set — the worker
                        // is its only consumer).
                        break;
                    }
                    player.append(SamplesBuffer::new(channels, rate, buf));
                    // NOTE: tts_active is set AFTER player.append(), not
                    // before. Setting it before synthesis would cause STT to
                    // discard user speech during the synthesis window as
                    // "echo" even though no audio is actually playing yet.
                    // See crossfire review C3.
                    tts_active.store(true, Ordering::Release);
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("buzz-desktop: TTS synth failed: {e}");
                }
            }
        }

        if shutdown.load(Ordering::Acquire) {
            break;
        }
    }

    // Stop the barge-in monitor before exiting — it holds a Player clone,
    // and an orphaned monitor would keep ticking against a dead pipeline.
    monitor_stop.store(true, Ordering::Release);
    if let Ok(handle) = monitor {
        let _ = handle.join();
    }

    tts_active.store(false, Ordering::Release);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Check for cancel or shutdown. Returns `true` if the caller should break/continue.
/// On cancel: drains the text queue and clears the cancel flag.
///
/// `player` pairs the Player with the `player_ops` mutex shared with the
/// barge-in monitor thread; the cancel/shutdown clear runs under that lock so
/// it is serialized with the monitor's stale-branch re-check (see the monitor
/// block in `tts_worker`).
fn handle_cancel_or_shutdown(
    cancel: &AtomicBool,
    shutdown: &AtomicBool,
    tts_active: &AtomicBool,
    text_rx: &mpsc::Receiver<String>,
    player: Option<(&rodio::Player, &Mutex<()>)>,
) -> bool {
    if shutdown.load(Ordering::Acquire) {
        if let Some((p, ops)) = player {
            let _ops = lock_player_ops(ops);
            p.clear();
        }
        tts_active.store(false, Ordering::Release);
        return true;
    }
    if cancel.load(Ordering::Acquire) {
        if let Some((p, ops)) = player {
            let _ops = lock_player_ops(ops);
            // `Player::clear()` removes queued sources AND pauses the player
            // (rodio 0.22 `clear()` ends with `self.pause()`). With one
            // persistent Player for the worker's lifetime, the un-pause is
            // mandatory: without `play()`, every append after a barge-in
            // would queue silently forever.
            p.clear();
            p.play();
            // Consume the flag under the lock: once released with
            // `cancel == false`, the monitor's stale branch no-ops instead
            // of clearing the fresh post-cancel utterance.
            while text_rx.try_recv().is_ok() {}
            cancel.store(false, Ordering::Release);
        } else {
            while text_rx.try_recv().is_ok() {}
            cancel.store(false, Ordering::Release);
        }
        tts_active.store(false, Ordering::Release);
        return true;
    }
    false
}

/// Acquire the `player_ops` lock, recovering from poison.
///
/// The data under the mutex is `()` — it only serializes Player mutations —
/// so a panicked holder leaves nothing inconsistent to observe and recovery
/// is always safe. Without this, a worker panic would wedge the monitor (or
/// vice versa) on `unwrap()`.
fn lock_player_ops(ops: &Mutex<()>) -> MutexGuard<'_, ()> {
    ops.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hard-clamp samples to ±1.0 full scale.
///
/// No gain is applied: Pocket TTS already emits speech-level audio
/// (peaks 0.4–0.97, RMS ≈ −20 dBFS across varied sentences — measured by
/// `examples/pocket_clip_probe`), matching the kyutai reference pipeline,
/// which applies no output scaling. Two earlier gain stages were both
/// regressions against that baseline: per-sentence peak normalization caused
/// level pumping between sentences, and the fixed 9.3× gain that replaced it
/// was calibrated on a single anomalously-quiet bench utterance (peak 0.076)
/// and clipped 13–34% of samples on real speech ("blown out", 2026-06-12).
/// The clamp alone remains as the safety net against outlier transients.
fn clamp_to_full_scale(samples: Vec<f32>) -> Vec<f32> {
    samples.into_iter().map(|s| s.clamp(-1.0, 1.0)).collect()
}

/// Apply a short linear fade-out at the *end* of `samples`.
///
/// Uses `FADE_OUT_SAMPLES` (8 ms) or half the buffer length, whichever is
/// smaller. Eliminates the click that occurs when a non-zero waveform
/// terminates abruptly at a sentence boundary.
///
/// # Why no fade-in
///
/// An earlier revision (pre 2026-05) symmetrically faded *in* over the same
/// 8 ms window. That swallowed the leading consonant attack on every
/// sentence — Pocket TTS produces real audio energy inside the first
/// millisecond (RMS ≈ 0.02, peak ≈ 0.03 measured across four prompts in
/// `examples/pocket_onset_probe.rs`), and a linear 0→1 ramp over 192 samples
/// scales those onset samples by ≤50 % for the first ~4 ms. The result was
/// the "first little sound or two is missing" regression heard on
/// 2026-05-18.
///
/// The first sample of Pocket output measures ≈ 0.0018 (≈ −54 dBFS) — well
/// below the threshold at which a DC-jump would be audible as a click — so
/// no fade-in is needed. The OS audio device gets its quiet ramp-up window
/// from `SENTENCE_LEAD_IN_SAMPLES` instead, inserted as pure silence before
/// each sentence buffer.
fn apply_fade_out(samples: &mut [f32]) {
    let len = samples.len();
    let fade = FADE_OUT_SAMPLES.min(len / 2);
    for i in 0..fade {
        samples[len - 1 - i] *= i as f32 / fade as f32;
    }
}

/// Build the single buffer appended to the rodio `Player` for one synthesised
/// sentence.
///
/// Every sentence chunk gets a short lead-in pad immediately before its audio.
/// This matters for chunks that start with soft first phonemes (`I'm`, `I've`):
/// the synthesized buffer can begin with speech within the first millisecond,
/// so the playback layer must provide the device/mixer cushion.
/// To keep the audible gap unchanged, the trailing silence after this chunk is
/// shortened by the same amount (`silence_buf_len - SENTENCE_LEAD_IN_SAMPLES`):
/// sentence N contributes 80 ms of post-speech silence and sentence N+1
/// contributes the remaining 20 ms of pre-speech cushion.
///
/// The lead-in, audio, and trailing silence are concatenated into one
/// `SamplesBuffer` before appending. This keeps rodio's queue shape at one
/// tracked source per synthesized sentence, avoiding source-boundary/drain
/// regressions from enqueueing the lead-in, audio, and tail as separate sounds.
///
/// `first_append` is flipped on the first call after the player goes idle.
/// The worker uses it in the idle branch of the main loop to distinguish
/// "never queued anything since last drain" from "drained after speaking",
/// which controls when `tts_active` is released and the lead-in re-armed.
fn build_sentence_append_buffer(
    first_append: &mut bool,
    audio: Vec<f32>,
    silence_buf_len: usize,
) -> Vec<f32> {
    if *first_append {
        *first_append = false;
    }

    let trailing_silence_len = silence_buf_len.saturating_sub(SENTENCE_LEAD_IN_SAMPLES);
    let mut buf = Vec::with_capacity(SENTENCE_LEAD_IN_SAMPLES + audio.len() + trailing_silence_len);
    buf.extend(std::iter::repeat_n(0.0_f32, SENTENCE_LEAD_IN_SAMPLES));
    buf.extend(audio);
    buf.extend(std::iter::repeat_n(0.0_f32, trailing_silence_len));
    buf
}

/// Group sentences into synthesis chunks.
///
/// The first sentence always stands alone — it is what the listener hears
/// first, and synthesizing it by itself keeps time-to-first-audio at the
/// single-sentence cost. Subsequent sentences pack greedily: a sentence
/// joins the current chunk while the combined length stays within
/// `max_chars`; otherwise it starts a new chunk. A single sentence longer
/// than `max_chars` becomes its own chunk unsplit — Pocket TTS handles long
/// single sentences fine (the ceiling is the 500-LM-step default), it's the
/// *seams* we're minimizing.
///
/// Sentences within a chunk are joined with a single space; sentence-ending
/// punctuation is preserved by `split_sentences`, so the model sees natural
/// multi-sentence prose — the same shape upstream's ~50-token chunker feeds it.
fn group_sentences_into_chunks(sentences: &[String], max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    for (i, sentence) in sentences.iter().enumerate() {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        if i == 0 || chunks.is_empty() {
            chunks.push(sentence.to_string());
            continue;
        }
        // Never merge into the first chunk — it's the latency-critical one.
        let can_merge = chunks.len() > 1
            && chunks
                .last()
                .is_some_and(|c| c.len() + 1 + sentence.len() <= max_chars);
        if can_merge {
            let last = chunks.last_mut().expect("non-empty checked above");
            last.push(' ');
            last.push_str(sentence);
        } else {
            chunks.push(sentence.to_string());
        }
    }
    chunks
}

// drain_until_shutdown lives in super (huddle/mod.rs) — shared with stt.rs.
use super::drain_until_shutdown;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;
