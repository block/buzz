//! Pocket callback-to-playback PCM assembly.

use super::{
    apply_fade_out, clamp_to_full_scale, CHUNK_LEAD_IN_SAMPLES, FADE_OUT_SAMPLES, SAMPLE_RATE,
};

/// Number of samples retained until the next Pocket callback.
///
/// Retaining a short suffix lets us determine which decoder block is final
/// and apply the utterance fade exactly once without delaying the preceding
/// block.
pub(super) const STREAM_TAIL_SAMPLES: usize = FADE_OUT_SAMPLES * 2;

#[derive(Debug, Default)]
pub(super) struct CumulativePcmDelta {
    previous_len: usize,
}

impl CumulativePcmDelta {
    /// Return only PCM that was not present in the preceding cumulative batch.
    ///
    /// Pocket may repeat an equal-length cumulative buffer while advancing to
    /// another model-safe text chunk. A shorter buffer violates the callback
    /// contract and must stop playback rather than duplicate or reorder PCM.
    pub(super) fn next<'a>(&mut self, cumulative: &'a [f32]) -> Result<Option<&'a [f32]>, String> {
        if cumulative.len() < self.previous_len {
            return Err(format!(
                "Pocket TTS cumulative callback decreased from {} to {} samples",
                self.previous_len,
                cumulative.len()
            ));
        }
        if cumulative.len() == self.previous_len {
            return Ok(None);
        }

        let delta = &cumulative[self.previous_len..];
        self.previous_len = cumulative.len();
        Ok(Some(delta))
    }
}

#[derive(Debug, Default)]
pub(super) struct PocketStreamAssembler {
    pending: Vec<f32>,
    callback_samples: usize,
    pub(super) callback_count: usize,
    pub(super) queued_samples: usize,
}

impl PocketStreamAssembler {
    /// Copy and enqueue one independent decoder-block delta.
    ///
    /// `enqueue` must only queue/copy the buffer; playback itself is expected
    /// to happen asynchronously.
    pub(super) fn push<F>(
        &mut self,
        samples: &[f32],
        playback_idle: bool,
        mut enqueue: F,
    ) -> Result<(), String>
    where
        F: FnMut(Vec<f32>) -> Result<(), String>,
    {
        if samples.is_empty() {
            return Ok(());
        }

        self.callback_count += 1;
        self.callback_samples += samples.len();
        self.pending.extend_from_slice(samples);

        let emit_end = self.pending.len().saturating_sub(STREAM_TAIL_SAMPLES);
        if emit_end == 0 {
            return Ok(());
        }

        let emit_start = if self.queued_samples == 0 {
            match leading_audio_start(&self.pending[..emit_end]) {
                Some(start) => start,
                // Do not mark silent prefix PCM as playback. Keep it until a
                // later callback contains a usable onset.
                None => return Ok(()),
            }
        } else {
            0
        };

        let needs_lead_in = self.queued_samples == 0 || playback_idle;
        let mut buffer = Vec::with_capacity(
            usize::from(needs_lead_in) * CHUNK_LEAD_IN_SAMPLES
                + emit_end.saturating_sub(emit_start),
        );
        if needs_lead_in {
            buffer.extend(std::iter::repeat_n(0.0_f32, CHUNK_LEAD_IN_SAMPLES));
        }
        buffer.extend(
            self.pending[emit_start..emit_end]
                .iter()
                .map(|sample| sample.clamp(-1.0, 1.0)),
        );
        enqueue(buffer)?;
        self.queued_samples += emit_end - emit_start;
        self.pending.drain(..emit_end);
        Ok(())
    }

    /// Queue the retained final suffix, or the complete waveform when the
    /// synthesis backend produced no progress callbacks.
    pub(super) fn finish<F>(
        &mut self,
        complete_samples: &[f32],
        silence_buf_len: usize,
        playback_idle: bool,
        mut enqueue: F,
    ) -> Result<(), String>
    where
        F: FnMut(Vec<f32>) -> Result<(), String>,
    {
        if self.callback_count == 0 {
            self.pending.extend_from_slice(complete_samples);
            self.callback_samples = complete_samples.len();
        } else if self.callback_samples != complete_samples.len() {
            return Err(format!(
                "Pocket TTS callback contract mismatch: callbacks yielded {} samples, final audio contained {}",
                self.callback_samples,
                complete_samples.len()
            ));
        }

        let start = if self.queued_samples == 0 {
            leading_audio_start(&self.pending).unwrap_or(0)
        } else {
            0
        };
        let mut audio = clamp_to_full_scale(self.pending[start..].to_vec());
        apply_fade_out(&mut audio);

        if !audio.is_empty() {
            let trailing_silence_len = silence_buf_len.saturating_sub(CHUNK_LEAD_IN_SAMPLES);
            let needs_lead_in = self.queued_samples == 0 || playback_idle;
            let mut buffer = Vec::with_capacity(
                usize::from(needs_lead_in) * CHUNK_LEAD_IN_SAMPLES
                    + audio.len()
                    + trailing_silence_len,
            );
            if needs_lead_in {
                buffer.extend(std::iter::repeat_n(0.0_f32, CHUNK_LEAD_IN_SAMPLES));
            }
            buffer.extend(audio);
            buffer.extend(std::iter::repeat_n(0.0_f32, trailing_silence_len));
            enqueue(buffer)?;
            self.queued_samples += self.pending.len().saturating_sub(start);
        }
        self.pending.clear();

        Ok(())
    }
}

/// Find the first sustained audio in a Pocket progress batch.
fn leading_audio_start(samples: &[f32]) -> Option<usize> {
    const WINDOW: usize = SAMPLE_RATE as usize / 100;
    const RETAIN: usize = SAMPLE_RATE as usize / 20;
    const THRESHOLD: f32 = 0.002;

    let threshold_squared = THRESHOLD * THRESHOLD;
    let mut energy = 0.0_f32;

    for (index, sample) in samples.iter().enumerate() {
        energy += sample * sample;
        if index >= WINDOW {
            let expired = samples[index - WINDOW];
            energy -= expired * expired;
        }
        if index + 1 >= WINDOW && energy / WINDOW as f32 >= threshold_squared {
            return Some((index + 1 - WINDOW).saturating_sub(RETAIN));
        }
    }

    None
}
