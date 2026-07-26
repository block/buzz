use super::*;

#[test]
fn leading_audio_start_trims_silence_with_fifty_ms_preroll() {
    let silence = SAMPLE_RATE as usize / 5;
    let retain = SAMPLE_RATE as usize / 20;
    let rms_window_overlap = SAMPLE_RATE as usize / 100 - 1;
    let mut samples = vec![0.0; silence];
    samples.extend(std::iter::repeat_n(0.1, SAMPLE_RATE as usize / 10));

    assert_eq!(
        leading_audio_start(&samples),
        silence - retain - rms_window_overlap
    );
}

#[test]
fn leading_audio_start_preserves_immediate_soft_onset() {
    let samples = vec![0.003; SAMPLE_RATE as usize / 10];
    assert_eq!(leading_audio_start(&samples), 0);
}

#[test]
fn leading_audio_start_leaves_all_silence_unchanged() {
    let samples = vec![0.0; SAMPLE_RATE as usize / 2];
    assert_eq!(leading_audio_start(&samples), 0);
}
