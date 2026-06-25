//! Cold-vs-warm latency bench for Pocket TTS.
//!
//! This duplicates the small config-building snippet from `huddle::pocket` so it
//! doesn't depend on changing module visibility for a one-off dev tool.
//! Keep in sync with `huddle::pocket::load_text_to_speech`.
//!
//! Run with the model files in a directory (defaults to /tmp/pocket-tts-bench):
//!   cargo run --release --example pocket_bench
//!   cargo run --release --example pocket_bench /path/to/pocket-tts

use std::path::PathBuf;
use std::time::Instant;

use sherpa_onnx::{
    self, GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsPocketModelConfig, Wave,
};

const SAMPLE_RATE: u32 = 24_000;
const TEST_TEXT: &str =
    "Hello, this is a test of the new Pocket TTS engine running on sherpa-onnx.";

fn main() {
    let model_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pocket-tts-bench".to_string());
    println!("Model dir: {model_dir}");

    let dir = PathBuf::from(&model_dir);
    let p = |name: &str| dir.join(name).to_string_lossy().into_owned();

    let t0 = Instant::now();
    let cfg = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            pocket: OfflineTtsPocketModelConfig {
                lm_main: Some(p("lm_main.int8.onnx")),
                lm_flow: Some(p("lm_flow.int8.onnx")),
                encoder: Some(p("encoder.onnx")),
                decoder: Some(p("decoder.int8.onnx")),
                text_conditioner: Some(p("text_conditioner.onnx")),
                vocab_json: Some(p("vocab.json")),
                token_scores_json: Some(p("token_scores.json")),
                voice_embedding_cache_capacity: 16,
            },
            num_threads: 1,
            debug: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = OfflineTts::create(&cfg).expect("engine create");
    let load_ms = t0.elapsed().as_secs_f32() * 1000.0;
    println!("Engine load:      {load_ms:.1} ms");

    let t0 = Instant::now();
    let voice_path = dir.join("reference_sample.wav");
    let wave = Wave::read(voice_path.to_str().unwrap()).expect("voice WAV");
    let samples = wave.samples().to_vec();
    let sr = wave.sample_rate();
    let voice_ms = t0.elapsed().as_secs_f32() * 1000.0;
    println!("Voice load:       {voice_ms:.1} ms");

    let gen = || GenerationConfig {
        speed: 1.05,
        num_steps: 1,
        silence_scale: 1.0, // production setting (huddle::pocket::SYNTH_SILENCE_SCALE)
        reference_audio: Some(samples.clone()),
        reference_sample_rate: sr,
        ..Default::default()
    };

    let t0 = Instant::now();
    let cold = engine
        .generate_with_config(TEST_TEXT, &gen(), None::<fn(&[f32], f32) -> bool>)
        .expect("cold synth");
    let cold_ms = t0.elapsed().as_secs_f32() * 1000.0;
    let cold_audio_ms = (cold.samples().len() as f32 / SAMPLE_RATE as f32) * 1000.0;
    let cold_rtf_x = cold_audio_ms / cold_ms;
    println!(
        "Cold synth:       {cold_ms:.1} ms  → {cold_audio_ms:.1} ms audio  → {cold_rtf_x:.2}× realtime"
    );

    let t0 = Instant::now();
    let warm = engine
        .generate_with_config(TEST_TEXT, &gen(), None::<fn(&[f32], f32) -> bool>)
        .expect("warm synth");
    let warm_ms = t0.elapsed().as_secs_f32() * 1000.0;
    let warm_audio_ms = (warm.samples().len() as f32 / SAMPLE_RATE as f32) * 1000.0;
    let warm_rtf_x = warm_audio_ms / warm_ms;
    println!(
        "Warm synth:       {warm_ms:.1} ms  → {warm_audio_ms:.1} ms audio  → {warm_rtf_x:.2}× realtime"
    );

    let out_path = "/tmp/pocket_bench_out.wav";
    let ok = sherpa_onnx::write(out_path, warm.samples(), SAMPLE_RATE as i32);
    println!(
        "Wrote {} ({} samples, ok={ok})",
        out_path,
        warm.samples().len()
    );

    let delta_ms = cold_ms - warm_ms;
    let delta_pct = (delta_ms / warm_ms) * 100.0;
    println!();
    println!("Cold/warm delta:  {delta_ms:+.1} ms  ({delta_pct:+.1}%)");
    println!(
        "Decision: warmup {}.",
        if delta_ms > 200.0 {
            "RECOMMENDED — significant cold-call penalty"
        } else if delta_ms > 50.0 {
            "OPTIONAL — small cold-call penalty"
        } else {
            "UNNECESSARY — cold and warm essentially equal"
        }
    );
}
