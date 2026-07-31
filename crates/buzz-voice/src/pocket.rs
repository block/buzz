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

use std::cell::RefCell;
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

thread_local! {
    static ACTIVE_SYNTHESIS_ENGINES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct SynthesisCallGuard {
    engine_id: usize,
}

impl SynthesisCallGuard {
    fn enter(engine_id: usize) -> Result<Self, String> {
        ACTIVE_SYNTHESIS_ENGINES.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&engine_id) {
                return Err("Pocket TTS callback re-entered the active engine".to_string());
            }
            active.push(engine_id);
            Ok(Self { engine_id })
        })
    }

    fn is_active(engine_id: usize) -> bool {
        ACTIVE_SYNTHESIS_ENGINES.with(|active| active.borrow().contains(&engine_id))
    }
}

impl Drop for SynthesisCallGuard {
    fn drop(&mut self) {
        ACTIVE_SYNTHESIS_ENGINES.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) = active.iter().rposition(|engine| *engine == self.engine_id) {
                active.remove(index);
            }
        });
    }
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
        inner: Mutex::new(AprilPocketTts::load(&dir, TTS_NUM_THREADS)?),
    })
}

impl PocketTts {
    /// Split text into synthesis units that satisfy the bundle's exact
    /// 50-token input limit.
    pub fn split_text_into_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        if SynthesisCallGuard::is_active(self as *const Self as usize) {
            return Err("Pocket TTS callback re-entered the active engine".to_string());
        }
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
        lang: &str,
        style: &VoiceStyle,
        steps: usize,
    ) -> Result<Vec<f32>, String> {
        self.synth_chunk_with_callback(text, lang, style, steps, None::<fn(&[f32], f32) -> bool>)
    }

    /// Synthesize text, allowing the caller to stop generation early.
    ///
    /// The callback receives PCM accumulated after each decoded text chunk
    /// and progress in `[0, 1]`. During latent generation the callback is
    /// invoked with an empty sample slice so cancellation can remain
    /// responsive before PCM is available. Return `true` to continue or
    /// `false` to stop and return the audio generated so far. Progress is
    /// monotonic across split text chunks. Calls back into the same
    /// [`PocketTts`] return an error instead of blocking on its engine lock.
    pub fn synth_chunk_with_callback<F>(
        &self,
        text: &str,
        _lang: &str,
        style: &VoiceStyle,
        _steps: usize,
        mut callback: Option<F>,
    ) -> Result<Vec<f32>, String>
    where
        F: FnMut(&[f32], f32) -> bool + 'static,
    {
        let _call_guard = SynthesisCallGuard::enter(self as *const Self as usize)?;
        let Some(prepared) = prepare_april_prompt(text) else {
            return Ok(Vec::new());
        };
        let mut engine = self
            .inner
            .lock()
            .map_err(|_| "Pocket TTS engine lock poisoned".to_string())?;
        let chunks = engine.split_prompt(&prepared)?;
        let mut samples = Vec::new();
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.into_iter().enumerate() {
            let prepared = prepare_april_prompt(&chunk)
                .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
            let progress_offset = index as f32 / chunk_count as f32;
            let progress_scale = 1.0 / chunk_count as f32;
            let (chunk_samples, cancelled) = engine.synth_chunk_with_callback(
                &prepared,
                style,
                &mut callback,
                progress_offset,
                progress_scale,
            )?;
            samples.extend(chunk_samples);
            if cancelled {
                break;
            }
            let progress = (index + 1) as f32 / chunk_count as f32;
            if !callback_allows_progress(&mut callback, &samples, progress)? {
                break;
            }
        }
        Ok(samples)
    }
}

fn callback_allows_progress<F>(
    callback: &mut Option<F>,
    samples: &[f32],
    progress: f32,
) -> Result<bool, String>
where
    F: FnMut(&[f32], f32) -> bool,
{
    let Some(callback) = callback.as_mut() else {
        return Ok(true);
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(samples, progress)))
        .map_err(|_| "Pocket TTS synthesis callback panicked".to_string())
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
    fn callback_can_cancel_before_pcm_is_available() {
        let mut callback = Some(|samples: &[f32], progress: f32| {
            assert!(samples.is_empty());
            assert_eq!(progress, 0.25);
            false
        });
        assert!(!callback_allows_progress(&mut callback, &[], 0.25).expect("callback"));
    }

    #[test]
    fn callback_panic_is_reported_without_unwinding() {
        let mut callback = Some(|_: &[f32], _: f32| -> bool {
            panic!("callback failure");
        });
        assert_eq!(
            callback_allows_progress(&mut callback, &[], 0.0).unwrap_err(),
            "Pocket TTS synthesis callback panicked"
        );
    }

    #[test]
    fn active_engine_reentry_is_rejected() {
        let _guard = SynthesisCallGuard::enter(42).expect("first call");
        assert!(SynthesisCallGuard::enter(42).is_err());
        assert!(SynthesisCallGuard::is_active(42));
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
