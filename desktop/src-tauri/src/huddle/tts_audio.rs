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
    appended: bool,
}

impl PlaybackChunkAudio {
    pub(super) fn new() -> Self {
        Self {
            pending: None,
            appended: false,
        }
    }

    pub(super) fn push(
        &mut self,
        samples: Vec<f32>,
        chunk_index: usize,
        first_append: &mut bool,
        silence_buf_len: usize,
        playback_idle: bool,
    ) -> Option<PreparedModelAudio> {
        if samples.is_empty() {
            return None;
        }
        let previous = self.pending.replace((samples, chunk_index))?;
        let prepared = prepare_model_audio(
            previous,
            first_append,
            silence_buf_len,
            !self.appended || playback_idle,
            false,
        );
        self.appended = true;
        Some(prepared)
    }

    pub(super) fn finish(
        &mut self,
        first_append: &mut bool,
        silence_buf_len: usize,
        playback_idle: bool,
    ) -> Option<PreparedModelAudio> {
        let pending = self.pending.take()?;
        Some(prepare_model_audio(
            pending,
            first_append,
            silence_buf_len,
            !self.appended || playback_idle,
            true,
        ))
    }
}

fn prepare_model_audio(
    (samples, chunk_index): (Vec<f32>, usize),
    first_append: &mut bool,
    silence_buf_len: usize,
    starts_playback_chunk: bool,
    ends_playback_chunk: bool,
) -> PreparedModelAudio {
    let sample_count = samples.len();
    let mut audio = clamp_to_full_scale(samples);
    if ends_playback_chunk {
        apply_fade_out(&mut audio);
    }
    PreparedModelAudio {
        buffer: build_sentence_append_buffer(
            first_append,
            audio,
            silence_buf_len,
            starts_playback_chunk,
            ends_playback_chunk,
        ),
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
    first_append: &mut bool,
    audio: Vec<f32>,
    silence_buf_len: usize,
    starts_playback_chunk: bool,
    ends_playback_chunk: bool,
) -> Vec<f32> {
    if *first_append {
        *first_append = false;
    }

    let lead_in_len = if starts_playback_chunk {
        SENTENCE_LEAD_IN_SAMPLES
    } else {
        0
    };
    let trailing_silence_len = if ends_playback_chunk {
        silence_buf_len.saturating_sub(SENTENCE_LEAD_IN_SAMPLES)
    } else {
        0
    };
    let mut buffer = Vec::with_capacity(lead_in_len + audio.len() + trailing_silence_len);
    buffer.extend(std::iter::repeat_n(0.0_f32, lead_in_len));
    buffer.extend(audio);
    buffer.extend(std::iter::repeat_n(0.0_f32, trailing_silence_len));
    buffer
}

pub(super) fn group_sentences_into_chunks(sentences: &[String], max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    for (index, sentence) in sentences.iter().enumerate() {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        if index == 0 || chunks.is_empty() {
            chunks.push(sentence.to_string());
            continue;
        }
        let can_merge = chunks.len() > 1
            && chunks
                .last()
                .is_some_and(|chunk| chunk.len() + 1 + sentence.len() <= max_chars);
        if can_merge {
            if let Some(last) = chunks.last_mut() {
                last.push(' ');
                last.push_str(sentence);
            }
        } else {
            chunks.push(sentence.to_string());
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_unit_audio_decorates_only_outer_playback_boundaries() {
        let mut chunk = PlaybackChunkAudio::new();
        let mut first_append = true;
        let silence = SENTENCE_LEAD_IN_SAMPLES + 100;

        assert!(chunk
            .push(vec![0.4; 16], 0, &mut first_append, silence, false)
            .is_none());
        let first = chunk
            .push(vec![0.5; 16], 1, &mut first_append, silence, false)
            .expect("first ready model unit");
        assert_eq!(first.buffer.len(), SENTENCE_LEAD_IN_SAMPLES + 16);
        assert!(first.buffer[..SENTENCE_LEAD_IN_SAMPLES]
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(first.buffer[SENTENCE_LEAD_IN_SAMPLES], 0.4);

        let last = chunk
            .finish(&mut first_append, silence, false)
            .expect("last ready model unit");
        assert_eq!(last.buffer.len(), 16 + 100);
        assert_eq!(last.buffer.last(), Some(&0.0));
    }

    #[test]
    fn empty_edge_units_do_not_steal_lead_in_or_trailing_boundary() {
        let mut chunk = PlaybackChunkAudio::new();
        let mut first_append = true;
        let silence = SENTENCE_LEAD_IN_SAMPLES + 100;

        assert!(chunk
            .push(Vec::new(), 0, &mut first_append, silence, false)
            .is_none());
        assert!(chunk
            .push(vec![0.5; 16], 1, &mut first_append, silence, false)
            .is_none());
        assert!(chunk
            .push(Vec::new(), 2, &mut first_append, silence, false)
            .is_none());

        let only = chunk
            .finish(&mut first_append, silence, false)
            .expect("only audible model unit");
        assert_eq!(only.buffer.len(), SENTENCE_LEAD_IN_SAMPLES + 16 + 100);
        assert!(only.buffer[..SENTENCE_LEAD_IN_SAMPLES]
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(only.buffer.last(), Some(&0.0));
    }

    #[test]
    fn playback_underrun_rearms_the_onset_cushion() {
        let mut chunk = PlaybackChunkAudio::new();
        let mut first_append = true;
        let silence = SENTENCE_LEAD_IN_SAMPLES + 100;

        assert!(chunk
            .push(vec![0.4; 16], 0, &mut first_append, silence, false)
            .is_none());
        let first = chunk
            .push(vec![0.5; 16], 1, &mut first_append, silence, false)
            .expect("first model unit");
        assert_eq!(first.buffer.len(), SENTENCE_LEAD_IN_SAMPLES + 16);

        let after_underrun = chunk
            .push(vec![0.6; 16], 2, &mut first_append, silence, true)
            .expect("model unit after underrun");
        assert_eq!(after_underrun.buffer.len(), SENTENCE_LEAD_IN_SAMPLES + 16);
        assert!(after_underrun.buffer[..SENTENCE_LEAD_IN_SAMPLES]
            .iter()
            .all(|sample| *sample == 0.0));
    }
}
