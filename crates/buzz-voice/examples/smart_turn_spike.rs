//! One-shot feasibility probe for Pipecat Smart Turn v3 through Buzz's linked ONNX Runtime.
//!
//! This is intentionally not production integration. It proves that the pinned
//! CPU graph loads and runs through the `ort` already linked by `buzz-voice`.

use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use ort::session::Session;
use ort::value::Tensor;
use prost::Message;
use sha2::{Digest, Sha256};

const MODEL_FILENAME: &str = "smart-turn-v3.1-cpu.onnx";
const MODEL_REVISION: &str = "16c8130e06d3af59663be3fd7c9ac80624850e6c";
const MODEL_SHA256: &str = "fb68d55c2d542ce79e44b12013bfd571e90df8594ab096d757198e851b0c6594";
const MODEL_SOURCE: &str = "https://huggingface.co/pipecat-ai/smart-turn-v3";
const MODEL_SAMPLE_RATE: usize = 16_000;
const MODEL_SECONDS: usize = 8;
const MODEL_MELS: usize = 80;
const MODEL_FRAMES: usize = 800;

#[derive(Clone, PartialEq, Message)]
struct ModelProto {
    #[prost(message, repeated, tag = "8")]
    opset_import: Vec<OperatorSetIdProto>,
}

#[derive(Clone, PartialEq, Message)]
struct OperatorSetIdProto {
    #[prost(string, tag = "1")]
    domain: String,
    #[prost(int64, tag = "2")]
    version: i64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let model_path = env::args().nth(1).ok_or_else(|| {
        format!(
            "usage: cargo run -p buzz-voice --example smart_turn_spike -- /path/{MODEL_FILENAME}"
        )
    })?;
    let model_path = Path::new(&model_path);
    let model_bytes = fs::read(model_path)?;
    let model_sha256 = hex::encode(Sha256::digest(&model_bytes));
    if model_sha256 != MODEL_SHA256 {
        return Err(
            format!("model SHA-256 mismatch: expected {MODEL_SHA256}, got {model_sha256}").into(),
        );
    }

    let model = ModelProto::decode(model_bytes.as_slice())?;
    let opsets = model
        .opset_import
        .iter()
        .map(|opset| {
            let domain = if opset.domain.is_empty() {
                "ai.onnx"
            } else {
                &opset.domain
            };
            format!("{domain}:{}", opset.version)
        })
        .collect::<Vec<_>>()
        .join(",");

    println!("model_source={MODEL_SOURCE}");
    println!("model_filename={MODEL_FILENAME}");
    println!("model_revision={MODEL_REVISION}");
    println!("model_sha256={model_sha256}");
    println!("model_opsets={opsets}");
    // Keep sherpa-onnx in the final link so `ort` resolves against the same
    // bundled static ONNX Runtime used by the desktop speech stack.
    println!("linked_sherpa_onnx={}", sherpa_onnx::version());
    println!("loaded_ort_api=1.{}", ort::MINOR_VERSION);
    println!("loaded_ort_build={}", ort::info());

    let load_started = Instant::now();
    let mut session = Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_file(model_path)?;
    println!("session_load_ms={:.3}", elapsed_ms(load_started));

    for input in session.inputs() {
        println!("input={} dtype={}", input.name(), input.dtype());
    }
    for output in session.outputs() {
        println!("output={} dtype={}", output.name(), output.dtype());
    }
    if session.inputs().len() != 1 || session.inputs()[0].name() != "input_features" {
        return Err("unexpected Smart Turn input contract".into());
    }

    // A one-second 16 kHz silence buffer becomes eight seconds after the
    // specified left zero-padding. Whisper normalization keeps it at zero;
    // the log-mel floor and scaling therefore produce -1.5 in every bin.
    let pcm_16k = vec![0.0_f32; MODEL_SAMPLE_RATE];
    let input_features = exact_silence_log_mel(&pcm_16k)?;
    let input = Tensor::from_array((
        vec![1_i64, MODEL_MELS as i64, MODEL_FRAMES as i64],
        input_features.into_boxed_slice(),
    ))?;

    let inference_started = Instant::now();
    let outputs = session.run(ort::inputs!["input_features" => input])?;
    let inference_ms = elapsed_ms(inference_started);
    let (_, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
    let probability = probabilities
        .first()
        .copied()
        .ok_or("Smart Turn returned an empty output")?;
    let decision = if probability > 0.5 { "SHIFT" } else { "HOLD" };

    println!("fixture_pcm_samples={}", pcm_16k.len());
    println!(
        "fixture_padded_samples={}",
        MODEL_SAMPLE_RATE * MODEL_SECONDS
    );
    println!("completion_probability={probability:.6}");
    println!("decision={decision}");
    println!("inference_wall_ms={inference_ms:.3}");
    Ok(())
}

fn exact_silence_log_mel(pcm_16k: &[f32]) -> Result<Vec<f32>, Box<dyn Error>> {
    if pcm_16k.len() > MODEL_SAMPLE_RATE * MODEL_SECONDS {
        return Err("silence fixture exceeds Smart Turn's eight-second window".into());
    }
    if pcm_16k.iter().any(|sample| *sample != 0.0) {
        return Err("the spike only implements the exact silence fixture frontend".into());
    }
    Ok(vec![-1.5; MODEL_MELS * MODEL_FRAMES])
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
