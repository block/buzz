use super::{FADE_OUT_SAMPLES, SENTENCE_LEAD_IN_SAMPLES};

pub(super) struct PreparedModelAudio {
    pub(super) buffer: Vec<f32>,
    pub(super) sample_count: usize,
    pub(super) chunk_index: usize,
}

/// Holds one synthesized model unit so playback-boundary decoration is based
/// on the first and last unit that actually produced audio.
pub(super) struct PlaybackChunkAudio {
    pending: Option<(Vec<f32>, usize)>,
}

impl PlaybackChunkAudio {
    pub(super) fn new() -> Self {
        Self { pending: None }
    }

    pub(super) fn push(
        &mut self,
        samples: Vec<f32>,
        chunk_index: usize,
        playback_idle: bool,
    ) -> Option<PreparedModelAudio> {
        if samples.is_empty() {
            return None;
        }
        let previous = self.pending.replace((samples, chunk_index))?;
        let prepared = prepare_model_audio(previous, playback_idle, false);
        Some(prepared)
    }

    pub(super) fn finish(&mut self, playback_idle: bool) -> Option<PreparedModelAudio> {
        let pending = self.pending.take()?;
        Some(prepare_model_audio(pending, playback_idle, true))
    }
}

fn prepare_model_audio(
    (samples, chunk_index): (Vec<f32>, usize),
    starts_playback_chunk: bool,
    ends_playback_chunk: bool,
) -> PreparedModelAudio {
    let sample_count = samples.len();
    let mut audio = clamp_to_full_scale(samples);
    if ends_playback_chunk {
        apply_fade_out(&mut audio);
    }
    PreparedModelAudio {
        buffer: build_sentence_append_buffer(audio, starts_playback_chunk),
        sample_count,
        chunk_index,
    }
}

/// Hard-clamp samples to ±1.0 full scale.
pub(super) fn clamp_to_full_scale(samples: Vec<f32>) -> Vec<f32> {
    samples.into_iter().map(|s| s.clamp(-1.0, 1.0)).collect()
}

/// Apply a short linear fade-out to avoid a discontinuity at playback boundaries.
pub(super) fn apply_fade_out(samples: &mut [f32]) {
    let len = samples.len();
    let fade = FADE_OUT_SAMPLES.min(len / 2);
    for i in 0..fade {
        samples[len - 1 - i] *= i as f32 / fade as f32;
    }
}

pub(super) fn build_sentence_append_buffer(
    audio: Vec<f32>,
    starts_playback_chunk: bool,
) -> Vec<f32> {
    let lead_in_len = if starts_playback_chunk {
        SENTENCE_LEAD_IN_SAMPLES
    } else {
        0
    };
    let mut buffer = Vec::with_capacity(lead_in_len + audio.len());
    buffer.extend(std::iter::repeat_n(0.0_f32, lead_in_len));
    buffer.extend(audio);
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_units_are_queued_contiguously_without_injected_silence() {
        let mut chunk = PlaybackChunkAudio::new();

        assert!(chunk.push(vec![0.4; 16], 0, false).is_none());
        let first = chunk
            .push(vec![0.5; 16], 1, false)
            .expect("first ready model unit");
        assert_eq!(first.buffer, vec![0.4; 16]);

        let last = chunk.finish(false).expect("last ready model unit");
        assert_eq!(last.buffer.len(), 16);
        assert_eq!(last.sample_count, 16);
    }

    #[test]
    fn empty_edge_units_do_not_steal_audio_boundaries() {
        let mut chunk = PlaybackChunkAudio::new();

        assert!(chunk.push(Vec::new(), 0, false).is_none());
        assert!(chunk.push(vec![0.5; 16], 1, false).is_none());
        assert!(chunk.push(Vec::new(), 2, false).is_none());

        let only = chunk.finish(false).expect("only audible model unit");
        assert_eq!(only.buffer.len(), 16);
    }

    #[test]
    fn playback_underrun_rearms_the_onset_cushion() {
        let mut chunk = PlaybackChunkAudio::new();

        assert!(chunk.push(vec![0.4; 16], 0, false).is_none());
        let first = chunk
            .push(vec![0.5; 16], 1, false)
            .expect("first ready model unit");
        assert_eq!(first.buffer.len(), 16);

        let after_underrun = chunk
            .push(vec![0.6; 16], 2, true)
            .expect("second ready model unit");
        assert_eq!(after_underrun.buffer.len(), SENTENCE_LEAD_IN_SAMPLES + 16);
        assert!(after_underrun.buffer[..SENTENCE_LEAD_IN_SAMPLES]
            .iter()
            .all(|sample| *sample == 0.0));
    }
}
