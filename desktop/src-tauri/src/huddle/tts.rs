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
    collections::{HashMap, VecDeque},
    num::NonZero,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread,
    time::{Duration, Instant},
};

use super::pocket::{
    load_text_to_speech, load_voice_style, DEFAULT_VOICE, SAMPLE_RATE, VOICE_FILE_EXT,
};
use super::preprocessing::{preprocess_for_tts, split_sentences};

#[path = "tts_voice_transition.rs"]
mod voice_transition;
use voice_transition::*;
#[path = "tts_startup.rs"]
mod startup;
use startup::await_worker_startup;
#[path = "tts_audio.rs"]
mod audio;
use audio::*;
#[path = "tts_activity.rs"]
mod activity;
use activity::*;
#[path = "tts_pipeline_controls.rs"]
mod pipeline_controls;
#[path = "tts_speaker_cancellation.rs"]
mod speaker_cancellation;
use speaker_cancellation::*;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of queued text items.
/// Prevents unbounded accumulation when the agent produces text faster than
/// TTS can play it. Excess items are dropped with a warning.
const TEXT_QUEUE_DEPTH: usize = 8;

/// How long the worker waits before checking the shutdown flag.
const RECV_TIMEOUT: Duration = Duration::from_millis(100);
/// Poll interval of the barge-in monitor thread. Bounds flag-to-silence
/// latency: a cancel is noticed within one tick, and rodio's internal
/// `periodic_access` wrapper stops the in-flight source within a further
/// ~5 ms — so playing audio dies ~15 ms after the flag is set, even while
/// the worker is blocked inside `synth_chunk`.
const MONITOR_TICK: Duration = Duration::from_millis(10);
const SPEAKER_ACTIVITY_TICK: Duration = Duration::from_millis(50);
const AUDIO_PRIME_TIMEOUT: Duration = Duration::from_secs(2);

/// Pocket TTS is a one-step consistency model, not diffusion. Kept for API compat.
const SYNTH_STEPS: usize = 1;

/// Fade-out length in samples (8 ms at 24 kHz ≈ 192 samples).
///
/// Applied only at the *end* of each synthesised sentence to eliminate the
/// click that would otherwise occur when a non-zero waveform terminates
/// abruptly. **No fade-in is applied** — see `apply_fade_out` for why preserving
/// the leading waveform is important.
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
/// This character budget performs only coarse sentence packing. The April
/// engine applies its SentencePiece tokenizer afterward and refines every
/// result at the bundle's exact 50-token boundary.
const MAX_CHUNK_CHARS: usize = 200;

/// Silence inserted between sentences by the TTS pipeline (seconds).
/// Injected as a silent buffer between each synthesized sentence chunk.
const INTER_SENTENCE_SILENCE: f32 = 0.1;

type WorkerControlState = (
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    WorkerCancelSignals,
    SpeakerGenerations,
    ActiveSpeaker,
    SpeakerCancellation,
    PlaybackProbe,
);

// ── Public pipeline handle ────────────────────────────────────────────────────

/// Handle to the running TTS pipeline.
///
/// Not Clone — wrap in `Arc` to share across threads.
#[derive(Debug)]
pub struct TtsPipeline {
    /// Send preprocessed text into the pipeline.
    text_tx: SyncSender<QueuedText>,
    /// `true` while the agent is speaking. Shared with the STT pipeline for gating.
    #[allow(dead_code)]
    pub tts_active: Arc<AtomicBool>,
    /// Signals the worker thread to stop.
    shutdown: Arc<AtomicBool>,
    /// Cancel flag: worker drains the queue and stops current playback.
    /// Kept alive here so the Arc isn't dropped — the worker holds a clone.
    #[allow(dead_code)]
    cancel: Arc<AtomicBool>,
    /// Internal cancellation used only for voice changes. Kept separate so a
    /// concurrent human barge-in always clears every queued message.
    voice_cancel: Arc<AtomicBool>,
    /// Selected manifest voice. The worker reloads only the lightweight style
    /// when this changes; the warmed Pocket engine and audio player stay alive.
    voice: Arc<Mutex<String>>,
    /// Tags messages so a voice change drops only pre-change queue entries.
    voice_generation: Arc<AtomicU64>,
    /// Per-agent generations let removal invalidate that agent's queued and
    /// in-flight text without poisoning speech queued after the agent rejoins.
    speaker_generations: SpeakerGenerations,
    /// Speaker whose audio currently owns the shared player queue.
    active_speaker: ActiveSpeaker,
    /// Targeted cancellation used when an agent leaves the huddle.
    speaker_cancel: SpeakerCancellation,
    /// Shared player handle used to reject Stop clicks after playback drains.
    playback_probe: PlaybackProbe,
    /// Completed after the worker drains pre-change text and installs the new style.
    voice_change_ack: VoiceChangeAck,
    /// Worker thread handle — taken on drop to join cleanly.
    thread: Option<thread::JoinHandle<()>>,
}

impl TtsPipeline {
    /// Spawn the TTS pipeline thread with a manifest-backed voice name.
    ///
    /// `cancel` is shared with STT for barge-in. The same handle survives voice
    /// changes so the warmed Pocket engine is retained.
    pub fn new_with_voice(
        model_dir: PathBuf,
        tts_active: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        voice: &str,
        output_device: Option<String>,
        activity_app: Option<tauri::AppHandle>,
    ) -> Result<Self, String> {
        let (text_tx, text_rx) = mpsc::sync_channel::<QueuedText>(TEXT_QUEUE_DEPTH);
        let shutdown = Arc::new(AtomicBool::new(false));
        // cancel is passed in from HuddleState.tts_cancel — shared with remote
        // participant interruption and the push-to-talk shortcut.

        let shutdown_worker = Arc::clone(&shutdown);
        let cancel_worker = Arc::clone(&cancel);
        let voice_cancel = Arc::new(AtomicBool::new(false));
        let worker_voice_cancel = Arc::clone(&voice_cancel);
        let tts_active_worker = Arc::clone(&tts_active);
        let voice = Arc::new(Mutex::new(voice.to_string()));
        let voice_worker = Arc::clone(&voice);
        let voice_generation = Arc::new(AtomicU64::new(1));
        let worker_voice_generation = Arc::clone(&voice_generation);
        let speaker_generations = Arc::new(Mutex::new(HashMap::new()));
        let worker_speaker_generations = Arc::clone(&speaker_generations);
        let active_speaker = Arc::new(Mutex::new(None));
        let worker_active_speaker = Arc::clone(&active_speaker);
        let speaker_cancel = Arc::new(Mutex::new(None));
        let worker_speaker_cancel = Arc::clone(&speaker_cancel);
        let playback_probe = PlaybackProbe::new();
        let worker_playback_probe = playback_probe.clone();
        let voice_change_ack = Arc::new(Mutex::new(None));
        let worker_voice_change_ack = Arc::clone(&voice_change_ack);
        let model_dir_worker = model_dir.clone();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);

        let handle = thread::Builder::new()
            .name("tts-worker".into())
            .spawn(move || {
                tts_worker(
                    model_dir_worker,
                    (
                        voice_worker,
                        worker_voice_generation,
                        worker_voice_change_ack,
                    ),
                    text_rx,
                    (
                        tts_active_worker,
                        shutdown_worker,
                        (cancel_worker, worker_voice_cancel),
                        worker_speaker_generations,
                        worker_active_speaker,
                        worker_speaker_cancel,
                        worker_playback_probe,
                    ),
                    output_device,
                    activity_app,
                    startup_tx,
                )
            })
            .map_err(|e| format!("failed to spawn tts-worker thread: {e}"))?;
        let handle = await_worker_startup(handle, startup_rx)?;

        Ok(Self {
            text_tx,
            tts_active,
            shutdown,
            cancel,
            voice_cancel,
            voice,
            voice_generation,
            speaker_generations,
            active_speaker,
            speaker_cancel,
            playback_probe,
            voice_change_ack,
            thread: Some(handle),
        })
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
    voice_state: WorkerVoiceState,
    text_rx: mpsc::Receiver<QueuedText>,
    control_state: WorkerControlState,
    output_device: Option<String>,
    activity_app: Option<tauri::AppHandle>,
    startup_tx: mpsc::SyncSender<Result<(), String>>,
) {
    let (selected_voice, voice_generation, voice_change_ack) = voice_state;
    let (
        tts_active,
        shutdown,
        cancel_signals,
        speaker_generations,
        active_speaker,
        speaker_cancel,
        playback_probe,
    ) = control_state;
    let (cancel, voice_cancel) = cancel_signals;
    // ── 1. Initialise TTS engine ──────────────────────────────────────────────
    let model_dir_str = model_dir.to_string_lossy().to_string();

    let engine = match load_text_to_speech(&model_dir_str) {
        Ok(e) => e,
        Err(e) => {
            let error = format!("TTS engine initialization failed: {e}");
            eprintln!("buzz-desktop: tts stage=startup status=failed reason=engine_load");
            let _ = startup_tx.send(Err(error));
            return;
        }
    };

    // ── 2. Load voice style ───────────────────────────────────────────────────
    let requested_voice = selected_voice
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let mut voice_name = DEFAULT_VOICE.to_string();
    let fallback_path = model_dir.join(format!("{DEFAULT_VOICE}.{VOICE_FILE_EXT}"));
    let mut style = match load_voice_style(&fallback_path) {
        Ok(s) => s,
        Err(e) => {
            let error = format!("TTS voice style initialization failed: {e}");
            eprintln!("buzz-desktop: tts stage=startup status=failed reason=fallback_voice_style");
            let _ = startup_tx.send(Err(error));
            return;
        }
    };
    if requested_voice != DEFAULT_VOICE
        && !reconcile_selected_voice(&model_dir, &selected_voice, &mut voice_name, &mut style)
    {
        let _ = startup_tx.send(Err(
            "TTS selected voice and Mary fallback are unavailable".to_string()
        ));
        return;
    }
    let mut style_cache = HashMap::from([(voice_name.clone(), style.clone())]);

    // ── 2b. Warmup inference ─────────────────────────────────────────────────
    // The first ONNX inference on any session is significantly slower than
    // subsequent ones — it can trigger native session initialization, memory
    // pool allocation, and graph-specific caches. Run a short dummy synthesis
    // and discard the output so the first real utterance runs at warm-session speed.
    {
        match engine.synth_chunk("warmup", "en", &style, SYNTH_STEPS) {
            Ok(_) => eprintln!("buzz-desktop: tts stage=warmup status=ready"),
            Err(_) => eprintln!(
                "buzz-desktop: tts stage=warmup status=failed reason=inference first_utterance_may_be_slow=true"
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
            let error = format!("TTS audio output initialization failed: {e}");
            eprintln!("buzz-desktop: tts stage=startup status=failed reason=output_open");
            let _ = startup_tx.send(Err(error));
            return;
        }
    };

    let channels = match NonZero::new(1u16) {
        Some(c) => c,
        None => {
            let _ = startup_tx.send(Err("TTS channel count invariant violated".to_string()));
            return;
        }
    };
    let rate = match NonZero::new(SAMPLE_RATE) {
        Some(r) => r,
        None => {
            let _ = startup_tx.send(Err("TTS sample rate invariant violated".to_string()));
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
    playback_probe.install(Arc::clone(&player));

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
        let deadline = std::time::Instant::now() + AUDIO_PRIME_TIMEOUT;
        while !player.empty() {
            if std::time::Instant::now() >= deadline {
                eprintln!("buzz-desktop: tts stage=startup status=failed reason=output_prime");
                let _ = startup_tx.send(Err(
                    "TTS audio output did not become ready before timeout".to_string(),
                ));
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    if startup_tx.send(Ok(())).is_err() {
        return;
    }
    eprintln!("buzz-desktop: tts stage=startup status=ready");

    let player_ops = Arc::clone(&playback_probe.player_ops);
    let activity_frames = Arc::new(Mutex::new(VecDeque::<TtsSpeakerActivityFrame>::new()));
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = spawn_tts_monitor(TtsMonitorState {
        player: Arc::clone(&player),
        cancel: Arc::clone(&cancel),
        voice_cancel: Arc::clone(&voice_cancel),
        tts_active: Arc::clone(&tts_active),
        stop: Arc::clone(&monitor_stop),
        player_ops: Arc::clone(&player_ops),
        activity_frames: Arc::clone(&activity_frames),
        active_speaker: Arc::clone(&active_speaker),
        speaker_cancel: Arc::clone(&speaker_cancel),
        activity_app,
    });
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
    let mut last_route_id = 0;
    let mut deferred_text = VecDeque::new();
    let append_audio = |prepared: PreparedModelAudio,
                        route_id: u64,
                        speaker_pubkey: Option<&str>,
                        speaker_generation: u64| {
        let _ops = lock_player_ops(&player_ops);
        if cancel.load(Ordering::Acquire)
            || voice_cancel.load(Ordering::Acquire)
            || shutdown.load(Ordering::Acquire)
        {
            let reason = if shutdown.load(Ordering::Acquire) {
                "shutdown"
            } else if cancel.load(Ordering::Acquire) {
                "barge_in"
            } else {
                "voice_switch"
            };
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=cancelled reason={reason} route_id={route_id}"
            );
            return false;
        }
        let speaker_is_current = speaker_pubkey.is_none_or(|pubkey| {
            current_speaker_generation(&speaker_generations, pubkey) == speaker_generation
        });
        if !speaker_is_current {
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=cancelled reason=speaker_removed route_id={route_id}"
            );
            return false;
        }
        if let Some(pubkey) = speaker_pubkey {
            let mut active = active_speaker
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if player.empty() {
                active.take();
            }
            if active
                .as_deref()
                .is_some_and(|current| !current.eq_ignore_ascii_case(pubkey))
            {
                return false;
            }
            active.get_or_insert_with(|| pubkey.to_ascii_lowercase());
        }
        if let Some(pubkey) = speaker_pubkey {
            activity_frames
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend(build_tts_speaker_activity_frames(
                    &prepared.buffer,
                    pubkey,
                    SAMPLE_RATE as usize,
                ));
        }
        player.append(SamplesBuffer::new(channels, rate, prepared.buffer));
        eprintln!(
            "buzz-desktop: tts stage=player status=append_accepted route_id={route_id} chunk_index={} sample_count={}",
            prepared.chunk_index, prepared.sample_count
        );
        // Set this only after append so STT remains open during synthesis.
        tts_active.store(true, Ordering::Release);
        true
    };

    loop {
        let mut no_current_text = None;
        if consume_speaker_cancel(
            &speaker_cancel,
            &active_speaker,
            &speaker_generations,
            &tts_active,
            (&text_rx, &mut deferred_text, &mut no_current_text),
            Some((&player, &player_ops)),
        ) {
            first_append = true;
            continue;
        }
        if handle_cancel_or_shutdown(
            (&cancel, &voice_cancel),
            &shutdown,
            &tts_active,
            (&text_rx, &mut deferred_text, &mut no_current_text),
            &voice_change_ack,
            None,
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

        // A global Settings voice change cancels the old utterance and is
        // acknowledged before receiving subsequent text. Per-agent voice
        // changes are carried by each queue item and never drain other agents.
        if has_pending_voice_change(&voice_change_ack) {
            let voice_ready =
                reconcile_selected_voice(&model_dir, &selected_voice, &mut voice_name, &mut style);
            if voice_ready {
                style_cache.insert(voice_name.clone(), style.clone());
            }
            acknowledge_voice_change(&voice_change_ack, &voice_cancel);
            if !voice_ready {
                continue;
            }
        }

        let mut queued_text = Some(match deferred_text.pop_front() {
            Some(text) => text,
            None => match text_rx.recv_timeout(RECV_TIMEOUT) {
                Ok(text) => text,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Nothing queued. If playback has also finished, the agent
                    // has gone quiet — release the mic gate and reset the
                    // lead-in so the next utterance gets a fresh cushion.
                    if player.empty() && !first_append {
                        tts_active.store(false, Ordering::Release);
                        active_speaker
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .take();
                        eprintln!(
                            "buzz-desktop: tts stage=player status=drained route_id={last_route_id}"
                        );
                        first_append = true;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        });

        // Check cancel again after unblocking — a cancel may have arrived
        // while we were waiting.
        let pending_route_id = queued_text.as_ref().map(|queued| queued.route_id);
        if handle_cancel_or_shutdown(
            (&cancel, &voice_cancel),
            &shutdown,
            &tts_active,
            (&text_rx, &mut deferred_text, &mut queued_text),
            &voice_change_ack,
            pending_route_id,
            Some((&player, &player_ops)),
        ) {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            first_append = true;
            continue;
        }
        let Some(queued_text) = queued_text else {
            continue;
        };
        if !queued_speaker_is_current(&speaker_generations, &queued_text) {
            eprintln!(
                "buzz-desktop: tts stage=queue status=dropped reason=speaker_removed route_id={}",
                queued_text.route_id
            );
            continue;
        }
        if queued_text.generation < voice_generation.load(Ordering::Acquire) {
            eprintln!(
                "buzz-desktop: tts stage=queue status=dropped reason=voice_switch route_id={}",
                queued_text.route_id
            );
            continue;
        }
        if !player.empty()
            && queued_text
                .speaker_pubkey
                .as_deref()
                .is_some_and(|speaker| {
                    active_speaker
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_deref()
                        .is_some_and(|active| !active.eq_ignore_ascii_case(speaker))
                })
        {
            deferred_text.push_front(queued_text);
            thread::sleep(RECV_TIMEOUT);
            continue;
        }
        let requested_voice = queued_text.voice_reference.unwrap_or_else(|| {
            selected_voice
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        });
        let raw_text = queued_text.text;
        let speaker_pubkey = queued_text.speaker_pubkey;
        let speaker_generation = queued_text.speaker_generation;
        let route_id = queued_text.route_id;
        eprintln!("buzz-desktop: tts stage=synthesis status=started route_id={route_id}");

        // If playback already drained while we were waiting for this item,
        // release stale ownership before doing any potentially slow voice or
        // synthesis work. Serialize the drain decision with Stop and append so
        // those paths observe one coherent utterance boundary.
        {
            let _ops = lock_player_ops(&player_ops);
            if player.empty() && !first_append {
                tts_active.store(false, Ordering::Release);
                active_speaker
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                eprintln!("buzz-desktop: tts stage=player status=drained route_id={last_route_id}");
                first_append = true;
            }
        }

        // From this point until the item finishes, an empty player can mean a
        // voice-preparation or synthesis gap rather than a drained utterance.
        // Stop must remain able to invalidate the in-flight speaker generation.
        let _synthesis_flight = playback_probe.begin_synthesis();

        // The selected per-agent voice travels with the queue item, preserving
        // message order while allowing one warmed Pocket engine to alternate
        // between cached reference styles.
        if !reconcile_queued_voice(
            &model_dir,
            &requested_voice,
            &selected_voice,
            &mut voice_name,
            &mut style,
            &mut style_cache,
        ) {
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=failed reason=voice_unavailable route_id={route_id}"
            );
            continue;
        }

        // Preprocess text.
        let text = preprocess_for_tts(&raw_text);
        if text.is_empty() {
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=empty reason=preprocess route_id={route_id}"
            );
            continue;
        }

        // Split into sentences, then group into synthesis chunks: the first
        // sentence stays alone (fast time-to-first-audio), the rest pack
        // greedily up to MAX_CHUNK_CHARS. Playback of each model unit overlaps
        // synthesis of the next one. The Pocket engine applies its exact
        // 50-token split; keeping those units within one playback chunk avoids
        // adding fades and pauses at token-only boundaries.
        let sentences: Vec<String> = split_sentences(&text)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        let chunks = group_sentences_into_chunks(&sentences, MAX_CHUNK_CHARS);
        if chunks.is_empty() {
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=empty reason=no_chunks route_id={route_id}"
            );
            continue;
        }

        let mut synthesis_outcome = "completed";
        let mut appended_audio = false;
        let mut model_unit_index = 0_usize;
        'playback_chunks: for chunk in &chunks {
            let mut no_current_text = None;
            if handle_cancel_or_shutdown(
                (&cancel, &voice_cancel),
                &shutdown,
                &tts_active,
                (&text_rx, &mut deferred_text, &mut no_current_text),
                &voice_change_ack,
                Some(route_id),
                Some((&player, &player_ops)),
            ) {
                first_append = true;
                synthesis_outcome = "cancelled";
                break;
            }

            let text = chunk.trim();
            if text.is_empty() {
                continue;
            }

            let model_chunks = match engine.split_text_into_chunks(text) {
                Ok(model_chunks) => model_chunks,
                Err(_) => {
                    eprintln!(
                        "buzz-desktop: tts stage=synthesis status=failed reason=chunking route_id={route_id}"
                    );
                    synthesis_outcome = "failed";
                    break 'playback_chunks;
                }
            };
            if model_chunks.is_empty() {
                eprintln!(
                    "buzz-desktop: tts stage=synthesis status=empty reason=no_chunks route_id={route_id}"
                );
                continue;
            }
            let mut playback_audio = PlaybackChunkAudio::new();
            for model_chunk in &model_chunks {
                let chunk_index = model_unit_index;
                model_unit_index += 1;
                let mut no_current_text = None;
                if handle_cancel_or_shutdown(
                    (&cancel, &voice_cancel),
                    &shutdown,
                    &tts_active,
                    (&text_rx, &mut deferred_text, &mut no_current_text),
                    &voice_change_ack,
                    Some(route_id),
                    Some((&player, &player_ops)),
                ) {
                    first_append = true;
                    synthesis_outcome = "cancelled";
                    break 'playback_chunks;
                }

                let synthesis = engine.synth_chunk(model_chunk, "en", &style, SYNTH_STEPS);
                if cancel.load(Ordering::Acquire)
                    || voice_cancel.load(Ordering::Acquire)
                    || shutdown.load(Ordering::Acquire)
                {
                    let reason = if shutdown.load(Ordering::Acquire) {
                        "shutdown"
                    } else if cancel.load(Ordering::Acquire) {
                        "barge_in"
                    } else {
                        "voice_switch"
                    };
                    eprintln!(
                        "buzz-desktop: tts stage=synthesis status=cancelled reason={reason} route_id={route_id}"
                    );
                    // The monitor already stopped any queued playback. Discard
                    // synthesis that completed after cancellation so stale audio
                    // never reaches the player, while keeping buzz-voice's
                    // extracted April engine API unchanged.
                    first_append = true;
                    synthesis_outcome = "cancelled";
                    break 'playback_chunks;
                }
                match synthesis {
                    Ok(samples) if !samples.is_empty() => {
                        if let Some(prepared) = playback_audio.push(
                            samples,
                            chunk_index,
                            &mut first_append,
                            silence_buf_len,
                            player.empty(),
                        ) {
                            if !append_audio(
                                prepared,
                                route_id,
                                speaker_pubkey.as_deref(),
                                speaker_generation,
                            ) {
                                first_append = true;
                                synthesis_outcome = "cancelled";
                                break 'playback_chunks;
                            }
                            appended_audio = true;
                            last_route_id = route_id;
                        }
                    }
                    Ok(_) => {
                        eprintln!(
                            "buzz-desktop: tts stage=synthesis status=empty route_id={route_id} chunk_index={chunk_index}"
                        );
                    }
                    Err(_) => {
                        eprintln!(
                            "buzz-desktop: tts stage=synthesis status=failed reason=inference route_id={route_id} chunk_index={chunk_index}"
                        );
                        synthesis_outcome = "failed";
                        break;
                    }
                }
            }
            if let Some(prepared) =
                playback_audio.finish(&mut first_append, silence_buf_len, player.empty())
            {
                if !append_audio(
                    prepared,
                    route_id,
                    speaker_pubkey.as_deref(),
                    speaker_generation,
                ) {
                    first_append = true;
                    synthesis_outcome = "cancelled";
                    break 'playback_chunks;
                }
                appended_audio = true;
                last_route_id = route_id;
            }
            if synthesis_outcome == "failed" {
                break 'playback_chunks;
            }
        }
        if synthesis_outcome == "completed" && appended_audio {
            eprintln!("buzz-desktop: tts stage=synthesis status=completed route_id={route_id}");
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

    finish_voice_change_ack(&voice_change_ack);
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
    cancel_signals: CancelSignals<'_>,
    shutdown: &AtomicBool,
    tts_active: &AtomicBool,
    text_state: CancelTextState<'_>,
    voice_change_ack: &VoiceChangeAck,
    active_route_id: Option<u64>,
    player: Option<(&rodio::Player, &Mutex<()>)>,
) -> bool {
    let (cancel, voice_cancel) = cancel_signals;
    let (text_rx, deferred_text, current_text) = text_state;
    if shutdown.load(Ordering::Acquire) {
        eprintln!(
            "buzz-desktop: tts stage=cancellation reason=shutdown route_id={}",
            active_route_id.unwrap_or(0)
        );
        if let Some((p, ops)) = player {
            let _ops = lock_player_ops(ops);
            p.clear();
        }
        tts_active.store(false, Ordering::Release);
        return true;
    }
    if cancel.load(Ordering::Acquire) || voice_cancel.load(Ordering::Acquire) {
        // Serialize with begin_voice_change so the generation boundary and
        // cancel consumption are observed as one transition.
        let pending_voice_change = voice_change_ack
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Consume at the serialization point. A later barge-in remains true
        // for the next pass instead of being overwritten after queue cleanup.
        let barge_in = cancel.swap(false, Ordering::AcqRel);
        voice_cancel.store(false, Ordering::Release);
        eprintln!(
            "buzz-desktop: tts stage=cancellation reason={} route_id={}",
            if barge_in { "barge_in" } else { "voice_switch" },
            active_route_id.unwrap_or(0)
        );
        let preserve_generation = (!barge_in)
            .then(|| {
                pending_voice_change
                    .as_ref()
                    .map(|pending| pending.generation)
            })
            .flatten();
        retain_cancelled_text(deferred_text, current_text, text_rx, preserve_generation);
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tts_voice_selection_tests.rs"]
mod voice_selection_tests;
