use super::*;

pub(super) struct TtsMonitorState {
    pub(super) player: Arc<rodio::Player>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) voice_cancel: Arc<AtomicBool>,
    pub(super) tts_active: Arc<AtomicBool>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) player_ops: Arc<Mutex<()>>,
    pub(super) activity_frames: Arc<Mutex<VecDeque<TtsSpeakerActivityFrame>>>,
    pub(super) active_speaker: ActiveSpeaker,
    pub(super) speaker_cancel: SpeakerCancellation,
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
                    let _ops = lock_player_ops(&state.player_ops);
                    if state.cancel.load(Ordering::Acquire)
                        || state.voice_cancel.load(Ordering::Acquire)
                    {
                        state.player.clear();
                        state.player.play();
                        state
                            .active_speaker
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .take();
                        state.tts_active.store(false, Ordering::Release);
                    }
                }
                silence_cancelled_speaker(
                    &state.speaker_cancel,
                    &state.active_speaker,
                    &state.player,
                    &state.player_ops,
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
    player: &rodio::Player,
    player_ops: &Mutex<()>,
    tts_active: &AtomicBool,
) {
    let Some(cancelled) = cancellation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    else {
        return;
    };
    let _ops = lock_player_ops(player_ops);
    if take_cancelled_active_speaker(&cancelled, active_speaker) {
        player.clear();
        player.play();
        tts_active.store(false, Ordering::Release);
    }
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
    player: Option<(&rodio::Player, &Mutex<()>)>,
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
    if let Some((player, player_ops)) = player {
        let _ops = lock_player_ops(player_ops);
        if take_cancelled_active_speaker(&cancelled, active_speaker) {
            player.clear();
            player.play();
            tts_active.store(false, Ordering::Release);
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

    #[test]
    fn stale_targeted_cancel_does_not_release_the_next_speaker() {
        let active_speaker = Arc::new(Mutex::new(Some("bob".to_string())));

        assert!(!take_cancelled_active_speaker("alice", &active_speaker));
        assert_eq!(
            active_speaker.lock().expect("active speaker").as_deref(),
            Some("bob")
        );
    }
}
