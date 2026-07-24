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

use super::models::SttFamily;

// ── Public pipeline handle ────────────────────────────────────────────────────

/// Bounded audio queue capacity: 100 ms batches at 48 kHz ≈ 19 KB each →
/// 300 slots ≈ 30 s / ~6 MB backlog, riding out the worst live-mode decode
/// stall (~2 s flush of a 30 s phrase). Overflow silently drops mic audio,
/// splicing the phrase so later decodes delete words already shown.
const AUDIO_QUEUE_DEPTH: usize = 300;

/// Maximum speech buffer size: 30 seconds at 16 kHz.
/// Prevents OOM if VAD stays in speech mode (noisy environment).
const MAX_SPEECH_SAMPLES: usize = 16_000 * 30;

/// Event on the live-transcript (dictation) channel. Strictly ordered — a
/// `Flushed` sent after a session's audio guarantees every `Transcript` of
/// that session was already delivered.
#[derive(Debug, PartialEq)]
pub enum LiveEvent {
    /// Partial (`is_final: false`) re-decodes replace each other; a final
    /// commits the phrase. An empty final means the phrase decoded to nothing.
    Transcript { text: String, is_final: bool },
    /// Echo of a zero-length audio batch (the stop-flush sentinel): all audio
    /// pushed before it has been processed, so the stopped session has no
    /// further transcripts coming.
    Flushed,
}

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
    /// `live_tx` (optional) switches the pipeline into live-transcript mode
    /// (used by composer dictation): the in-progress phrase is re-decoded
    /// every `PARTIAL_DECODE_STEP` new samples and sent as a partial
    /// `Transcript`, and finals commit it — all on this one channel so
    /// ordering is preserved. A zero-length audio batch is echoed back as
    /// `Flushed` (see `LiveEvent`). In this mode nothing is sent on the
    /// returned text receiver. Pass `None` for huddle transcription (finals
    /// only).
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
        family: SttFamily,
        tts_active: Arc<AtomicBool>,
        tts_cancel: Option<Arc<AtomicBool>>,
        ptt_active: Option<Arc<AtomicBool>>,
        live_tx: Option<tokio_mpsc::Sender<LiveEvent>>,
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
                    family,
                    audio_rx,
                    text_tx,
                    shutdown_worker,
                    tts_active,
                    tts_cancel_worker,
                    ptt_active_worker,
                    live_tx,
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
        if self.audio_tx.try_send(pcm_bytes).is_err() {
            // DEBUG(dictation-yeah): temporary — audio drops splice the phrase
            // buffer and corrupt later decodes; log to confirm/rule out.
            eprintln!("buzz-desktop: STT audio queue full — batch dropped");
        }
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

/// Silence-flush threshold for live (dictation) mode: ~608 ms.
///
/// Parakeet only commits sentence-final punctuation and capitalization when
/// the decoded audio ends in ≥600 ms of silence (measured: 300 ms → "is there
/// anything we can do about that", 600 ms → "…about that?"). The huddle's
/// ~300 ms window cut finals just under that threshold, so dictated text lost
/// its punctuation at every phrase commit. The longer window also merges
/// mid-sentence thinking pauses into fuller phrases; perceived latency is
/// unchanged because partials keep streaming while the window runs.
const LIVE_SILENCE_FLUSH_FRAMES: usize = 38;

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

/// Live-transcript mode: re-decode the in-progress phrase every this many new
/// 16 kHz samples (~300 ms) so words stream out while the user is talking.
/// The model is an offline recognizer, so "streaming" is repeated decoding of
/// the growing phrase buffer, and decode cost grows with it (~70 ms per second
/// of audio for the 0.6B model, measured in `v3_long_buffer`). So this is only
/// the MINIMUM step: after every decode (partials AND phrase flushes) the
/// worker holds off 2× that decode's duration before the next partial, capping
/// decode duty at ~50%. The hold must be WALL-CLOCK, not new-samples: a
/// post-stall backlog arrives faster than real time, so samples-based back-off
/// collapses into decode storms that overflow the audio queue.
const PARTIAL_DECODE_STEP: usize = 4800;

/// Consecutive VAD speech frames (16 ms each, ~96 ms total) required to open a
/// phrase in live (dictation) mode. Single-frame blips (keyboard click,
/// breath, background noise) make Parakeet hallucinate filler ("yeah", "uh");
/// real speech sustains the VAD easily, and the onset audio is buffered while
/// it proves itself, so nothing is lost. Huddle mode is untouched.
const ONSET_DEBOUNCE_FRAMES: usize = 6;

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

/// ONNX threads in live-transcript (dictation) mode. Live mode re-decodes the
/// whole in-progress phrase every ~300 ms, so per-decode latency bounds how
/// fresh the streamed words are. Measured with Parakeet v3 0.6B int8
/// (`v3_punctuation` experiment): a 6.5 s phrase decodes in ~1.4 s at 1
/// thread but ~460 ms at 4 — the difference between laggy and live. No
/// huddle/LiveKit stack runs during dictation, so the extra cores are free.
const LIVE_STT_NUM_THREADS: i32 = 4;

#[allow(clippy::too_many_arguments)]
fn stt_worker(
    model_dir: PathBuf,
    family: SttFamily,
    audio_rx: Receiver<Vec<u8>>,
    text_tx: tokio_mpsc::Sender<String>,
    shutdown: Arc<AtomicBool>,
    tts_active: Arc<AtomicBool>,
    tts_cancel: Option<Arc<AtomicBool>>,
    ptt_active: Option<Arc<AtomicBool>>,
    live_tx: Option<tokio_mpsc::Sender<LiveEvent>>,
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
    // sherpa-onnx infers the model family from which inner config has model
    // paths set, so we populate exactly one family sub-config (issue #2478):
    //
    //   NemoCtc     — single `model.int8.onnx` (CTC head), e.g. Parakeet 110M en.
    //   Transducer  — `encoder/decoder/joiner.int8.onnx`, e.g. Parakeet 0.6B v3
    //                 (multilingual). See k2-fsa/sherpa-onnx offline-transducer
    //                 NeMo examples.
    //
    // `tokens.txt` is shared by every family and lives on the parent config.
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

    let tokens_path = model_dir.join("tokens.txt");
    if !tokens_path.exists() {
        eprintln!(
            "buzz-desktop: STT tokens.txt not found at {} — STT disabled",
            model_dir.display()
        );
        drain_until_shutdown(audio_rx, &shutdown);
        return;
    }

    let mut cfg = OfflineRecognizerConfig::default();
    match family {
        SttFamily::NemoCtc => {
            let model_path = model_dir.join("model.int8.onnx");
            if !model_path.exists() {
                eprintln!(
                    "buzz-desktop: STT model.int8.onnx not found at {} — STT disabled",
                    model_dir.display()
                );
                drain_until_shutdown(audio_rx, &shutdown);
                return;
            }
            cfg.model_config.nemo_ctc.model = Some(model_path.to_string_lossy().into_owned());
        }
        SttFamily::Transducer => {
            let encoder = model_dir.join("encoder.int8.onnx");
            let decoder = model_dir.join("decoder.int8.onnx");
            let joiner = model_dir.join("joiner.int8.onnx");
            if !encoder.exists() || !decoder.exists() || !joiner.exists() {
                eprintln!(
                    "buzz-desktop: STT transducer files (encoder/decoder/joiner) missing at {} \
                     — STT disabled",
                    model_dir.display()
                );
                drain_until_shutdown(audio_rx, &shutdown);
                return;
            }
            cfg.model_config.transducer.encoder = Some(encoder.to_string_lossy().into_owned());
            cfg.model_config.transducer.decoder = Some(decoder.to_string_lossy().into_owned());
            cfg.model_config.transducer.joiner = Some(joiner.to_string_lossy().into_owned());
        }
    }
    cfg.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    cfg.model_config.num_threads = if live_tx.is_some() {
        LIVE_STT_NUM_THREADS
    } else {
        STT_NUM_THREADS
    };
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
    // Live mode: speech_buf length at the last partial decode.
    let mut last_partial_len: usize = 0;
    // Live mode: no partial decode before this instant (see
    // PARTIAL_DECODE_STEP — the 2× wall-clock hold after every decode).
    let mut decode_hold_until = std::time::Instant::now();
    // Live mode: VAD-positive frames buffered while a speech onset proves
    // itself (ONSET_DEBOUNCE_FRAMES) before opening a phrase.
    let mut onset_buf: Vec<f32> = Vec::new();
    // Live mode: most recent partial decode of the current phrase that ended
    // with terminal punctuation — see prefer_punctuated.
    let mut punct_partial: Option<String> = None;
    // Live mode: best (most words) partial decode of the current phrase —
    // the collapse guard, see decode_collapsed.
    let mut best_partial: Option<String> = None;
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
                flush_to_stt(
                    &speech_buf,
                    &recognizer,
                    &text_tx,
                    live_tx.as_ref(),
                    punct_partial.take(),
                    best_partial.take(),
                );
                speech_buf.clear();
                silence_frames = 0;
                in_speech = false;
                last_partial_len = 0;
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
            // Zero-length batch = stop-flush sentinel (dictation.rs). All
            // audio pushed before it has been processed by now, so the
            // stopped session has no further transcripts — tell the frontend.
            if bytes.is_empty() {
                if let Some(tx) = live_tx.as_ref() {
                    if let Err(e) = tx.blocking_send(LiveEvent::Flushed) {
                        eprintln!("buzz-desktop: STT live channel closed: {e}");
                    }
                }
                continue;
            }
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
                    live_tx.as_ref(),
                    &mut last_partial_len,
                    &mut decode_hold_until,
                    &mut onset_buf,
                    &mut punct_partial,
                    &mut best_partial,
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
    live_tx: Option<&tokio_mpsc::Sender<LiveEvent>>,
    last_partial_len: &mut usize,
    decode_hold_until: &mut std::time::Instant,
    onset_buf: &mut Vec<f32>,
    punct_partial: &mut Option<String>,
    best_partial: &mut Option<String>,
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
            if !*in_speech && live_tx.is_some() {
                // Live mode: debounce the onset (see ONSET_DEBOUNCE_FRAMES).
                // ponytail: any non-speech frame resets it; add grace if real onsets clip.
                onset_buf.extend_from_slice(&frame);
                if onset_buf.len() < ONSET_DEBOUNCE_FRAMES * VAD_FRAME_SAMPLES {
                    continue;
                }
                speech_buf.append(onset_buf);
            } else {
                speech_buf.extend_from_slice(&frame);
            }
            *in_speech = true;

            // OOM guard: flush and reset if the buffer exceeds 30 s of audio.
            if speech_buf.len() >= MAX_SPEECH_SAMPLES {
                let t0 = std::time::Instant::now();
                flush_to_stt(
                    speech_buf,
                    recognizer,
                    text_tx,
                    live_tx,
                    punct_partial.take(),
                    best_partial.take(),
                );
                *decode_hold_until = std::time::Instant::now() + t0.elapsed() * 2;
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
                *last_partial_len = 0;
            }
        } else if *in_speech {
            // Still accumulate during brief silence gaps.
            speech_buf.extend_from_slice(&frame);
            *silence_frames += 1;

            // In PTT mode, don't flush on silence — accumulate the entire
            // key-hold as one utterance. The PTT release edge in the main
            // loop handles the flush. In VAD mode, flush after the silence
            // threshold so each natural pause becomes a separate message.
            let flush_frames = if live_tx.is_some() {
                LIVE_SILENCE_FLUSH_FRAMES
            } else {
                SILENCE_FLUSH_FRAMES
            };
            if ptt_active.is_none() && *silence_frames >= flush_frames {
                // End of utterance — transcribe. Flush decodes join the same
                // wall-clock hold as partials (see PARTIAL_DECODE_STEP).
                let t0 = std::time::Instant::now();
                flush_to_stt(
                    speech_buf,
                    recognizer,
                    text_tx,
                    live_tx,
                    punct_partial.take(),
                    best_partial.take(),
                );
                *decode_hold_until = std::time::Instant::now() + t0.elapsed() * 2;
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
                *last_partial_len = 0;
            }
        } else {
            // Not in speech: discard the frame, and drop any pending onset —
            // the blip ended before it proved itself to be speech.
            onset_buf.clear();
        }

        // Live mode: re-decode the in-progress phrase every PARTIAL_DECODE_STEP
        // new samples so words appear while the user is still talking. Later
        // partials of the same phrase supersede earlier ones — the frontend
        // replaces the partial region — so decode wobble is self-correcting.
        if let Some(tx) = live_tx {
            if *in_speech
                && speech_buf.len() >= *last_partial_len + PARTIAL_DECODE_STEP
                && std::time::Instant::now() >= *decode_hold_until
            {
                *last_partial_len = speech_buf.len();
                let t0 = std::time::Instant::now();
                let text = decode_speech(speech_buf, recognizer);
                // 2× wall-clock hold (see PARTIAL_DECODE_STEP); 1× measured
                // at ~100% duty on a 35 s ramble — no headroom.
                *decode_hold_until = std::time::Instant::now() + t0.elapsed() * 2;
                // Collapse guard (see decode_collapsed): a re-decode of the
                // same growing phrase that lost words is a collapse, not a
                // revision — suppress it so it neither replaces the shown
                // partial nor poisons the best-partial baseline.
                let collapsed = best_partial
                    .as_deref()
                    .is_some_and(|best| decode_collapsed(&text, best));
                if collapsed {
                    eprintln!("buzz-desktop: STT partial collapsed ({text:?}) — suppressed");
                } else if !text.is_empty() {
                    let mut text = text;
                    // Second collapse shape — see stitch_prefix_collapse.
                    if let Some(best) = best_partial.as_deref() {
                        if let Some(stitched) = stitch_prefix_collapse(best, &text) {
                            eprintln!(
                                "buzz-desktop: STT partial dropped its front ({text:?}) — stitched"
                            );
                            text = stitched;
                        }
                    }
                    if text.ends_with(['.', '?', '!', ',']) {
                        *punct_partial = Some(text.clone());
                    }
                    *best_partial = Some(text.clone());
                    let event = LiveEvent::Transcript {
                        text,
                        is_final: false,
                    };
                    if let Err(e) = tx.blocking_send(event) {
                        eprintln!("buzz-desktop: STT live channel closed: {e}");
                    }
                }
            }
        }
    }
}

/// Run sherpa-onnx on the accumulated speech buffer and send the text.
///
/// Uses `blocking_send` because this runs on a `std::thread` (not async).
/// Lowercased words only — all punctuation and case stripped. Two decodes of
/// the same audio that differ only in the model's (unstable) punctuation and
/// capitalization normalize to the same string.
fn norm_words(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Punctuation from the 110M model is unstable: re-decodes of the same phrase
/// flicker between "…that" and "…that?" (and internally, "question." vs
/// "question,") depending on the exact trailing-noise composition — see the
/// `silence_experiment` tests. If a partial of this phrase ended in
/// punctuation, prefer it:
/// - same words as the final (punctuation/case-insensitive) → keep the whole
///   punctuated hint;
/// - words differ but the last word agrees and the hint ends a sentence
///   (. ? !) → graft just that mark onto the final.
///
/// Never invents words: any word-level difference and the final decode wins.
fn prefer_punctuated(final_text: String, punct_hint: Option<&str>) -> String {
    let Some(hint) = punct_hint else {
        return final_text;
    };
    if final_text.is_empty() || hint.is_empty() {
        return final_text;
    }
    let norm_final = norm_words(&final_text);
    if norm_words(hint) == norm_final {
        return hint.to_string();
    }
    let term = hint.chars().last().unwrap_or(' ');
    if matches!(term, '.' | '?' | '!') && !final_text.ends_with(['.', '?', '!']) {
        let last_word = |s: &str| s.rsplit(' ').next().map(str::to_string);
        if last_word(&norm_final) == last_word(&norm_words(hint)) {
            let mut grafted = final_text.trim_end_matches([',', ' ']).to_string();
            grafted.push(term);
            return grafted;
        }
    }
    final_text
}

/// True when a re-decode of the same phrase lost words against the best
/// decode seen so far — a collapse, not a revision. Every decode of a phrase
/// sees a superset of the audio any earlier one saw (partials decode a
/// growing buffer; the final decodes all of it), so word count can honestly
/// grow or hold but not shrink. The v3 transducer occasionally collapses a
/// re-decode to empty or a junk word ("Yeah") on audio whose earlier decodes
/// were fine; trusting it deletes the sentence the user watched appear.
/// ponytail: a rare legitimate word-merge revision ("any better"→"anybody")
/// trips this and keeps the earlier decode — words the user already saw,
/// far cheaper than losing real speech.
fn decode_collapsed(new_text: &str, best: &str) -> bool {
    let words = |s: &str| norm_words(s).split_whitespace().count();
    words(new_text) < words(best)
}

/// The v3 collapse's second shape: once a phrase contains an internal pause,
/// a re-decode sometimes returns only the words after the pause — dropping
/// the front while gaining newly spoken tail words, so decode_collapsed's
/// word-count check passes it. Detect: an established phrase (best ≥ 4
/// words) whose re-decode agrees on neither of its first two words — early
/// decodes legitimately rewrite a 1–3 word front, but a settled front never
/// vanishes wholesale. Repair: anchor the new decode's opening words inside
/// best and splice best's preserved front onto new (new wins from the anchor
/// on, so tail revisions survive); with no anchor the drop swallowed the
/// overlap too, so append new after best.
/// ponytail: anchor window is 3 words then 2 — a 1-word anchor false-matches
/// on common words, and a missed overlap only duplicates ≤2 words.
fn stitch_prefix_collapse(best: &str, new: &str) -> Option<String> {
    // Per-token norms keep display and normalized tokens 1:1 aligned.
    let b_disp: Vec<&str> = best.split_whitespace().collect();
    let b_norm: Vec<String> = b_disp.iter().map(|t| norm_words(t)).collect();
    let n_norm: Vec<String> = new.split_whitespace().map(norm_words).collect();
    if b_norm.len() < 4 || n_norm.len() < 2 {
        return None;
    }
    if b_norm[0] == n_norm[0] || b_norm[1] == n_norm[1] {
        return None; // front agrees — a revision, not a drop
    }
    for m in [3usize, 2] {
        if n_norm.len() < m {
            continue;
        }
        let window = &n_norm[..m];
        if let Some(j) = (0..=b_norm.len() - m)
            .rev()
            .find(|&j| &b_norm[j..j + m] == window)
        {
            let mut out = b_disp[..j].join(" ");
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(new);
            return Some(out);
        }
    }
    Some(format!("{best} {new}"))
}

#[cfg(test)]
mod stitch_prefix_collapse_tests {
    use super::stitch_prefix_collapse;

    // Strings from chl's 2026-07-24 trace (/tmp/buzz-dev-dictation.log).
    #[test]
    fn splices_at_the_overlap_anchor() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out, I need to start asking.",
                "I need to start asking whether it's a little bit more."
            )
            .as_deref(),
            Some("Big questions are coming out, I need to start asking whether it's a little bit more.")
        );
    }

    #[test]
    fn appends_when_no_overlap_survived() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out.",
                "I need to start asking whether it's"
            )
            .as_deref(),
            Some("Big questions are coming out. I need to start asking whether it's")
        );
    }

    #[test]
    fn tail_revision_wins_from_the_anchor_on() {
        assert_eq!(
            stitch_prefix_collapse(
                "Big questions are coming out, I need to start asking whether it's a little bit more.",
                "I need to start asking whether or not that's the same"
            )
            .as_deref(),
            Some("Big questions are coming out, I need to start asking whether or not that's the same")
        );
    }

    #[test]
    fn honest_revisions_pass_through() {
        // Front agrees → not a drop.
        assert!(stitch_prefix_collapse(
            "Big question to come.",
            "Big questions are coming up."
        )
        .is_none());
        // Short phrases rewrite themselves legitimately.
        assert!(stitch_prefix_collapse("Yeah.", "Big question.").is_none());
        assert!(stitch_prefix_collapse("Costing.", "Calls passing.").is_none());
    }
}

#[cfg(test)]
mod decode_collapsed_tests {
    use super::decode_collapsed;

    #[test]
    fn collapse_detection() {
        let best = "caused by some other things";
        assert!(decode_collapsed("", best));
        assert!(decode_collapsed("Yeah.", best));
        assert!(decode_collapsed("Cause personal things.", best));
        // Honest revisions and growth keep the new decode.
        assert!(!decode_collapsed("Caused by some other things.", best));
        assert!(!decode_collapsed("caused by some other things too", best));
        // No baseline words → nothing to protect.
        assert!(!decode_collapsed("", ""));
    }
}

/// The tokio channel's `blocking_send` is safe to call from sync contexts.
fn flush_to_stt(
    speech_buf: &[f32],
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
    live_tx: Option<&tokio_mpsc::Sender<LiveEvent>>,
    punct_hint: Option<String>,
    best_partial: Option<String>,
) {
    let mut text = prefer_punctuated(decode_speech(speech_buf, recognizer), punct_hint.as_deref());
    // Collapse guard — see decode_collapsed. A collapsed final would replace
    // the sentence on screen with nothing (or junk); keep the best partial
    // the user was looking at instead.
    if let Some(best) = best_partial {
        if decode_collapsed(&text, &best) {
            eprintln!("buzz-desktop: STT final collapsed ({text:?}) — keeping best partial");
            text = best;
        } else if let Some(stitched) = stitch_prefix_collapse(&best, &text) {
            eprintln!("buzz-desktop: STT final dropped its front ({text:?}) — stitched");
            text = stitched;
        }
    }
    // In live mode finals ride the same channel as partials so the receiver
    // never sees a stale partial after its final — and an EMPTY final is still
    // sent so the frontend clears the partial shown for a phrase that decoded
    // to nothing (noise); skipping it strands that partial on screen.
    // (With the collapse guard above, "empty" here means no partial was ever
    // shown either — a phrase the guard let through.)
    let send_err = match live_tx {
        Some(tx) => tx
            .blocking_send(LiveEvent::Transcript {
                text,
                is_final: true,
            })
            .err()
            .map(|e| e.to_string()),
        None if text.is_empty() => None,
        None => text_tx.blocking_send(text).err().map(|e| e.to_string()),
    };
    if let Some(e) = send_err {
        eprintln!("buzz-desktop: STT text channel closed: {e}");
    }
}

/// Run sherpa-onnx on the accumulated speech and return the trimmed text
/// (empty on empty input or decode failure).
fn decode_speech(speech_buf: &[f32], recognizer: &sherpa_onnx::OfflineRecognizer) -> String {
    if speech_buf.is_empty() {
        return String::new();
    }
    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, speech_buf);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
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

#[cfg(test)]
mod prefer_punctuated_tests {
    use super::prefer_punctuated;

    #[test]
    fn keeps_dropped_terminal_punctuation() {
        let hint = Some("Is there anything we can do about that?");
        assert_eq!(
            prefer_punctuated("is there anything we can do about that".into(), hint),
            "Is there anything we can do about that?"
        );
    }

    #[test]
    fn internal_punctuation_flicker_still_matches() {
        // Observed in live_partial_sequence: the final decode swapped an
        // internal "." for "," — same words, so the punctuated hint wins.
        let hint = Some("I'm going to ask a question. Do you know why that happened?");
        assert_eq!(
            prefer_punctuated(
                "I'm going to ask a question, Do you know why that happened".into(),
                hint
            ),
            "I'm going to ask a question. Do you know why that happened?"
        );
    }

    #[test]
    fn grafts_terminal_mark_when_only_last_word_agrees() {
        assert_eq!(
            prefer_punctuated(
                "do you know er why that happened,".into(),
                Some("Do you know why that happened?")
            ),
            "do you know er why that happened?"
        );
    }

    #[test]
    fn comma_hint_is_never_grafted() {
        assert_eq!(
            prefer_punctuated("hello there friend".into(), Some("hello there,")),
            "hello there friend"
        );
    }

    #[test]
    fn ignores_hint_when_text_differs() {
        assert_eq!(
            prefer_punctuated("hello world again".into(), Some("hello world.")),
            "hello world again"
        );
    }

    #[test]
    fn never_replaces_empty_final() {
        assert_eq!(prefer_punctuated(String::new(), Some("hello.")), "");
    }

    #[test]
    fn no_hint_passes_through() {
        assert_eq!(prefer_punctuated("hello".into(), None), "hello");
    }
}

#[cfg(test)]
mod silence_experiment {
    use super::{decode_speech, prefer_punctuated, process_16k_samples};

    /// Manual experiment: does Parakeet v3 (0.6B transducer) produce stable
    /// punctuation under noisy pauses where the 110M CTC model flickers?
    /// Also times each decode — the live mode re-decodes the whole phrase
    /// every ~300 ms, so per-decode latency bounds the streaming cadence.
    /// Run: cargo test v3_punctuation -- --ignored --nocapture
    /// Needs ~/.buzz/models/parakeet-tdt-0.6b-v3 downloaded (see models.rs).
    #[test]
    #[ignore]
    fn v3_punctuation() {
        for threads in [1i32, 2, 4] {
            let recognizer = v3_recognizer(threads);
            println!("--- num_threads = {threads} ---");
            run_decodes(&recognizer);
        }
    }

    fn v3_recognizer(threads: i32) -> sherpa_onnx::OfflineRecognizer {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-0.6b-v3");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.transducer.encoder = Some(
            model_dir
                .join("encoder.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.transducer.decoder = Some(
            model_dir
                .join("decoder.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.transducer.joiner = Some(
            model_dir
                .join("joiner.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens =
            Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.debug = false;
        cfg.model_config.num_threads = threads;
        sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap()
    }

    fn read_wav_16k(path: &str) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap();
        let data_pos = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("no data chunk")
            + 8;
        bytes[data_pos..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    /// Manual experiment: chl reported that during a long continuous ramble
    /// "a whole bunch of text just got removed from the end". Two suspects:
    /// (a) v3 transducer output degrades/truncates as the phrase buffer grows
    ///     toward the 30 s MAX_SPEECH_SAMPLES cap, so a later partial decode
    ///     yields LESS text than an earlier one and the frontend diff deletes
    ///     the tail;
    /// (b) per-decode latency on long buffers (~70 ms/s of audio) blows past
    ///     the 300 ms partial cadence, the worker falls behind and the 5 s
    ///     audio queue overflows.
    /// This decodes growing prefixes of a 35 s ramble, timing each decode and
    /// flagging any shrinkage between consecutive prefixes.
    /// Run: cargo test v3_long_buffer -- --ignored --nocapture
    /// Setup: say -v Samantha "<long ~35s text>" -o /tmp/dict_long.wav
    ///   --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn v3_long_buffer() {
        let recognizer = v3_recognizer(super::LIVE_STT_NUM_THREADS);
        let samples = read_wav_16k("/tmp/dict_long.wav");
        println!("total: {:.1}s", samples.len() as f32 / 16_000.0);
        let mut prev = String::new();
        for secs in [4, 8, 12, 16, 20, 22, 24, 26, 28, 30] {
            let end = (16_000 * secs).min(samples.len());
            let t0 = std::time::Instant::now();
            let text = decode_speech(&samples[..end], &recognizer);
            let shrink = if text.len() < prev.len() { "  <<< SHRANK" } else { "" };
            println!(
                "@{secs:>2}s [{:>6.0?}ms] {} chars{shrink}: {text:?}",
                t0.elapsed().as_millis(),
                text.len()
            );
            prev = text;
            if end == samples.len() {
                break;
            }
        }

        // Simulate the worker's adaptive partial cadence over the same audio:
        // next partial waits max(PARTIAL_DECODE_STEP, last decode duration).
        // Real-time safe iff cumulative decode time stays under audio time.
        println!("--- adaptive cadence simulation ---");
        let mut last_len = 0usize;
        let mut step = super::PARTIAL_DECODE_STEP;
        let mut decode_total = std::time::Duration::ZERO;
        let mut n = 0u32;
        while last_len + step <= samples.len() {
            last_len += step;
            let t0 = std::time::Instant::now();
            let text = decode_speech(&samples[..last_len], &recognizer);
            let dt = t0.elapsed();
            decode_total += dt;
            step = super::PARTIAL_DECODE_STEP.max(32 * dt.as_millis() as usize);
            n += 1;
            println!(
                "partial {n} @ audio {:>4.1}s: decode {:>4.0?}ms, next step {:.1}s, tail {:?}",
                last_len as f32 / 16_000.0,
                dt.as_millis(),
                step as f32 / 16_000.0,
                &text[text.len().saturating_sub(40)..]
            );
        }
        let audio_secs = last_len as f32 / 16_000.0;
        println!(
            "{n} partials over {audio_secs:.1}s audio, total decode {:.1}s → real-time safe: {}",
            decode_total.as_secs_f32(),
            decode_total.as_secs_f32() < audio_secs
        );
    }

    fn run_decodes(recognizer: &sherpa_onnx::OfflineRecognizer) {
        for path in ["/tmp/dict_multi.wav", "/tmp/dict_q.wav"] {
            let bytes = std::fs::read(path).unwrap();
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            println!(
                "=== {path} ({:.1}s speech) ===",
                samples.len() as f32 / 16_000.0
            );
            for (label, amp, tail_ms) in [
                ("bare", 0.0f32, 0usize),
                ("300ms noise", 0.01, 300),
                ("608ms noise", 0.01, 608),
                ("608ms loud noise", 0.03, 608),
            ] {
                let mut buf = samples.clone();
                let mut state = 0x2545_f491u32;
                for _ in 0..(16 * tail_ms) {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    buf.push((state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp);
                }
                let t0 = std::time::Instant::now();
                let text = decode_speech(&buf, recognizer);
                println!("  +{label}: [{:?}] {text:?}", t0.elapsed());
            }
        }
    }

    /// Manual experiment: simulate the live worker's exact cadence over real
    /// (synthesized) speech with a noisy tail — decode the growing buffer
    /// every PARTIAL_DECODE_STEP samples, track `punct_partial` like the
    /// worker does, then run the final flush through `prefer_punctuated`.
    /// Shows whether the sticky-punctuation hint actually matches on
    /// multi-sentence phrases.
    /// Run: cargo test live_partial_sequence -- --ignored --nocapture
    /// Setup: say -v Samantha "It missed the full stop on the first
    ///   sentence. I'm now going to try and ask a question. Do you know why
    ///   that happened" -o /tmp/dict_multi.wav --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn live_partial_sequence() {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-ctc-110m-en");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.nemo_ctc.model = Some(
            model_dir
                .join("model.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens =
            Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.num_threads = 1;
        cfg.model_config.debug = false;
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap();

        for (path, amp) in [
            ("/tmp/dict_multi.wav", 0.01f32),
            ("/tmp/dict_q.wav", 0.01f32),
        ] {
            let bytes = std::fs::read(path).unwrap();
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let mut samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            // Noisy silence tail up to the live flush window (~608 ms).
            let mut state = 0x2545_f491u32;
            for _ in 0..(16 * 608) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                samples.push((state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp);
            }
            println!("=== {path} (noise amp {amp}) ===");
            let mut punct_partial: Option<String> = None;
            let mut last_len = 0usize;
            while last_len + super::PARTIAL_DECODE_STEP <= samples.len() {
                last_len += super::PARTIAL_DECODE_STEP;
                let text = decode_speech(&samples[..last_len], &recognizer);
                if text.ends_with(['.', '?', '!', ',']) {
                    punct_partial = Some(text.clone());
                }
                println!("  partial@{last_len}: {text:?}");
            }
            let final_text = decode_speech(&samples, &recognizer);
            println!("  FINAL:  {final_text:?}");
            println!("  HINT:   {punct_partial:?}");
            println!(
                "  KEPT:   {:?}",
                prefer_punctuated(final_text, punct_partial.as_deref())
            );
            // Rejected candidate fix (measured 2026-07-24): replacing the
            // noisy VAD hangover with digital zeros at flush time sometimes
            // REMOVES punctuation the noisy final had — tail composition is
            // non-monotonic, so don't retry audio-level recipes.
            let speech_end = samples.len() - 16 * 608;
            let mut zeroed = samples[..speech_end].to_vec();
            zeroed.extend(std::iter::repeat_n(0.0f32, 16 * 600));
            println!("  ZEROED-TAIL: {:?}", decode_speech(&zeroed, &recognizer));
            // Same but with noise overlaid on the speech itself (real mics
            // pick up room noise under the voice too, not just in pauses).
            let mut state = 0x1234_5678u32;
            let mut noisy: Vec<f32> = samples[..speech_end]
                .iter()
                .map(|s| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    s + (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect();
            noisy.extend(std::iter::repeat_n(0.0f32, 16 * 600));
            println!(
                "  NOISY-SPEECH+ZEROED-TAIL: {:?}",
                decode_speech(&noisy, &recognizer)
            );
        }
    }

    /// Manual experiment: reproduce chl's "captured it, then deleted it and
    /// wrote Yeah" report (2026-07-24, Enhanced/v3 model) at the worker level.
    /// Drives `process_16k_samples` exactly like the live worker — real VAD,
    /// onset debounce, silence flush, partial cadence — over composed audio:
    /// room noise, the spoken sentence (noise overlaid), a post-speech tail,
    /// then the 1 s stop-flush zeros. A frontend simulator applies the events
    /// the way useDictation.ts does and prints what the composer would show.
    /// Run: cargo test v3_live_worker_sim -- --ignored --nocapture
    /// Setup: say -v Samantha "just testing to see if this is working any
    ///   better" -o /tmp/dict_better.wav --data-format=LEF32@16000
    #[test]
    #[ignore]
    fn v3_live_worker_sim() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let recognizer = v3_recognizer(super::LIVE_STT_NUM_THREADS);
        let speech = read_wav_16k("/tmp/dict_better.wav");

        let noise = |len: usize, amp: f32, seed: u32| {
            let mut state = seed;
            (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect::<Vec<f32>>()
        };
        let overlay = |samples: &[f32], amp: f32| {
            let mut state = 0x1234_5678u32;
            samples
                .iter()
                .map(|s| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    s + (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                })
                .collect::<Vec<f32>>()
        };

        // (label, pre-speech, post-speech tail before the stop flush)
        let scenarios: Vec<(&str, Vec<f32>, Vec<f32>)> = vec![
            (
                "release right after speaking",
                noise(16 * 500, 0.01, 0xA1),
                noise(16 * 400, 0.01, 0xB2),
            ),
            (
                "pause >608ms then breath then release",
                noise(16 * 500, 0.01, 0xA1),
                [
                    noise(16 * 900, 0.01, 0xB2),
                    noise(16 * 250, 0.08, 0xC3), // breath/exhale burst
                    noise(16 * 300, 0.01, 0xD4),
                ]
                .concat(),
            ),
            (
                "louder room noise",
                noise(16 * 500, 0.03, 0xA1),
                noise(16 * 700, 0.03, 0xB2),
            ),
        ];

        for (label, pre, tail) in scenarios {
            println!("=== scenario: {label} ===");
            let mut audio = pre;
            audio.extend(overlay(&speech, 0.01));
            audio.extend(tail);
            audio.extend(std::iter::repeat_n(0.0f32, 16_000)); // stop-flush 1s zeros

            let (live_tx, mut live_rx) = tokio::sync::mpsc::channel::<super::LiveEvent>(1024);
            let (text_tx, _text_rx) = tokio::sync::mpsc::channel::<String>(64);
            let tts_active = Arc::new(AtomicBool::new(false));

            let mut vad = earshot::Detector::new(earshot::DefaultPredictor::new());
            let mut leftover = Vec::new();
            let mut speech_buf = Vec::new();
            let mut silence_frames = 0usize;
            let mut in_speech = false;
            let mut barge_in_frames = 0usize;
            let mut tts_stopped_at = None;
            let mut last_partial_len = 0usize;
            let mut decode_hold_until = std::time::Instant::now();
            let mut onset_buf = Vec::new();
            let mut punct_partial: Option<String> = None;
            let mut best_partial: Option<String> = None;

            for chunk in audio.chunks(320) {
                process_16k_samples(
                    chunk,
                    &mut leftover,
                    &mut vad,
                    &mut speech_buf,
                    &mut silence_frames,
                    &mut in_speech,
                    &mut barge_in_frames,
                    &recognizer,
                    &text_tx,
                    &tts_active,
                    None,
                    &mut tts_stopped_at,
                    None,
                    Some(&live_tx),
                    &mut last_partial_len,
                    &mut decode_hold_until,
                    &mut onset_buf,
                    &mut punct_partial,
                    &mut best_partial,
                );
            }
            drop(live_tx);

            // Frontend simulator — the transcriptDiff mechanics reduce to this.
            let mut committed = String::new();
            let mut partial = String::new();
            while let Ok(event) = live_rx.try_recv() {
                match event {
                    super::LiveEvent::Transcript { text, is_final } => {
                        let trimmed = text.trim();
                        println!(
                            "  {} {trimmed:?}",
                            if is_final { "FINAL  " } else { "partial" }
                        );
                        if is_final {
                            if !trimmed.is_empty() {
                                committed.push_str(trimmed);
                                committed.push(' ');
                            } else if !partial.is_empty() {
                                println!("  !!! empty final wiped shown partial {partial:?}");
                            }
                            partial.clear();
                        } else if !trimmed.is_empty() {
                            if trimmed.len() + 10 < partial.len() {
                                println!("  !!! partial shrank {partial:?} -> {trimmed:?}");
                            }
                            partial = trimmed.to_string();
                        }
                    }
                    super::LiveEvent::Flushed => println!("  FLUSHED"),
                }
            }
            println!("  composer: {:?}", format!("{committed}{partial}"));
        }
    }

    /// Manual experiment: how much trailing silence does the model need before
    /// it commits sentence-final punctuation? (Answer when tuned: ≥600 ms —
    /// see LIVE_SILENCE_FLUSH_FRAMES.)
    /// Run: cargo test silence_vs_punctuation -- --ignored --nocapture
    /// Setup: say -v Samantha "is there anything we can do about that?" \
    ///          -o /tmp/dict_q.wav --data-format=LEF32@16000   (same for dict_s)
    #[test]
    #[ignore]
    fn silence_vs_punctuation() {
        let model_dir = dirs::home_dir()
            .unwrap()
            .join(".buzz/models/parakeet-tdt-ctc-110m-en");
        let mut cfg = sherpa_onnx::OfflineRecognizerConfig::default();
        cfg.model_config.nemo_ctc.model = Some(
            model_dir
                .join("model.int8.onnx")
                .to_string_lossy()
                .into_owned(),
        );
        cfg.model_config.tokens =
            Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        cfg.model_config.num_threads = 1;
        cfg.model_config.debug = false;
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&cfg).unwrap();

        for path in ["/tmp/dict_q.wav", "/tmp/dict_s.wav"] {
            let bytes = std::fs::read(path).unwrap();
            // Naive WAV parse: find the "data" chunk, samples are f32 LE.
            let data_pos = bytes
                .windows(4)
                .position(|w| w == b"data")
                .expect("no data chunk")
                + 8;
            let samples: Vec<f32> = bytes[data_pos..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            println!("=== {path} ({} samples) ===", samples.len());
            for silence_ms in [0usize, 100, 300, 600] {
                let mut buf = samples.clone();
                buf.extend(std::iter::repeat_n(0.0f32, 16 * silence_ms));
                println!(
                    "  +{silence_ms}ms zeros: {:?}",
                    decode_speech(&buf, &recognizer)
                );
            }
            // Real mic "silence" is room noise, not zeros — simulate with
            // low-level deterministic pseudo-noise (LCG; no rand dep) at
            // amplitudes well below the VAD speech threshold.
            for amp in [0.002f32, 0.01, 0.03] {
                let noise = |len: usize| {
                    let mut state = 0x2545_f491u32;
                    (0..len)
                        .map(|_| {
                            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (state as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
                        })
                        .collect::<Vec<f32>>()
                };
                let mut buf = samples.clone();
                buf.extend(noise(16 * 608));
                println!("  +608ms noise(amp {amp}): {:?}", decode_speech(&buf, &recognizer));
                buf.extend(std::iter::repeat_n(0.0f32, 16 * 600));
                println!(
                    "  +608ms noise(amp {amp}) + 600ms zeros: {:?}",
                    decode_speech(&buf, &recognizer)
                );
            }
        }
    }
}
