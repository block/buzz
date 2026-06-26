//! Clipping probe for any fixed playback gain applied after Pocket TTS synth.
//!
//! Synthesises a spread of sentences (short/long, calm/energetic) and reports
//! the raw peak of each, the post-gain peak, and the fraction of samples that
//! would hit a ±1.0 clamp — i.e. how much a fixed gain would flat-top the
//! waveform ("blown out" distortion).
//!
//! History: the production pipeline briefly shipped a fixed 9.3× gain
//! calibrated on a single bench utterance that peaked at 0.076. This probe
//! showed real output peaks at 0.4–0.97, so that gain clipped 13–34% of all
//! samples (the 2026-06-12 "blown out" report). Production now applies no
//! gain — run this probe before reintroducing one.
//!
//! Run with model files in ~/.buzz/models/pocket-tts (override with arg 1):
//!   cargo run --release --example pocket_clip_probe

use std::path::PathBuf;

use sherpa_onnx::{
    self, GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsPocketModelConfig, Wave,
};

/// Candidate gain under test (the regressed production value).
const GAIN: f32 = 9.3;

const PROMPTS: &[&str] = &[
    "Hello, this is a test of the new Pocket TTS engine running on sherpa-onnx.",
    "Yep, I can hear you.",
    "Absolutely! That sounds fantastic, let's do it right now!",
    "The quick brown fox jumps over the lazy dog near the riverbank.",
    "I found three problems in the code: a race condition, a memory leak, and an off-by-one error in the loop bounds.",
    "No.",
    "Warning! The build failed because seventeen tests crashed unexpectedly!",
    "Sure, I can walk you through the whole pipeline step by step whenever you're ready.",
];

fn main() {
    let model_dir = std::env::args().nth(1).unwrap_or_else(|| {
        dirs::home_dir()
            .expect("home dir")
            .join(".buzz/models/pocket-tts")
            .to_string_lossy()
            .into_owned()
    });
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

    let gen = || GenerationConfig {
        speed: 1.05,
        num_steps: 1,
        silence_scale: 1.0,
        reference_audio: Some(voice_samples.clone()),
        reference_sample_rate: voice_sr,
        ..Default::default()
    };

    let _ = engine.generate_with_config("warmup.", &gen(), None::<fn(&[f32], f32) -> bool>);

    println!(
        "{:<46} | {:>8} | {:>9} | {:>9} | {:>10}",
        "prompt", "raw peak", "raw RMS", "post-gain", "% clipped"
    );
    println!("{}", "-".repeat(95));

    let mut worst_clip = 0.0f32;
    for prompt in PROMPTS {
        let out = engine
            .generate_with_config(prompt, &gen(), None::<fn(&[f32], f32) -> bool>)
            .expect("synth");
        let samples = out.samples();

        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let post = peak * GAIN;
        let clipped = samples.iter().filter(|s| s.abs() * GAIN > 1.0).count();
        let clip_pct = 100.0 * clipped as f32 / samples.len() as f32;
        worst_clip = worst_clip.max(clip_pct);

        let label: String = prompt.chars().take(44).collect();
        println!("{label:<46} | {peak:>8.4} | {rms:>9.4} | {post:>9.3} | {clip_pct:>9.3}%");
    }

    println!();
    println!(
        "Verdict: worst-case clipped fraction {worst_clip:.3}% — {}",
        if worst_clip > 0.1 {
            "AUDIBLE DISTORTION LIKELY (gain too hot)"
        } else if worst_clip > 0.0 {
            "marginal — occasional transient clipping"
        } else {
            "no clipping at this gain"
        }
    );
}
