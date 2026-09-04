//! EXPERIMENTAL (latency bench): streaming synthesis path for the TTS worker.
//!
//! `BUZZ_TTS_STREAMING=1` streams PCM deltas out of Pocket as they are
//! generated instead of waiting for the full first-chunk synthesis.
//! `BUZZ_TTS_EMIT_FRAMES` tunes the delta size in Flow LM frames (80 ms of
//! audio each). Default 12 = the Mimi decoder's native chunk, which keeps
//! streamed audio bit-identical to the batch path; smaller deltas are faster
//! to first audio but diverge (~23 dB SNR vs batch — decoder intra-chunk
//! lookahead).

use super::*;

use crate::huddle::pocket::{PocketTts, VoiceStyle};

/// Read the streaming env overrides once per worker: `Some(emit_frames)`
/// when `BUZZ_TTS_STREAMING=1`, `None` for the production batch path.
pub(super) fn streaming_emit_frames() -> Option<usize> {
    std::env::var("BUZZ_TTS_STREAMING")
        .is_ok_and(|v| v == "1")
        .then(|| {
            std::env::var("BUZZ_TTS_EMIT_FRAMES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(12)
        })
}

/// Playback context threaded through one streamed chunk.
pub(super) struct StreamingPlayback<'a> {
    pub(super) playback: &'a PlaybackCoordinator,
    pub(super) route_id: u64,
}

/// Synthesize one text chunk through `synth_chunk_streaming`, appending PCM
/// deltas to the player as they are generated so first audio lands after
/// ~`emit_frames` of generation instead of after the whole first-chunk
/// synthesis. Delta boundary decoration reuses `PlaybackChunkAudio`: lead-in
/// on the first delta, fade-out only on the final one.
///
/// `signals` = (cancel, voice_cancel, shutdown); `append_audio` returns
/// `false` to abort (its own cancellation checks and logging apply). Returns
/// `None` on success or `Some(outcome)` — the worker's `synthesis_outcome`
/// label — when the chunk was cancelled or failed.
pub(super) fn synthesize_streaming(
    engine: &PocketTts,
    text: &str,
    style: &VoiceStyle,
    emit_frames: usize,
    signals: (&AtomicBool, &AtomicBool, &AtomicBool),
    playback: StreamingPlayback<'_>,
    append_audio: &mut dyn FnMut(PreparedModelAudio) -> bool,
) -> Option<&'static str> {
    drive_streaming(
        |on_audio| engine.synth_chunk_streaming(text, style, emit_frames, on_audio),
        signals,
        playback,
        append_audio,
    )
}

fn drive_streaming(
    synthesize: impl FnOnce(&mut dyn FnMut(Vec<f32>) -> bool) -> Result<bool, String>,
    signals: (&AtomicBool, &AtomicBool, &AtomicBool),
    playback: StreamingPlayback<'_>,
    append_audio: &mut dyn FnMut(PreparedModelAudio) -> bool,
) -> Option<&'static str> {
    let (cancel, voice_cancel, shutdown) = signals;
    let StreamingPlayback { playback, route_id } = playback;
    let mut playback_audio = PlaybackChunkAudio::new();
    let mut delta_index = 0usize;
    let stream_result = synthesize(&mut |samples| {
        if cancel.load(Ordering::Acquire)
            || voice_cancel.load(Ordering::Acquire)
            || shutdown.load(Ordering::Acquire)
        {
            return false;
        }
        let chunk_index = delta_index;
        delta_index += 1;
        if let Some(prepared) =
            playback.prepare_audio(|empty| playback_audio.push(samples, chunk_index, empty))
        {
            if !append_audio(prepared) {
                return false;
            }
        }
        true
    });
    match stream_result {
        Ok(true) => {
            if let Some(prepared) = playback.prepare_audio(|empty| playback_audio.finish(empty)) {
                if !append_audio(prepared) {
                    return Some("cancelled");
                }
            }
            None
        }
        Ok(false) => {
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=cancelled reason=stream_callback route_id={route_id}"
            );
            Some("cancelled")
        }
        Err(_) => {
            // Preserve earlier queued speech, but finish the retained tail so
            // a partial phrase ends with the same fade as a completed stream.
            if let Some(prepared) = playback_audio.finish(first_append, player.empty()) {
                if !append_audio(prepared) {
                    *first_append = true;
                    return Some("cancelled");
                }
            }
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=failed reason=inference route_id={route_id}"
            );
            Some("failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_failure_fades_the_retained_final_block() {
        let cancel = AtomicBool::new(false);
        let voice_cancel = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);
        let mut first_append = true;
        let (player, _output) = rodio::Player::new();
        let mut appended = Vec::new();

        let outcome = drive_streaming(
            |on_audio| {
                assert!(on_audio(vec![0.4; 16]));
                assert!(on_audio(vec![0.5; 16]));
                Err("inference failed after output".into())
            },
            (&cancel, &voice_cancel, &shutdown),
            StreamingPlayback {
                player: &player,
                first_append: &mut first_append,
                route_id: 7,
            },
            &mut |prepared| {
                appended.push(prepared);
                true
            },
        );

        assert_eq!(outcome, Some("failed"));
        assert_eq!(appended.len(), 2);
        assert!(appended[0].buffer.ends_with(&[0.4; 16]));
        let final_samples = &appended[1].buffer[appended[1].buffer.len() - 16..];
        assert_eq!(final_samples.first(), Some(&0.5));
        assert_eq!(final_samples.last(), Some(&0.0));
        assert!(!first_append, "the partial phrase remains queued");
    }

    #[test]
    fn inference_failure_before_output_preserves_existing_playback() {
        let cancel = AtomicBool::new(false);
        let voice_cancel = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);
        let mut first_append = false;
        let (player, _output) = rodio::Player::new();

        let outcome = drive_streaming(
            |_| Err("inference failed before output".into()),
            (&cancel, &voice_cancel, &shutdown),
            StreamingPlayback {
                player: &player,
                first_append: &mut first_append,
                route_id: 7,
            },
            &mut |_| panic!("no audio should be appended"),
        );

        assert_eq!(outcome, Some("failed"));
        assert!(!first_append, "the existing utterance still owns playback");
    }

    #[test]
    fn each_stop_signal_prevents_later_stream_output() {
        for signal_index in 0..3 {
            let signals = [
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ];
            let mut first_append = true;
            let (player, _output) = rodio::Player::new();
            let mut appended = 0;

            let outcome = drive_streaming(
                |on_audio| {
                    assert!(on_audio(vec![0.4; 16]));
                    signals[signal_index].store(true, Ordering::Release);
                    assert!(!on_audio(vec![0.5; 16]));
                    Ok(false)
                },
                (&signals[0], &signals[1], &signals[2]),
                StreamingPlayback {
                    player: &player,
                    first_append: &mut first_append,
                    route_id: 7,
                },
                &mut |_| {
                    appended += 1;
                    true
                },
            );

            assert_eq!(outcome, Some("cancelled"));
            assert_eq!(appended, 0, "no retained block may escape after Stop");
            assert!(first_append);
        }
    }

    #[test]
    fn rejected_append_never_flushes_the_retained_final_block() {
        let cancel = AtomicBool::new(false);
        let voice_cancel = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);
        let mut first_append = true;
        let (player, _output) = rodio::Player::new();
        let mut append_attempts = 0;

        let outcome = drive_streaming(
            |on_audio| {
                assert!(on_audio(vec![0.4; 16]));
                assert!(!on_audio(vec![0.5; 16]));
                Ok(false)
            },
            (&cancel, &voice_cancel, &shutdown),
            StreamingPlayback {
                player: &player,
                first_append: &mut first_append,
                route_id: 7,
            },
            &mut |_| {
                append_attempts += 1;
                false
            },
        );

        assert_eq!(outcome, Some("cancelled"));
        assert_eq!(append_attempts, 1);
        assert!(first_append);
    }
}
