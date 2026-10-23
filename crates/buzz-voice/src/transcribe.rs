//! Offline speech-to-text for Buzz voice notes.
//!
//! A Buzz voice note is a `video/mp4` whose video track is a 16x16 black still
//! and whose audio track carries the speech (the relay has no audio allowlist,
//! so the client wraps speech in the canonical video envelope). Decoding one
//! therefore means reading the AAC track out of an MP4 container, not reading
//! an audio file.
//!
//! [`transcribe`] takes mono samples and returns text. It deliberately does NOT
//! own file IO, model discovery, or the network: the caller supplies samples
//! and a [`TranscribeConfig`], which keeps this module testable and lets
//! `buzz-acp` reuse the exact same recognizer it already uses for huddles.
//!
//! ## Sample rate
//!
//! Models run at a fixed rate (16 kHz for the Whisper/Paraformer family). Audio
//! arriving at any other rate is resampled with `sherpa_onnx::LinearResampler`
//! rather than being rejected, because the canonical Buzz voice note is
//! authored at 24 kHz while the recognizer wants 16 kHz.
//!
//! ## Attribution
//!
//! - `sherpa-onnx` (k2-fsa): Apache-2.0.
//! - Model weights are NOT bundled. They are downloaded separately and their
//!   licenses apply to them (Whisper: MIT; Paraformer: Apache-2.0).

use std::path::PathBuf;

use sherpa_onnx::{
    LinearResampler, OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, Wave,
};

/// Sample rate the bundled model families expect.
pub const TRANSCRIBE_SAMPLE_RATE: u32 = 16_000;

/// Longest audio the recognizer will accept, in seconds.
///
/// A Buzz voice note is a short spoken message; the cap bounds the work a
/// single inbound attachment can cause.
pub const MAX_TRANSCRIBE_SECONDS: f32 = 120.0;

/// Where the ASR model files live, and which model family they belong to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscribeConfig {
    /// Directory holding the model's files.
    pub model_dir: PathBuf,
    /// Model family. See [`ModelType`].
    pub model_type: ModelType,
    /// Optional LM file for models that support one.
    pub lm: Option<PathBuf>,
}

impl TranscribeConfig {
    /// A config for a `sherpa-onnx` Whisper model directory.
    pub fn whisper(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            model_type: ModelType::Whisper,
            lm: None,
        }
    }

    /// A config for a `sherpa-onnx` Paraformer model directory.
    pub fn paraformer(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            model_type: ModelType::Paraformer,
            lm: None,
        }
    }

    /// A config for a `sherpa-onnx` Zipformer transducer model directory.
    pub fn transducer(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            model_type: ModelType::Transducer,
            lm: None,
        }
    }

    /// A config for a `sherpa-onnx` NeMo Parakeet TDT model directory.
    pub fn parakeet(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            model_type: ModelType::Parakeet,
            lm: None,
        }
    }

    /// Configure a NeMo Parakeet TDT-CTC model directory (single
    /// `model*.onnx` + `tokens.txt`).
    pub fn parakeet_ctc(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            model_type: ModelType::ParakeetCtc,
            lm: None,
        }
    }
}

/// The `sherpa-onnx` model family a [`TranscribeConfig`] points at.
///
/// Each family has a different set of files in its model directory; the
/// recognizer needs the right one to be told which family it is loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelType {
    /// Whisper (the family Hermes' `faster-whisper` also uses). The model
    /// directory holds `encoder*.onnx`, `decoder*.onnx` and `tokens.txt`.
    Whisper,
    /// Paraformer. The model directory holds a single `model*.onnx` and
    /// `tokens.txt`.
    Paraformer,
    /// Zipformer transducer. The model directory holds `encoder*.onnx`,
    /// `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`.
    Transducer,
    /// NeMo Parakeet TDT. The model directory holds `encoder*.onnx`,
    /// `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`.
    ///
    /// Parakeet is a transducer model, so it uses the same file set as
    /// [`ModelType::Transducer`]; it differs in the `model_type` string the
    /// sherpa-onnx runtime is given.
    Parakeet,
    /// NeMo Parakeet TDT-CTC. The model directory holds a single `model*.onnx`
    /// (the CTC head) and `tokens.txt`.
    ///
    /// This is the layout the Buzz desktop ships: the
    /// `sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8` release has one
    /// ONNX file, not the encoder/decoder/joiner triple that
    /// [`ModelType::Parakeet`] needs. Pointing a TDT model type at it fails at
    /// model resolution.
    ///
    /// CTC blank-token decoding also removes the silence/cut-audio
    /// hallucination class that hurts encoder-decoder models on short notes.
    ParakeetCtc,
}

impl ModelType {
    /// The `model_type` string `sherpa-onnx` expects.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Paraformer => "paraformer",
            Self::Transducer => "nemo_transducer",
            Self::Parakeet => "nemo_parakeet_tdt",
            Self::ParakeetCtc => "nemo_ctc",
        }
    }
}

/// Errors from [`transcribe`].
#[derive(Debug)]
pub enum TranscribeError {
    /// The model directory does not contain the files the family needs.
    ModelUnavailable(String),
    /// The recognizer could not be created from the supplied config.
    RecognizerUnavailable(String),
    /// The audio could not be decoded or resampled.
    Audio(String),
    /// The recognizer ran but produced no result.
    NoResult,
}

impl std::fmt::Display for TranscribeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelUnavailable(detail) => {
                write!(formatter, "speech model is unavailable: {detail}")
            }
            Self::RecognizerUnavailable(detail) => {
                write!(formatter, "speech recognizer could not start: {detail}")
            }
            Self::Audio(detail) => write!(formatter, "audio could not be prepared: {detail}"),
            Self::NoResult => write!(formatter, "speech recognizer returned no result"),
        }
    }
}

impl std::error::Error for TranscribeError {}

/// Transcribe mono `samples` into text.
///
/// `samples` are mono PCM at `sample_rate`. Audio at any other rate is
/// resampled to [`TRANSCRIBE_SAMPLE_RATE`] first, so a 24 kHz Buzz voice note
/// and a 16 kHz recording both work.
///
/// The returned text is trimmed; an empty string means the recognizer heard no
/// speech. That is a normal outcome (a silent or non-speech note), not an
/// error.
pub fn transcribe(
    samples: &[f32],
    sample_rate: u32,
    config: &TranscribeConfig,
) -> Result<String, TranscribeError> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    if sample_rate == 0 {
        return Err(TranscribeError::Audio(
            "sample rate must be non-zero".to_string(),
        ));
    }

    let max_samples = (MAX_TRANSCRIBE_SECONDS * sample_rate as f32) as usize;
    if samples.len() > max_samples {
        return Err(TranscribeError::Audio(format!(
            "audio is longer than {MAX_TRANSCRIBE_SECONDS} seconds"
        )));
    }

    let resampled = resample_to_model_rate(samples, sample_rate)?;
    let recognizer = build_recognizer(config)?;
    let stream = recognizer.create_stream();
    stream.accept_waveform(TRANSCRIBE_SAMPLE_RATE as i32, &resampled);
    recognizer.decode(&stream);
    let result = stream.get_result().ok_or(TranscribeError::NoResult)?;
    Ok(result.text.trim().to_string())
}

/// Resample mono samples to [`TRANSCRIBE_SAMPLE_RATE`].
fn resample_to_model_rate(samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, TranscribeError> {
    if sample_rate == TRANSCRIBE_SAMPLE_RATE {
        return Ok(samples.to_vec());
    }
    let resampler = LinearResampler::create(sample_rate as i32, TRANSCRIBE_SAMPLE_RATE as i32)
        .ok_or_else(|| {
            TranscribeError::Audio(format!(
                "could not resample {sample_rate} Hz audio to {TRANSCRIBE_SAMPLE_RATE} Hz"
            ))
        })?;
    let mut out = resampler.resample(samples, true);
    if out.is_empty() {
        // A very short clip can flush to nothing at the target rate; fall back to
        // the input so the recognizer still gets something to look at.
        out = samples.to_vec();
    }
    Ok(out)
}

/// Build a recognizer from the config, resolving the model's files on disk.
fn build_recognizer(config: &TranscribeConfig) -> Result<OfflineRecognizer, TranscribeError> {
    let dir = &config.model_dir;
    let tokens = resolve_tokens(dir)?;

    // Build only the family the caller asked for: sherpa-onnx rejects a config
    // whose model_type names a family whose file set is absent.
    let mut model_config = OfflineModelConfig {
        model_type: Some(config.model_type.as_str().to_string()),
        tokens: Some(tokens),
        ..Default::default()
    };
    match config.model_type {
        ModelType::Whisper => {
            model_config.whisper = sherpa_onnx::OfflineWhisperModelConfig {
                encoder: Some(resolve_file(dir, "encoder.onnx")?),
                decoder: Some(resolve_file(dir, "decoder.onnx")?),
                ..Default::default()
            };
        }
        ModelType::Paraformer => {
            model_config.paraformer = sherpa_onnx::OfflineParaformerModelConfig {
                model: Some(resolve_file(dir, "model.onnx")?),
            };
        }
        ModelType::Transducer | ModelType::Parakeet => {
            model_config.transducer = sherpa_onnx::OfflineTransducerModelConfig {
                encoder: Some(resolve_file(dir, "encoder.onnx")?),
                decoder: Some(resolve_file(dir, "decoder.onnx")?),
                joiner: Some(resolve_file(dir, "joiner.onnx")?),
            };
        }
        ModelType::ParakeetCtc => {
            model_config.nemo_ctc = sherpa_onnx::OfflineNemoEncDecCtcModelConfig {
                model: Some(resolve_file(dir, "model.onnx")?),
            };
        }
    }

    let mut recognizer_config = OfflineRecognizerConfig {
        model_config,
        ..Default::default()
    };
    if let Some(lm) = config.lm.as_ref() {
        recognizer_config.lm_config = sherpa_onnx::OfflineLMConfig {
            model: Some(lm.display().to_string()),
            ..Default::default()
        };
    }

    OfflineRecognizer::create(&recognizer_config).ok_or_else(|| {
        TranscribeError::RecognizerUnavailable(format!(
            "sherpa-onnx rejected the model at {}",
            dir.display()
        ))
    })
}

/// Resolve a model file by its role, across the naming conventions the
/// `sherpa-onnx` model releases use.
///
/// Releases name their files either bare (`encoder.int8.onnx`) or prefixed
/// with the model id (`base.en-encoder.int8.onnx`). Both layouts ship in the
/// wild, so the resolver tries the prefixed form first and then the bare one,
/// preferring the int8 variant of either.
///
/// The int8 variant is preferred: it is a fraction of the size and the
/// recognizer's accuracy difference is negligible at this model size.
fn resolve_file(dir: &std::path::Path, name: &str) -> Result<String, TranscribeError> {
    // `encoder.int8.onnx` -> the stem and the int8/full variants to look for.
    let stem = name.trim_end_matches(".onnx");
    let candidates = [
        // Prefixed int8, then prefixed full precision.
        format!("{stem}.int8.onnx"),
        format!("{stem}.onnx"),
    ];

    // Pass 1: exact names (covers the bare layout).
    for candidate in &candidates {
        let path = dir.join(candidate);
        if path.exists() {
            return Ok(path.display().to_string());
        }
    }

    // Pass 2: any file in the directory whose name ENDS WITH one of the
    // candidates (covers the `base.en-` prefixed layout).
    let entries = std::fs::read_dir(dir).map_err(|error| {
        TranscribeError::ModelUnavailable(format!(
            "could not read model directory {}: {error}",
            dir.display()
        ))
    })?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        for candidate in &candidates {
            if file_name.ends_with(candidate.as_str()) {
                return Ok(entry.path().display().to_string());
            }
        }
    }

    Err(TranscribeError::ModelUnavailable(format!(
        "{name} is missing from {}",
        dir.display()
    )))
}

/// Resolve the model's token vocabulary file.
///
/// The vocabulary is always a `.txt` file, and releases name it either
/// `tokens.txt` or `<model-id>-tokens.txt`, so it needs its own resolver
/// rather than the `.onnx` one.
fn resolve_tokens(dir: &std::path::Path) -> Result<String, TranscribeError> {
    let exact = dir.join("tokens.txt");
    if exact.exists() {
        return Ok(exact.display().to_string());
    }
    let entries = std::fs::read_dir(dir).map_err(|error| {
        TranscribeError::ModelUnavailable(format!(
            "could not read model directory {}: {error}",
            dir.display()
        ))
    })?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        if file_name.to_string_lossy().ends_with("-tokens.txt") {
            return Ok(entry.path().display().to_string());
        }
    }
    Err(TranscribeError::ModelUnavailable(format!(
        "tokens.txt is missing from {}",
        dir.display()
    )))
}

/// Transcribe a WAV file from disk. Convenience wrapper for CLI use.
///
/// The file is read as mono PCM; a stereo file's channels are averaged.
pub fn transcribe_wav_file(
    path: &std::path::Path,
    config: &TranscribeConfig,
) -> Result<String, TranscribeError> {
    let wave = Wave::read(&path.display().to_string()).ok_or_else(|| {
        TranscribeError::Audio(format!("could not read WAV file {}", path.display()))
    })?;
    let samples = wave.samples();
    transcribe(samples, wave.sample_rate() as u32, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_type_strings_match_sherpa_onnx_expectations() {
        assert_eq!(ModelType::Whisper.as_str(), "whisper");
        assert_eq!(ModelType::Paraformer.as_str(), "paraformer");
        assert_eq!(ModelType::Transducer.as_str(), "nemo_transducer");
        assert_eq!(ModelType::Parakeet.as_str(), "nemo_parakeet_tdt");
        assert_eq!(ModelType::ParakeetCtc.as_str(), "nemo_ctc");
    }

    #[test]
    fn empty_audio_transcribes_to_empty_text_without_a_model() {
        // No recognizer is needed: there is nothing to recognize, and this must
        // not fail just because the model is missing.
        let config = TranscribeConfig::whisper("/nonexistent/model/dir");
        let result = transcribe(&[], 16_000, &config);
        assert_eq!(result.expect("empty audio is not an error"), "");
    }

    #[test]
    fn zero_sample_rate_is_rejected() {
        let config = TranscribeConfig::whisper("/nonexistent/model/dir");
        let error = transcribe(&[0.0; 100], 0, &config).expect_err("zero rate must fail");
        assert!(
            matches!(error, TranscribeError::Audio(_)),
            "expected an audio error, got {error:?}"
        );
    }

    /// The cap boundary is a PAIR, and the recognizer's guard is a strict `>`.
    ///
    /// Audio of EXACTLY the cap must be accepted (it goes on to the
    /// recognizer), and one sample more must be rejected. Pinning only the
    /// rejection side leaves the guard's comparison operator unconstrained —
    /// flipping `>` to `>=` would silently reject a note that sits exactly
    /// on the limit.
    ///
    /// This boundary matters because ffmpeg's `-t` is INCLUSIVE: a decode
    /// trimmed to exactly the cap produces exactly `max_samples` samples, so
    /// the caller must overshoot the cap to keep this rejection reachable.
    #[test]
    fn the_cap_boundary_is_inclusive_at_the_cap_and_rejected_one_over() {
        let config = TranscribeConfig::whisper("/nonexistent/model/dir");
        let cap_samples = (MAX_TRANSCRIBE_SECONDS as usize) * 16_000;

        // Exactly at the cap: NOT a length rejection. It proceeds far enough
        // to fail on the missing model, which proves the length guard let it
        // through.
        let at_cap = vec![0.0f32; cap_samples];
        let error = transcribe(&at_cap, 16_000, &config)
            .expect_err("a missing model must still fail at the cap");
        assert!(
            !matches!(error, TranscribeError::Audio(ref message) if message.contains("longer than")),
            "audio of exactly the cap must not be rejected for its length, got {error:?}"
        );

        // One sample over: rejected for its length, with a message that
        // names the limit.
        let over_cap = vec![0.0f32; cap_samples + 1];
        let error = transcribe(&over_cap, 16_000, &config)
            .expect_err("audio one sample over the cap must fail");
        assert!(
            matches!(error, TranscribeError::Audio(ref message) if message.contains("longer than")),
            "expected a length rejection, got {error:?}"
        );
    }

    #[test]
    fn a_missing_model_directory_reports_model_unavailable() {
        let config = TranscribeConfig::whisper("/nonexistent/model/dir");
        let error = transcribe(&[0.0; 100], 16_000, &config).expect_err("missing model must fail");
        assert!(
            matches!(error, TranscribeError::ModelUnavailable(_)),
            "expected a model error, got {error:?}"
        );
    }

    #[test]
    fn resampling_to_the_model_rate_preserves_the_sample_count_approximately() {
        let samples: Vec<f32> = (0..16_000).map(|i| (i as f32 / 100.0).sin()).collect();
        let resampled =
            resample_to_model_rate(&samples, 16_000).expect("16 kHz input needs no resampling");
        assert_eq!(resampled.len(), samples.len());
    }

    #[test]
    fn resampling_from_a_different_rate_changes_the_length() {
        let samples: Vec<f32> = (0..24_000).map(|i| (i as f32 / 100.0).sin()).collect();
        let resampled =
            resample_to_model_rate(&samples, 24_000).expect("24 kHz input resamples to 16 kHz");
        // 24 kHz -> 16 kHz is a 2:3 ratio.
        let expected = 24_000 * 2 / 3;
        assert!(
            (resampled.len() as i64 - expected as i64).abs() < expected as i64 / 20,
            "expected roughly {expected} samples, got {}",
            resampled.len()
        );
    }
}
