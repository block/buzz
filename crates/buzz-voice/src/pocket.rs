//! April 2026 Pocket TTS engine for Buzz Desktop.
//!
//! The `english_2026-04` bundle uses SentencePiece tokenization, a learned
//! voice BOS embedding, recurrent FlowLM state, and stateful Mimi decoding.
//! Buzz selects the upstream three-graph INT8 variant while retaining the
//! full-precision Mimi encoder and text conditioner specified by that variant.
//!
//! ## Attribution
//!
//! - Pocket TTS and Mimi: Kyutai, CC-BY-4.0.
//! - ONNX export: KevinAHM/pocket-tts-onnx, CC-BY-4.0.
//! - Reference voice: Kyutai's Mary preset (VCTK p333), CC-BY-4.0.
//!
//! `huddle::models` writes the complete attribution beside the cached bytes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sherpa_onnx::Wave;

#[path = "pocket_april.rs"]
mod pocket_april;
#[path = "pocket_models.rs"]
mod pocket_models;

use pocket_april::{prepare_april_prompt, AprilPocketTts};
pub use pocket_models::{
    april_model_info, PocketModelArtifact, PocketModelInfo, APRIL_BUNDLE_ID, APRIL_MODEL_ID,
    APRIL_MODEL_REVISION,
};

/// Pocket TTS emits 24 kHz mono PCM.
pub const SAMPLE_RATE: u32 = 24_000;

/// Bundled reference voice name without its extension.
pub const DEFAULT_VOICE: &str = "reference_sample";

/// Pocket voice files are reference WAVs.
pub const VOICE_FILE_EXT: &str = "wav";

const TTS_NUM_THREADS: usize = 1;

/// EXPERIMENTAL (latency): override ONNX intra-op threads for the Pocket
/// sessions via `BUZZ_TTS_THREADS`. Default preserves production's 1.
fn tts_num_threads() -> usize {
    std::env::var("BUZZ_TTS_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(TTS_NUM_THREADS)
}

/// Loaded reference voice samples and their original sample rate.
#[derive(Debug, Clone)]
pub struct VoiceStyle {
    samples: Vec<f32>,
    sample_rate: i32,
}

/// Load a Pocket reference voice WAV from disk.
pub fn load_voice_style(path: &Path) -> Result<VoiceStyle, String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| format!("voice path is not valid UTF-8: {}", path.display()))?;
    let wave = Wave::read(path_str)
        .ok_or_else(|| format!("could not read voice WAV at {}", path.display()))?;
    let samples = wave.samples().to_vec();
    if samples.is_empty() {
        return Err(format!("voice WAV is empty: {}", path.display()));
    }
    Ok(VoiceStyle {
        samples,
        sample_rate: wave.sample_rate(),
    })
}

/// Resident April INT8 Pocket TTS engine.
pub struct PocketTts {
    inner: Mutex<AprilPocketTts>,
}

/// Load Buzz Desktop's pinned April INT8 model.
pub fn load_text_to_speech(model_dir: &str) -> Result<PocketTts, String> {
    let dir = PathBuf::from(model_dir);
    for artifact in april_model_info().artifacts {
        let path = dir.join(artifact.filename);
        if !path.is_file() {
            return Err(format!(
                "incomplete Pocket TTS {} INT8 bundle: missing {}",
                APRIL_BUNDLE_ID,
                path.display()
            ));
        }
    }
    Ok(PocketTts {
        inner: Mutex::new(AprilPocketTts::load(&dir, tts_num_threads())?),
    })
}

impl PocketTts {
    /// Split text into model-safe synthesis units that satisfy the bundle's
    /// exact 50-token input limit, packing sentences whenever they fit.
    pub fn split_text_into_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        self.inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?
            .split_prompt(&prepared)
    }

    /// Split text into ordered playback units, keeping the first sentence
    /// separate so it reaches synthesis before the remainder is packed.
    ///
    /// Units are contiguous substrings of the prepared model prompt and may
    /// retain boundary whitespace. Concatenating them with `chunks.concat()`
    /// reconstructs that prompt exactly, and each unit's prepared token count
    /// is at most 50.
    pub fn split_text_for_playback(&self, text: &str) -> Result<Vec<String>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        self.inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?
            .split_playback_prompt(&prepared)
    }

    /// Synthesize text with the supplied reference voice.
    ///
    /// Pocket detects language from text and this model uses one synthesis
    /// step, so `_lang` and `_steps` intentionally do not affect output.
    pub fn synth_chunk(
        &self,
        text: &str,
        _lang: &str,
        style: &VoiceStyle,
        _steps: usize,
    ) -> Result<Vec<f32>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?;
        let chunks = engine.split_prompt(&prepared)?;
        let mut samples = Vec::new();
        for chunk in chunks {
            let prepared = prepare_april_prompt(&chunk)
                .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
            samples.extend(engine.synth_chunk(&prepared, style)?);
        }
        Ok(samples)
    }

    /// EXPERIMENTAL (latency): streaming synthesis. Invokes `on_audio` with
    /// PCM deltas as soon as roughly `emit_frames` Flow LM frames (80 ms of
    /// audio each) have been generated and decoded. Concatenated deltas equal
    /// one `synth_chunk` result. The callback runs on the caller thread and
    /// returns `false` to cancel; the function then returns Ok(false).
    pub fn synth_chunk_streaming(
        &self,
        text: &str,
        style: &VoiceStyle,
        emit_frames: usize,
        on_audio: &mut dyn FnMut(Vec<f32>) -> bool,
    ) -> Result<bool, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(true);
        };
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?;
        let chunks = engine.split_prompt(&prepared)?;
        for chunk in chunks {
            let prepared = prepare_april_prompt(&chunk)
                .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
            if !engine.synth_chunk_streaming(&prepared, style, emit_frames, on_audio)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_model_is_april_int8_only() {
        let info = april_model_info();
        assert_eq!(info.max_token_per_chunk, 50);
        assert_eq!(info.sample_rate, SAMPLE_RATE);
        assert!(info
            .artifacts
            .iter()
            .any(|artifact| artifact.filename == "flow_lm_main_int8.onnx"));
        assert!(!info
            .artifacts
            .iter()
            .any(|artifact| artifact.filename == "flow_lm_main.onnx"));
    }

    /// Which splitter each production function delegates to, across the whole
    /// file rather than one hand-picked window.
    ///
    /// A wrong delegation can reinstate either shipped defect in one token:
    /// removing first-sentence priority from playback, or re-isolating sentence
    /// one inside units that already fit. Asserting the whole map means a new
    /// delegation must be declared here to compile green.
    fn splitter_delegations(source: &str) -> Vec<(String, Vec<String>)> {
        let production = source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(production, _)| production);
        // Scan code only. Prose cannot call a splitter, but it can contain
        // ` fn `, which would end a body early and hide a call after it, and it
        // can name a splitter, which would report a call the code never makes.
        let production: String = production
            .lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = Vec::new();
        let mut rest = production.as_str();
        while let Some((_, after)) = rest.split_once(" fn ") {
            let (name, body) = after
                .split_once('(')
                .expect("a function signature has an argument list");
            // End at this function's own closing brace, not at the next ` fn `:
            // a body provably stops where its braces balance, so no later
            // function's calls are attributed here and none of this one's are
            // dropped.
            let inner = body.split_once('{').map_or("", |(_, inner)| inner);
            let mut depth = 1usize;
            let body = inner
                .char_indices()
                .find(|&(_, ch)| {
                    depth = match ch {
                        '{' => depth + 1,
                        '}' => depth - 1,
                        _ => depth,
                    };
                    depth == 0
                })
                .map_or(inner, |(end, _)| &inner[..end]);
            let mut calls = Vec::new();
            // Check the isolating spelling first: ".split_prompt(" is a
            // substring of neither, but a naive contains() on the shorter name
            // would also match the longer one.
            for _ in 0..body.matches(".split_playback_prompt(").count() {
                calls.push("split_playback_prompt".to_string());
            }
            let plain = body.matches(".split_prompt(").count();
            for _ in 0..plain {
                calls.push("split_prompt".to_string());
            }
            if !calls.is_empty() {
                out.push((name.trim().to_string(), calls));
            }
            rest = after;
        }
        out
    }

    #[test]
    fn every_production_splitter_delegation_is_declared() {
        let source = include_str!("pocket.rs");
        let actual = splitter_delegations(source);
        let expected: Vec<(String, Vec<String>)> = vec![
            // Model units: pack sentences, never isolate.
            ("split_text_into_chunks".into(), vec!["split_prompt".into()]),
            // Playback units: isolate sentence one for time-to-first-audio.
            (
                "split_text_for_playback".into(),
                vec!["split_playback_prompt".into()],
            ),
            // Synthesis receives an already-packed unit: re-isolating here
            // re-adds the per-sentence seam this PR removes.
            ("synth_chunk".into(), vec!["split_prompt".into()]),
            ("synth_chunk_streaming".into(), vec!["split_prompt".into()]),
        ];
        assert_eq!(
            actual, expected,
            "a production function changed which splitter it calls (or a new \
             one appeared); isolating outside split_text_for_playback delays \
             first audio, packing inside it removes the guarantee"
        );
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn production_api_emits_non_silent_april_int8_pcm() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to an April INT8 model directory");
        let engine = load_text_to_speech(&dir).expect("load April INT8 engine");
        let style = load_voice_style(&Path::new(&dir).join("reference_sample.wav"))
            .expect("load reference voice");
        let samples = engine
            .synth_chunk("Bright birds begin beside the bay.", "en", &style, 1)
            .expect("synthesize through the production API");

        assert!(!samples.is_empty());
        assert!(samples.iter().all(|sample| sample.is_finite()));
        assert!(samples.iter().any(|sample| sample.abs() > 1.0e-6));
    }
}
