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
    collections::VecDeque,
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

use super::{human_floor::HumanFloor, local_barge_in};

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
    audio_tx: SyncSender<SttAudioInput>,
    /// Signals the worker thread to stop.
    shutdown: Arc<AtomicBool>,
    /// Worker thread handle — taken on drop to join cleanly.
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Debug)]
struct SttAudioInput {
    pcm_bytes: Vec<u8>,
    origin: SttAudioOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SttAudioOrigin {
    Local,
    RemoteHuman,
}

impl SttPipeline {
    /// Spawn the pipeline thread.
    ///
    /// Mic input is transcribed even while agent TTS is playing. In open-mic
    /// VAD mode, confirmed speech acquires the shared human floor: immediately
    /// on an isolated output route, or after the restored 320 ms sustained-
    /// speech debounce on an acoustically coupled route. Push-to-talk retains
    /// its explicit shortcut cancellation path.
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
        human_floor: HumanFloor,
        output_device: Option<String>,
    ) -> Result<(Self, tokio_mpsc::Receiver<String>), String> {
        let (audio_tx, audio_rx) = mpsc::sync_channel::<SttAudioInput>(AUDIO_QUEUE_DEPTH);
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
                    human_floor,
                    output_device,
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
        self.push_audio_from(pcm_bytes, SttAudioOrigin::Local)
    }

    /// Feed decoded remote-human PCM into transcription. Unlike the desktop
    /// microphone path, this is not gated by the desktop PTT or mute state: the
    /// remote participant already made their transmission choice on their own
    /// device before the relay delivered these samples.
    pub fn push_remote_audio(&self, pcm_bytes: Vec<u8>) -> Result<(), String> {
        self.push_audio_from(pcm_bytes, SttAudioOrigin::RemoteHuman)
    }

    fn push_audio_from(&self, pcm_bytes: Vec<u8>, origin: SttAudioOrigin) -> Result<(), String> {
        // Reject non-4-byte-aligned input — would silently truncate in bytes_to_f32.
        if !pcm_bytes.len().is_multiple_of(4) {
            return Err(format!(
                "audio input not 4-byte aligned ({} bytes) — expected f32 LE samples",
                pcm_bytes.len()
            ));
        }
        // Drop audio if the pipeline can't keep up — better than blocking the UI.
        let _ = self.audio_tx.try_send(SttAudioInput { pcm_bytes, origin });
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
/// 500 ms × 16 000 Hz / 256 samples-per-frame ≈ 31 frames.
/// This favors natural conversational pauses over the lower latency of the
/// previous 19-frame / 304 ms window.
///
/// This window is a turn-taking quality knob, not a latency lever: an earlier
/// env override (`BUZZ_STT_FLUSH_MS`) let it be lowered to 150 ms, which split
/// natural mid-sentence pauses into separate messages and confused the
/// listening agents. Reverted — the window is fixed at the production value.
const SILENCE_FLUSH_FRAMES: usize = 31;

/// earshot requires exactly 256 samples per frame at 16 kHz.
const VAD_FRAME_SAMPLES: usize = 256;

/// Earshot 1.1.0 onset operating point. Any Earshot model/version change
/// invalidates this and `VAD_OFFSET_THRESHOLD`; re-run the matched-corpus
/// threshold harness before updating either constant.
const VAD_ONSET_THRESHOLD: f32 = 0.55;

/// Earshot 1.1.0 offset operating point. The lower threshold keeps borderline
/// speech inside the active utterance without changing the onset sensitivity.
const VAD_OFFSET_THRESHOLD: f32 = 0.35;

/// Consecutive onset frames required before an utterance begins.
const VAD_ONSET_FRAMES: usize = 3;

/// Audio retained before confirmed onset so initial phonemes are not clipped.
/// A rolling pre-roll that survived a hard boundary would leak segment N into
/// segment N+1 when the next confirmed onset occurs within
/// `VAD_PRE_ROLL_FRAMES - VAD_ONSET_FRAMES` frames (13 frames, or 208 ms, at
/// the shipped values) of the previous flush. Hangover and the silence flush
/// window do not enter this bound; `reset_segment` keeps them independent by
/// clearing pre-roll.
const VAD_PRE_ROLL_FRAMES: usize = 16;

/// Trailing silence retained in the transcript buffer (about 100 ms).
const VAD_HANGOVER_FRAMES: usize = 6;

/// Minimum voiced audio needed before an utterance may be decoded.
/// One earshot false-positive frame is only 16 ms; requiring 192 ms prevents
/// silence/room-noise blips from reaching Parakeet and becoming hallucinated
/// transcript text while still preserving short replies such as "yes".
const MIN_VOICED_FRAMES: usize = 12;

#[derive(Debug, PartialEq, Eq)]
enum VadFrameAction {
    None,
    ConfirmedOnset,
    Speech,
    FirstSilence,
    Flush,
}

struct VadEndpoint {
    pre_roll: VecDeque<Vec<f32>>,
    speech_buf: Vec<f32>,
    onset_frames: usize,
    silence_frames: usize,
    voiced_frames: usize,
    in_speech: bool,
}

impl VadEndpoint {
    fn new() -> Self {
        Self {
            pre_roll: VecDeque::with_capacity(VAD_PRE_ROLL_FRAMES),
            speech_buf: Vec::new(),
            onset_frames: 0,
            silence_frames: 0,
            voiced_frames: 0,
            in_speech: false,
        }
    }

    fn process_frame(
        &mut self,
        frame: Vec<f32>,
        probability: f32,
        accepts_audio: bool,
        flush_allowed: bool,
        flush_frames: usize,
    ) -> VadFrameAction {
        if !accepts_audio {
            self.pre_roll.clear();
            self.onset_frames = 0;
            return VadFrameAction::None;
        }

        if !self.in_speech {
            self.pre_roll.push_back(frame);
            if self.pre_roll.len() > VAD_PRE_ROLL_FRAMES {
                self.pre_roll.pop_front();
            }

            if probability > VAD_ONSET_THRESHOLD {
                self.onset_frames += 1;
            } else {
                self.onset_frames = 0;
            }

            if self.onset_frames < VAD_ONSET_FRAMES {
                return VadFrameAction::None;
            }

            self.in_speech = true;
            self.silence_frames = 0;
            self.voiced_frames = self.onset_frames;
            self.onset_frames = 0;
            for buffered in self.pre_roll.drain(..) {
                self.speech_buf.extend_from_slice(&buffered);
            }
            return VadFrameAction::ConfirmedOnset;
        }

        if probability > VAD_OFFSET_THRESHOLD {
            self.silence_frames = 0;
            self.voiced_frames += 1;
            self.speech_buf.extend_from_slice(&frame);
            return VadFrameAction::Speech;
        }

        self.silence_frames += 1;
        self.speech_buf.extend_from_slice(&frame);
        if flush_allowed && self.silence_frames >= flush_frames {
            let excess_silence = self.silence_frames.saturating_sub(VAD_HANGOVER_FRAMES);
            let retained_samples = self
                .speech_buf
                .len()
                .saturating_sub(excess_silence * VAD_FRAME_SAMPLES);
            self.speech_buf.truncate(retained_samples);
            VadFrameAction::Flush
        } else if self.silence_frames == 1 {
            VadFrameAction::FirstSilence
        } else {
            VadFrameAction::None
        }
    }

    fn reset_segment(&mut self) {
        self.speech_buf.clear();
        // A hard message boundary also clears pre-roll: fast follow-up turns
        // may receive less than the full window, but no frame can be decoded
        // into both adjacent transcript messages.
        self.pre_roll.clear();
        self.onset_frames = 0;
        self.silence_frames = 0;
        self.voiced_frames = 0;
        self.in_speech = false;
    }
}

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

struct SttStreamState {
    resampler: rubato::Fft<f32>,
    chunk_in: usize,
    input_buf_48k: Vec<f32>,
    leftover_16k: Vec<f32>,
    vad: earshot::Detector<earshot::DefaultPredictor>,
    endpoint: VadEndpoint,
    speculative: Option<(String, usize)>,
}

impl SttStreamState {
    fn new() -> Result<Self, String> {
        use rubato::{FixedSync, Resampler};

        let resampler = rubato::Fft::<f32>::new(48_000, 16_000, 1024, 2, 1, FixedSync::Input)
            .map_err(|error| format!("STT resampler init failed: {error}"))?;
        let chunk_in = resampler.input_frames_next();
        Ok(Self {
            resampler,
            chunk_in,
            input_buf_48k: Vec::with_capacity(chunk_in * 2),
            leftover_16k: Vec::new(),
            vad: earshot::Detector::new(earshot::DefaultPredictor::new()),
            endpoint: VadEndpoint::new(),
            speculative: None,
        })
    }
}

#[derive(Debug)]
enum SttLoopInput {
    Tick,
    Batch(Vec<SttAudioInput>),
}

fn run_stt_receive_loop(
    audio_rx: Receiver<SttAudioInput>,
    shutdown: &AtomicBool,
    human_floor: HumanFloor,
    mut process: impl FnMut(SttLoopInput, &mut local_barge_in::LocalBargeIn),
) {
    let mut local_barge_in_state = local_barge_in::WorkerLocalBargeIn::new(human_floor);

    loop {
        // Check shutdown flag before blocking.
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        process(SttLoopInput::Tick, &mut local_barge_in_state);

        // Use recv_timeout so we can periodically check the shutdown flag.
        let input = match audio_rx.recv_timeout(RECV_TIMEOUT) {
            Ok(input) => input,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break, // Sender dropped.
        };

        // Drain any additional pending messages to batch-process.
        let mut batch = vec![input];
        while let Ok(input) = audio_rx.try_recv() {
            batch.push(input);
        }
        process(SttLoopInput::Batch(batch), &mut local_barge_in_state);
    }
}

#[allow(clippy::too_many_arguments)]
fn stt_worker(
    model_dir: PathBuf,
    audio_rx: Receiver<SttAudioInput>,
    text_tx: tokio_mpsc::Sender<String>,
    shutdown: Arc<AtomicBool>,
    ptt_active: Option<Arc<AtomicBool>>,
    manual_mic_unmuted: Option<Arc<AtomicBool>>,
    human_floor: HumanFloor,
    output_device: Option<String>,
) {
    // ── 1. Initialise sherpa-onnx recognizer ─────────────────────────────────
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

    // ── 2. Independent local and remote processing state ─────────────────────
    // Separate resampler/VAD state prevents simultaneous desktop and remote
    // speech from being serialized into one artificial utterance.
    let mut local_stream = match SttStreamState::new() {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("buzz-desktop: {error}");
            return;
        }
    };
    let mut remote_stream = match SttStreamState::new() {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("buzz-desktop: {error}");
            return;
        }
    };
    let speculative_enabled = stt_speculative_decode();
    let mut transmit_was_active = ptt_active
        .as_ref()
        .is_some_and(|ptt| ptt.load(Ordering::Acquire))
        || manual_mic_unmuted
            .as_ref()
            .is_some_and(|manual| manual.load(Ordering::Acquire));

    run_stt_receive_loop(
        audio_rx,
        &shutdown,
        human_floor.clone(),
        |input, local_barge_in_state| match input {
            SttLoopInput::Tick => {
                // The worklet stops sending frames when both local transmit
                // paths close, so flush on that edge instead of waiting for
                // silence that will never arrive.
                if let Some(ref ptt) = ptt_active {
                    let transmit_now = ptt.load(Ordering::Acquire)
                        || manual_mic_unmuted
                            .as_ref()
                            .is_some_and(|manual| manual.load(Ordering::Acquire));
                    if transmit_was_active
                        && !transmit_now
                        && local_stream.endpoint.in_speech
                        && !local_stream.endpoint.speech_buf.is_empty()
                    {
                        flush_to_stt(
                            &local_stream.endpoint.speech_buf,
                            local_stream.endpoint.voiced_frames,
                            &recognizer,
                            &text_tx,
                        );
                        local_stream.endpoint.reset_segment();
                        local_stream.speculative.take();
                        local_barge_in_state.release(&human_floor);
                    }
                    transmit_was_active = transmit_now;
                }
            }
            SttLoopInput::Batch(batch) => {
                for input in batch {
                    let (stream, ptt_gate, manual_gate, track_local_floor) = match input.origin {
                        SttAudioOrigin::Local => (
                            &mut local_stream,
                            ptt_active.as_ref(),
                            manual_mic_unmuted.as_ref(),
                            true,
                        ),
                        SttAudioOrigin::RemoteHuman => (&mut remote_stream, None, None, false),
                    };
                    process_stt_input(
                        stream,
                        &input.pcm_bytes,
                        speculative_enabled,
                        &recognizer,
                        &text_tx,
                        ptt_gate,
                        manual_gate,
                        &human_floor,
                        local_barge_in_state,
                        output_device.as_deref(),
                        track_local_floor,
                    );
                }
            }
        },
    );

    // No final flush — leave_huddle/end_huddle emit lifecycle events before
    // the STT worker exits, so a final flush would post a kind:9 message AFTER
    // the user has "left." Losing the last partial utterance is acceptable.
}

#[allow(clippy::too_many_arguments)]
fn process_stt_input(
    stream: &mut SttStreamState,
    pcm_bytes: &[u8],
    speculative_enabled: bool,
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
    ptt_active: Option<&Arc<AtomicBool>>,
    manual_mic_unmuted: Option<&Arc<AtomicBool>>,
    human_floor: &HumanFloor,
    local_barge_in_state: &mut local_barge_in::LocalBargeIn,
    output_device: Option<&str>,
    track_local_floor: bool,
) {
    stream
        .input_buf_48k
        .extend_from_slice(&bytes_to_f32(pcm_bytes));

    while stream.input_buf_48k.len() >= stream.chunk_in {
        let chunk: Vec<f32> = stream.input_buf_48k.drain(..stream.chunk_in).collect();
        let resampled = resample_chunk(&mut stream.resampler, &chunk);
        process_16k_samples(
            &resampled,
            &mut stream.leftover_16k,
            &mut stream.vad,
            &mut stream.endpoint,
            SILENCE_FLUSH_FRAMES,
            (speculative_enabled, &mut stream.speculative),
            recognizer,
            text_tx,
            ptt_active,
            manual_mic_unmuted,
            human_floor,
            local_barge_in_state,
            output_device,
            track_local_floor,
        );
    }
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
    endpoint: &mut VadEndpoint,
    flush_frames: usize,
    speculative: (bool, &mut Option<(String, usize)>),
    recognizer: &sherpa_onnx::OfflineRecognizer,
    text_tx: &tokio_mpsc::Sender<String>,
    ptt_active: Option<&Arc<AtomicBool>>,
    manual_mic_unmuted: Option<&Arc<AtomicBool>>,
    human_floor: &HumanFloor,
    local_barge_in_state: &mut local_barge_in::LocalBargeIn,
    output_device: Option<&str>,
    track_local_floor: bool,
) {
    let (speculative_enabled, speculative) = speculative;
    leftover.extend_from_slice(samples);

    while leftover.len() >= VAD_FRAME_SAMPLES {
        let frame: Vec<f32> = leftover.drain(..VAD_FRAME_SAMPLES).collect();
        let clamped: Vec<f32> = frame.iter().map(|&s| s.clamp(-1.0, 1.0)).collect();
        let prob = vad.predict_f32(&clamped);
        let manually_open = manual_mic_unmuted.is_some_and(|manual| manual.load(Ordering::Acquire));
        let ptt_held = ptt_active.is_some_and(|ptt| ptt.load(Ordering::Acquire));
        let accepts_audio = ptt_active.is_none() || ptt_held || manually_open;
        // A held shortcut means "I am not done talking": silence never ends
        // the utterance while it is held. VAD pause flushing applies in pure
        // VAD mode, or with a manually open mic once the shortcut is up.
        let flush_allowed = vad_flush_allowed(ptt_active.is_some(), manually_open, ptt_held);

        let action =
            endpoint.process_frame(frame, prob, accepts_audio, flush_allowed, flush_frames);
        // Open-mic VAD semantics also apply when a PTT-mode user manually
        // opens the mic. A held shortcut keeps its explicit key-down cancel.
        let local_barge_in = track_local_floor
            && local_barge_in::enabled(ptt_active.is_some(), manually_open, ptt_held);
        if track_local_floor {
            if local_barge_in {
                local_barge_in_state.observe(
                    prob,
                    action == VadFrameAction::ConfirmedOnset,
                    human_floor,
                    output_device,
                    VAD_ONSET_THRESHOLD,
                );
            } else {
                local_barge_in_state.release(human_floor);
            }
        }

        match action {
            VadFrameAction::ConfirmedOnset => {
                speculative.take();
            }
            VadFrameAction::Speech => {
                // New voiced audio invalidates any speculative decode.
                speculative.take();
            }
            VadFrameAction::FirstSilence => {
                // Start speculative decode at the first silent frame. Any
                // resumed speech invalidates this result in the arm above.
                if speculative_enabled
                    && speculative.is_none()
                    && flush_allowed
                    && has_enough_voiced_audio(endpoint.voiced_frames)
                {
                    speculative.replace((
                        decode_speech(recognizer, &endpoint.speech_buf),
                        endpoint.voiced_frames,
                    ));
                }
            }
            VadFrameAction::Flush => {
                match speculative.take() {
                    Some((text, decoded_at)) if decoded_at == endpoint.voiced_frames => {
                        send_transcript(text, text_tx);
                    }
                    _ => flush_to_stt(
                        &endpoint.speech_buf,
                        endpoint.voiced_frames,
                        recognizer,
                        text_tx,
                    ),
                }
                endpoint.reset_segment();
                if local_barge_in {
                    local_barge_in_state.release(human_floor);
                }
            }
            VadFrameAction::None => {}
        }

        // Preserve the 30 s guard even while PTT suppresses silence flushing.
        if endpoint.speech_buf.len() >= MAX_SPEECH_SAMPLES {
            flush_to_stt(
                &endpoint.speech_buf,
                endpoint.voiced_frames,
                recognizer,
                text_tx,
            );
            endpoint.reset_segment();
            if local_barge_in {
                local_barge_in_state.release(human_floor);
            }
            speculative.take();
        }
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
    if speech_buf.is_empty() {
        return;
    }
    if !has_enough_voiced_audio(voiced_frames) {
        eprintln!(
            "buzz-desktop: STT dropped short VAD segment ({voiced_frames}/{MIN_VOICED_FRAMES} voiced frames)"
        );
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
#[path = "stt_tests.rs"]
mod tests;
