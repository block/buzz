//! Speech-to-Text pipeline for huddle voice transcription.
//!
//! Mental model:
//!
//! ```text
//! AudioWorklet (48 kHz f32 PCM)
//!   → push_audio_pcm (Tauri cmd)
//!   → SttPipeline::push_audio  [bounded sync_channel]
//!   → stt_worker thread
//!       rubato: 48 kHz → 16 kHz mono
//!       earshot VAD: accumulate speech frames
//!       sherpa-onnx Parakeet TDT-CTC 110M: transcribe on silence
//!   → text_rx  [mpsc channel]
//!   → tokio task (start_stt_pipeline)
//!       builds kind:9 event → relay
//! ```
//!
//! The worker runs on a dedicated `std::thread` (not async) because
//! sherpa-onnx is CPU-bound and not Send-safe across await points.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};

use tokio::sync::mpsc as tokio_mpsc;

// ── Public pipeline handle ────────────────────────────────────────────────────

/// Bounded audio queue capacity.
/// 100 ms batches at 48 kHz ≈ 19 KB each → 50 slots ≈ 5 s / ~1 MB max backlog.
const AUDIO_QUEUE_DEPTH: usize = 50;

/// Maximum speech buffer size: 30 seconds at 16 kHz.
/// Prevents OOM if VAD stays in speech mode (noisy environment).
const MAX_SPEECH_SAMPLES: usize = 16_000 * 30;

/// Handle to the running STT pipeline.
///
/// Not Clone — wrap in `Arc` to share across threads.
///
/// The text receiver (`tokio::sync::mpsc::Receiver<String>`) is returned
/// separately from `new()` so the caller can move it directly into an async
/// task without holding a Mutex across await points.
#[derive(Debug)]
pub struct SttPipeline {
    /// Send raw PCM bytes (f32 LE, 48 kHz mono) into the pipeline.
    audio_tx: SyncSender<Vec<u8>>,
    /// Signals the worker thread to stop.
    shutdown: Arc<AtomicBool>,
    /// Worker thread handle — taken on drop to join cleanly.
    thread: Option<thread::JoinHandle<()>>,
}

impl SttPipeline {
    /// Spawn the pipeline thread.
    ///
    /// `tts_active` is a shared flag set by the TTS pipeline while audio is
    /// playing. The STT worker uses it to:
    ///   - discard accumulated speech (echo prevention / barge-in gating)
    ///   - apply a 200 ms cooldown after TTS stops before re-enabling STT
    ///   - detect barge-in: speech onset during TTS → set `tts_cancel`
    ///
    /// `tts_cancel` (optional) is the TTS pipeline's cancel flag. When the STT
    /// worker detects speech onset while TTS is active, it sets this flag to
    /// stop playback immediately (barge-in). Pass `None` if TTS is unavailable.
    ///
    /// `ptt_active` (optional) is the push-to-talk flag. When `Some`, the STT
    /// pipeline only accumulates speech while the flag is true (key held).
    /// When `None`, the pipeline runs in continuous VAD mode.
    ///
    /// Returns `Err` only if the thread cannot be spawned (OS error).
    /// If model files are missing, the worker logs and exits cleanly —
    /// the pipeline handle is still returned but will never produce text.
    ///
    /// The `tokio::sync::mpsc::Receiver<String>` is returned separately so the
    /// caller can move it directly into an async task. This avoids holding a
    /// `Mutex<Receiver>` across await points (which would block a Tokio worker
    /// thread on every `recv_timeout` call).
    pub fn new(
        model_dir: PathBuf,
        tts_active: Arc<AtomicBool>,
        tts_cancel: Option<Arc<AtomicBool>>,
        ptt_active: Option<Arc<AtomicBool>>,
    ) -> Result<(Self, tokio_mpsc::Receiver<String>), String> {
        let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<u8>>(AUDIO_QUEUE_DEPTH);
        let (text_tx, text_rx) = tokio_mpsc::channel::<String>(64);
        let shutdown = Arc::new(AtomicBool::new(false));

        let shutdown_worker = Arc::clone(&shutdown);
        let tts_cancel_worker = tts_cancel.as_ref().map(Arc::clone);
        let ptt_active_worker = ptt_active.as_ref().map(Arc::clone);
        let handle = thread::Builder::new()
            .name("stt-worker".into())
            .spawn(move || {
                stt_worker(
                    model_dir,
                    audio_rx,
                    text_tx,
                    shutdown_worker,
                    tts_active,
                    tts_cancel_worker,
                    ptt_active_worker,
                )
            })
            .map_err(|e| format!("failed to spawn stt-worker thread: {e}"))?;

        let pipeline = Self {
            audio_tx,
            shutdown,
            thread: Some(handle),
        };
        Ok((pipeline, text_rx))
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

    /// Feed raw PCM bytes into the pipeline.
    ///
    /// Non-blocking. Drops audio silently if the pipeline can't keep up —
    /// better to lose frames than to stall the UI thread.
    pub fn push_audio(&self, pcm_bytes: Vec<u8>) -> Result<(), String> {
        // Reject non-4-byte-aligned input — would silently truncate in bytes_to_f32.
        if !pcm_bytes.len().is_multiple_of(4) {
            return Err(format!(
                "audio input not 4-byte aligned ({} bytes) — expected f32 LE samples",
                pcm_bytes.len()
            ));
        }
        // Drop audio if the pipeline can't keep up — better than blocking the UI.
        let _ = self.audio_tx.try_send(pcm_bytes);
        Ok(())
    }
}

impl Drop for SttPipeline {
    fn drop(&mut self) {
        // Signal the worker to stop.
        self.shutdown.store(true, Ordering::Release);
        // Dropping `audio_tx` (implicitly when self is dropped after this fn)
        // unblocks the worker's recv_timeout loop. Join to ensure clean exit.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────────

/// How many 16 kHz samples of silence before we flush to STT.
/// 300 ms × 16 000 Hz / 256 samples-per-frame ≈ 19 frames.
/// Previous value (28 frames / 450 ms) felt sluggish in conversation.
const SILENCE_FLUSH_FRAMES: usize = 19;

/// Consecutive VAD speech frames required before triggering barge-in during TTS.
/// 20 frames × 256 samples / 16 kHz ≈ 320 ms — must be long enough to filter
/// speaker-to-mic feedback (TTS audio bleeding through the mic) while still
/// catching real human interruptions. 80 ms (previous: 5 frames) was too
/// aggressive — laptop speakers without headphones triggered false barge-in
/// within the first word of TTS playback.
const BARGE_IN_DEBOUNCE_FRAMES: usize = 20;

/// earshot requires exactly 256 samples per frame at 16 kHz.
const VAD_FRAME_SAMPLES: usize = 256;

/// VAD probability threshold — above this is considered speech.
const VAD_THRESHOLD: f32 = 0.5;

/// How long the worker waits on the audio channel before checking the shutdown flag.
const RECV_TIMEOUT: Duration = Duration::from_millis(50);

/// 50 ms cooldown after TTS stops before STT re-enables.
/// Prevents the tail of TTS audio from being transcribed as speech.
/// Previous value (200 ms) was eating the first word when the user spoke
/// immediately after the agent finished.
const TTS_COOLDOWN: Duration = Duration::from_millis(50);

/// Number of ONNX Runtime intra-op threads used by the offline recognizer.
///
/// Held at 1 (conservative) until we have a local A/B on real huddle audio.
/// Sherpa-onnx's Parakeet example uses 2 and most published RTF numbers are
/// at 2 threads on x86_64 server class hardware, but the encoder runs only
/// on VAD chunk boundaries on a dedicated thread, so the threading knob
/// trades worker latency against potential oversubscription with the audio
/// worklet on small Macs (4-core Intel especially). Bump to 2 once the A/B
/// shows it's safe on the minimum-spec target.
const STT_NUM_THREADS: i32 = 1;

fn stt_worker(
    model_dir: PathBuf,
    audio_rx: Receiver<Vec<u8>>,
    text_tx: tokio_mpsc::Sender<String>,
    shutdown: Arc<AtomicBool>,
    tts_active: Arc<AtomicBool>,
    tts_cancel: Option<Arc<AtomicBool>>,
    ptt_active: Option<Arc<AtomicBool>>,
) {
    // ── 1. Initialise rubato resampler (48 kHz → 16 kHz, mono) ───────────────
    use rubato::{Fft, FixedSync, Resampler};

    let mut resampler = match Fft::<f32>::new(48_000, 16_000, 1024, 2, 1, FixedSync::Input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("buzz-desktop: STT resampler init failed: {e}");
            return;
        }
    };
    let chunk_in = resampler.input_frames_next();

    // ── 2. Initialise earshot VAD ─────────────────────────────────────────────
    use earshot::{DefaultPredictor, Detector};
    let mut vad = Detector::new(DefaultPredictor::new());

    // ── 3. Initialise sherpa-onnx recognizer ─────────────────────────────────
    //
    // Parakeet TDT-CTC 110M ships as a single `model.int8.onnx` (CTC head) plus
    // `tokens.txt`. sherpa-onnx infers the model family from which inner config
    // has a `model` path set, so we don't need to set `model_type` explicitly.
    // (See rust-api-examples/parakeet_tdt_ctc_simulate_streaming_microphone.rs
    // in k2-fsa/sherpa-onnx.)
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

    let tokens_path = model_dir.join("tokens.txt");
    let model_path = model_dir.join("model.int8.onnx");
    if !tokens_path.exists() || !model_path.exists() {
        eprintln!(
            "buzz-desktop: STT model not found at {} — STT disabled",
            model_dir.display()
        );
        drain_until_shutdown(audio_rx, &shutdown);
        return;
    }

    let mut cfg = OfflineRecognizerConfig::default();
    cfg.model_config.nemo_ctc.model = Some(model_path.to_string_lossy().into_owned());
    cfg.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    cfg.model_config.num_threads = STT_NUM_THREADS;
    // Explicit — defaults are not part of the API contract, and noisy debug
    // logging in release builds would be expensive on every VAD chunk.
    cfg.model_config.debug = false;

    let recognizer = match OfflineRecognizer::create(&cfg) {
        Some(r) => r,
        None => {
            eprintln!("buzz-desktop: OfflineRecognizer::create returned None — STT disabled");
            drain_until_shutdown(audio_rx, &shutdown);
            return;
        }
    };

    // ── 4. Processing state ───────────────────────────────────────────────────
    // Leftover 48 kHz samples that didn't fill a full resampler chunk.
    let mut input_buf_48k: Vec<f32> = Vec::with_capacity(chunk_in * 2);
    // Leftover 16 kHz samples that didn't fill a full VAD frame.
    let mut leftover_16k: Vec<f32> = Vec::new();
    // Accumulated speech frames (16 kHz).
    let mut speech_buf: Vec<f32> = Vec::new();
    // Consecutive silence frame count.
    let mut silence_frames: usize = 0;
    // Whether we're currently in a speech segment.
    let mut in_speech = false;
    // Consecutive speech frames seen during TTS — used for barge-in debounce.
    let mut barge_in_frames: usize = 0;
    // Timestamp when TTS last stopped — used for the 200 ms cooldown.
    let mut tts_stopped_at: Option<std::time::Instant> = None;

    // ── 5. Main loop ──────────────────────────────────────────────────────────
    let mut tts_was_active = false;
    let mut ptt_was_active = ptt_active
        .as_ref()
        .is_some_and(|p| p.load(Ordering::Acquire));
    loop {
        // Check shutdown flag before blocking.
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        // Track TTS transitions to set the cooldown timer.
        let tts_now = tts_active.load(Ordering::Acquire);
        if tts_was_active && !tts_now {
            // TTS just stopped — record the timestamp for the cooldown window.
            tts_stopped_at = Some(std::time::Instant::now());
        }
        tts_was_active = tts_now;

        // Track PTT transitions — flush accumulated speech when key is released.
        // The worklet stops sending frames when PTT is inactive, so the normal
        // silence-accumulation flush path never runs. We must flush here on the
        // active→inactive edge to avoid buffering speech across PTT presses.
        if let Some(ref ptt) = ptt_active {
            let ptt_now = ptt.load(Ordering::Acquire);
            if ptt_was_active && !ptt_now && in_speech && !speech_buf.is_empty() {
                flush_to_stt(&speech_buf, &recognizer, &text_tx);
                speech_buf.clear();
                silence_frames = 0;
                in_speech = false;
            }
            ptt_was_active = ptt_now;
        }

        // Use recv_timeout so we can periodically check the shutdown flag.
        let bytes = match audio_rx.recv_timeout(RECV_TIMEOUT) {
            Ok(b) => b,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break, // Sender dropped.
        };

        // Drain any additional pending messages to batch-process.
        let mut batch = vec![bytes];
        while let Ok(b) = audio_rx.try_recv() {
            batch.push(b);
        }

        for bytes in batch {
            // Convert raw bytes to f32 samples (little-endian).
            let samples_48k = bytes_to_f32(&bytes);
            input_buf_48k.extend_from_slice(&samples_48k);

            // Resample in chunk_in-sized blocks.
            while input_buf_48k.len() >= chunk_in {
                let chunk: Vec<f32> = input_buf_48k.drain(..chunk_in).collect();
                let resampled = resample_chunk(&mut resampler, &chunk);
                process_16k_samples(
                    &resampled,
                    &mut leftover_16k,
                    &mut vad,
                    &mut speech_buf,
                    &mut silence_frames,
                    &mut in_speech,
                    &mut barge_in_frames,
                    &recognizer,
                    &text_tx,
                    &tts_active,
                    tts_cancel.as_deref(),
                    &mut tts_stopped_at,
                    ptt_active.as_ref(),
                );
            }
        }
    }

    // No final flush — leave_huddle/end_huddle emit lifecycle events before
    // the STT worker exits, so a final flush would post a kind:9 message AFTER
    // the user has "left." Losing the last partial utterance is acceptable.
}

/// Resample a mono 48 kHz chunk to 16 kHz using rubato.
/// Returns the resampled samples (may be empty on error).
fn resample_chunk(resampler: &mut rubato::Fft<f32>, chunk_48k: &[f32]) -> Vec<f32> {
    use audioadapter_buffers::direct::InterleavedSlice;
    use rubato::Resampler;

    // rubato expects interleaved layout even for mono.
    let input = match InterleavedSlice::new(chunk_48k, 1, chunk_48k.len()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("buzz-desktop: STT resample input error: {e}");
            return Vec::new();
        }
    };

    match resampler.process(&input, 0, None) {
        Ok(out) => out.take_data(),
        Err(e) => {
            eprintln!("buzz-desktop: STT resample error: {e}");
            Vec::new()
        }
    }
}

/// Feed 16 kHz samples through the VAD and accumulate speech.
/// Flushes to STT when silence exceeds threshold.
///
/// When `tts_active` is set:
///   - In PTT mode: skip accumulation (PTT press handles TTS cancellation).
///   - In VAD mode: speech onset triggers barge-in via `tts_cancel`.
///   - After TTS stops, a cooldown prevents tail audio from being transcribed.
///
/// When `ptt_active` is `Some`:
///   - VAD `is_speech` is ANDed with the PTT flag — when the key is released,
///     `is_speech` becomes false, silence_frames accumulates, and the existing
///     flush logic kicks in naturally. The 200 ms release delay + ~300 ms
///     silence flush gives a natural utterance tail.
#[allow(clippy::too_many_arguments)]
fn process_16k_samples(
    samples: &[f32],
    leftover: &mut Vec<f32>,
    vad: &mut earshot::Detector<earshot::DefaultPredictor>,
    speech_buf: &mut Vec<f32>,
    silence_frames: &mut usize,
    in_speech: &mut bool,
    barge_in_frames: &mut usize,
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
    tts_active: &Arc<AtomicBool>,
    tts_cancel: Option<&AtomicBool>,
    tts_stopped_at: &mut Option<std::time::Instant>,
    ptt_active: Option<&Arc<AtomicBool>>,
) {
    leftover.extend_from_slice(samples);

    while leftover.len() >= VAD_FRAME_SAMPLES {
        let frame: Vec<f32> = leftover.drain(..VAD_FRAME_SAMPLES).collect();
        let clamped: Vec<f32> = frame.iter().map(|&s| s.clamp(-1.0, 1.0)).collect();
        let prob = vad.predict_f32(&clamped);
        let is_speech = prob > VAD_THRESHOLD;

        // PTT gating: when PTT key is not held, treat as silence.
        // This causes natural flush when the key is released — silence_frames
        // accumulates and the existing flush logic kicks in after
        // SILENCE_FLUSH_FRAMES. The 200 ms release delay + ~300 ms silence
        // flush gives a natural utterance tail.
        let is_speech = if let Some(ptt) = ptt_active {
            is_speech && ptt.load(Ordering::Acquire)
        } else {
            is_speech
        };

        let tts_playing = tts_active.load(Ordering::Acquire);

        // While TTS is playing: skip accumulation (echo prevention).
        if tts_playing {
            if ptt_active.is_some() {
                // PTT mode — PTT press handles TTS cancellation directly
                // (via the global shortcut handler). Just skip accumulation.
                *in_speech = false;
                *barge_in_frames = 0;
                speech_buf.clear();
                *silence_frames = 0;
                continue;
            }

            // VAD mode — barge-in detection.
            // Without acoustic echo cancellation, this requires a longer
            // debounce (BARGE_IN_DEBOUNCE_FRAMES ≈ 320 ms) to filter
            // speaker-to-mic feedback.
            if is_speech {
                *barge_in_frames += 1;
                if *barge_in_frames >= BARGE_IN_DEBOUNCE_FRAMES {
                    // Real speech detected during TTS — trigger barge-in.
                    if let Some(cancel) = tts_cancel {
                        cancel.store(true, Ordering::Release);
                    }
                    *barge_in_frames = 0;
                }
            } else {
                *barge_in_frames = 0;
            }
            // Don't accumulate speech during TTS (echo prevention).
            *in_speech = false;
            speech_buf.clear();
            *silence_frames = 0;
            continue;
        }

        // TTS not playing — check cooldown window.
        if let Some(stopped) = *tts_stopped_at {
            if stopped.elapsed() < TTS_COOLDOWN {
                // Still in cooldown — discard but keep tracking speech state.
                if !is_speech {
                    *in_speech = false;
                }
                speech_buf.clear();
                *silence_frames = 0;
                *barge_in_frames = 0;
                continue;
            } else {
                // Cooldown expired — clear the timer and reset all segment state.
                *tts_stopped_at = None;
                *in_speech = false;
                *silence_frames = 0;
                *barge_in_frames = 0;
            }
        }

        if is_speech {
            *silence_frames = 0;
            *in_speech = true;
            speech_buf.extend_from_slice(&frame);

            // OOM guard: flush and reset if the buffer exceeds 30 s of audio.
            if speech_buf.len() >= MAX_SPEECH_SAMPLES {
                flush_to_stt(speech_buf, recognizer, text_tx);
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
            }
        } else if *in_speech {
            // Still accumulate during brief silence gaps.
            speech_buf.extend_from_slice(&frame);
            *silence_frames += 1;

            // In PTT mode, don't flush on silence — accumulate the entire
            // key-hold as one utterance. The PTT release edge in the main
            // loop handles the flush. In VAD mode, flush after the silence
            // threshold so each natural pause becomes a separate message.
            if ptt_active.is_none() && *silence_frames >= SILENCE_FLUSH_FRAMES {
                // End of utterance — transcribe.
                flush_to_stt(speech_buf, recognizer, text_tx);
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
            }
        }
        // If not in speech and not accumulating, just discard the frame.
    }
}

/// Run sherpa-onnx on the accumulated speech buffer and send the text.
///
/// Uses `blocking_send` because this runs on a `std::thread` (not async).
/// The tokio channel's `blocking_send` is safe to call from sync contexts.
fn flush_to_stt(
    speech_buf: &[f32],
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
) {
    if speech_buf.is_empty() {
        return;
    }

    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, speech_buf);
    recognizer.decode(&stream);

    let text = stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default();

    if !text.is_empty() {
        if let Err(e) = text_tx.blocking_send(text) {
            eprintln!("buzz-desktop: STT text channel closed: {e}");
        }
    }
}

/// Convert raw bytes (f32 LE) to f32 samples.
/// Caller should ensure `bytes.len() % 4 == 0`; extra bytes are silently truncated.
///
/// Assumes little-endian — matches all current Tauri targets (macOS ARM64,
/// Windows/Linux x86). The JS AudioWorklet's Float32Array uses platform-native
/// byte order, which is LE on all supported platforms.
fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

// drain_until_shutdown lives in super (huddle/mod.rs) — shared with tts.rs.
use super::drain_until_shutdown;
