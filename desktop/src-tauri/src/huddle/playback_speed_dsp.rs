//! Pure pitch-preserving playback-speed processing.
//!
//! This module intentionally has no application, persistence, or filesystem
//! dependencies so production playback and diagnostic examples can compile
//! the exact same DSP boundary.

/// Slowest supported generated-speech playback speed.
pub const MIN_PLAYBACK_SPEED: f32 = 0.25;
/// Fastest supported generated-speech playback speed.
pub const MAX_PLAYBACK_SPEED: f32 = 4.0;
/// Default generated-speech playback speed.
pub const DEFAULT_PLAYBACK_SPEED: f32 = 1.0;

pub(super) const UNITY_EPSILON: f32 = 0.000_1;

/// Pitch-preserve one already-buffered Pocket chunk.
///
/// The returned buffer has exactly `input.len() / speed` samples. Signalsmith
/// starts a reset processor `input_latency` samples before the supplied audio
/// and emits another `output_latency` samples of pre-roll. Both components are
/// removed after draining, so the returned chunk retains its beginning and
/// tail without buffering any later Pocket chunk.
pub fn process_complete_chunk(
    input: &[f32],
    speed: f32,
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    validate_speed(speed)?;
    if input.is_empty() || (speed - DEFAULT_PLAYBACK_SPEED).abs() <= UNITY_EPSILON {
        return Ok(input.to_vec());
    }

    let expected = (input.len() as f64 / speed as f64).round() as usize;
    let mut stretch = ssstretch::Stretch::new();
    stretch.preset_default(1, sample_rate as f32);
    let input_latency = stretch.input_latency().max(0) as usize;
    let output_latency = stretch.output_latency().max(0) as usize;
    let reset_pre_roll = (input_latency as f64 / speed as f64).ceil() as usize;

    let inputs = [input.to_vec()];
    let mut output = [Vec::with_capacity(expected)];
    stretch.process_vec(
        &inputs,
        i32_len(input.len())?,
        &mut output,
        i32_len(expected)?,
    );

    let latency_input = [vec![0.0; input_latency]];
    let mut latency_output = [Vec::with_capacity(reset_pre_roll)];
    stretch.process_vec(
        &latency_input,
        i32_len(input_latency)?,
        &mut latency_output,
        i32_len(reset_pre_roll)?,
    );
    output[0].extend_from_slice(&latency_output[0]);

    let mut flushed = [Vec::with_capacity(output_latency)];
    stretch.flush_vec(&mut flushed, i32_len(output_latency)?);
    output[0].extend_from_slice(&flushed[0]);

    let start = reset_pre_roll.saturating_add(output_latency);
    let end = start.saturating_add(expected);
    if output[0].len() < end {
        return Err(format!(
            "time stretcher produced {} samples, need {end}",
            output[0].len()
        ));
    }
    Ok(output[0][start..end].to_vec())
}

/// Pitch-preserve one complete playback chunk across its model-unit splits.
///
/// Pocket may divide a natural synthesis chunk into multiple model-valid
/// units. Joining those units before processing keeps one stretcher timeline
/// across the hidden boundaries instead of resetting the DSP at each unit.
// This file is also compiled directly by the single-buffer diagnostic example.
#[allow(dead_code)]
pub fn process_complete_playback_chunk(
    model_units: &[Vec<f32>],
    speed: f32,
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    let sample_count = model_units.iter().try_fold(0_usize, |total, unit| {
        total
            .checked_add(unit.len())
            .ok_or_else(|| "audio chunk is too large to process".to_string())
    })?;
    let mut samples = Vec::with_capacity(sample_count);
    for unit in model_units {
        samples.extend_from_slice(unit);
    }
    process_complete_chunk(&samples, speed, sample_rate)
}

/// Return Signalsmith's compensated output lookahead for descriptive reporting.
#[allow(dead_code)]
pub(crate) fn compensated_output_latency_samples(sample_rate: u32) -> usize {
    let mut stretch = ssstretch::Stretch::new();
    stretch.preset_default(1, sample_rate as f32);
    stretch.output_latency().max(0) as usize
}

/// Validate a generated-speech playback speed.
pub fn validate_speed(speed: f32) -> Result<(), String> {
    if speed.is_finite() && (MIN_PLAYBACK_SPEED..=MAX_PLAYBACK_SPEED).contains(&speed) {
        Ok(())
    } else {
        Err(format!(
            "Speech playback speed must be between {MIN_PLAYBACK_SPEED} and {MAX_PLAYBACK_SPEED}"
        ))
    }
}

fn i32_len(length: usize) -> Result<i32, String> {
    i32::try_from(length).map_err(|_| "audio chunk is too large to process".to_string())
}
