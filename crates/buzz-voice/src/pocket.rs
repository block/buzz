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

use pocket_april::{prepare_april_prompt, AprilPocketTts, AprilSynthesisOutcome};
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

/// Result of a callback-driven Pocket synthesis request.
#[derive(Debug, Clone, PartialEq)]
pub enum SynthesisOutcome {
    /// Synthesis finished and contains the same PCM exposed cumulatively to
    /// the callback.
    Complete(Vec<f32>),
    /// The callback requested cancellation before synthesis completed.
    Interrupted,
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
    ///
    /// The first sentence remains its own unit when it fits. Oversized
    /// sentences fall back to clause, word, and UTF-8 scalar boundaries, while
    /// later sentences pack into the largest natural unit that fits.
    ///
    /// Chunks are contiguous substrings of the prepared model prompt and may
    /// retain boundary whitespace. Concatenating them with `chunks.concat()`
    /// reconstructs that prompt exactly, and each chunk's prepared token count
    /// is at most 50.
    pub fn split_text_into_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        self.inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?
            .split_playback_prompt(&prepared)
    }

    /// Group prepared text into playback units.
    ///
    /// A leading sentence remains separate so playback can begin promptly.
    /// The remaining prompt stays in one playback unit; synthesis applies the
    /// model's 50-token split internally without introducing playback pauses at
    /// those model-only boundaries.
    pub fn split_text_into_playback_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        Ok(self
            .inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?
            .group_playback_prompt(&prepared))
    }

    /// Synthesize text with the supplied reference voice.
    ///
    /// Pocket detects language from text and this model uses one synthesis
    /// step, so `_lang` and `_steps` intentionally do not affect output.
    pub fn synth_chunk(
        &self,
        text: &str,
        lang: &str,
        style: &VoiceStyle,
        steps: usize,
    ) -> Result<Vec<f32>, String> {
        match self.synth_chunk_streaming(text, lang, style, steps, |_, _| true)? {
            SynthesisOutcome::Complete(samples) => Ok(samples),
            SynthesisOutcome::Interrupted => Ok(Vec::new()),
        }
    }

    /// Synthesize text while reporting cumulative PCM as decoder blocks finish.
    ///
    /// Callback sample buffers contain all PCM produced for this call so far.
    /// Their lengths never decrease, but equal lengths are allowed while the
    /// engine advances between internal model-safe text chunks. Returning
    /// `false` interrupts synthesis before the next decoder block.
    pub fn synth_chunk_streaming<F>(
        &self,
        text: &str,
        _lang: &str,
        style: &VoiceStyle,
        _steps: usize,
        mut callback: F,
    ) -> Result<SynthesisOutcome, String>
    where
        F: FnMut(&[f32], f32) -> bool,
    {
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(SynthesisOutcome::Complete(Vec::new()));
        };
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?;
        let chunks = engine.split_prompt(&prepared)?;
        let chunk_count = chunks.len();
        let mut samples = Vec::new();
        for (chunk_index, chunk) in chunks.into_iter().enumerate() {
            if chunk_index > 0 && !callback(&samples, chunk_index as f32 / chunk_count as f32) {
                return Ok(SynthesisOutcome::Interrupted);
            }
            let prepared = prepare_april_prompt(&chunk)
                .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
            let outcome = engine.synth_chunk_streaming(&prepared, style, |block, progress| {
                samples.extend_from_slice(block);
                callback(
                    &samples,
                    (chunk_index as f32 + progress) / chunk_count as f32,
                )
            })?;
            if matches!(outcome, AprilSynthesisOutcome::Interrupted) {
                return Ok(SynthesisOutcome::Interrupted);
            }
        }
        Ok(SynthesisOutcome::Complete(samples))
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

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn production_streaming_callbacks_are_cumulative_across_model_chunks() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to an April INT8 model directory");
        let engine = load_text_to_speech(&dir).expect("load April INT8 engine");
        let style = load_voice_style(&Path::new(&dir).join("reference_sample.wav"))
            .expect("load reference voice");
        engine
            .synth_chunk("Warm up.", "en", &style, 1)
            .expect("warm production engine");
        let text = "And sometimes, when I am certain the reader is rested, I will engage him with a sentence of considerable length, a sentence that burns with energy and builds with all the impetus of a crescendo, the roll of the drums, the crash of the cymbals–sounds that say listen to this, it is important.";
        let mut reconstructed = Vec::new();
        let mut previous_len = 0;
        let mut saw_equal_repeat = false;
        let mut callback_count = 0;
        let mut first_callback = None;
        let started = std::time::Instant::now();

        let outcome = engine
            .synth_chunk_streaming(text, "en", &style, 1, |cumulative, _| {
                callback_count += 1;
                first_callback.get_or_insert_with(|| started.elapsed());
                assert!(cumulative.len() >= previous_len);
                saw_equal_repeat |= cumulative.len() == previous_len;
                reconstructed.extend_from_slice(&cumulative[previous_len..]);
                previous_len = cumulative.len();
                true
            })
            .expect("stream through the production API");
        let SynthesisOutcome::Complete(samples) = outcome else {
            panic!("uninterrupted synthesis must complete");
        };
        let total = started.elapsed();
        let first_callback = first_callback.expect("decoder must produce a callback");
        let audio_duration =
            std::time::Duration::from_secs_f64(samples.len() as f64 / SAMPLE_RATE as f64);
        eprintln!(
            "first_callback_ms={:.1} total_ms={:.1} audio_seconds={:.3} rtf={:.3} callbacks={callback_count}",
            first_callback.as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0,
            audio_duration.as_secs_f64(),
            total.as_secs_f64() / audio_duration.as_secs_f64(),
        );

        assert!(saw_equal_repeat);
        assert_eq!(reconstructed, samples);
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn production_streaming_callback_interrupts_after_first_decoder_block() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to an April INT8 model directory");
        let engine = load_text_to_speech(&dir).expect("load April INT8 engine");
        let style = load_voice_style(&Path::new(&dir).join("reference_sample.wav"))
            .expect("load reference voice");
        engine
            .synth_chunk("Warm up.", "en", &style, 1)
            .expect("warm production engine");
        let mut callback_at = None;
        let started = std::time::Instant::now();

        let outcome = engine
            .synth_chunk_streaming(
                "This sentence is long enough to require more than one decoder block.",
                "en",
                &style,
                1,
                |_, _| {
                    callback_at = Some(started.elapsed());
                    false
                },
            )
            .expect("interrupt production streaming");
        let callback_at = callback_at.expect("decoder must produce a callback");
        let cancellation_latency = started.elapsed().saturating_sub(callback_at);
        eprintln!(
            "callback_to_cancel_return_ms={:.1}",
            cancellation_latency.as_secs_f64() * 1000.0
        );

        assert!(matches!(outcome, SynthesisOutcome::Interrupted));
    }
}
