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
//!       4. Apply volume boost + fade out to each sentence
//!       5. Append each buffer to the persistent rodio Player (gapless)
//!       6. While audio is draining, keep pulling queued text items and
//!          synthesizing ahead — playback of item N overlaps synthesis of
//!          item N+1
//!   → tts_active = true while audio is queued/playing, false when idle
//!   → cancel flag: player.clear() + drain queue + player.play() (un-pause)
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
        Arc,
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

/// Pocket TTS is a one-step consistency model, not diffusion. Kept for API compat.
const SYNTH_STEPS: usize = 1;

/// Synthesis speed multiplier. Slightly faster than natural speech.
const SYNTH_SPEED: f32 = 1.05;

/// Fixed playback gain applied to every synthesized sentence, in linear scale.
///
/// Pocket TTS reference-voice output measured ~7.6% peak on a 75-character
/// utterance (`examples/pocket_bench`); 9.3 × 0.076 ≈ 0.71 lands a typical
/// peak at the Tyler-approved −3 dBFS loudness. The `clamp(±1.0)` in
/// [`apply_playback_gain`] is the safety net for outlier transients.
///
/// Why a *fixed* gain rather than per-sentence peak normalization (which this
/// replaced): normalizing each sentence to its own peak makes loudness a
/// function of that sentence's loudest transient — a sentence with one sharp
/// consonant gets less gain than its neighbors, producing audible level
/// pumping between consecutive sentences. The kyutai reference pipeline
/// applies no normalization at all; a fixed gain is the minimal deviation
/// that keeps the desired output level while making loudness text-invariant.
const PLAYBACK_GAIN: f32 = 9.3;

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

/// Sentence-by-sentence synthesis — keeps first-sentence latency low and lets
/// playback of sentence N overlap with synthesis of sentence N+1 (see the
/// lookahead pipelining note in the module doc-comment above).
const BATCH_SIZE: usize = 1;

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
        match engine.synth_chunk("warmup", "en", &style, SYNTH_STEPS, SYNTH_SPEED) {
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
    let player = Player::connect_new(sink_handle.mixer());

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
        if handle_cancel_or_shutdown(&cancel, &shutdown, &tts_active, &text_rx, Some(&player)) {
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
        if handle_cancel_or_shutdown(&cancel, &shutdown, &tts_active, &text_rx, Some(&player)) {
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

        // Split into sentences. Each sentence is synthesized individually and
        // appended to the Player immediately — synthesis of sentence N+1 overlaps
        // with playback of sentence N (lookahead pipelining).
        let sentences: Vec<String> = split_sentences(&text)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();

        for sentence in &sentences {
            if handle_cancel_or_shutdown(&cancel, &shutdown, &tts_active, &text_rx, Some(&player)) {
                first_append = true;
                break;
            }

            let text = sentence.trim();
            if text.is_empty() {
                continue;
            }

            match engine.synth_chunk(text, "en", &style, SYNTH_STEPS, SYNTH_SPEED) {
                Ok(samples) if !samples.is_empty() => {
                    let mut boosted = apply_playback_gain(samples);
                    // Fade-out only — fading-in would attenuate the consonant
                    // onset (see `apply_fade_out` docstring + the
                    // 2026-05-18 "first little sound is missing" regression).
                    apply_fade_out(&mut boosted);

                    // Build one contiguous buffer per synthesized sentence:
                    // lead-in cushion + audio + trailing gap. Keeping this as
                    // a single rodio source preserves the original queue/drain
                    // semantics (one append per sentence) while still giving
                    // every chunk a quiet device warm-up window.
                    let buf =
                        build_sentence_append_buffer(&mut first_append, boosted, silence_buf_len);
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

    tts_active.store(false, Ordering::Release);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Check for cancel or shutdown. Returns `true` if the caller should break/continue.
/// On cancel: drains the text queue and clears the cancel flag.
fn handle_cancel_or_shutdown(
    cancel: &AtomicBool,
    shutdown: &AtomicBool,
    tts_active: &AtomicBool,
    text_rx: &mpsc::Receiver<String>,
    player: Option<&rodio::Player>,
) -> bool {
    if shutdown.load(Ordering::Acquire) {
        if let Some(p) = player {
            p.clear();
        }
        tts_active.store(false, Ordering::Release);
        return true;
    }
    if cancel.load(Ordering::Acquire) {
        if let Some(p) = player {
            // `Player::clear()` removes queued sources AND pauses the player
            // (rodio 0.22 `clear()` ends with `self.pause()`). With one
            // persistent Player for the worker's lifetime, the un-pause is
            // mandatory: without `play()`, every append after a barge-in
            // would queue silently forever.
            p.clear();
            p.play();
        }
        while text_rx.try_recv().is_ok() {}
        cancel.store(false, Ordering::Release);
        tts_active.store(false, Ordering::Release);
        return true;
    }
    false
}

/// Apply the fixed playback gain ([`PLAYBACK_GAIN`]), hard-clamped to ±1.0.
///
/// Replaces the earlier per-sentence peak normalization — see the
/// [`PLAYBACK_GAIN`] doc-comment for why fixed gain wins (level pumping
/// between consecutive sentences). The clamp is the only nonlinearity and is
/// expected to be inert for typical Pocket output (peak ≈ 0.076 × 9.3 ≈
/// 0.71); it exists to catch outlier transients before they wrap.
fn apply_playback_gain(samples: Vec<f32>) -> Vec<f32> {
    samples
        .into_iter()
        .map(|s| (s * PLAYBACK_GAIN).clamp(-1.0, 1.0))
        .collect()
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
    boosted: Vec<f32>,
    silence_buf_len: usize,
) -> Vec<f32> {
    if *first_append {
        *first_append = false;
    }

    let trailing_silence_len = silence_buf_len.saturating_sub(SENTENCE_LEAD_IN_SAMPLES);
    let mut buf =
        Vec::with_capacity(SENTENCE_LEAD_IN_SAMPLES + boosted.len() + trailing_silence_len);
    buf.extend(std::iter::repeat_n(0.0_f32, SENTENCE_LEAD_IN_SAMPLES));
    buf.extend(boosted);
    buf.extend(std::iter::repeat_n(0.0_f32, trailing_silence_len));
    buf
}

// drain_until_shutdown lives in super (huddle/mod.rs) — shared with stt.rs.
use super::drain_until_shutdown;

// BATCH_SIZE is used implicitly (one sentence per iteration). Suppress dead_code
// lint since it documents the design intent.
const _: () = assert!(BATCH_SIZE == 1);

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;
