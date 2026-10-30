use super::*;

/// The onset cushion rounds rodio's 512-sample bootstrap span up to 22 ms at
/// the 24 kHz production sample rate (528 samples).
#[test]
fn chunk_lead_in_is_sane() {
    assert_eq!(RODIO_ADD_BOOTSTRAP_SPAN_SAMPLES, 512);
    assert_eq!(SENTENCE_LEAD_IN_SAMPLES, 528, "22 ms × 24 kHz");
}

/// Model-token splits remain contiguous: only the playback chunk as a whole
/// receives its onset cushion and trailing sentence gap.
#[test]
fn token_split_units_do_not_add_sentence_boundary_padding() {
    let first_unit = build_sentence_append_buffer(vec![0.5; 100], false);
    let last_unit = build_sentence_append_buffer(vec![0.25; 100], false);

    assert_eq!(first_unit.len(), 100);
    assert_eq!(first_unit.last(), Some(&0.5));
    assert_eq!(last_unit.first(), Some(&0.25));
    assert_eq!(first_unit.len() + last_unit.len(), 200);
}
