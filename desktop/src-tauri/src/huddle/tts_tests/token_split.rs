use super::*;

/// The onset cushion covers 20 ms at the production sample rate.
#[test]
fn chunk_lead_in_is_sane() {
    assert_eq!(CHUNK_LEAD_IN_SAMPLES, 480, "20 ms × 24 kHz");
}
