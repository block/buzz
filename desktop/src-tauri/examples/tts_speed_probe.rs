use std::time::Instant;

#[path = "../src/huddle/playback_speed_dsp.rs"]
mod playback_speed_dsp;

const SAMPLE_RATE: u32 = 24_000;
const TONE_SECONDS: f64 = 10.0;

fn main() {
    let tone: Vec<f32> = (0..SAMPLE_RATE * TONE_SECONDS as u32)
        .map(|index| {
            (2.0 * std::f32::consts::PI * 220.0 * index as f32 / SAMPLE_RATE as f32).sin() * 0.25
        })
        .collect();

    let compensated_lookahead = playback_speed_dsp::compensated_output_latency_samples(SAMPLE_RATE);
    let compensated_lookahead_ms = compensated_lookahead as f64 * 1_000.0 / SAMPLE_RATE as f64;

    for speed in [0.75_f32, 1.25, 1.5] {
        let started = Instant::now();
        let output = playback_speed_dsp::process_complete_chunk(&tone, speed, SAMPLE_RATE)
            .expect("process synthesized model audio");
        let elapsed = started.elapsed();
        println!(
            "{speed:.2}x: processing={:.2}ms, {:.3}% realtime, output={} samples, compensated \
             DSP lookahead={compensated_lookahead_ms:.1}ms",
            elapsed.as_secs_f64() * 1_000.0,
            elapsed.as_secs_f64() / TONE_SECONDS * 100.0,
            output.len(),
        );
    }
}
