//! Onset-attenuation probe for Pocket TTS.
//!
//! Synthesises a handful of short sentences and dumps per-sentence onset
//! statistics (samples[0], 1ms/5ms/20ms peak + RMS) so we can decide whether
//! the production `apply_fades` 8 ms fade-in is masking real audio.
//!
//! Also writes the raw (un-faded, un-normalised) audio of each sentence to
//! /tmp so they can be inspected in Audacity / aplay without rodio in the
//! loop.
//!
//! Run with model files in /tmp/pocket-tts-bench (override with arg 1):
//!   cargo run --release --example pocket_onset_probe
//!   cargo run --release --example pocket_onset_probe /path/to/pocket-tts

use std::path::PathBuf;

use sherpa_onnx::{
    self, GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsPocketModelConfig, Wave,
};

const SAMPLE_RATE: u32 = 24_000;

/// Test prompts chosen to span different onsets:
/// - palatal glide 'Y' (soft onset)
/// - voiceless fricative 'H' (very soft onset)
/// - labio-velar glide 'W' (medium onset)
/// - voiceless stop 'T' (hard onset)
const PROMPTS: &[&str] = &[
    "Yep, I can hear you.",
    "Hello there friend.",
    "What can I help with?",
    "Try this experiment now.",
];

fn main() {
    let model_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pocket-tts-bench".to_string());
    eprintln!("Model dir: {model_dir}");

    let dir = PathBuf::from(&model_dir);
    let p = |name: &str| dir.join(name).to_string_lossy().into_owned();

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

    let voice_path = dir.join("reference_sample.wav");
    let wave = Wave::read(voice_path.to_str().unwrap()).expect("voice WAV");
    let voice_samples = wave.samples().to_vec();
    let voice_sr = wave.sample_rate();

    // Warmup so we're not measuring cold-call jitter.
    {
        let cfg = GenerationConfig {
            speed: 1.05,
            num_steps: 1,
            silence_scale: 1.0, // production setting (huddle::pocket::SYNTH_SILENCE_SCALE)
            reference_audio: Some(voice_samples.clone()),
            reference_sample_rate: voice_sr,
            ..Default::default()
        };
        let _ = engine.generate_with_config("warmup.", &cfg, None::<fn(&[f32], f32) -> bool>);
    }

    println!(
        "{:<28} | {:>10} | {:>10} {:>10} | {:>10} {:>10} | {:>10} {:>10}",
        "prompt",
        "samples[0]",
        "peak@1ms",
        "rms@1ms",
        "peak@5ms",
        "rms@5ms",
        "peak@20ms",
        "rms@20ms"
    );
    println!("{}", "-".repeat(120));

    for prompt in PROMPTS {
        // Mirror the production prompt-prep (capitalise + terminal punctuation).
        // These prompts already have it, so this is just to match what
        // sherpa-onnx sees in production.
        let cfg = GenerationConfig {
            speed: 1.05,
            num_steps: 1,
            silence_scale: 1.0, // production setting (huddle::pocket::SYNTH_SILENCE_SCALE)
            reference_audio: Some(voice_samples.clone()),
            reference_sample_rate: voice_sr,
            ..Default::default()
        };
        let out = engine
            .generate_with_config(prompt, &cfg, None::<fn(&[f32], f32) -> bool>)
            .expect("synth");
        let samples = out.samples();

        let n_1ms = (SAMPLE_RATE as f32 * 0.001) as usize;
        let n_5ms = (SAMPLE_RATE as f32 * 0.005) as usize;
        let n_20ms = (SAMPLE_RATE as f32 * 0.020) as usize;

        let stats = |range: &[f32]| -> (f32, f32) {
            if range.is_empty() {
                return (0.0, 0.0);
            }
            let peak = range.iter().fold(0.0_f32, |a, &x| a.max(x.abs()));
            let sumsq: f32 = range.iter().map(|x| x * x).sum();
            let rms = (sumsq / range.len() as f32).sqrt();
            (peak, rms)
        };

        let first = samples.first().copied().unwrap_or(0.0);
        let (p1, r1) = stats(&samples[..n_1ms.min(samples.len())]);
        let (p5, r5) = stats(&samples[..n_5ms.min(samples.len())]);
        let (p20, r20) = stats(&samples[..n_20ms.min(samples.len())]);

        println!(
            "{:<28} | {:>10.6} | {:>10.6} {:>10.6} | {:>10.6} {:>10.6} | {:>10.6} {:>10.6}",
            prompt, first, p1, r1, p5, r5, p20, r20
        );

        let safe: String = prompt
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let out_path = format!("/tmp/pocket_onset_{}.wav", &safe[..safe.len().min(24)]);
        let _ = sherpa_onnx::write(&out_path, samples, SAMPLE_RATE as i32);
        eprintln!(
            "  → wrote {out_path} ({} samples = {:.3} s)",
            samples.len(),
            samples.len() as f32 / SAMPLE_RATE as f32
        );
    }
}
