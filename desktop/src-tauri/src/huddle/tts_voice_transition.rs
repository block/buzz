use std::{
    collections::{HashMap, VecDeque},
    fmt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
};

use crate::huddle::pocket::{load_voice_style, VoiceStyle, DEFAULT_VOICE, VOICE_FILE_EXT};

#[derive(Debug)]
pub(super) struct PendingVoiceChange {
    pub(super) generation: u64,
    acknowledged: tokio::sync::oneshot::Sender<()>,
}

pub(super) type VoiceChangeAck = Arc<Mutex<Option<PendingVoiceChange>>>;
pub(super) type WorkerVoiceState = (Arc<Mutex<String>>, Arc<AtomicU64>, VoiceChangeAck);
pub(super) type WorkerCancelSignals = (Arc<AtomicBool>, Arc<AtomicBool>);
pub(super) type SpeakerGenerations = Arc<Mutex<HashMap<String, u64>>>;
pub(super) type ActiveSpeaker = Arc<Mutex<Option<String>>>;
pub(super) type SpeakerCancellation = Arc<Mutex<Option<String>>>;
pub(super) type CancelTextState<'a> = (
    &'a mpsc::Receiver<QueuedText>,
    &'a mut VecDeque<QueuedText>,
    &'a mut Option<QueuedText>,
);
pub(super) type CancelSignals<'a> = (&'a AtomicBool, &'a AtomicBool);

#[derive(Clone)]
pub(super) struct PlaybackProbe {
    player: Arc<Mutex<Option<Arc<rodio::Player>>>>,
    pub(super) player_ops: Arc<Mutex<()>>,
    synthesis_in_flight: Arc<AtomicBool>,
}

pub(super) struct SynthesisFlightGuard {
    playback_probe: PlaybackProbe,
}

impl Drop for SynthesisFlightGuard {
    fn drop(&mut self) {
        self.playback_probe.set_synthesis_in_flight(false);
    }
}

impl PlaybackProbe {
    pub(super) fn new() -> Self {
        Self {
            player: Arc::new(Mutex::new(None)),
            player_ops: Arc::new(Mutex::new(())),
            synthesis_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn install(&self, player: Arc<rodio::Player>) {
        self.player
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(player);
    }

    pub(super) fn set_synthesis_in_flight(&self, in_flight: bool) {
        let _ops = super::lock_player_ops(&self.player_ops);
        self.synthesis_in_flight.store(in_flight, Ordering::Release);
    }

    pub(super) fn begin_synthesis(&self) -> SynthesisFlightGuard {
        self.set_synthesis_in_flight(true);
        SynthesisFlightGuard {
            playback_probe: self.clone(),
        }
    }

    fn player(&self) -> Option<Arc<rodio::Player>> {
        self.player
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl fmt::Debug for PlaybackProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackProbe")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(super) struct QueuedText {
    pub(super) generation: u64,
    pub(super) route_id: u64,
    pub(super) speaker_pubkey: Option<String>,
    pub(super) speaker_generation: u64,
    pub(super) voice_reference: Option<String>,
    pub(super) text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct TtsTextSender {
    pub(super) text_tx: SyncSender<QueuedText>,
    pub(super) generation: u64,
    pub(super) speaker_generations: SpeakerGenerations,
}

impl TtsTextSender {
    pub(crate) fn send(
        &self,
        route_id: u64,
        speaker_pubkey: String,
        speaker_generation: u64,
        voice_reference: String,
        text: String,
    ) -> Result<(), String> {
        self.text_tx
            .send(QueuedText {
                generation: self.generation,
                route_id,
                speaker_pubkey: Some(speaker_pubkey),
                speaker_generation,
                voice_reference: Some(voice_reference),
                text,
            })
            .map_err(|error| error.to_string())
    }

    pub(crate) fn speaker_generation(&self, speaker_pubkey: &str) -> u64 {
        current_speaker_generation(&self.speaker_generations, speaker_pubkey)
    }
}

pub(super) fn current_speaker_generation(
    generations: &SpeakerGenerations,
    speaker_pubkey: &str,
) -> u64 {
    generations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&speaker_pubkey.to_ascii_lowercase())
        .copied()
        .unwrap_or(0)
}

pub(super) fn advance_speaker_generation(
    generations: &SpeakerGenerations,
    speaker_pubkey: &str,
) -> u64 {
    let mut generations = generations
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let generation = generations
        .entry(speaker_pubkey.to_ascii_lowercase())
        .or_default();
    *generation = generation.saturating_add(1);
    *generation
}

pub(super) fn queued_speaker_is_current(
    generations: &SpeakerGenerations,
    queued: &QueuedText,
) -> bool {
    queued
        .speaker_pubkey
        .as_deref()
        .is_none_or(|speaker_pubkey| {
            current_speaker_generation(generations, speaker_pubkey) == queued.speaker_generation
        })
}

pub(super) fn request_speaker_cancel(
    generations: &SpeakerGenerations,
    active_speaker: &ActiveSpeaker,
    cancellation: &SpeakerCancellation,
    speaker_pubkey: &str,
) {
    advance_speaker_generation(generations, speaker_pubkey);
    let owns_player = active_speaker
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_deref()
        .is_some_and(|active| active.eq_ignore_ascii_case(speaker_pubkey));
    if owns_player {
        cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(speaker_pubkey.to_ascii_lowercase());
    }
}

pub(super) fn request_active_speaker_cancel(
    generations: &SpeakerGenerations,
    active_speaker: &ActiveSpeaker,
    cancellation: &SpeakerCancellation,
    playback_probe: &PlaybackProbe,
    expected_speaker_pubkey: &str,
) -> bool {
    let Some(player) = playback_probe.player() else {
        return false;
    };
    let _ops = super::lock_player_ops(&playback_probe.player_ops);
    let playback_live =
        !player.empty() || playback_probe.synthesis_in_flight.load(Ordering::Acquire);
    request_active_speaker_cancel_while_locked(
        generations,
        active_speaker,
        cancellation,
        playback_live,
        expected_speaker_pubkey,
    )
}

fn request_active_speaker_cancel_while_locked(
    generations: &SpeakerGenerations,
    active_speaker: &ActiveSpeaker,
    cancellation: &SpeakerCancellation,
    playback_live: bool,
    expected_speaker_pubkey: &str,
) -> bool {
    if !playback_live {
        return false;
    }
    let active = active_speaker
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(speaker_pubkey) = active.as_deref() else {
        return false;
    };
    if !speaker_pubkey.eq_ignore_ascii_case(expected_speaker_pubkey) {
        return false;
    }

    // Keep ownership locked until the generation and cancellation request are
    // committed. The drain path takes the same lock, so the request is bound
    // to the utterance the Stop action actually observed.
    let mut cancellation = cancellation
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if cancellation
        .as_deref()
        .is_some_and(|pending| pending.eq_ignore_ascii_case(speaker_pubkey))
    {
        return false;
    }
    advance_speaker_generation(generations, speaker_pubkey);
    cancellation.replace(speaker_pubkey.to_ascii_lowercase());
    true
}

pub(super) fn retain_current_speaker_text(
    generations: &SpeakerGenerations,
    deferred_text: &mut VecDeque<QueuedText>,
    current_text: &mut Option<QueuedText>,
    text_rx: &mpsc::Receiver<QueuedText>,
) {
    deferred_text.retain(|text| queued_speaker_is_current(generations, text));
    if let Some(text) = current_text.take() {
        if queued_speaker_is_current(generations, &text) {
            deferred_text.push_front(text);
        } else {
            log_cancelled_route(text.route_id, "speaker_removed");
        }
    }
    while let Ok(text) = text_rx.try_recv() {
        if queued_speaker_is_current(generations, &text) {
            deferred_text.push_back(text);
        } else {
            log_cancelled_route(text.route_id, "speaker_removed");
        }
    }
}

pub(super) fn has_pending_voice_change(voice_change_ack: &VoiceChangeAck) -> bool {
    voice_change_ack
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some()
}

pub(super) fn begin_voice_change(
    selected_voice: &Mutex<String>,
    voice_generation: &AtomicU64,
    voice_cancel: &AtomicBool,
    voice_change_ack: &VoiceChangeAck,
    voice: &str,
) -> Option<tokio::sync::oneshot::Receiver<()>> {
    let mut pending_ack = voice_change_ack
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut selected = selected_voice
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if selected.as_str() == voice {
        return None;
    }

    let (sender, receiver) = tokio::sync::oneshot::channel();
    voice_cancel.store(true, Ordering::Release);
    let generation = voice_generation.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(superseded) = pending_ack.replace(PendingVoiceChange {
        generation,
        acknowledged: sender,
    }) {
        let _ = superseded.acknowledged.send(());
    }
    *selected = voice.to_string();
    Some(receiver)
}

pub(super) fn acknowledge_voice_change(
    voice_change_ack: &VoiceChangeAck,
    voice_cancel: &AtomicBool,
) {
    let mut pending_ack = voice_change_ack
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if voice_cancel.load(Ordering::Acquire) {
        return;
    }
    if let Some(pending) = pending_ack.take() {
        let _ = pending.acknowledged.send(());
    }
}

pub(super) fn finish_voice_change_ack(voice_change_ack: &VoiceChangeAck) {
    if let Some(pending) = voice_change_ack
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        let _ = pending.acknowledged.send(());
    }
}

pub(super) fn reconcile_selected_voice(
    model_dir: &Path,
    selected_voice: &Mutex<String>,
    voice_name: &mut String,
    style: &mut VoiceStyle,
) -> bool {
    let requested_voice = selected_voice
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if requested_voice == *voice_name {
        return true;
    }

    let requested_path = voice_path(model_dir, &requested_voice);
    match load_voice_style(&requested_path) {
        Ok(requested_style) => {
            *style = requested_style;
            *voice_name = requested_voice;
            true
        }
        Err(_) => {
            eprintln!("buzz-desktop: tts stage=voice_switch status=fallback reason=voice_style");
            let fallback_path = model_dir.join(format!("{DEFAULT_VOICE}.{VOICE_FILE_EXT}"));
            match load_voice_style(&fallback_path) {
                Ok(fallback_style) => {
                    *style = fallback_style;
                    *voice_name = DEFAULT_VOICE.to_string();
                    *selected_voice
                        .lock()
                        .unwrap_or_else(|lock_error| lock_error.into_inner()) =
                        DEFAULT_VOICE.to_string();
                    true
                }
                Err(_) => {
                    eprintln!(
                        "buzz-desktop: tts stage=voice_switch status=failed reason=fallback_voice_style"
                    );
                    false
                }
            }
        }
    }
}

pub(super) fn reconcile_queued_voice(
    model_dir: &Path,
    requested_voice: &str,
    selected_voice: &Mutex<String>,
    voice_name: &mut String,
    style: &mut VoiceStyle,
    style_cache: &mut HashMap<String, VoiceStyle>,
) -> bool {
    if requested_voice == voice_name.as_str() {
        return true;
    }
    if let Some(cached) = style_cache.get(requested_voice) {
        *style = cached.clone();
        *voice_name = requested_voice.to_owned();
        return true;
    }

    match load_voice_style(&voice_path(model_dir, requested_voice)) {
        Ok(requested_style) => {
            style_cache.insert(requested_voice.to_owned(), requested_style.clone());
            *style = requested_style;
            *voice_name = requested_voice.to_owned();
            true
        }
        Err(_) => {
            eprintln!(
                "buzz-desktop: tts stage=agent_voice_switch status=fallback reason=voice_style"
            );
            let ready = reconcile_selected_voice(model_dir, selected_voice, voice_name, style);
            if ready {
                style_cache.insert(voice_name.clone(), style.clone());
            }
            ready
        }
    }
}

pub(super) fn voice_path(model_dir: &Path, voice: &str) -> std::path::PathBuf {
    let path = Path::new(voice);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        model_dir.join(format!("{voice}.{VOICE_FILE_EXT}"))
    }
}

pub(super) fn retain_cancelled_text(
    deferred_text: &mut VecDeque<QueuedText>,
    current_text: &mut Option<QueuedText>,
    text_rx: &mpsc::Receiver<QueuedText>,
    preserve_generation: Option<u64>,
) {
    if let Some(generation) = preserve_generation {
        deferred_text.retain(|text| {
            let preserve = text.generation >= generation;
            if !preserve {
                log_cancelled_route(text.route_id, "voice_switch");
            }
            preserve
        });
        if let Some(text) = current_text.take() {
            if text.generation >= generation {
                deferred_text.push_front(text);
            } else {
                log_cancelled_route(text.route_id, "voice_switch");
            }
        }
        while let Ok(text) = text_rx.try_recv() {
            if text.generation >= generation {
                deferred_text.push_back(text);
            } else {
                log_cancelled_route(text.route_id, "voice_switch");
            }
        }
    } else {
        for text in deferred_text.drain(..) {
            log_cancelled_route(text.route_id, "barge_in");
        }
        if let Some(text) = current_text.take() {
            log_cancelled_route(text.route_id, "barge_in");
        }
        while let Ok(text) = text_rx.try_recv() {
            log_cancelled_route(text.route_id, "barge_in");
        }
    }
}

fn log_cancelled_route(route_id: u64, reason: &str) {
    eprintln!("buzz-desktop: tts stage=queue status=dropped reason={reason} route_id={route_id}");
}

#[cfg(test)]
mod speaker_generation_tests {
    use super::*;

    fn playback_probe(playback_live: bool) -> PlaybackProbe {
        let channels = std::num::NonZero::new(1).expect("non-zero channels");
        let sample_rate = std::num::NonZero::new(24_000).expect("non-zero sample rate");
        let (mixer, _mixer_source) = rodio::mixer::mixer(channels, sample_rate);
        let player = Arc::new(rodio::Player::connect_new(&mixer));
        if playback_live {
            player.append(rodio::buffer::SamplesBuffer::new(
                channels,
                sample_rate,
                vec![0.0; 24_000],
            ));
        }
        let probe = PlaybackProbe::new();
        probe.install(player);
        probe
    }

    fn queued_speech(speaker_pubkey: &str, speaker_generation: u64) -> QueuedText {
        QueuedText {
            generation: 1,
            route_id: 1,
            speaker_pubkey: Some(speaker_pubkey.to_string()),
            speaker_generation,
            voice_reference: Some("pocket:mary".to_string()),
            text: "Hello".to_string(),
        }
    }

    #[test]
    fn removing_a_speaker_invalidates_only_that_speakers_queued_text() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let alice = queued_speech("ALICE", current_speaker_generation(&generations, "alice"));
        let bob = queued_speech("bob", current_speaker_generation(&generations, "bob"));

        advance_speaker_generation(&generations, "alice");

        assert!(!queued_speaker_is_current(&generations, &alice));
        assert!(queued_speaker_is_current(&generations, &bob));

        let rejoined_alice =
            queued_speech("alice", current_speaker_generation(&generations, "alice"));
        assert!(queued_speaker_is_current(&generations, &rejoined_alice));
    }

    #[test]
    fn removing_a_silent_speaker_does_not_cancel_the_active_speaker() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));

        request_speaker_cancel(&generations, &active_speaker, &cancellation, "bob");

        assert!(cancellation.lock().expect("cancellation").is_none());
        assert_eq!(
            active_speaker.lock().expect("active speaker").as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn targeted_cancellation_preserves_other_speakers_queue_entries() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));
        let alice = queued_speech("alice", 0);
        let bob = queued_speech("bob", 0);
        let (_text_tx, text_rx) = mpsc::sync_channel(1);
        let mut deferred = VecDeque::from([alice, bob]);
        let mut current = None;

        request_speaker_cancel(&generations, &active_speaker, &cancellation, "alice");
        retain_current_speaker_text(&generations, &mut deferred, &mut current, &text_rx);

        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].speaker_pubkey.as_deref(), Some("bob"));
    }

    #[test]
    fn stop_request_is_bound_to_the_observed_speaker_generation() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));

        assert!(request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &playback_probe(true),
            "alice",
        ));
        assert_eq!(current_speaker_generation(&generations, "alice"), 1);
        assert_eq!(
            cancellation.lock().expect("cancellation").as_deref(),
            Some("alice")
        );

        active_speaker.lock().expect("active speaker").take();
        cancellation.lock().expect("cancellation").take();
        assert!(!request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &playback_probe(true),
            "alice",
        ));

        let next_utterance =
            queued_speech("alice", current_speaker_generation(&generations, "alice"));
        assert!(queued_speaker_is_current(&generations, &next_utterance));
        assert!(cancellation.lock().expect("cancellation").is_none());
    }

    #[test]
    fn stop_request_does_not_cancel_a_different_active_speaker() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("bob".to_string())));
        let cancellation = Arc::new(Mutex::new(None));

        assert!(!request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &playback_probe(true),
            "alice",
        ));
        assert_eq!(current_speaker_generation(&generations, "alice"), 0);
        assert_eq!(current_speaker_generation(&generations, "bob"), 0);
        assert!(cancellation.lock().expect("cancellation").is_none());
        assert_eq!(
            active_speaker.lock().expect("active speaker").as_deref(),
            Some("bob"),
        );
    }

    #[test]
    fn stop_request_during_empty_synthesis_gap_cancels_in_flight_speech() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));
        let next_chunk = queued_speech("alice", 0);
        let probe = playback_probe(false);
        let _synthesis_flight = probe.begin_synthesis();

        assert!(request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &probe,
            "alice",
        ));

        assert_eq!(current_speaker_generation(&generations, "alice"), 1);
        assert!(!queued_speaker_is_current(&generations, &next_chunk));
        assert_eq!(
            cancellation.lock().expect("cancellation").as_deref(),
            Some("alice"),
        );
    }

    #[test]
    fn repeated_stop_for_same_in_flight_utterance_is_idempotent() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));
        let probe = playback_probe(false);
        let _synthesis_flight = probe.begin_synthesis();

        assert!(request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &probe,
            "alice",
        ));
        let speech_queued_after_first_stop = queued_speech("alice", 1);

        assert!(!request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &probe,
            "alice",
        ));

        assert_eq!(current_speaker_generation(&generations, "alice"), 1);
        assert!(queued_speaker_is_current(
            &generations,
            &speech_queued_after_first_stop,
        ));
        assert_eq!(
            cancellation.lock().expect("cancellation").as_deref(),
            Some("alice"),
        );
    }

    #[test]
    fn stop_request_after_playback_drains_preserves_queued_speech() {
        let generations = Arc::new(Mutex::new(HashMap::new()));
        let active_speaker = Arc::new(Mutex::new(Some("alice".to_string())));
        let cancellation = Arc::new(Mutex::new(None));
        let next_utterance = queued_speech("alice", 0);

        assert!(!request_active_speaker_cancel(
            &generations,
            &active_speaker,
            &cancellation,
            &playback_probe(false),
            "alice",
        ));

        assert_eq!(current_speaker_generation(&generations, "alice"), 0);
        assert!(queued_speaker_is_current(&generations, &next_utterance));
        assert!(cancellation.lock().expect("cancellation").is_none());
        assert_eq!(
            active_speaker.lock().expect("active speaker").as_deref(),
            Some("alice"),
        );
    }
}
