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
        inner: Mutex::new(AprilPocketTts::load(&dir, TTS_NUM_THREADS)?),
    })
}

impl PocketTts {
    /// Split text into synthesis units that satisfy the bundle's exact
    /// 50-token input limit.
    pub fn split_text_into_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        self.inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?
            .split_prompt(&prepared)
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
