//! Commits synthesized audio to local playback and remote broadcast atomically.

use super::*;

pub(super) struct TtsAppendContext<'a> {
    pub(super) playback: &'a PlaybackCoordinator,
    #[cfg(test)]
    pub(super) human_floor: &'a HumanFloor,
    pub(super) cancel: &'a AtomicBool,
    pub(super) voice_cancel: &'a AtomicBool,
    pub(super) shutdown: &'a AtomicBool,
    pub(super) tts_active: &'a AtomicBool,
    pub(super) speaker_generations: &'a SpeakerGenerations,
    pub(super) active_speaker: &'a ActiveSpeaker,
    pub(super) activity_frames: &'a Mutex<VecDeque<TtsSpeakerActivityFrame>>,
    pub(super) broadcasters: &'a TtsBroadcasters,
    pub(super) channels: NonZero<u16>,
    pub(super) rate: NonZero<u32>,
}

pub(super) fn append_worker_audio(
    context: &TtsAppendContext<'_>,
    prepared: PreparedModelAudio,
    route_id: u64,
    speaker_pubkey: Option<&str>,
    speaker_generation: u64,
    floor_epoch: u64,
    publish_broadcast: impl FnOnce(),
) -> bool {
    // Keep the shared floor in this context so the regression can mutation-check
    // that authorization never moves back inside the coordinator callback.
    #[cfg(test)]
    let _ = context.human_floor;
    let sample_count = prepared.sample_count;
    let chunk_index = prepared.chunk_index;
    let activity = speaker_pubkey.map(|pubkey| {
        build_tts_speaker_activity_frames(&prepared.buffer, pubkey, SAMPLE_RATE as usize)
    });
    let floor_authorization = context.playback.append_if_human_floor_permits(
        rodio::buffer::SamplesBuffer::new(context.channels, context.rate, prepared.buffer),
        floor_epoch,
        |player_empty| {
            if context.cancel.load(Ordering::Acquire)
                || context.voice_cancel.load(Ordering::Acquire)
                || context.shutdown.load(Ordering::Acquire)
            {
                context.broadcasters.cancel_all();
                let reason = if context.shutdown.load(Ordering::Acquire) {
                    "shutdown"
                } else if context.cancel.load(Ordering::Acquire) {
                    "barge_in"
                } else {
                    "voice_switch"
                };
                eprintln!(
                    "buzz-desktop: tts stage=synthesis status=cancelled reason={reason} route_id={route_id}"
                );
                return false;
            }
            if speaker_pubkey.is_some_and(|pubkey| {
                current_speaker_generation(context.speaker_generations, pubkey)
                    != speaker_generation
            }) {
                eprintln!(
                    "buzz-desktop: tts stage=synthesis status=cancelled reason=speaker_removed route_id={route_id}"
                );
                return false;
            }
            if let Some(pubkey) = speaker_pubkey {
                let mut active = context
                    .active_speaker
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if player_empty {
                    active.take();
                }
                if active
                    .as_deref()
                    .is_some_and(|current| !current.eq_ignore_ascii_case(pubkey))
                {
                    return false;
                }
                active.get_or_insert_with(|| pubkey.to_ascii_lowercase());
                context
                    .activity_frames
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .extend(activity.unwrap_or_default());
            }
            true
        },
        // Commit local and published activity under the same playback lock.
        // A concurrent floor onset/cancel therefore cannot invalidate the
        // player and then let this remote packet escape afterward.
        || {
            publish_broadcast();
            context.tts_active.store(true, Ordering::Release);
        },
    );
    if floor_authorization != HumanFloorAuthorization::Permitted {
        return false;
    }
    eprintln!(
        "buzz-desktop: tts stage=player status=append_accepted route_id={route_id} chunk_index={chunk_index} sample_count={sample_count}"
    );
    true
}
