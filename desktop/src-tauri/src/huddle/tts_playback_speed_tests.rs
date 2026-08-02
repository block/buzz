use super::*;
use crate::huddle::playback_speed::process_complete_chunk;

/// Regression guard: playback decoration happens after speed processing, so
/// the fixed device cushion is never time-stretched.
#[test]
fn playback_speed_preserves_production_sentence_lead_in() {
    let processed = process_complete_chunk(&vec![0.5; 2_400], 1.5, SAMPLE_RATE)
        .expect("process synthesized model audio");
    let mut first = true;
    let processed = build_sentence_append_buffer(&mut first, processed, 2_400, true, true);

    assert_eq!(SENTENCE_LEAD_IN_SAMPLES, 480);
    assert!(
        processed[..SENTENCE_LEAD_IN_SAMPLES]
            .iter()
            .all(|&sample| sample == 0.0),
        "the fixed device cushion must remain pure zero"
    );
    assert!(
        processed[SENTENCE_LEAD_IN_SAMPLES..]
            .iter()
            .any(|sample| sample.abs() > 0.1),
        "speech energy must remain after the fixed device cushion"
    );
}

#[test]
fn playback_speed_keeps_one_timeline_across_model_units() {
    let frequency = 220.0_f32;
    let input: Vec<f32> = (0..SAMPLE_RATE)
        .map(|sample| {
            (2.0 * std::f32::consts::PI * frequency * sample as f32 / SAMPLE_RATE as f32).sin()
        })
        .collect();
    let units = vec![input[..12_000].to_vec(), input[12_000..].to_vec()];

    let joined = process_complete_playback_chunk(&units, 1.25, SAMPLE_RATE)
        .expect("process complete playback chunk");
    let expected =
        process_complete_chunk(&input, 1.25, SAMPLE_RATE).expect("process already-joined input");

    assert_eq!(joined, expected, "model units must share one stretcher run");
    let boundary = joined.len() / 2;
    let largest_step = joined[boundary - 240..boundary + 240]
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        largest_step < 0.1,
        "hidden model boundary introduced a {largest_step} sample discontinuity"
    );
}
