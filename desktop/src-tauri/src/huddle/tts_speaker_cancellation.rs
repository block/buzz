use super::*;

pub(super) struct TtsMonitorState {
    pub(super) playback: Arc<PlaybackCoordinator>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) voice_cancel: Arc<AtomicBool>,
    pub(super) tts_active: Arc<AtomicBool>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) activity_frames: Arc<Mutex<VecDeque<TtsSpeakerActivityFrame>>>,
    pub(super) active_speaker: ActiveSpeaker,
    pub(super) speaker_cancel: SpeakerCancellation,
    pub(super) broadcasters: TtsBroadcasters,
    pub(super) activity_app: Option<tauri::AppHandle>,
}

pub(super) fn spawn_tts_monitor(state: TtsMonitorState) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("tts-barge-in-monitor".into())
        .spawn(move || {
            let mut last_activity_pubkey: Option<String> = None;
            let mut next_activity_tick = Instant::now();
            while !state.stop.load(Ordering::Acquire) {
                if state.cancel.load(Ordering::Acquire)
                    || state.voice_cancel.load(Ordering::Acquire)
                {
                    state.playback.cancel_if_live(
                        || {
                            state.cancel.load(Ordering::Acquire)
                                || state.voice_cancel.load(Ordering::Acquire)
                        },
                        // Publish the release inside the replacement so an
                        // append winning the lock handoff cannot have its own
                        // activity publication overwritten by this `false`.
                        || {
                            state.broadcasters.cancel_all();
                            state
                                .active_speaker
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .take();
                            state.tts_active.store(false, Ordering::Release);
                        },
                    );
                }
                silence_cancelled_speaker(
                    &state.speaker_cancel,
                    &state.active_speaker,
                    &state.playback,
                    &state.tts_active,
                );
                if let Some(ref app) = state.activity_app {
                    if state.tts_active.load(Ordering::Acquire) {
                        let now = Instant::now();
                        if now >= next_activity_tick {
                            let frame = state
                                .activity_frames
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .pop_front();
                            if let Some(frame) = frame {
                                use tauri::Emitter;
                                let _ = app.emit(
                                    "huddle-tts-speaker-level",
                                    TtsSpeakerActivityPayload {
                                        pubkey: Some(frame.pubkey.clone()),
                                        level: frame.level,
                                    },
                                );
                                last_activity_pubkey = Some(frame.pubkey);
                            }
                            next_activity_tick = now + SPEAKER_ACTIVITY_TICK;
                        }
                    } else {
                        let had_activity = last_activity_pubkey.take().is_some();
                        state
                            .activity_frames
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .clear();
                        if had_activity {
                            use tauri::Emitter;
                            let _ = app.emit(
                                "huddle-tts-speaker-level",
                                TtsSpeakerActivityPayload {
                                    pubkey: None,
                                    level: 0.0,
                                },
                            );
                        }
                        next_activity_tick = Instant::now();
                    }
                }
                thread::sleep(MONITOR_TICK);
            }
        })
}

pub(super) fn silence_cancelled_speaker(
    cancellation: &SpeakerCancellation,
    active_speaker: &ActiveSpeaker,
    playback: &PlaybackCoordinator,
    tts_active: &AtomicBool,
) {
    let Some(cancelled) = cancellation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    else {
        return;
    };
    playback.cancel_if_live(
        || take_cancelled_active_speaker(&cancelled, active_speaker),
        || tts_active.store(false, Ordering::Release),
    );
}

fn take_cancelled_active_speaker(cancelled: &str, active_speaker: &ActiveSpeaker) -> bool {
    let mut active = active_speaker
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !active
        .as_deref()
        .is_some_and(|speaker| speaker.eq_ignore_ascii_case(cancelled))
    {
        return false;
    }
    active.take();
    true
}

pub(super) fn consume_speaker_cancel(
    cancellation: &SpeakerCancellation,
    active_speaker: &ActiveSpeaker,
    generations: &SpeakerGenerations,
    tts_active: &AtomicBool,
    text_state: CancelTextState<'_>,
    playback: Option<&PlaybackCoordinator>,
) -> bool {
    let Some(cancelled) = cancellation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    else {
        return false;
    };
    let (text_rx, deferred_text, current_text) = text_state;
    retain_current_speaker_text(generations, deferred_text, current_text, text_rx);
    let mut cleared_player = false;
    if let Some(playback) = playback {
        if playback.cancel_if_live(
            || take_cancelled_active_speaker(&cancelled, active_speaker),
            || tts_active.store(false, Ordering::Release),
        ) {
            cleared_player = true;
        }
    }
    // The monitor may already have cleared the cancelled speaker while the
    // worker was blocked. If another speaker has since claimed the player,
    // preserve that speaker's activity flag and lead-in state.
    cleared_player
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Barrier;

    use rodio::buffer::SamplesBuffer;

    /// Headless coordinator: a real mixer with its source held unpulled, so
    /// the queue never drains and no output device is opened.
    fn coordinator() -> (Arc<PlaybackCoordinator>, rodio::mixer::MixerSource) {
        let channels = std::num::NonZero::new(1).expect("nonzero channels");
        let rate = std::num::NonZero::new(24_000).expect("nonzero rate");
        let (mixer, source) = rodio::mixer::mixer(channels, rate);
        (Arc::new(PlaybackCoordinator::new(&mixer)), source)
    }

    /// Append as `speaker` the way the worker does: activity is published
    /// inside the append transition.
    fn speak(playback: &PlaybackCoordinator, tts_active: &AtomicBool) {
        let channels = std::num::NonZero::new(1).expect("nonzero channels");
        let rate = std::num::NonZero::new(24_000).expect("nonzero rate");
        assert!(playback.append_if(
            SamplesBuffer::new(channels, rate, vec![0.25; 24_000]),
            |_| true,
            || tts_active.store(true, Ordering::Release),
        ));
    }

    #[test]
    fn stale_targeted_cancel_does_not_release_the_next_speaker() {
        let active_speaker = Arc::new(Mutex::new(Some("bob".to_string())));

        assert!(!take_cancelled_active_speaker("alice", &active_speaker));
        assert_eq!(
            active_speaker.lock().expect("active speaker").as_deref(),
            Some("bob")
        );
    }

    /// The wedge Mari found: the monitor silences the cancelled speaker while
    /// the worker is mid-append. The monitor takes `active_speaker`, so the
    /// worker's later `consume_speaker_cancel` fails authorization and never
    /// clears `tts_active` — if the worker's `true` could land after the
    /// monitor's `false`, mic gating stays active with nothing playing.
    #[test]
    fn a_targeted_cancel_racing_an_append_leaves_the_mic_gate_released() {
        for _ in 0..64 {
            let (playback, _unpulled_source) = coordinator();
            let tts_active = Arc::new(AtomicBool::new(false));
            let active_speaker: ActiveSpeaker = Arc::new(Mutex::new(None));
            let speaker_cancel: SpeakerCancellation = Arc::new(Mutex::new(None));

            speak(&playback, &tts_active);
            active_speaker
                .lock()
                .expect("active speaker")
                .replace("alice".to_string());
            speaker_cancel
                .lock()
                .expect("speaker cancel")
                .replace("alice".to_string());

            let barrier = Arc::new(Barrier::new(2));
            let monitor = {
                let playback = Arc::clone(&playback);
                let tts_active = Arc::clone(&tts_active);
                let active_speaker = Arc::clone(&active_speaker);
                let speaker_cancel = Arc::clone(&speaker_cancel);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    silence_cancelled_speaker(
                        &speaker_cancel,
                        &active_speaker,
                        &playback,
                        &tts_active,
                    );
                })
            };
            let worker = {
                let playback = Arc::clone(&playback);
                let tts_active = Arc::clone(&tts_active);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    // The worker appends the next chunk of the utterance the
                    // monitor is cancelling.
                    playback.append_if(
                        SamplesBuffer::new(
                            std::num::NonZero::new(1).expect("nonzero channels"),
                            std::num::NonZero::new(24_000).expect("nonzero rate"),
                            vec![0.25; 24_000],
                        ),
                        |_| true,
                        || tts_active.store(true, Ordering::Release),
                    )
                })
            };

            let appended = worker.join().expect("worker");
            monitor.join().expect("monitor");

            // Whichever order the two took the coordinator, the surviving
            // activity flag must describe the surviving player.
            assert_eq!(
                tts_active.load(Ordering::Acquire),
                !playback.empty(),
                "the mic gate must agree with the player that survived \
                 (appended={appended})"
            );
        }
    }

    /// The worker arm of the same race: the monitor already took the speaker,
    /// so `consume_speaker_cancel` is not authorized to clear anything. It
    /// must not report a clear it did not perform, and it must not disturb the
    /// activity flag the monitor already published.
    #[test]
    fn consuming_a_cancel_the_monitor_already_handled_preserves_the_released_gate() {
        let (playback, _unpulled_source) = coordinator();
        let tts_active = Arc::new(AtomicBool::new(false));
        let active_speaker: ActiveSpeaker = Arc::new(Mutex::new(None));
        let speaker_cancel: SpeakerCancellation = Arc::new(Mutex::new(None));
        let generations: SpeakerGenerations = Arc::new(Mutex::new(HashMap::new()));
        let (_text_tx, text_rx) = mpsc::channel::<QueuedText>();
        let mut deferred_text = VecDeque::new();
        let mut current_text = None;

        speak(&playback, &tts_active);
        active_speaker
            .lock()
            .expect("active speaker")
            .replace("alice".to_string());
        speaker_cancel
            .lock()
            .expect("speaker cancel")
            .replace("alice".to_string());

        silence_cancelled_speaker(&speaker_cancel, &active_speaker, &playback, &tts_active);
        assert!(
            !tts_active.load(Ordering::Acquire),
            "the monitor releases the mic gate it cancelled"
        );

        let cleared = consume_speaker_cancel(
            &speaker_cancel,
            &active_speaker,
            &generations,
            &tts_active,
            (&text_rx, &mut deferred_text, &mut current_text),
            Some(&playback),
        );

        assert!(
            !cleared,
            "the worker must not claim a clear the monitor already performed"
        );
        assert!(
            !tts_active.load(Ordering::Acquire),
            "the released mic gate must survive the worker's pass"
        );
        assert!(playback.empty(), "the cancelled utterance stays silenced");
    }
}
