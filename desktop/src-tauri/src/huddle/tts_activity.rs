//! Agent TTS activity envelope shared with the participant film strip.

#[derive(Clone, serde::Serialize)]
pub(super) struct TtsSpeakerActivityPayload {
    pub(super) pubkey: Option<String>,
    pub(super) level: f32,
}

pub(super) struct TtsSpeakerActivityFrame {
    pub(super) pubkey: String,
    pub(super) level: f32,
}

/// Build a 50 ms RMS envelope from the exact audio queued for playback.
/// The UI consumes these frames at the same cadence as remote speaker levels,
/// so an agent uses the normal participant ring rather than a generic pulse.
pub(super) fn build_tts_speaker_activity_frames(
    samples: &[f32],
    pubkey: &str,
    sample_rate: usize,
) -> Vec<TtsSpeakerActivityFrame> {
    let samples_per_frame = (sample_rate / 20).max(1);
    samples
        .chunks(samples_per_frame)
        .map(|frame| {
            let mean_square = frame
                .iter()
                .map(|sample| f64::from(*sample) * f64::from(*sample))
                .sum::<f64>()
                / frame.len().max(1) as f64;
            let rms = mean_square.sqrt() as f32;
            let level = if rms <= 0.000_5 {
                0.0
            } else {
                // Map roughly -60 dB..-12 dB into the same normalized range
                // used by remote Opus speaker levels.
                ((20.0 * rms.log10() + 60.0) / 48.0).clamp(0.12, 1.0)
            };
            TtsSpeakerActivityFrame {
                pubkey: pubkey.to_string(),
                level,
            }
        })
        .collect()
}
