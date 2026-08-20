//! Local Smart Turn v3.1 CPU inference.
//!
//! The frontend mirrors the pinned Pipecat v3.1 preprocessing contract:
//! retain the last eight seconds, left-pad, normalize the complete padded
//! waveform, then compute Whisper's 80 x 800 Slaney log-mel tensor.

use std::path::Path;

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use realfft::RealFftPlanner;

const SAMPLE_RATE: usize = 16_000;
const WINDOW_SAMPLES: usize = SAMPLE_RATE * 8;
const FFT_LENGTH: usize = 400;
const HOP_LENGTH: usize = 160;
const FREQUENCY_BINS: usize = FFT_LENGTH / 2 + 1;
const MEL_BINS: usize = 80;
const MODEL_FRAMES: usize = WINDOW_SAMPLES / HOP_LENGTH;
const CENTER_PADDING: usize = FFT_LENGTH / 2;
const SPECTROGRAM_FRAMES: usize = MODEL_FRAMES + 1;

/// Smart Turn's binary endpoint decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartTurnDecision {
    Hold,
    Shift,
}

/// A loaded Smart Turn v3.1 CPU session.
pub struct SmartTurnClassifier {
    session: Session,
}

impl SmartTurnClassifier {
    /// Load the pinned model through the ONNX Runtime already linked by sherpa-onnx.
    pub fn load(model_path: &Path) -> Result<Self, String> {
        // `ort-sys` uses `disable-linking`; this reference keeps sherpa-onnx's
        // bundled static ONNX Runtime in the final desktop link.
        let _linked_runtime_version = sherpa_onnx::version();
        let session = Session::builder()
            .map_err(ort_error)?
            .with_intra_threads(1)
            .map_err(ort_error)?
            .with_inter_threads(1)
            .map_err(ort_error)?
            .with_parallel_execution(false)
            .map_err(ort_error)?
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(ort_error)?
            .commit_from_file(model_path)
            .map_err(ort_error)?;

        if session.inputs().len() != 1 || session.inputs()[0].name() != "input_features" {
            return Err("Smart Turn model has an unexpected input contract".to_string());
        }
        if session.outputs().len() != 1 || session.outputs()[0].name() != "logits" {
            return Err("Smart Turn model has an unexpected output contract".to_string());
        }

        Ok(Self { session })
    }

    /// Classify one accumulated 16 kHz utterance.
    ///
    /// The model output is already a sigmoid probability despite its `logits`
    /// tensor name. Applying another sigmoid would change the trained threshold.
    pub fn classify(&mut self, pcm_16k: &[f32]) -> Result<(SmartTurnDecision, f32), String> {
        let features = smart_turn_log_mel(pcm_16k)?;
        let input = Tensor::from_array((
            vec![1_i64, MEL_BINS as i64, MODEL_FRAMES as i64],
            features.into_boxed_slice(),
        ))
        .map_err(ort_error)?;
        let outputs = self
            .session
            .run(ort::inputs!["input_features" => input])
            .map_err(ort_error)?;
        let (_, probabilities) = outputs[0].try_extract_tensor::<f32>().map_err(ort_error)?;
        let probability = probabilities
            .first()
            .copied()
            .ok_or_else(|| "Smart Turn returned an empty output".to_string())?;
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err(format!(
                "Smart Turn returned an invalid completion probability: {probability}"
            ));
        }
        Ok((decision_from_probability(probability), probability))
    }
}

/// Convert 16 kHz mono PCM into Smart Turn's `(80, 800)` row-major tensor.
pub fn smart_turn_log_mel(pcm_16k: &[f32]) -> Result<Vec<f32>, String> {
    let mut waveform = retain_and_left_pad(pcm_16k);
    normalize_waveform(&mut waveform);

    let window = periodic_hann();
    let mel_filters = slaney_mel_filter_bank();
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(FFT_LENGTH);
    let mut fft_input = fft.make_input_vec();
    let mut fft_output = fft.make_output_vec();
    let mut mel_spectrogram = vec![0.0_f64; MEL_BINS * SPECTROGRAM_FRAMES];

    for frame in 0..SPECTROGRAM_FRAMES {
        let start = frame * HOP_LENGTH;
        for (sample, (slot, coefficient)) in
            centered_frame(&waveform, start).zip(fft_input.iter_mut().zip(window.iter()))
        {
            *slot = sample as f64 * coefficient;
        }
        fft.process(&mut fft_input, &mut fft_output)
            .map_err(|error| format!("Smart Turn FFT failed: {error}"))?;

        for mel in 0..MEL_BINS {
            let mut energy = 0.0_f64;
            for frequency in 0..FREQUENCY_BINS {
                // NumPy's reference stores each FFT frame in complex64 before
                // taking the f64 magnitude. Preserve that quantization point.
                let re = fft_output[frequency].re as f32 as f64;
                let im = fft_output[frequency].im as f32 as f64;
                let power = re.mul_add(re, im * im);
                energy += mel_filters[frequency * MEL_BINS + mel] * power;
            }
            mel_spectrogram[mel * SPECTROGRAM_FRAMES + frame] = energy.max(1e-10).log10();
        }
    }

    // Whisper drops frame 800, floors relative to this clip's maximum, then
    // applies the model's affine scale.
    let clip_max = mel_spectrogram
        .chunks_exact(SPECTROGRAM_FRAMES)
        .flat_map(|mel| &mel[..MODEL_FRAMES])
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let floor = clip_max - 8.0;
    let mut features = Vec::with_capacity(MEL_BINS * MODEL_FRAMES);
    for mel in mel_spectrogram.chunks_exact(SPECTROGRAM_FRAMES) {
        features.extend(
            mel[..MODEL_FRAMES]
                .iter()
                .map(|value| ((value.max(floor) + 4.0) / 4.0) as f32),
        );
    }
    Ok(features)
}

fn decision_from_probability(probability: f32) -> SmartTurnDecision {
    if probability > 0.5 {
        SmartTurnDecision::Shift
    } else {
        SmartTurnDecision::Hold
    }
}

fn retain_and_left_pad(pcm_16k: &[f32]) -> Vec<f32> {
    let retained = if pcm_16k.len() > WINDOW_SAMPLES {
        &pcm_16k[pcm_16k.len() - WINDOW_SAMPLES..]
    } else {
        pcm_16k
    };
    let mut padded = vec![0.0; WINDOW_SAMPLES - retained.len()];
    padded.extend_from_slice(retained);
    padded
}

fn normalize_waveform(waveform: &mut [f32]) {
    let mean =
        (waveform.iter().map(|sample| *sample as f64).sum::<f64>() / waveform.len() as f64) as f32;
    let variance = (waveform
        .iter()
        .map(|sample| {
            let centered = *sample - mean;
            centered * centered
        })
        .map(f64::from)
        .sum::<f64>()
        / waveform.len() as f64) as f32;
    let scale = (f64::from(variance) + 1e-7).sqrt() as f32;
    for sample in waveform {
        *sample = (*sample - mean) / scale;
    }
}

fn periodic_hann() -> [f64; FFT_LENGTH] {
    std::array::from_fn(|index| {
        0.5 - 0.5 * (2.0 * std::f64::consts::PI * index as f64 / FFT_LENGTH as f64).cos()
    })
}

fn centered_frame(waveform: &[f32], start: usize) -> impl Iterator<Item = f32> + '_ {
    (start..start + FFT_LENGTH).map(|padded_index| {
        if padded_index < CENTER_PADDING {
            waveform[CENTER_PADDING - padded_index]
        } else if padded_index < CENTER_PADDING + waveform.len() {
            waveform[padded_index - CENTER_PADDING]
        } else {
            let right_offset = padded_index - (CENTER_PADDING + waveform.len());
            waveform[waveform.len() - 2 - right_offset]
        }
    })
}

fn slaney_mel_filter_bank() -> Vec<f64> {
    let mel_min = hertz_to_slaney_mel(0.0);
    let mel_max = hertz_to_slaney_mel(8_000.0);
    let filter_frequencies = (0..MEL_BINS + 2)
        .map(|index| {
            let mel = mel_min + (mel_max - mel_min) * index as f64 / (MEL_BINS + 1) as f64;
            slaney_mel_to_hertz(mel)
        })
        .collect::<Vec<_>>();
    let fft_frequencies = (0..FREQUENCY_BINS)
        .map(|index| 8_000.0 * index as f64 / (FREQUENCY_BINS - 1) as f64)
        .collect::<Vec<_>>();
    let mut filters = vec![0.0; FREQUENCY_BINS * MEL_BINS];

    for frequency in 0..FREQUENCY_BINS {
        for mel in 0..MEL_BINS {
            let lower_width = filter_frequencies[mel + 1] - filter_frequencies[mel];
            let upper_width = filter_frequencies[mel + 2] - filter_frequencies[mel + 1];
            let down = (fft_frequencies[frequency] - filter_frequencies[mel]) / lower_width;
            let up = (filter_frequencies[mel + 2] - fft_frequencies[frequency]) / upper_width;
            let triangle = down.min(up).max(0.0);
            let slaney_norm = 2.0 / (filter_frequencies[mel + 2] - filter_frequencies[mel]);
            filters[frequency * MEL_BINS + mel] = triangle * slaney_norm;
        }
    }
    filters
}

fn hertz_to_slaney_mel(hertz: f64) -> f64 {
    if hertz >= 1_000.0 {
        15.0 + (hertz / 1_000.0).ln() * (27.0 / 6.4_f64.ln())
    } else {
        3.0 * hertz / 200.0
    }
}

fn slaney_mel_to_hertz(mel: f64) -> f64 {
    if mel >= 15.0 {
        1_000.0 * ((6.4_f64.ln() / 27.0) * (mel - 15.0)).exp()
    } else {
        200.0 * mel / 3.0
    }
}

fn ort_error<R>(error: ort::Error<R>) -> String {
    format!("Smart Turn ONNX Runtime error: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const INPUT_BYTES: &[u8] = include_bytes!("../testdata/smart_turn_input_440hz_1s.f32");
    const GOLDEN_BYTES: &[u8] = include_bytes!("../testdata/smart_turn_logmel_golden_440hz.f32");

    fn f32_fixture(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte f32")))
            .collect()
    }

    #[test]
    fn g4_1_log_mel_matches_pinned_python_frontend() {
        assert_eq!(
            hex::encode(Sha256::digest(INPUT_BYTES)),
            "79414a32cd6b6bee5fd82e3a74a35fe61e1b59f70b22a4c91d848a3dbd9de5c1"
        );
        assert_eq!(
            hex::encode(Sha256::digest(GOLDEN_BYTES)),
            "52a43e334909828506b74750cab67cbe4999d440044c11b93f61fdbff3c868cc"
        );
        let actual = smart_turn_log_mel(&f32_fixture(INPUT_BYTES)).expect("extract features");
        let expected = f32_fixture(GOLDEN_BYTES);
        assert_eq!(actual.len(), MEL_BINS * MODEL_FRAMES);
        assert_eq!(actual.len(), expected.len());

        let min = actual.iter().copied().fold(f32::INFINITY, f32::min);
        let max = actual.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(((max - min) - 2.0).abs() <= 1e-6, "unexpected feature span");
        assert!((actual[750] - expected[750]).abs() <= 1e-5);

        let (index, max_abs_error) = actual
            .iter()
            .zip(&expected)
            .enumerate()
            .map(|(index, (actual, expected))| (index, (actual - expected).abs()))
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .expect("non-empty feature tensor");
        eprintln!(
            "G4.1 max abs error {max_abs_error} at row-major index {index} (mel {}, frame {})",
            index / MODEL_FRAMES,
            index % MODEL_FRAMES
        );
        assert!(
            max_abs_error <= 1e-5,
            "max abs error {max_abs_error} at row-major index {index} (mel {}, frame {})",
            index / MODEL_FRAMES,
            index % MODEL_FRAMES
        );
    }

    #[test]
    fn model_probability_is_not_sigmoided_again() {
        assert_eq!(decision_from_probability(0.5), SmartTurnDecision::Hold);
        assert_eq!(
            decision_from_probability(0.500_001),
            SmartTurnDecision::Shift
        );
    }

    #[test]
    #[ignore = "requires SMART_TURN_MODEL_PATH to point to the pinned external ONNX graph"]
    fn classifier_runs_pinned_external_model() {
        let path = std::env::var("SMART_TURN_MODEL_PATH").expect("SMART_TURN_MODEL_PATH");
        let mut classifier = SmartTurnClassifier::load(Path::new(&path)).expect("load classifier");
        let (decision, probability) = classifier
            .classify(&f32_fixture(INPUT_BYTES))
            .expect("run classifier");

        eprintln!("completion probability {probability}, decision {decision:?}");
        assert!(probability.is_finite());
        assert_eq!(decision, decision_from_probability(probability));
    }
}
