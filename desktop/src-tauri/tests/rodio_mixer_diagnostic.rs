//! Diagnostic regression test: rodio's `Player` is a single-queue FIFO, not a summing mixer.
//!
//! This pins the failure mode we hit at 3+ simultaneous speakers in `huddle`:
//! `Player::connect_new(&device_mixer)` calls `device_mixer.add(queue_source)` exactly once,
//! and every `player.append(SamplesBuffer)` enqueues onto that single queue. Multiple peers'
//! 20 ms frames therefore serialize — peer B plays after peer A finishes, etc. — instead of
//! mixing.
//!
//! The fix is per-peer `Player` (each peer gets its own queue added to the device mixer), so
//! the device mixer actually sums across queues. This test asserts both halves of the
//! diagnosis with a deterministic, device-free harness:
//!
//!   * A single queue receiving two sources → samples arrive **serially**.
//!   * A summing mixer fed by two sources → samples arrive **concurrently and summed**.
//!
//! If rodio ever changes either invariant under us we want CI to scream, not the next
//! 3-person huddle.

use rodio::buffer::SamplesBuffer;
use rodio::mixer;
use rodio::queue;
use rodio::source::Source;
use std::num::NonZero;

const SR: u32 = 48_000;
const CH: u16 = 1;
/// 20 ms at 48 kHz mono — one Opus frame's worth of decoded samples.
const FRAME_SAMPLES: usize = 960;

fn channels() -> NonZero<u16> {
    NonZero::new(CH).unwrap()
}

fn sample_rate() -> NonZero<u32> {
    NonZero::new(SR).unwrap()
}

/// Build a `SamplesBuffer` of length `n` samples filled with `value`.
fn buf(value: f32, n: usize) -> SamplesBuffer {
    SamplesBuffer::new(channels(), sample_rate(), vec![value; n])
}

/// Drain at most `limit` samples from any `Source` and return them.
fn drain<S: Source<Item = f32>>(src: S, limit: usize) -> Vec<f32> {
    src.take(limit).collect()
}

/// A single `queue` plays sources **one after the other** (FIFO).
///
/// This is the shape `rodio::Player::connect_new` produces: one queue is added to the
/// device mixer at start, and every `Player::append` enqueues onto that one queue.
/// In the huddle path, every peer's decoded frame was appended here — serializing them.
#[test]
fn single_queue_serializes_sources() {
    let (input, output) = queue::queue(false /* don't keep alive when empty */);

    // Two "peers": A produces all 1.0s, B produces all -1.0s. If they were summed, the
    // overlap would be ~0.0; if they're serialized we see a clean 1.0 → -1.0 transition.
    input.append(buf(1.0, FRAME_SAMPLES));
    input.append(buf(-1.0, FRAME_SAMPLES));

    let samples = drain(output, FRAME_SAMPLES * 4);

    // Total samples produced = sum of inputs (serial). If it were summed we'd only get
    // FRAME_SAMPLES of output (and at amplitude 0.0).
    assert_eq!(
        samples.len(),
        FRAME_SAMPLES * 2,
        "single queue should drain both sources back-to-back (serial), not mix them",
    );

    // The first source's samples come first, then the second's. No interleaving, no summing.
    assert!(
        samples[..FRAME_SAMPLES].iter().all(|&s| s == 1.0),
        "first half of queue output should be source A's samples (1.0), not a mix",
    );
    assert!(
        samples[FRAME_SAMPLES..].iter().all(|&s| s == -1.0),
        "second half of queue output should be source B's samples (-1.0)",
    );
}

/// A `mixer` plays sources **concurrently** and **sums** overlapping samples.
///
/// This is the shape we want for huddle playout: each peer gets its own source added to
/// the device mixer, so simultaneous speakers actually mix instead of serializing.
#[test]
fn mixer_sums_overlapping_sources() {
    let (controller, mixer_source) = mixer::mixer(channels(), sample_rate());

    // Same two "peers" — 1.0 and -1.0. If the mixer sums correctly, overlap == 0.0.
    controller.add(buf(1.0, FRAME_SAMPLES));
    controller.add(buf(-1.0, FRAME_SAMPLES));

    // Take exactly one frame's worth — both sources should be active across that window.
    let samples = drain(mixer_source, FRAME_SAMPLES);

    assert_eq!(
        samples.len(),
        FRAME_SAMPLES,
        "mixer should produce one frame's worth of mixed output, not two frames serialized",
    );

    // Both sources are active over this window. Their sum is 0.0 at every sample.
    // Allow for f32 rounding (rodio's UniformSourceIterator + sum can introduce a few ULPs).
    let max_abs = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    assert!(
        max_abs < 1e-5,
        "mixer should sum 1.0 + -1.0 ≈ 0.0 per sample; saw max |sample| = {max_abs} \
         which means the sources weren't actually mixed concurrently",
    );
}

/// One mixer fed by **two distinct queues** (= two distinct `Player`s on one device sink)
/// behaves the same way: the queues drain concurrently, the mixer sums them.
///
/// This is the exact shape the huddle fix moves to: `HashMap<peer_index, Player>`, each
/// added to `MixerDeviceSink::mixer()`. The test pins that this composes correctly.
#[test]
fn mixer_of_queues_mixes_per_peer_streams() {
    let (mixer_in, mixer_source) = mixer::mixer(channels(), sample_rate());

    // Two queues — one per "peer".
    let (peer_a_in, peer_a_out) =
        queue::queue(true /* keep alive — players outlive frames */);
    let (peer_b_in, peer_b_out) = queue::queue(true);
    mixer_in.add(peer_a_out);
    mixer_in.add(peer_b_out);

    // Each peer pushes one 20 ms frame at the same wall-clock moment.
    peer_a_in.append(buf(1.0, FRAME_SAMPLES));
    peer_b_in.append(buf(-1.0, FRAME_SAMPLES));

    let samples = drain(mixer_source, FRAME_SAMPLES);
    assert_eq!(samples.len(), FRAME_SAMPLES);

    let max_abs = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    assert!(
        max_abs < 1e-5,
        "per-peer queues into one mixer should sum concurrently; saw max |sample| = {max_abs}",
    );
}
