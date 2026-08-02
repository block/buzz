use super::FADE_OUT_SAMPLES;

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
