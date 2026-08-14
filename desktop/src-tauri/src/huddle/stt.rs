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
    /// Mic input is transcribed even while agent TTS is playing: the huddle UI
    /// already tells users to wear headphones, so speaker bleed is accepted in
    /// exchange for never dropping human speech that overlaps agent audio.
    /// Local mic frames still never cancel TTS — push-to-talk and remote
    /// participant speech remain the explicit barge-in paths.
    ///
    /// `ptt_active` and `manual_mic_unmuted` are present when the PTT shortcut
    /// is enabled. The pipeline accepts speech while either input path is open;
    /// manual unmute uses normal VAD flushing while a shortcut hold is grouped
    /// into one utterance.
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
        ptt_active: Option<Arc<AtomicBool>>,
        manual_mic_unmuted: Option<Arc<AtomicBool>>,
    ) -> Result<(Self, tokio_mpsc::Receiver<String>), String> {
        let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<u8>>(AUDIO_QUEUE_DEPTH);
        let (text_tx, text_rx) = tokio_mpsc::channel::<String>(64);
        let shutdown = Arc::new(AtomicBool::new(false));

        let shutdown_worker = Arc::clone(&shutdown);
        let ptt_active_worker = ptt_active.as_ref().map(Arc::clone);
        let manual_mic_unmuted_worker = manual_mic_unmuted.as_ref().map(Arc::clone);
        let handle = thread::Builder::new()
            .name("stt-worker".into())
            .spawn(move || {
                stt_worker(
                    model_dir,
                    audio_rx,
                    text_tx,
                    shutdown_worker,
                    ptt_active_worker,
                    manual_mic_unmuted_worker,
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
///
/// This window is a turn-taking quality knob, not a latency lever: an earlier
/// env override (`BUZZ_STT_FLUSH_MS`) let it be lowered to 150 ms, which split
/// natural mid-sentence pauses into separate messages and confused the
/// listening agents. Reverted — the window is fixed at the production value.
const SILENCE_FLUSH_FRAMES: usize = 19;

/// earshot requires exactly 256 samples per frame at 16 kHz.
const VAD_FRAME_SAMPLES: usize = 256;

/// VAD probability threshold — above this is considered speech.
const VAD_THRESHOLD: f32 = 0.5;

/// Minimum voiced audio needed before an utterance may be decoded.
/// One earshot false-positive frame is only 16 ms; requiring 192 ms prevents
/// silence/room-noise blips from reaching Parakeet and becoming hallucinated
/// transcript text while still preserving short replies such as "yes".
const MIN_VOICED_FRAMES: usize = 12;

/// How long the worker waits on the audio channel before checking the shutdown flag.
const RECV_TIMEOUT: Duration = Duration::from_millis(50);

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

/// EXPERIMENTAL (latency bench): override recognizer intra-op threads via
/// `BUZZ_STT_THREADS`. Default preserves the production single thread.
fn stt_num_threads() -> i32 {
    std::env::var("BUZZ_STT_THREADS")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(STT_NUM_THREADS)
}

/// EXPERIMENTAL (latency bench): `BUZZ_STT_SPECULATIVE=1` starts the Parakeet
/// decode at the FIRST silent VAD frame instead of after the full flush
/// window, overlapping the ~150-250 ms decode with the silence wait. If
/// speech resumes, the speculative result is discarded. When silence holds
/// to the flush threshold the transcript is emitted immediately, so the STT
/// leg collapses to ~max(flush window, decode time).
fn stt_speculative_decode() -> bool {
    std::env::var("BUZZ_STT_SPECULATIVE").is_ok_and(|v| v == "1")
}

fn stt_worker(
    model_dir: PathBuf,
    audio_rx: Receiver<Vec<u8>>,
    text_tx: tokio_mpsc::Sender<String>,
    shutdown: Arc<AtomicBool>,
    ptt_active: Option<Arc<AtomicBool>>,
    manual_mic_unmuted: Option<Arc<AtomicBool>>,
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
    cfg.model_config.num_threads = stt_num_threads();
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
    // Number of frames earshot classified as voiced in the current segment.
    let mut voiced_frames = 0;
    // Silence flush window (frames) — fixed at the production value.
    let flush_frames = SILENCE_FLUSH_FRAMES;
    // EXPERIMENTAL: speculative decode result + the voiced-frame count it was
    // computed at. Valid only while no new voiced frame has arrived since.
    let speculative_enabled = stt_speculative_decode();
    let mut speculative: Option<(String, usize)> = None;

    // ── 5. Main loop ──────────────────────────────────────────────────────────
    let mut transmit_was_active = ptt_active
        .as_ref()
        .is_some_and(|ptt| ptt.load(Ordering::Acquire))
        || manual_mic_unmuted
            .as_ref()
            .is_some_and(|manual| manual.load(Ordering::Acquire));
    loop {
        // Check shutdown flag before blocking.
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        // Track the combined manual/PTT transmission edge. When both paths
        // close, the worklet stops sending frames, so flush here rather than
        // waiting for silence that will never arrive.
        if let Some(ref ptt) = ptt_active {
            let transmit_now = ptt.load(Ordering::Acquire)
                || manual_mic_unmuted
                    .as_ref()
                    .is_some_and(|manual| manual.load(Ordering::Acquire));
            if transmit_was_active && !transmit_now && in_speech && !speech_buf.is_empty() {
                flush_to_stt(&speech_buf, voiced_frames, &recognizer, &text_tx);
                speech_buf.clear();
                silence_frames = 0;
                in_speech = false;
                voiced_frames = 0;
            }
            transmit_was_active = transmit_now;
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
                    &mut voiced_frames,
                    flush_frames,
                    (speculative_enabled, &mut speculative),
                    &recognizer,
                    &text_tx,
                    ptt_active.as_ref(),
                    manual_mic_unmuted.as_ref(),
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
/// Mic input keeps flowing while agent TTS plays: the huddle UI instructs
/// users to wear headphones, so overlapping human speech is transcribed
/// instead of discarded.
///
/// When `ptt_active` is `Some`, input is accepted while either the shortcut is
/// held or the microphone is manually unmuted. A held shortcut is an explicit
/// "I am not done talking" signal, so silence NEVER flushes while it is held —
/// even when the microphone is also manually open. The utterance flushes on
/// shortcut release (the transmit-edge flush in the worker loop) or, with a
/// manually open mic, via normal VAD pause flushing once the shortcut is up.
#[allow(clippy::too_many_arguments)]
fn process_16k_samples(
    samples: &[f32],
    leftover: &mut Vec<f32>,
    vad: &mut earshot::Detector<earshot::DefaultPredictor>,
    speech_buf: &mut Vec<f32>,
    silence_frames: &mut usize,
    in_speech: &mut bool,
    voiced_frames: &mut usize,
    flush_frames: usize,
    speculative: (bool, &mut Option<(String, usize)>),
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
    ptt_active: Option<&Arc<AtomicBool>>,
    manual_mic_unmuted: Option<&Arc<AtomicBool>>,
) {
    let (speculative_enabled, speculative) = speculative;
    leftover.extend_from_slice(samples);

    while leftover.len() >= VAD_FRAME_SAMPLES {
        let frame: Vec<f32> = leftover.drain(..VAD_FRAME_SAMPLES).collect();
        let clamped: Vec<f32> = frame.iter().map(|&s| s.clamp(-1.0, 1.0)).collect();
        let prob = vad.predict_f32(&clamped);
        let is_speech = prob > VAD_THRESHOLD;

        let manually_open = manual_mic_unmuted.is_some_and(|manual| manual.load(Ordering::Acquire));
        let ptt_held = ptt_active.is_some_and(|ptt| ptt.load(Ordering::Acquire));
        // Shortcut-enabled mode accepts input from either the held shortcut or
        // a manually open microphone.
        let is_speech = if ptt_active.is_some() {
            is_speech && (ptt_held || manually_open)
        } else {
            is_speech
        };
        // A held shortcut means "I am not done talking": silence never ends
        // the utterance while it is held. VAD pause flushing applies in pure
        // VAD mode, or with a manually open mic once the shortcut is up.
        let vad_flush_allowed = vad_flush_allowed(ptt_active.is_some(), manually_open, ptt_held);

        if is_speech {
            *silence_frames = 0;
            *in_speech = true;
            *voiced_frames += 1;
            speech_buf.extend_from_slice(&frame);
            // New voiced audio invalidates any speculative decode.
            speculative.take();

            // OOM guard: flush and reset if the buffer exceeds 30 s of audio.
            if speech_buf.len() >= MAX_SPEECH_SAMPLES {
                flush_to_stt(speech_buf, *voiced_frames, recognizer, text_tx);
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
                *voiced_frames = 0;
            }
        } else if *in_speech {
            // Still accumulate during brief silence gaps.
            speech_buf.extend_from_slice(&frame);
            *silence_frames += 1;

            // EXPERIMENTAL: kick the Parakeet decode at the first silent
            // frame so it overlaps the flush window. speech_buf keeps
            // accumulating silence afterwards, but trailing silence does not
            // change the transcript; any resumed speech invalidates the
            // speculative result above.
            if speculative_enabled
                && speculative.is_none()
                && vad_flush_allowed
                && has_enough_voiced_audio(*voiced_frames)
            {
                speculative.replace((decode_speech(recognizer, speech_buf), *voiced_frames));
            }

            // A manually open microphone behaves like normal VAD. A held
            // shortcut keeps the utterance grouped until key release.
            if vad_flush_allowed && *silence_frames >= flush_frames {
                // End of utterance — transcribe (or emit the speculative decode).
                match speculative.take() {
                    Some((text, decoded_at)) if decoded_at == *voiced_frames => {
                        send_transcript(text, text_tx);
                    }
                    _ => flush_to_stt(speech_buf, *voiced_frames, recognizer, text_tx),
                }
                speech_buf.clear();
                *silence_frames = 0;
                *in_speech = false;
                *voiced_frames = 0;
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
    voiced_frames: usize,
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
) {
    if speech_buf.is_empty() || !has_enough_voiced_audio(voiced_frames) {
        return;
    }
    send_transcript(decode_speech(recognizer, speech_buf), text_tx);
}

/// Run the Parakeet decode on a speech buffer and return the trimmed text.
fn decode_speech(recognizer: &sherpa_onnx::OfflineRecognizer, speech_buf: &[f32]) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, speech_buf);
    recognizer.decode(&stream);

    stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
}

fn send_transcript(text: String, text_tx: &tokio_mpsc::Sender<String>) {
    if !text.is_empty() {
        if let Err(e) = text_tx.blocking_send(text) {
            eprintln!("buzz-desktop: STT text channel closed: {e}");
        }
    }
}

fn has_enough_voiced_audio(voiced_frames: usize) -> bool {
    voiced_frames >= MIN_VOICED_FRAMES
}

/// Whether a silence run may end the current utterance and flush it to STT.
///
/// Pure VAD mode (no shortcut configured) always allows pause flushing. When
/// the push-to-talk shortcut is configured, a held shortcut is an explicit
/// "I am not done talking" signal, so silence never flushes while it is held
/// — even if the microphone is also manually open. A manually open mic with
/// the shortcut up behaves like normal VAD.
fn vad_flush_allowed(ptt_mode: bool, manually_open: bool, ptt_held: bool) -> bool {
    !ptt_mode || (manually_open && !ptt_held)
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
mod tests {
    use super::{has_enough_voiced_audio, vad_flush_allowed, MIN_VOICED_FRAMES};

    #[test]
    fn short_vad_blips_do_not_reach_the_recognizer() {
        assert!(!has_enough_voiced_audio(1));
        assert!(!has_enough_voiced_audio(MIN_VOICED_FRAMES - 1));
        assert!(has_enough_voiced_audio(MIN_VOICED_FRAMES));
    }

    #[test]
    fn held_push_to_talk_never_silence_flushes() {
        // Pure VAD mode: silence always ends the utterance.
        assert!(vad_flush_allowed(false, false, false));
        // Shortcut configured, nothing transmitting: nothing to flush anyway,
        // but the pause path stays closed.
        assert!(!vad_flush_allowed(true, false, false));
        // Shortcut held: "I am not done talking" — never flush on silence,
        // regardless of the manual mic state.
        assert!(!vad_flush_allowed(true, false, true));
        assert!(!vad_flush_allowed(true, true, true));
        // Manually open mic with the shortcut up: normal VAD behavior.
        assert!(vad_flush_allowed(true, true, false));
    }
}
