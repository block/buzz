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
    let (cancel, voice_cancel, shutdown) = signals;
    let StreamingPlayback { playback, route_id } = playback;
    let mut playback_audio = PlaybackChunkAudio::new();
    let mut delta_index = 0usize;
    let stream_result = engine.synth_chunk_streaming(text, style, emit_frames, &mut |samples| {
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
            eprintln!(
                "buzz-desktop: tts stage=synthesis status=failed reason=inference route_id={route_id}"
            );
            Some("failed")
        }
    }
}
