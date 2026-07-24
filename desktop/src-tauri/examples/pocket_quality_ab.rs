//! Reproducible blind Pocket TTS quality corpus generator.
//!
//! Renders Buzz's production prompt preparation and post-processing across:
//! INT8/FP32 × per-sentence/grouped generation. The generated filenames are
//! deterministically blinded; keep `key.json` away from listeners until their
//! scoring sheet is complete.
//!
//! Usage:
//!   cargo run --release --example pocket_quality_ab -- \
//!     <int8-model-dir> <fp32-model-dir> <output-dir> [--idle-minutes N --only ITEM]
//!
//! The optional idle run intentionally creates one engine per condition, warms
//! all four, sleeps once, and then makes each clip the first generation after
//! dormancy. It requires `--only` because only the first synthesis after an
//! uninterrupted idle is a valid post-idle observation. Run each 5/15-minute
//! item as a separate process.

// Importing the production module also brings in runtime-only helpers that this
// standalone corpus generator deliberately does not call.
#![allow(dead_code)]

#[path = "../src/huddle/pocket.rs"]
mod production_pocket;
#[path = "../src/huddle/preprocessing.rs"]
mod production_preprocessing;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, Wave};

use production_pocket::{prepare_pocket_prompt, SAMPLE_RATE};
use production_preprocessing::{preprocess_for_tts, split_sentences};

const NUM_STEPS: i32 = 1;
const SILENCE_SCALE: f32 = 1.0;
const INTER_SENTENCE_SILENCE_SAMPLES: usize = SAMPLE_RATE as usize / 10;
const LEAD_IN_SAMPLES: usize = SAMPLE_RATE as usize / 50;
const FADE_OUT_SAMPLES: usize = SAMPLE_RATE as usize * 8 / 1000;
const TARGET_RMS_DBFS: f32 = -23.0;
const BLINDING_SEED: &str = "pocket-quality-2026-07-21-v1";

const CORPUS: &[CorpusItem] = &[
    CorpusItem { id: "short_one_word", kind: "short", text: "Yep." },
    CorpusItem { id: "short_four_words", kind: "short", text: "Sounds good to me." },
    CorpusItem {
        id: "multi_relay_review",
        kind: "multi-sentence",
        text: "I looked at the relay code this morning. The lease logic is solid. There's one race in the worker claim path, though. I'll write it up and send you a patch.",
    },
    CorpusItem {
        id: "multi_community_size",
        kind: "multi-sentence",
        text: "Great question. The answer is it depends on the community size. For small ones, keep it simple.",
    },
    CorpusItem {
        id: "mixed_agent_message",
        kind: "mixed",
        text: "That's 42 open PRs right now — mostly small. I'll triage them after lunch.",
    },
];

#[derive(Clone, Copy)]
struct CorpusItem {
    id: &'static str,
    kind: &'static str,
    text: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Precision {
    Int8,
    Fp32,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Chunking {
    PerSentence,
    Grouped,
}

#[derive(Clone, Copy, Debug)]
struct Condition {
    precision: Precision,
    chunking: Chunking,
}

const CONDITIONS: [Condition; 4] = [
    Condition {
        precision: Precision::Int8,
        chunking: Chunking::PerSentence,
    },
    Condition {
        precision: Precision::Int8,
        chunking: Chunking::Grouped,
    },
    Condition {
        precision: Precision::Fp32,
        chunking: Chunking::PerSentence,
    },
    Condition {
        precision: Precision::Fp32,
        chunking: Chunking::Grouped,
    },
];

#[derive(Serialize)]
struct KeyFile {
    warning: &'static str,
    blinding_seed: &'static str,
    target_rms_dbfs: f32,
    items: Vec<KeyItem>,
}

#[derive(Serialize)]
struct KeyItem {
    id: String,
    kind: String,
    text: String,
    clips: Vec<KeyClip>,
}

#[derive(Serialize)]
struct KeyClip {
    file: String,
    precision: Precision,
    chunking: Chunking,
    cold_start: bool,
    idle_minutes: Option<u64>,
    synthesis_ms: u128,
    audio_seconds: f32,
}

struct Voice {
    samples: Vec<f32>,
    sample_rate: i32,
}

struct Engine {
    inner: OfflineTts,
    voice: Voice,
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let int8_dir = required_path(args.next(), "INT8 model directory")?;
    let fp32_dir = required_path(args.next(), "FP32 model directory")?;
    let output_dir = required_path(args.next(), "output directory")?;
    let mut idle_minutes = None;
    let mut only_item = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--idle-minutes" => {
                idle_minutes = Some(
                    args.next()
                        .ok_or("--idle-minutes requires a value")?
                        .parse::<u64>()
                        .map_err(|e| format!("invalid idle minutes: {e}"))?,
                );
            }
            "--only" => only_item = Some(args.next().ok_or("--only requires an item ID")?),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    if idle_minutes.is_some() && only_item.is_none() {
        return Err("--idle-minutes requires --only so every clip is first-after-idle".into());
    }
    if let Some(ref requested) = only_item {
        if !CORPUS.iter().any(|item| item.id == requested) {
            return Err(format!("unknown corpus item for --only: {requested}"));
        }
    }

    validate_model_dir(&int8_dir, Precision::Int8)?;
    validate_model_dir(&fp32_dir, Precision::Fp32)?;
    fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;

    let mut engines = Vec::with_capacity(CONDITIONS.len());
    for condition in CONDITIONS {
        let dir = match condition.precision {
            Precision::Int8 => &int8_dir,
            Precision::Fp32 => &fp32_dir,
        };
        let engine = load_engine(dir, condition.precision)?;
        // Production warms once before serving a real utterance. Cold cases use
        // separate fresh engines below and deliberately skip this call.
        synth_chunks(&engine, &["warmup".to_string()])?;
        engines.push(engine);
    }

    if let Some(minutes) = idle_minutes {
        eprintln!("All four warmed engines idle for {minutes} minute(s)…");
        std::thread::sleep(Duration::from_secs(minutes * 60));
    }

    let mut key_items = Vec::new();
    for item in CORPUS {
        if only_item
            .as_deref()
            .is_some_and(|requested| requested != item.id)
        {
            continue;
        }
        let preprocessed = preprocess_for_tts(item.text);
        let per_sentence: Vec<String> = split_sentences(&preprocessed)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        // These corpus texts are deliberately below the upstream ~50-token
        // grouping target, so grouped mode is one exact generate() call.
        let grouped = vec![per_sentence.join(" ")];
        let item_dir = output_dir.join(item.id);
        fs::create_dir_all(&item_dir).map_err(|e| e.to_string())?;
        let clip_order = blinded_order(item.id);
        let mut clips = Vec::new();

        let mut rendered = Vec::new();
        for (condition_index, engine) in engines.iter().enumerate() {
            let condition = CONDITIONS[condition_index];
            let chunks = match condition.chunking {
                Chunking::PerSentence => &per_sentence,
                Chunking::Grouped => &grouped,
            };
            let started = Instant::now();
            let audio = synth_chunks(engine, chunks)?;
            rendered.push((
                condition_index,
                condition,
                audio,
                started.elapsed().as_millis(),
            ));
        }
        loudness_match_item(&mut rendered);
        for (condition_index, condition, audio, synth_ms) in rendered {
            let clip_number = clip_order[condition_index] + 1;
            let file_name = format!("clip{clip_number}.wav");
            write_wav(&item_dir.join(&file_name), &audio)?;
            clips.push(KeyClip {
                file: format!("{}/{file_name}", item.id),
                precision: condition.precision,
                chunking: condition.chunking,
                cold_start: false,
                idle_minutes,
                synthesis_ms: synth_ms,
                audio_seconds: audio.len() as f32 / SAMPLE_RATE as f32,
            });
        }
        clips.sort_by(|a, b| a.file.cmp(&b.file));
        key_items.push(KeyItem {
            id: item.id.to_string(),
            kind: item.kind.to_string(),
            text: item.text.to_string(),
            clips,
        });
    }

    // Explicit fresh-engine cold-start clips for the two highest-signal texts.
    // Idle runs intentionally omit them: they happen after the post-idle clips
    // and add no valid idle observation.
    for item in if idle_minutes.is_none() { CORPUS } else { &[] } {
        if !matches!(item.id, "short_one_word" | "multi_relay_review") {
            continue;
        }
        if only_item
            .as_deref()
            .is_some_and(|requested| requested != item.id)
        {
            continue;
        }
        let cold_id = format!("cold_{}", item.id);
        let preprocessed = preprocess_for_tts(item.text);
        let sentences: Vec<String> = split_sentences(&preprocessed)
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        let grouped = vec![sentences.join(" ")];
        let item_dir = output_dir.join(&cold_id);
        fs::create_dir_all(&item_dir).map_err(|e| e.to_string())?;
        let clip_order = blinded_order(&cold_id);
        let mut clips = Vec::new();
        let mut rendered = Vec::new();
        for (condition_index, condition) in CONDITIONS.iter().copied().enumerate() {
            let dir = match condition.precision {
                Precision::Int8 => &int8_dir,
                Precision::Fp32 => &fp32_dir,
            };
            let engine = load_engine(dir, condition.precision)?;
            let chunks = match condition.chunking {
                Chunking::PerSentence => &sentences,
                Chunking::Grouped => &grouped,
            };
            let started = Instant::now();
            let audio = synth_chunks(&engine, chunks)?;
            rendered.push((
                condition_index,
                condition,
                audio,
                started.elapsed().as_millis(),
            ));
        }
        loudness_match_item(&mut rendered);
        for (condition_index, condition, audio, synth_ms) in rendered {
            let clip_number = clip_order[condition_index] + 1;
            let file_name = format!("clip{clip_number}.wav");
            write_wav(&item_dir.join(&file_name), &audio)?;
            clips.push(KeyClip {
                file: format!("{cold_id}/{file_name}"),
                precision: condition.precision,
                chunking: condition.chunking,
                cold_start: true,
                idle_minutes: None,
                synthesis_ms: synth_ms,
                audio_seconds: audio.len() as f32 / SAMPLE_RATE as f32,
            });
        }
        clips.sort_by(|a, b| a.file.cmp(&b.file));
        key_items.push(KeyItem {
            id: cold_id,
            kind: "cold-start".to_string(),
            text: item.text.to_string(),
            clips,
        });
    }

    let key = KeyFile {
        warning: "DO NOT OPEN UNTIL LISTENING SCORES ARE FINAL",
        blinding_seed: BLINDING_SEED,
        target_rms_dbfs: TARGET_RMS_DBFS,
        items: key_items,
    };
    fs::write(
        output_dir.join("key.json"),
        serde_json::to_vec_pretty(&key).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    write_scoring_sheet(&output_dir, &key)?;
    println!("Wrote blind corpus to {}", output_dir.display());
    println!("Give listeners the WAV folders and SCORING.md; withhold key.json.");
    Ok(())
}

fn required_path(value: Option<String>, label: &str) -> Result<PathBuf, String> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {label}"))
}

fn model_file(precision: Precision, base: &str) -> String {
    match precision {
        Precision::Int8 => format!("{base}.int8.onnx"),
        Precision::Fp32 => format!("{base}.onnx"),
    }
}

fn validate_model_dir(dir: &Path, precision: Precision) -> Result<(), String> {
    for file in [
        model_file(precision, "lm_main"),
        model_file(precision, "lm_flow"),
        "encoder.onnx".into(),
        model_file(precision, "decoder"),
        "text_conditioner.onnx".into(),
        "vocab.json".into(),
        "token_scores.json".into(),
        "reference_sample.wav".into(),
    ] {
        if !dir.join(&file).is_file() {
            return Err(format!("missing {}", dir.join(file).display()));
        }
    }
    Ok(())
}

fn load_engine(dir: &Path, precision: Precision) -> Result<Engine, String> {
    let p = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let mut cfg = OfflineTtsConfig::default();
    cfg.model.pocket.lm_main = Some(p(&model_file(precision, "lm_main")));
    cfg.model.pocket.lm_flow = Some(p(&model_file(precision, "lm_flow")));
    cfg.model.pocket.encoder = Some(p("encoder.onnx"));
    cfg.model.pocket.decoder = Some(p(&model_file(precision, "decoder")));
    cfg.model.pocket.text_conditioner = Some(p("text_conditioner.onnx"));
    cfg.model.pocket.vocab_json = Some(p("vocab.json"));
    cfg.model.pocket.token_scores_json = Some(p("token_scores.json"));
    cfg.model.pocket.voice_embedding_cache_capacity = 16;
    cfg.model.num_threads = 1;
    cfg.model.debug = false;
    let inner =
        OfflineTts::create(&cfg).ok_or_else(|| format!("failed to create {precision:?} engine"))?;
    let wave =
        Wave::read(&p("reference_sample.wav")).ok_or("failed to read reference_sample.wav")?;
    Ok(Engine {
        inner,
        voice: Voice {
            samples: wave.samples().to_vec(),
            sample_rate: wave.sample_rate(),
        },
    })
}

fn synth_chunks(engine: &Engine, chunks: &[String]) -> Result<Vec<f32>, String> {
    let mut out = Vec::new();
    for chunk in chunks {
        let prepared = prepare_pocket_prompt(chunk).ok_or("empty prepared prompt")?;
        let extra = prepared.max_frames.map(|max_frames| {
            HashMap::from([(
                "max_frames".to_string(),
                serde_json::Value::from(max_frames),
            )])
        });
        let cfg = GenerationConfig {
            num_steps: NUM_STEPS,
            silence_scale: SILENCE_SCALE,
            reference_audio: Some(engine.voice.samples.clone()),
            reference_sample_rate: engine.voice.sample_rate,
            extra,
            ..Default::default()
        };
        let audio = engine
            .inner
            .generate_with_config(&prepared.text, &cfg, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| format!("synthesis failed for {chunk:?}"))?;
        let mut samples: Vec<f32> = audio.samples().iter().map(|s| s.clamp(-1.0, 1.0)).collect();
        apply_fade_out(&mut samples);
        out.extend(std::iter::repeat_n(0.0, LEAD_IN_SAMPLES));
        out.extend(samples);
        out.extend(std::iter::repeat_n(
            0.0,
            INTER_SENTENCE_SILENCE_SAMPLES - LEAD_IN_SAMPLES,
        ));
    }
    Ok(out)
}

fn apply_fade_out(samples: &mut [f32]) {
    let fade = FADE_OUT_SAMPLES.min(samples.len() / 2);
    for i in 0..fade {
        samples[samples.len() - 1 - i] *= i as f32 / fade as f32;
    }
}

fn active_rms(samples: &[f32]) -> Option<f32> {
    let (sum_squares, count) = samples
        .iter()
        .filter(|sample| sample.abs() > 1.0e-4)
        .fold((0.0_f32, 0_usize), |(sum, count), sample| {
            (sum + sample * sample, count + 1)
        });
    (count > 0).then(|| (sum_squares / count as f32).sqrt())
}

/// Attenuate every clip in one comparison set to the quietest active-speech RMS.
/// This removes the louder-is-better confound without normalizing dynamics or
/// claiming standards-compliant integrated LUFS. The dBFS value is a ceiling.
fn loudness_match_item(rendered: &mut [(usize, Condition, Vec<f32>, u128)]) {
    let ceiling = 10.0_f32.powf(TARGET_RMS_DBFS / 20.0);
    let target = rendered
        .iter()
        .filter_map(|(_, _, samples, _)| active_rms(samples))
        .fold(ceiling, f32::min);
    for (_, _, samples, _) in rendered {
        let Some(rms) = active_rms(samples) else {
            continue;
        };
        let gain = (target / rms).min(1.0);
        for sample in samples {
            *sample *= gain;
        }
    }
}

fn blinded_order(item_id: &str) -> [usize; 4] {
    let mut keyed: Vec<(usize, Vec<u8>)> = (0..4)
        .map(|index| {
            let digest = Sha256::digest(format!("{BLINDING_SEED}:{item_id}:{index}"));
            (index, digest.to_vec())
        })
        .collect();
    keyed.sort_by(|a, b| a.1.cmp(&b.1));
    let mut condition_to_clip = [0; 4];
    for (clip, (condition, _)) in keyed.into_iter().enumerate() {
        condition_to_clip[condition] = clip;
    }
    condition_to_clip
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 path: {}", path.display()))?;
    if sherpa_onnx::write(path, samples, SAMPLE_RATE as i32) {
        Ok(())
    } else {
        Err(format!("failed to write {path}"))
    }
}

fn write_scoring_sheet(output_dir: &Path, key: &KeyFile) -> Result<(), String> {
    let mut sheet = String::from("# Pocket TTS blind listening sheet\n\nDo not open `key.json` until this sheet is complete. Rank best to worst; ties are allowed.\n\n");
    for item in &key.items {
        sheet.push_str(&format!(
            "## {} ({})\n\n> {}\n\n",
            item.id, item.kind, item.text
        ));
        sheet.push_str("Rank: `____ > ____ > ____ > ____`\n\n| Clip | seam | onset | garble | robotic | timbre | truncate | note |\n|---|---|---|---|---|---|---|---|\n");
        for clip in 1..=4 {
            sheet.push_str(&format!(
                "| clip{clip} | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] | |\n"
            ));
        }
        sheet.push('\n');
    }
    fs::write(output_dir.join("SCORING.md"), sheet).map_err(|e| e.to_string())
}
