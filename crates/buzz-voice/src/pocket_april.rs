//! Native ONNX loader for Pocket TTS `english_2026-04`.
//!
//! The bundle uses SentencePiece, prepends a learned BOS voice embedding, and
//! describes recurrent state tensors in `bundle.json`. This module supplies
//! that frontend and state loop while reusing the ONNX Runtime linked by the
//! Desktop speech stack.

use std::borrow::Cow;
use std::f32::consts::TAU;
use std::fs;
use std::path::{Path, PathBuf};

use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use rand::{Rng, RngExt};
use sentencepiece_model::SentencePieceModel;
use serde::Deserialize;
use sherpa_onnx::LinearResampler;
use tokenizers::models::unigram::Unigram;
use tokenizers::pre_tokenizers::metaspace::{Metaspace, PrependScheme};
use tokenizers::Tokenizer;

use super::VoiceStyle;

const FILE_BUNDLE: &str = "bundle.json";
const FILE_MIMI_ENCODER: &str = "mimi_encoder.onnx";
const FILE_TEXT_CONDITIONER: &str = "text_conditioner.onnx";
const FILE_FLOW_MAIN_INT8: &str = "flow_lm_main_int8.onnx";
const FILE_FLOW_INT8: &str = "flow_lm_flow_int8.onnx";
const FILE_MIMI_DECODER_INT8: &str = "mimi_decoder_int8.onnx";

const MODEL_LANGUAGE: &str = "english_2026-04";
const DEFAULT_TEMPERATURE: f32 = 0.7;
const EOS_LOGIT_THRESHOLD: f32 = -4.0;
const DECODER_CHUNK_FRAMES: usize = 12;
const TOKENS_PER_SECOND_ESTIMATE: f32 = 3.0;
const GENERATION_SECONDS_PADDING: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextBoundary {
    Sentence,
    Clause,
    Word,
}

#[derive(Debug, Deserialize)]
struct Bundle {
    schema_version: u32,
    language: String,
    sample_rate: usize,
    frame_rate: f32,
    samples_per_frame: usize,
    latent_dim: usize,
    conditioning_dim: usize,
    insert_bos_before_voice: bool,
    pad_with_spaces_for_short_inputs: bool,
    remove_semicolons: bool,
    model_recommended_frames_after_eos: Option<usize>,
    max_token_per_chunk: usize,
    tokenizer_file: String,
    bos_before_voice_file: String,
    flow_lm_state_manifest: Vec<StateSpec>,
    mimi_state_manifest: Vec<StateSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct StateSpec {
    input_name: String,
    output_name: String,
    dtype: StateDtype,
    shape: Vec<i64>,
    fill: StateFill,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum StateDtype {
    #[serde(rename = "float32")]
    Float32,
    #[serde(rename = "int64")]
    Int64,
    Bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum StateFill {
    Empty,
    Nan,
    Ones,
    Zeros,
}

struct StateValue {
    spec: StateSpec,
    value: DynValue,
}

/// Stable identity for a reference voice: a content hash of the sample
/// buffer plus its length and rate. Buffer addresses are NOT part of the
/// key — voice switching clones and drops sample buffers, so the allocator
/// can hand a different voice the same address, and an address-based key
/// would then restore the previous voice's cached state.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
struct VoiceKey {
    content_hash: u64,
    samples_len: usize,
    sample_rate: i32,
}

fn voice_key(style: &VoiceStyle) -> VoiceKey {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    for sample in &style.samples {
        hasher.write_u32(sample.to_bits());
    }
    VoiceKey {
        content_hash: hasher.finish(),
        samples_len: style.samples.len(),
        sample_rate: style.sample_rate,
    }
}

struct CachedVoice {
    key: VoiceKey,
    embeddings: Vec<f32>,
}

/// EXPERIMENTAL (latency): a dtype-tagged copy of one recurrent state tensor,
/// used to snapshot the Flow LM state right after voice conditioning so
/// subsequent chunks skip the ~160 ms `condition_voice` pass entirely.
enum SnapshotTensor {
    F32(Vec<i64>, Vec<f32>),
    I64(Vec<i64>, Vec<i64>),
    Bool(Vec<i64>, Vec<bool>),
}

struct CachedConditioning {
    key: VoiceKey,
    state: Vec<(StateSpec, SnapshotTensor)>,
}

fn snapshot_state(state: &[StateValue]) -> Result<Vec<(StateSpec, SnapshotTensor)>, String> {
    state
        .iter()
        .map(|value| {
            let tensor = match value.spec.dtype {
                StateDtype::Float32 => {
                    let (shape, data) = value
                        .value
                        .try_extract_tensor::<f32>()
                        .map_err(ort_error("snapshot f32 state"))?;
                    SnapshotTensor::F32(shape.to_vec(), data.to_vec())
                }
                StateDtype::Int64 => {
                    let (shape, data) = value
                        .value
                        .try_extract_tensor::<i64>()
                        .map_err(ort_error("snapshot i64 state"))?;
                    SnapshotTensor::I64(shape.to_vec(), data.to_vec())
                }
                StateDtype::Bool => {
                    let (shape, data) = value
                        .value
                        .try_extract_tensor::<bool>()
                        .map_err(ort_error("snapshot bool state"))?;
                    SnapshotTensor::Bool(shape.to_vec(), data.to_vec())
                }
            };
            Ok((value.spec.clone(), tensor))
        })
        .collect()
}

fn restore_state(snapshot: &[(StateSpec, SnapshotTensor)]) -> Result<Vec<StateValue>, String> {
    snapshot
        .iter()
        .map(|(spec, tensor)| {
            let value = match tensor {
                SnapshotTensor::F32(shape, data) => {
                    if data.is_empty() {
                        Tensor::<f32>::new(&ort::memory::Allocator::default(), shape.clone())
                            .map_err(ort_error("restore empty f32 state"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((shape.clone(), data.clone().into_boxed_slice()))
                            .map_err(ort_error("restore f32 state"))?
                            .into_dyn()
                    }
                }
                SnapshotTensor::I64(shape, data) => {
                    if data.is_empty() {
                        Tensor::<i64>::new(&ort::memory::Allocator::default(), shape.clone())
                            .map_err(ort_error("restore empty i64 state"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((shape.clone(), data.clone().into_boxed_slice()))
                            .map_err(ort_error("restore i64 state"))?
                            .into_dyn()
                    }
                }
                SnapshotTensor::Bool(shape, data) => {
                    if data.is_empty() {
                        Tensor::<bool>::new(&ort::memory::Allocator::default(), shape.clone())
                            .map_err(ort_error("restore empty bool state"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((shape.clone(), data.clone().into_boxed_slice()))
                            .map_err(ort_error("restore bool state"))?
                            .into_dyn()
                    }
                }
            };
            Ok(StateValue {
                spec: spec.clone(),
                value,
            })
        })
        .collect()
}

pub(crate) struct AprilPocketTts {
    bundle: Bundle,
    tokenizer: Tokenizer,
    bos_embedding: Vec<f32>,
    mimi_encoder: Session,
    text_conditioner: Session,
    flow_main: Session,
    flow: Session,
    mimi_decoder: Session,
    cached_voice: Option<CachedVoice>,
    /// EXPERIMENTAL (latency): post-`condition_voice` Flow LM state, cached
    /// per reference voice. Restoring it replaces the ~160 ms conditioning
    /// pass on every chunk after the first for a given voice.
    cached_conditioning: Option<CachedConditioning>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AprilPreparedPrompt {
    pub(crate) text: String,
    pub(crate) frames_after_eos: usize,
}

pub(crate) fn prepare_april_prompt(input: &str) -> Option<AprilPreparedPrompt> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut cleaned = String::with_capacity(trimmed.len());
    let mut last_was_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }

    let first = cleaned.chars().next().expect("cleaned non-empty above");
    if first.is_lowercase() {
        let upper: String = first.to_uppercase().collect();
        let mut iter = cleaned.chars();
        iter.next();
        cleaned = upper + iter.as_str();
    }

    let last = cleaned
        .chars()
        .next_back()
        .expect("cleaned non-empty above");
    if last.is_alphanumeric() {
        cleaned.push('.');
    }

    let word_count = cleaned.split_whitespace().count();
    Some(AprilPreparedPrompt {
        text: cleaned,
        // Mirror the bundle's upstream heuristic: three generated frames plus
        // two trailing frames for short prompts, one plus two otherwise.
        frames_after_eos: if word_count <= 4 { 5 } else { 3 },
    })
}

impl AprilPocketTts {
    pub(crate) fn load(dir: &Path, num_threads: usize) -> Result<Self, String> {
        if num_threads == 0 {
            return Err("Pocket TTS num_threads must be at least 1".to_string());
        }
        let bundle_path = dir.join(FILE_BUNDLE);
        let bundle: Bundle = serde_json::from_slice(
            &fs::read(&bundle_path)
                .map_err(|err| format!("read {}: {err}", bundle_path.display()))?,
        )
        .map_err(|err| format!("parse {}: {err}", bundle_path.display()))?;

        if bundle.schema_version != 2 {
            return Err(format!(
                "unsupported Pocket TTS bundle schema {} in {}",
                bundle.schema_version,
                bundle_path.display()
            ));
        }
        if bundle.language != MODEL_LANGUAGE {
            return Err(format!(
                "expected Pocket TTS language {MODEL_LANGUAGE}, got {}",
                bundle.language
            ));
        }
        if bundle.sample_rate != 24_000
            || bundle.frame_rate != 12.5
            || bundle.samples_per_frame != 1_920
            || bundle.latent_dim != 32
            || bundle.conditioning_dim != 1024
        {
            return Err(format!(
                "unexpected Pocket TTS dimensions: sample_rate={}, frame_rate={}, samples_per_frame={}, latent_dim={}, conditioning_dim={}",
                bundle.sample_rate,
                bundle.frame_rate,
                bundle.samples_per_frame,
                bundle.latent_dim,
                bundle.conditioning_dim
            ));
        }
        if !bundle.insert_bos_before_voice {
            return Err("April Pocket TTS bundle must insert BOS before voice".to_string());
        }
        if bundle.pad_with_spaces_for_short_inputs
            || bundle.remove_semicolons
            || bundle.model_recommended_frames_after_eos.is_some()
            || bundle.max_token_per_chunk != 50
        {
            return Err("unsupported April Pocket TTS prompt-policy metadata".to_string());
        }

        let tokenizer_path = dir.join(&bundle.tokenizer_file);
        let tokenizer = load_tokenizer(&tokenizer_path)?;
        let bos_path = dir.join(&bundle.bos_before_voice_file);
        let bos_embedding = read_npy_f32(&bos_path)?;
        if bos_embedding.len() != bundle.conditioning_dim {
            return Err(format!(
                "{} has {} values; expected {}",
                bos_path.display(),
                bos_embedding.len(),
                bundle.conditioning_dim
            ));
        }

        let flow_main = FILE_FLOW_MAIN_INT8;
        let flow = FILE_FLOW_INT8;
        let mimi_decoder = FILE_MIMI_DECODER_INT8;

        Ok(Self {
            // The INT8 layout quantizes only the three generation graphs;
            // voice encoding and text conditioning remain full precision.
            mimi_encoder: load_session(dir.join(FILE_MIMI_ENCODER), num_threads)?,
            text_conditioner: load_session(dir.join(FILE_TEXT_CONDITIONER), num_threads)?,
            flow_main: load_session(dir.join(flow_main), num_threads)?,
            flow: load_session(dir.join(flow), num_threads)?,
            mimi_decoder: load_session(dir.join(mimi_decoder), num_threads)?,
            bundle,
            tokenizer,
            bos_embedding,
            cached_voice: None,
            cached_conditioning: None,
        })
    }

    pub(crate) fn split_prompt(
        &self,
        prepared: &AprilPreparedPrompt,
    ) -> Result<Vec<String>, String> {
        if self.prepared_token_count(&prepared.text)? <= self.bundle.max_token_per_chunk {
            return Ok(vec![prepared.text.clone()]);
        }
        split_model_at_natural_boundaries(&prepared.text, self.bundle.max_token_per_chunk, |text| {
            self.prepared_token_count(text)
        })
    }

    pub(crate) fn split_playback_prompt(
        &self,
        prepared: &AprilPreparedPrompt,
    ) -> Result<Vec<String>, String> {
        split_playback_at_natural_boundaries(
            &prepared.text,
            self.bundle.max_token_per_chunk,
            |text| self.prepared_token_count(text),
        )
    }

    pub(crate) fn synth_chunk(
        &mut self,
        prepared: &AprilPreparedPrompt,
        style: &VoiceStyle,
    ) -> Result<Vec<f32>, String> {
        // EXPERIMENTAL (latency bench): phase timing, enabled by BUZZ_TTS_PHASE_LOG=1.
        let phase_log = std::env::var("BUZZ_TTS_PHASE_LOG").is_ok_and(|v| v == "1");
        let t0 = std::time::Instant::now();
        let mut flow_state = self.conditioned_flow_state(style)?;
        let t_condition = t0.elapsed();
        let token_ids = self
            .tokenizer
            .encode(prepared.text.as_str(), false)
            .map_err(|err| format!("tokenize Pocket TTS prompt: {err}"))?
            .get_ids()
            .iter()
            .copied()
            .map(i64::from)
            .collect::<Vec<_>>();
        if token_ids.is_empty() {
            return Ok(Vec::new());
        }
        if token_ids.len() > self.bundle.max_token_per_chunk {
            return Err(format!(
                "Pocket TTS prompt has {} tokens; split_text_into_chunks maximum is {}",
                token_ids.len(),
                self.bundle.max_token_per_chunk
            ));
        }

        let token_count = token_ids.len();
        let text_embeddings = self.text_embeddings(token_ids)?;
        self.run_flow_main_prefix(&text_embeddings, &mut flow_state)?;
        let t_prefix = t0.elapsed();
        let max_frames = estimate_max_frames(token_count, self.bundle.frame_rate);
        let latents =
            self.generate_latents(max_frames, prepared.frames_after_eos, &mut flow_state)?;
        let t_generate = t0.elapsed();
        let audio = self.decode_latents(&latents)?;
        if phase_log {
            eprintln!(
                "tts-phase: condition={:.0}ms prefix={:.0}ms generate={:.0}ms decode={:.0}ms frames={} audio_s={:.2}",
                t_condition.as_secs_f64() * 1e3,
                (t_prefix - t_condition).as_secs_f64() * 1e3,
                (t_generate - t_prefix).as_secs_f64() * 1e3,
                (t0.elapsed() - t_generate).as_secs_f64() * 1e3,
                latents.len() / self.bundle.latent_dim,
                audio.len() as f64 / self.bundle.sample_rate as f64,
            );
        }
        Ok(audio)
    }

    /// EXPERIMENTAL (latency): return a fresh Flow LM state conditioned on
    /// the reference voice, restoring a cached snapshot when the same voice
    /// samples were conditioned before. Keyed by voice content, like
    /// `cached_voice` — never by buffer address.
    fn conditioned_flow_state(&mut self, style: &VoiceStyle) -> Result<Vec<StateValue>, String> {
        let key = voice_key(style);
        if let Some(cached) = &self.cached_conditioning {
            if cached.key == key {
                return restore_state(&cached.state);
            }
        }
        let voice_embeddings = self.voice_embeddings(style)?;
        let state = self.condition_voice(&voice_embeddings)?;
        self.cached_conditioning = Some(CachedConditioning {
            key,
            state: snapshot_state(&state)?,
        });
        Ok(state)
    }

    /// EXPERIMENTAL (latency): streaming synthesis — interleaves the Flow LM
    /// frame loop with incremental stateful Mimi decoding, invoking
    /// `on_audio` with each decoded delta as soon as ~`emit_frames` latent
    /// frames exist (80 ms of audio per frame). The Mimi decoder carries its
    /// recurrent state across deltas, so the concatenated deltas are the same
    /// audio `synth_chunk` would return. Returns Ok(false) when the callback
    /// requested cancellation.
    pub(crate) fn synth_chunk_streaming(
        &mut self,
        prepared: &AprilPreparedPrompt,
        style: &VoiceStyle,
        emit_frames: usize,
        on_audio: &mut dyn FnMut(Vec<f32>) -> bool,
    ) -> Result<bool, String> {
        let mut flow_state = self.conditioned_flow_state(style)?;
        let token_ids = self
            .tokenizer
            .encode(prepared.text.as_str(), false)
            .map_err(|err| format!("tokenize Pocket TTS prompt: {err}"))?
            .get_ids()
            .iter()
            .copied()
            .map(i64::from)
            .collect::<Vec<_>>();
        if token_ids.is_empty() {
            return Ok(true);
        }
        if token_ids.len() > self.bundle.max_token_per_chunk {
            return Err(format!(
                "Pocket TTS prompt has {} tokens; split_text_into_chunks maximum is {}",
                token_ids.len(),
                self.bundle.max_token_per_chunk
            ));
        }

        let token_count = token_ids.len();
        let text_embeddings = self.text_embeddings(token_ids)?;
        self.run_flow_main_prefix(&text_embeddings, &mut flow_state)?;
        let max_frames = estimate_max_frames(token_count, self.bundle.frame_rate);
        let emit_frames = emit_frames.max(1);

        let mut mimi_state = initialize_state(&self.bundle.mimi_state_manifest)?;
        let mut pending: Vec<f32> = Vec::with_capacity(emit_frames * self.bundle.latent_dim);
        let mut current = vec![f32::NAN; self.bundle.latent_dim];
        let mut eos_step = None;
        let mut rng = rand::rng();

        for step in 0..max_frames {
            let sequence = Tensor::from_array((
                vec![1_i64, 1, self.bundle.latent_dim as i64],
                current.clone().into_boxed_slice(),
            ))
            .map_err(ort_error("create latent input"))?;
            let text_embeddings = Tensor::<f32>::new(
                &ort::memory::Allocator::default(),
                [1_i64, 0, self.bundle.conditioning_dim as i64],
            )
            .map_err(ort_error("create empty text input"))?;
            let mut inputs = vec![
                (Cow::Borrowed("sequence"), SessionInputValue::from(sequence)),
                (
                    Cow::Borrowed("text_embeddings"),
                    SessionInputValue::from(text_embeddings),
                ),
            ];
            append_state_inputs(&mut inputs, &flow_state);
            // Scoped: `outputs` borrows `self.flow_main`; it must drop before
            // `decode_frames` takes `&mut self` below.
            let (conditioning, eos_logit) = {
                let mut outputs = self
                    .flow_main
                    .run(inputs)
                    .map_err(ort_error("run Pocket TTS Flow LM"))?;
                let conditioning = outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(ort_error("extract Flow LM conditioning"))?
                    .1
                    .to_vec();
                let eos_logit = outputs[1]
                    .try_extract_tensor::<f32>()
                    .map_err(ort_error("extract Flow LM EOS logit"))?
                    .1
                    .first()
                    .copied()
                    .ok_or_else(|| "Flow LM returned empty EOS logit".to_string())?;
                replace_state_from_outputs(&mut flow_state, &mut outputs)?;
                (conditioning, eos_logit)
            };

            if eos_logit > EOS_LOGIT_THRESHOLD && eos_step.is_none() {
                eos_step = Some(step);
            }
            if eos_step.is_some_and(|eos| step >= eos + prepared.frames_after_eos) {
                break;
            }

            let mut noise =
                normal_noise(&mut rng, self.bundle.latent_dim, DEFAULT_TEMPERATURE.sqrt());
            let conditioning = Tensor::from_array((
                vec![1_i64, self.bundle.conditioning_dim as i64],
                conditioning.into_boxed_slice(),
            ))
            .map_err(ort_error("create flow conditioning"))?;
            let s = Tensor::from_array((vec![1_i64, 1], vec![0.0_f32].into_boxed_slice()))
                .map_err(ort_error("create flow start tensor"))?;
            let t = Tensor::from_array((vec![1_i64, 1], vec![1.0_f32].into_boxed_slice()))
                .map_err(ort_error("create flow end tensor"))?;
            let x = Tensor::from_array((
                vec![1_i64, self.bundle.latent_dim as i64],
                noise.clone().into_boxed_slice(),
            ))
            .map_err(ort_error("create flow noise tensor"))?;
            let outputs = self
                .flow
                .run(ort::inputs![
                    "c" => conditioning,
                    "s" => s,
                    "t" => t,
                    "x" => x,
                ])
                .map_err(ort_error("run Pocket TTS flow"))?;
            let flow = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Pocket TTS flow"))?
                .1;
            if flow.len() != noise.len() {
                return Err(format!(
                    "flow returned {} values; expected {}",
                    flow.len(),
                    noise.len()
                ));
            }
            for (sample, delta) in noise.iter_mut().zip(flow) {
                *sample += *delta;
            }
            drop(outputs);
            current.clone_from(&noise);
            pending.extend_from_slice(&noise);

            if pending.len() >= emit_frames * self.bundle.latent_dim {
                let audio = self.decode_frames(&pending, &mut mimi_state)?;
                pending.clear();
                if !audio.is_empty() && !on_audio(audio) {
                    return Ok(false);
                }
            }
        }
        if !pending.is_empty() {
            let audio = self.decode_frames(&pending, &mut mimi_state)?;
            if !audio.is_empty() && !on_audio(audio) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// EXPERIMENTAL (latency): decode a batch of latent frames with a
    /// caller-held Mimi state, so successive calls continue one stream.
    fn decode_frames(
        &mut self,
        latents: &[f32],
        state: &mut [StateValue],
    ) -> Result<Vec<f32>, String> {
        if latents.is_empty() {
            return Ok(Vec::new());
        }
        if !latents.len().is_multiple_of(self.bundle.latent_dim) {
            return Err(format!(
                "latent buffer has {} values, not divisible by {}",
                latents.len(),
                self.bundle.latent_dim
            ));
        }
        let frame_count = latents.len() / self.bundle.latent_dim;
        let mut audio = Vec::new();
        for start in (0..frame_count).step_by(DECODER_CHUNK_FRAMES) {
            let end = (start + DECODER_CHUNK_FRAMES).min(frame_count);
            let values =
                latents[start * self.bundle.latent_dim..end * self.bundle.latent_dim].to_vec();
            let latent = Tensor::from_array((
                vec![1_i64, (end - start) as i64, self.bundle.latent_dim as i64],
                values.into_boxed_slice(),
            ))
            .map_err(ort_error("create Mimi latent tensor"))?;
            let mut inputs = vec![(Cow::Borrowed("latent"), SessionInputValue::from(latent))];
            append_state_inputs(&mut inputs, state);
            let mut outputs = self
                .mimi_decoder
                .run(inputs)
                .map_err(ort_error("run Mimi decoder"))?;
            let samples = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Mimi audio"))?
                .1;
            audio.extend_from_slice(samples);
            replace_state_from_outputs(state, &mut outputs)?;
        }
        Ok(audio)
    }

    fn prepared_token_count(&self, text: &str) -> Result<usize, String> {
        let prepared = prepare_april_prompt(text)
            .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
        self.token_count(&prepared.text)
    }

    fn token_count(&self, text: &str) -> Result<usize, String> {
        Ok(self
            .tokenizer
            .encode(text, false)
            .map_err(|err| format!("tokenize Pocket TTS prompt: {err}"))?
            .get_ids()
            .len())
    }

    fn voice_embeddings(&mut self, style: &VoiceStyle) -> Result<Vec<f32>, String> {
        let key = voice_key(style);
        if let Some(cached) = &self.cached_voice {
            if cached.key == key {
                return Ok(cached.embeddings.clone());
            }
        }

        let samples = if style.sample_rate == self.bundle.sample_rate as i32 {
            style.samples.clone()
        } else {
            LinearResampler::create(style.sample_rate, self.bundle.sample_rate as i32)
                .ok_or_else(|| {
                    format!(
                        "create Pocket TTS resampler {}Hz -> {}Hz",
                        style.sample_rate, self.bundle.sample_rate
                    )
                })?
                .resample(&style.samples, true)
        };
        let audio = Tensor::from_array((
            vec![1_i64, 1, samples.len() as i64],
            samples.into_boxed_slice(),
        ))
        .map_err(ort_error("create voice audio tensor"))?;
        let outputs = self
            .mimi_encoder
            .run(ort::inputs!["audio" => audio])
            .map_err(ort_error("run Mimi encoder"))?;
        let (_, encoded) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_error("extract Mimi encoder output"))?;
        if !encoded.len().is_multiple_of(self.bundle.conditioning_dim) {
            return Err(format!(
                "Mimi encoder returned {} values, not divisible by {}",
                encoded.len(),
                self.bundle.conditioning_dim
            ));
        }
        let mut embeddings =
            Vec::with_capacity(self.bos_embedding.len().saturating_add(encoded.len()));
        embeddings.extend_from_slice(&self.bos_embedding);
        embeddings.extend_from_slice(encoded);
        self.cached_voice = Some(CachedVoice {
            key,
            embeddings: embeddings.clone(),
        });
        Ok(embeddings)
    }

    fn condition_voice(&mut self, embeddings: &[f32]) -> Result<Vec<StateValue>, String> {
        let frames = embeddings.len() / self.bundle.conditioning_dim;
        let sequence = Tensor::<f32>::new(
            &ort::memory::Allocator::default(),
            [1_i64, 0, self.bundle.latent_dim as i64],
        )
        .map_err(ort_error("create empty voice sequence"))?;
        let text_embeddings = Tensor::from_array((
            vec![1_i64, frames as i64, self.bundle.conditioning_dim as i64],
            embeddings.to_vec().into_boxed_slice(),
        ))
        .map_err(ort_error("create voice embedding tensor"))?;
        let mut state = initialize_state(&self.bundle.flow_lm_state_manifest)?;
        let mut inputs = vec![
            (Cow::Borrowed("sequence"), SessionInputValue::from(sequence)),
            (
                Cow::Borrowed("text_embeddings"),
                SessionInputValue::from(text_embeddings),
            ),
        ];
        append_state_inputs(&mut inputs, &state);
        let mut outputs = self
            .flow_main
            .run(inputs)
            .map_err(ort_error("condition Pocket TTS voice"))?;
        replace_state_from_outputs(&mut state, &mut outputs)?;
        Ok(state)
    }

    fn text_embeddings(&mut self, token_ids: Vec<i64>) -> Result<Vec<f32>, String> {
        let tokens = Tensor::from_array((
            vec![1_i64, token_ids.len() as i64],
            token_ids.into_boxed_slice(),
        ))
        .map_err(ort_error("create token tensor"))?;
        let outputs = self
            .text_conditioner
            .run(ort::inputs!["token_ids" => tokens])
            .map_err(ort_error("run text conditioner"))?;
        let (_, embeddings) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(ort_error("extract text embeddings"))?;
        Ok(embeddings.to_vec())
    }

    fn run_flow_main_prefix(
        &mut self,
        text_embeddings: &[f32],
        state: &mut [StateValue],
    ) -> Result<(), String> {
        if !text_embeddings
            .len()
            .is_multiple_of(self.bundle.conditioning_dim)
        {
            return Err(format!(
                "text conditioner returned {} values, not divisible by {}",
                text_embeddings.len(),
                self.bundle.conditioning_dim
            ));
        }
        let frames = text_embeddings.len() / self.bundle.conditioning_dim;
        let sequence = Tensor::<f32>::new(
            &ort::memory::Allocator::default(),
            [1_i64, 0, self.bundle.latent_dim as i64],
        )
        .map_err(ort_error("create empty text sequence"))?;
        let text_embeddings = Tensor::from_array((
            vec![1_i64, frames as i64, self.bundle.conditioning_dim as i64],
            text_embeddings.to_vec().into_boxed_slice(),
        ))
        .map_err(ort_error("create text embedding tensor"))?;
        let mut inputs = vec![
            (Cow::Borrowed("sequence"), SessionInputValue::from(sequence)),
            (
                Cow::Borrowed("text_embeddings"),
                SessionInputValue::from(text_embeddings),
            ),
        ];
        append_state_inputs(&mut inputs, state);
        let mut outputs = self
            .flow_main
            .run(inputs)
            .map_err(ort_error("prime Pocket TTS text state"))?;
        replace_state_from_outputs(state, &mut outputs)
    }

    fn generate_latents(
        &mut self,
        max_frames: usize,
        frames_after_eos: usize,
        state: &mut [StateValue],
    ) -> Result<Vec<f32>, String> {
        let mut current = vec![f32::NAN; self.bundle.latent_dim];
        let mut latents = Vec::with_capacity(max_frames * self.bundle.latent_dim);
        let mut eos_step = None;
        let mut rng = rand::rng();

        for step in 0..max_frames {
            let sequence = Tensor::from_array((
                vec![1_i64, 1, self.bundle.latent_dim as i64],
                current.clone().into_boxed_slice(),
            ))
            .map_err(ort_error("create latent input"))?;
            let text_embeddings = Tensor::<f32>::new(
                &ort::memory::Allocator::default(),
                [1_i64, 0, self.bundle.conditioning_dim as i64],
            )
            .map_err(ort_error("create empty text input"))?;
            let mut inputs = vec![
                (Cow::Borrowed("sequence"), SessionInputValue::from(sequence)),
                (
                    Cow::Borrowed("text_embeddings"),
                    SessionInputValue::from(text_embeddings),
                ),
            ];
            append_state_inputs(&mut inputs, state);
            let mut outputs = self
                .flow_main
                .run(inputs)
                .map_err(ort_error("run Pocket TTS Flow LM"))?;
            let conditioning = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Flow LM conditioning"))?
                .1
                .to_vec();
            let eos_logit = outputs[1]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Flow LM EOS logit"))?
                .1
                .first()
                .copied()
                .ok_or_else(|| "Flow LM returned empty EOS logit".to_string())?;
            replace_state_from_outputs(state, &mut outputs)?;

            if eos_logit > EOS_LOGIT_THRESHOLD && eos_step.is_none() {
                eos_step = Some(step);
            }
            if eos_step.is_some_and(|eos| step >= eos + frames_after_eos) {
                break;
            }

            let mut noise =
                normal_noise(&mut rng, self.bundle.latent_dim, DEFAULT_TEMPERATURE.sqrt());
            let conditioning = Tensor::from_array((
                vec![1_i64, self.bundle.conditioning_dim as i64],
                conditioning.into_boxed_slice(),
            ))
            .map_err(ort_error("create flow conditioning"))?;
            let s = Tensor::from_array((vec![1_i64, 1], vec![0.0_f32].into_boxed_slice()))
                .map_err(ort_error("create flow start tensor"))?;
            let t = Tensor::from_array((vec![1_i64, 1], vec![1.0_f32].into_boxed_slice()))
                .map_err(ort_error("create flow end tensor"))?;
            let x = Tensor::from_array((
                vec![1_i64, self.bundle.latent_dim as i64],
                noise.clone().into_boxed_slice(),
            ))
            .map_err(ort_error("create flow noise tensor"))?;
            let outputs = self
                .flow
                .run(ort::inputs![
                    "c" => conditioning,
                    "s" => s,
                    "t" => t,
                    "x" => x,
                ])
                .map_err(ort_error("run Pocket TTS flow"))?;
            let flow = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Pocket TTS flow"))?
                .1;
            if flow.len() != noise.len() {
                return Err(format!(
                    "flow returned {} values; expected {}",
                    flow.len(),
                    noise.len()
                ));
            }
            for (sample, delta) in noise.iter_mut().zip(flow) {
                *sample += *delta;
            }
            current.clone_from(&noise);
            latents.extend_from_slice(&noise);
        }
        Ok(latents)
    }

    fn decode_latents(&mut self, latents: &[f32]) -> Result<Vec<f32>, String> {
        if latents.is_empty() {
            return Ok(Vec::new());
        }
        if !latents.len().is_multiple_of(self.bundle.latent_dim) {
            return Err(format!(
                "latent buffer has {} values, not divisible by {}",
                latents.len(),
                self.bundle.latent_dim
            ));
        }
        let frame_count = latents.len() / self.bundle.latent_dim;
        let mut state = initialize_state(&self.bundle.mimi_state_manifest)?;
        let mut audio = Vec::new();

        for start in (0..frame_count).step_by(DECODER_CHUNK_FRAMES) {
            let end = (start + DECODER_CHUNK_FRAMES).min(frame_count);
            let values =
                latents[start * self.bundle.latent_dim..end * self.bundle.latent_dim].to_vec();
            let latent = Tensor::from_array((
                vec![1_i64, (end - start) as i64, self.bundle.latent_dim as i64],
                values.into_boxed_slice(),
            ))
            .map_err(ort_error("create Mimi latent tensor"))?;
            let mut inputs = vec![(Cow::Borrowed("latent"), SessionInputValue::from(latent))];
            append_state_inputs(&mut inputs, &state);
            let mut outputs = self
                .mimi_decoder
                .run(inputs)
                .map_err(ort_error("run Mimi decoder"))?;
            let samples = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ort_error("extract Mimi audio"))?
                .1;
            audio.extend_from_slice(samples);
            replace_state_from_outputs(&mut state, &mut outputs)?;
        }
        Ok(audio)
    }
}

fn split_model_at_natural_boundaries<F>(
    text: &str,
    max_tokens: usize,
    token_count: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(&str) -> Result<usize, String>,
{
    split_at_natural_boundaries(text, max_tokens, false, token_count)
}

fn split_playback_at_natural_boundaries<F>(
    text: &str,
    max_tokens: usize,
    token_count: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(&str) -> Result<usize, String>,
{
    split_at_natural_boundaries(text, max_tokens, true, token_count)
}

fn split_at_natural_boundaries<F>(
    text: &str,
    max_tokens: usize,
    isolate_first_sentence: bool,
    mut token_count: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(&str) -> Result<usize, String>,
{
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        while text[start..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            start += text[start..]
                .chars()
                .next()
                .expect("checked above")
                .len_utf8();
        }
        if start == text.len() {
            break;
        }

        let mut first_sentence_end = None;
        let mut sentence_end = None;
        let mut clause_end = None;
        let mut word_end = None;
        for (offset, ch) in text[start..].char_indices() {
            let end = start + offset + ch.len_utf8();
            let at_word_end =
                end == text.len() || text[end..].chars().next().is_some_and(char::is_whitespace);
            let at_clause_end = matches!(ch, '—' | '–')
                && !text[end..]
                    .chars()
                    .next()
                    .is_some_and(is_closing_punctuation);
            if !at_word_end && !at_clause_end {
                continue;
            }
            // Prepared token counts are monotonic in prefix length, so once a
            // candidate overflows the limit no longer candidate can fit. Stop
            // scanning instead of tokenizing every remaining boundary: that
            // kept this loop superlinear in prompt length, and the cost landed
            // before the first chunk reached synthesis.
            if token_count(&text[start..end])? > max_tokens {
                break;
            }

            word_end = Some(end);
            match natural_boundary(&text[start..end], end == text.len()) {
                TextBoundary::Sentence => {
                    first_sentence_end.get_or_insert(end);
                    sentence_end = Some(end);
                }
                TextBoundary::Clause => clause_end = Some(end),
                TextBoundary::Word => {}
            }
        }

        let preferred_end = if isolate_first_sentence && chunks.is_empty() {
            first_sentence_end.or(clause_end).or(word_end)
        } else {
            sentence_end.or(clause_end).or(word_end)
        };
        let end = if let Some(end) = preferred_end {
            end
        } else {
            // A single word can itself exceed the model limit. Preserve a
            // scalar boundary as the final safety case without losing UTF-8.
            let mut scalar_end = None;
            for (offset, ch) in text[start..].char_indices() {
                if ch.is_whitespace() {
                    break;
                }
                let end = start + offset + ch.len_utf8();
                if token_count(&text[start..end])? <= max_tokens {
                    scalar_end = Some(end);
                }
            }
            scalar_end.ok_or_else(|| {
                format!(
                    "Pocket TTS prompt cannot fit one character within the {max_tokens}-token limit"
                )
            })?
        };

        let mut next_start = end;
        while text[next_start..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            next_start += text[next_start..]
                .chars()
                .next()
                .expect("checked above")
                .len_utf8();
        }
        chunks.push(text[start..next_start].to_string());
        start = next_start;
    }

    debug_assert_eq!(chunks.concat(), text);
    Ok(chunks)
}

fn natural_boundary(candidate: &str, is_end_of_text: bool) -> TextBoundary {
    if is_end_of_text {
        return TextBoundary::Sentence;
    }

    let mut chars = candidate.chars().rev();
    let mut last = chars.next();
    while last.is_some_and(is_closing_punctuation) {
        last = chars.next();
    }
    match last {
        Some('.' | '!' | '?') if !looks_like_abbreviation(candidate) => TextBoundary::Sentence,
        Some(',' | ';' | ':' | '—' | '–') => TextBoundary::Clause,
        _ => TextBoundary::Word,
    }
}

fn is_closing_punctuation(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '”' | '’' | ')' | ']' | '}')
}

fn looks_like_abbreviation(candidate: &str) -> bool {
    const ABBREVIATIONS: &[&str] = &[
        "Dr.", "Mr.", "Mrs.", "Ms.", "Prof.", "Sr.", "Jr.", "St.", "Ave.", "Rd.", "Blvd.", "Dept.",
        "Inc.", "Ltd.", "Co.", "Corp.", "etc.", "vs.", "i.e.", "e.g.", "Ph.D.",
    ];

    let candidate = candidate.trim_end_matches(is_closing_punctuation);
    let last_word = candidate
        .rsplit_once(char::is_whitespace)
        .map_or(candidate, |(_, word)| word);
    ABBREVIATIONS.contains(&last_word)
        || (last_word.ends_with('.')
            && last_word[..last_word.len() - 1]
                .chars()
                .all(|ch| ch.is_ascii_digit()))
}

fn load_session(path: PathBuf, num_threads: usize) -> Result<Session, String> {
    if !path.is_file() {
        return Err(format!("missing Pocket TTS file: {}", path.display()));
    }
    Session::builder()
        .map_err(ort_error("create ONNX session builder"))?
        .with_intra_threads(num_threads)
        .map_err(|err| format!("configure ONNX intra-op threads: {err}"))?
        .with_inter_threads(1)
        .map_err(|err| format!("configure ONNX inter-op threads: {err}"))?
        .commit_from_file(&path)
        .map_err(|err| format!("load {}: {err}", path.display()))
}

fn load_tokenizer(path: &Path) -> Result<Tokenizer, String> {
    let sentencepiece = SentencePieceModel::from_file(path)
        .map_err(|err| format!("load {}: {err}", path.display()))?;
    let trainer = sentencepiece
        .trainer()
        .ok_or_else(|| format!("{} has no SentencePiece trainer metadata", path.display()))?;
    let normalizer = sentencepiece.normalizer().ok_or_else(|| {
        format!(
            "{} has no SentencePiece normalizer metadata",
            path.display()
        )
    })?;
    if normalizer.name() != "identity" {
        return Err(format!(
            "{} uses unsupported SentencePiece normalizer {:?}",
            path.display(),
            normalizer.name()
        ));
    }

    let vocab = sentencepiece
        .pieces()
        .iter()
        .map(|piece| (piece.piece().to_owned(), f64::from(piece.score())))
        .collect();
    let mut tokenizer = Tokenizer::new(
        Unigram::from(
            vocab,
            Some(trainer.unk_id() as usize),
            trainer.byte_fallback(),
        )
        .map_err(|err| format!("construct tokenizer from {}: {err}", path.display()))?,
    );
    // SentencePiece's identity normalizer still escapes spaces as U+2581 and
    // prepends one marker to the input before unigram segmentation.
    tokenizer.with_pre_tokenizer(Some(Metaspace::new('▁', PrependScheme::Always, false)));
    Ok(tokenizer)
}

fn initialize_state(specs: &[StateSpec]) -> Result<Vec<StateValue>, String> {
    specs
        .iter()
        .cloned()
        .map(|spec| {
            let len = shape_len(&spec.shape)?;
            let value = match spec.dtype {
                StateDtype::Float32 => {
                    let fill = match spec.fill {
                        StateFill::Nan => f32::NAN,
                        StateFill::Empty | StateFill::Zeros => 0.0,
                        StateFill::Ones => 1.0,
                    };
                    if len == 0 {
                        Tensor::<f32>::new(&ort::memory::Allocator::default(), spec.shape.clone())
                            .map_err(ort_error("create empty float state tensor"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((spec.shape.clone(), vec![fill; len].into_boxed_slice()))
                            .map_err(ort_error("create float state tensor"))?
                            .into_dyn()
                    }
                }
                StateDtype::Int64 => {
                    let fill = i64::from(matches!(spec.fill, StateFill::Ones));
                    if len == 0 {
                        Tensor::<i64>::new(&ort::memory::Allocator::default(), spec.shape.clone())
                            .map_err(ort_error("create empty integer state tensor"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((spec.shape.clone(), vec![fill; len].into_boxed_slice()))
                            .map_err(ort_error("create integer state tensor"))?
                            .into_dyn()
                    }
                }
                StateDtype::Bool => {
                    let fill = matches!(spec.fill, StateFill::Ones);
                    if len == 0 {
                        Tensor::<bool>::new(&ort::memory::Allocator::default(), spec.shape.clone())
                            .map_err(ort_error("create empty bool state tensor"))?
                            .into_dyn()
                    } else {
                        Tensor::from_array((spec.shape.clone(), vec![fill; len].into_boxed_slice()))
                            .map_err(ort_error("create bool state tensor"))?
                            .into_dyn()
                    }
                }
            };
            Ok(StateValue { spec, value })
        })
        .collect()
}

fn append_state_inputs<'a>(
    inputs: &mut Vec<(Cow<'a, str>, SessionInputValue<'a>)>,
    state: &'a [StateValue],
) {
    for value in state {
        inputs.push((
            Cow::Borrowed(value.spec.input_name.as_str()),
            SessionInputValue::from(&value.value),
        ));
    }
}

fn replace_state_from_outputs(
    state: &mut [StateValue],
    outputs: &mut ort::session::SessionOutputs<'_>,
) -> Result<(), String> {
    for value in state {
        value.value = outputs
            .remove(&value.spec.output_name)
            .ok_or_else(|| format!("missing state output {}", value.spec.output_name))?;
    }
    Ok(())
}

fn shape_len(shape: &[i64]) -> Result<usize, String> {
    shape.iter().try_fold(1_usize, |len, &dim| {
        let dim = usize::try_from(dim).map_err(|_| format!("negative state dimension {dim}"))?;
        len.checked_mul(dim)
            .ok_or_else(|| format!("state shape overflows usize: {shape:?}"))
    })
}

fn estimate_max_frames(token_count: usize, frame_rate: f32) -> usize {
    ((token_count as f32 / TOKENS_PER_SECOND_ESTIMATE + GENERATION_SECONDS_PADDING) * frame_rate)
        .ceil() as usize
}

fn normal_noise(rng: &mut impl Rng, len: usize, std_dev: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let u1 = rng.random::<f32>().max(f32::MIN_POSITIVE);
        let u2 = rng.random::<f32>();
        let radius = (-2.0_f32 * u1.ln()).sqrt() * std_dev;
        out.push(radius * (TAU * u2).cos());
        if out.len() < len {
            out.push(radius * (TAU * u2).sin());
        }
    }
    out
}

fn read_npy_f32(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(format!("{} is not a NumPy array", path.display()));
    }
    let major = bytes[6];
    let header_len_bytes = match major {
        1 => 2,
        2 | 3 => 4,
        _ => {
            return Err(format!(
                "unsupported NumPy version {major} in {}",
                path.display()
            ))
        }
    };
    let header_start = 8 + header_len_bytes;
    if bytes.len() < header_start {
        return Err(format!("truncated NumPy header in {}", path.display()));
    }
    let header_len = if header_len_bytes == 2 {
        u16::from_le_bytes([bytes[8], bytes[9]]) as usize
    } else {
        u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize
    };
    let data_start = header_start
        .checked_add(header_len)
        .ok_or_else(|| format!("NumPy header overflow in {}", path.display()))?;
    if data_start > bytes.len() {
        return Err(format!("truncated NumPy data in {}", path.display()));
    }
    let header = std::str::from_utf8(&bytes[header_start..data_start])
        .map_err(|err| format!("invalid NumPy header in {}: {err}", path.display()))?;
    if !(header.contains("'descr': '<f4'") || header.contains("\"descr\": \"<f4\""))
        || header.contains("fortran_order': True")
        || header.contains("fortran_order\": true")
    {
        return Err(format!(
            "{} must be a little-endian, C-order float32 NumPy array",
            path.display()
        ));
    }
    let data = &bytes[data_start..];
    if data.len() % 4 != 0 {
        return Err(format!("misaligned float32 data in {}", path.display()));
    }
    Ok(data
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn ort_error(context: &'static str) -> impl FnOnce(ort::Error) -> String {
    move |err| format!("{context}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_len_supports_empty_state_dimensions() {
        assert_eq!(shape_len(&[1, 128, 0]).expect("shape"), 0);
        assert_eq!(shape_len(&[2, 1, 8, 1000, 64]).expect("shape"), 1_024_000);
    }

    /// The two engine splitters must keep OPPOSITE isolation polarity.
    ///
    /// The guards in `pocket.rs` pin which engine method each public API calls,
    /// but they cannot see what the method itself does: pointing
    /// `split_playback_prompt` at the model wrapper leaves every call site's
    /// source text untouched while first-sentence isolation silently stops
    /// happening, so the first playback unit becomes the whole utterance and
    /// first audio waits on generating all of it.
    #[test]
    fn engine_splitters_keep_opposite_isolation_polarity() {
        let source = include_str!("pocket_april.rs");
        let production = source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(production, _)| production);

        // A method's own code, and nothing else. Ending at the method's own
        // closing brace keeps the NEXT method's doc comment out, and stripping
        // `//` to end of line keeps prose out: neither can call a splitter, so
        // scanning either reports drift in a method that has not changed.
        let method_code = |name: &str| -> String {
            let (_, body) = production
                .split_once(name)
                .unwrap_or_else(|| panic!("{name} exists"));
            let (body, _) = body
                .split_once("\n    }\n")
                .unwrap_or_else(|| panic!("{name} has a closing brace"));
            body.lines()
                .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let model = method_code("fn split_prompt");
        let model = model.as_str();
        let playback = method_code("fn split_playback_prompt");
        let playback = playback.as_str();

        assert_eq!(
            (
                model.matches("split_model_at_natural_boundaries(").count(),
                model
                    .matches("split_playback_at_natural_boundaries(")
                    .count(),
            ),
            (1, 0),
            "split_prompt must pack sentences: isolating here peels sentence \
             one off every already-packed unit"
        );
        assert_eq!(
            (
                playback
                    .matches("split_playback_at_natural_boundaries(")
                    .count(),
                playback
                    .matches("split_model_at_natural_boundaries(")
                    .count(),
            ),
            (1, 0),
            "split_playback_prompt must isolate sentence one: packing here \
             makes the first playback unit the whole utterance and delays \
             first audio by the full generation"
        );

        // Calling the isolating splitter is necessary but not sufficient: a
        // short circuit before the call can return the whole utterance as one
        // unit while leaving the delegated splitter unchanged. Playback must
        // delegate unconditionally so sentence one remains the first unit.
        for control_flow in ["if ", "match ", "else", "return"] {
            assert!(
                !playback.contains(control_flow),
                "split_playback_prompt must delegate unconditionally, found \
                 `{control_flow}`: a branch before the split can return the \
                 whole utterance as the first playback unit, delaying first \
                 audio by the full generation"
            );
        }
    }

    fn whitespace_token_count(text: &str) -> Result<usize, String> {
        Ok(text.split_whitespace().count())
    }

    #[test]
    fn playback_split_keeps_first_sentence_separate_then_packs_the_remainder() {
        let text = "One two. Three four. Five six.";
        let chunks = split_playback_at_natural_boundaries(text, 4, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["One two. ", "Three four. Five six."]);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn model_split_packs_multiple_sentences_within_limit() {
        let text = "One two. Three four. Five six.";
        let chunks = split_model_at_natural_boundaries(text, 4, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["One two. Three four. ", "Five six."]);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn playback_then_model_split_does_not_isolate_later_sentences_again() {
        let text = "Alpha one. Beta two. Gamma three.";
        let playback =
            split_playback_at_natural_boundaries(text, 50, whitespace_token_count).unwrap();
        assert_eq!(playback, ["Alpha one. ", "Beta two. Gamma three."]);

        let model: Vec<_> = playback
            .iter()
            .flat_map(|chunk| {
                split_model_at_natural_boundaries(chunk.trim(), 50, whitespace_token_count).unwrap()
            })
            .collect();
        assert_eq!(model, ["Alpha one.", "Beta two. Gamma three."]);
    }

    #[test]
    fn natural_split_prefers_preceding_sentence_boundary() {
        let text = "One two. Three four five six.";
        let chunks = split_at_natural_boundaries(text, 5, true, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["One two. ", "Three four five six."]);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn oversized_sentence_uses_clause_then_word_fallback() {
        let clause_text = "One two three, four five six seven.";
        let clause_chunks =
            split_at_natural_boundaries(clause_text, 5, true, whitespace_token_count).unwrap();
        assert_eq!(clause_chunks, ["One two three, ", "four five six seven."]);
        assert_eq!(clause_chunks.concat(), clause_text);

        let word_text = "One two three four five six.";
        let word_chunks =
            split_at_natural_boundaries(word_text, 4, true, whitespace_token_count).unwrap();
        assert_eq!(word_chunks, ["One two three four ", "five six."]);
        assert_eq!(word_chunks.concat(), word_text);
    }

    #[test]
    fn natural_split_preserves_unicode_punctuation_and_abbreviations() {
        let text = "“Café naïve?” Maybe—yes, definitely; 東京 speaks.";
        let chunks = split_at_natural_boundaries(text, 3, true, whitespace_token_count).unwrap();
        assert_eq!(
            chunks,
            ["“Café naïve?” ", "Maybe—yes, definitely; ", "東京 speaks."]
        );
        assert_eq!(chunks.concat(), text);

        let abbreviation = "Dr. Smith waits. Then leaves.";
        let chunks =
            split_at_natural_boundaries(abbreviation, 3, true, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["Dr. Smith waits. ", "Then leaves."]);
        assert_eq!(chunks.concat(), abbreviation);

        let unspaced_clause = "alpha beta—gamma delta";
        let chunks =
            split_at_natural_boundaries(unspaced_clause, 2, true, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["alpha beta—", "gamma delta"]);
        assert_eq!(chunks.concat(), unspaced_clause);
    }

    #[test]
    fn natural_split_does_not_treat_numeric_punctuation_as_unspaced_clauses() {
        let text = "Meet at 12:30 with 1,000 guests onward.";
        let chunks = split_at_natural_boundaries(text, 3, true, whitespace_token_count).unwrap();
        assert_eq!(chunks, ["Meet at 12:30 ", "with 1,000 guests ", "onward."]);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn oversized_word_uses_utf8_scalar_boundary_without_loss() {
        let text = "éééé";
        let chunks =
            split_at_natural_boundaries(text, 3, true, |chunk| Ok(chunk.chars().count())).unwrap();
        assert_eq!(chunks, ["ééé", "é"]);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn natural_split_stops_counting_tokens_past_the_limit() {
        // Each boundary scan must stop at the first overflowing candidate
        // rather than tokenizing every remaining boundary. Scanning to
        // end-of-text makes tokenizer input grow superlinearly in prompt
        // length, and that cost is paid before the first chunk reaches
        // synthesis, taxing time-to-first-audio on long prompts.
        let sentence = "The relay finished its migration and the channel list refreshed. ";
        let tokenized_bytes = |repeats: usize| -> usize {
            let text = sentence.repeat(repeats).trim_end().to_string();
            let total = std::cell::Cell::new(0_usize);
            let chunks = split_at_natural_boundaries(&text, 50, true, |chunk| {
                total.set(total.get() + chunk.len());
                whitespace_token_count(chunk)
            })
            .expect("split repeated sentences");
            assert_eq!(chunks.concat(), text);
            assert!(chunks.len() > 1);
            total.get()
        };

        // Doubling the prompt must not multiply tokenizer work superlinearly.
        // Bounded scans grow ~2x here; scanning to end-of-text grows ~5.5x.
        let single = tokenized_bytes(12);
        let double = tokenized_bytes(24);
        assert!(
            double < single * 3,
            "doubling the prompt grew tokenizer input from {single} to {double} bytes \
             ({:.1}x); bounded scans stay near 2x",
            double as f64 / single as f64,
        );
    }

    #[test]
    fn normal_noise_has_requested_length() {
        let mut rng = rand::rng();
        assert_eq!(normal_noise(&mut rng, 1, 1.0).len(), 1);
        assert_eq!(normal_noise(&mut rng, 32, 1.0).len(), 32);
    }

    #[test]
    fn generation_frame_estimate_scales_with_token_count() {
        assert_eq!(estimate_max_frames(3, 12.5), 38);
        assert_eq!(estimate_max_frames(300, 12.5), 1_275);
    }

    /// Regression (review finding): the voice caches must key on CONTENT.
    /// Voice switching clones and drops sample buffers, so a new voice with
    /// the same length and rate can land at a recycled address — an
    /// address-based key would then restore the previous voice's state and
    /// speak with the wrong voice.
    #[test]
    fn voice_key_is_content_based_not_address_based() {
        let style_a = VoiceStyle {
            samples: vec![0.1, -0.2, 0.3, -0.4],
            sample_rate: 24_000,
        };
        // Same length, same rate, different content — MUST key differently,
        // regardless of what address the allocator hands out.
        let style_b = VoiceStyle {
            samples: vec![0.4, -0.3, 0.2, -0.1],
            sample_rate: 24_000,
        };
        assert_ne!(voice_key(&style_a), voice_key(&style_b));

        // Same content in a fresh allocation — MUST key identically, so the
        // cache still hits across clones of the same voice.
        let style_a_clone = VoiceStyle {
            samples: style_a.samples.clone(),
            sample_rate: style_a.sample_rate,
        };
        assert_ne!(
            style_a.samples.as_ptr(),
            style_a_clone.samples.as_ptr(),
            "clone must be a distinct allocation for this test to mean anything"
        );
        assert_eq!(voice_key(&style_a), voice_key(&style_a_clone));

        // Same content at a different rate is a different voice identity.
        let style_a_resampled = VoiceStyle {
            samples: style_a.samples.clone(),
            sample_rate: 16_000,
        };
        assert_ne!(voice_key(&style_a), voice_key(&style_a_resampled));
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn switching_between_equal_length_voices_reconditions_the_flow_state() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to the verified April bundle");
        let style_a =
            crate::pocket::load_voice_style(&Path::new(&dir).join("reference_sample.wav"))
                .expect("load reference voice");
        // Voice B: same length, same rate, different content (reversed
        // samples) — the exact shape an address-recycling collision takes.
        let style_b = VoiceStyle {
            samples: style_a.samples.iter().rev().copied().collect(),
            sample_rate: style_a.sample_rate,
        };
        assert_eq!(style_a.samples.len(), style_b.samples.len());

        // Engine 1: condition A (primes both caches), then switch to B.
        let mut engine = AprilPocketTts::load(Path::new(&dir), 1).expect("load April bundle");
        let state_a = snapshot_state(
            &engine
                .conditioned_flow_state(&style_a)
                .expect("condition A"),
        )
        .expect("snapshot A");
        let state_b_after_switch = snapshot_state(
            &engine
                .conditioned_flow_state(&style_b)
                .expect("condition B"),
        )
        .expect("snapshot B after switch");
        // Warm hit on the SAME voice: the cached restore must reproduce the
        // original conditioning bit-for-bit (cache warm == cache cold).
        let state_b_warm_hit = snapshot_state(
            &engine
                .conditioned_flow_state(&style_b)
                .expect("condition B warm"),
        )
        .expect("snapshot B warm hit");

        // Engine 2: fresh process conditions B with no cache in play.
        let mut fresh = AprilPocketTts::load(Path::new(&dir), 1).expect("load April bundle");
        let state_b_fresh = snapshot_state(
            &fresh
                .conditioned_flow_state(&style_b)
                .expect("condition B fresh"),
        )
        .expect("snapshot B fresh");

        // The switched state must equal a from-scratch conditioning of B and
        // must NOT be A's cached state.
        assert!(
            snapshots_equal(&state_b_after_switch, &state_b_fresh),
            "switching voices must recondition, not replay the cache"
        );
        assert!(
            !snapshots_equal(&state_b_after_switch, &state_a),
            "equal-length distinct voices must produce distinct conditioning"
        );
        // And the warm cache hit must be indistinguishable from recomputing.
        assert!(
            snapshots_equal(&state_b_warm_hit, &state_b_fresh),
            "a warm conditioning-cache hit must equal a cold recompute"
        );
    }

    fn snapshots_equal(
        a: &[(StateSpec, SnapshotTensor)],
        b: &[(StateSpec, SnapshotTensor)],
    ) -> bool {
        // f32 compares bitwise: state tensors legitimately contain NaN fill,
        // and NaN != NaN under float equality would make identical states
        // compare unequal.
        a.len() == b.len()
            && a.iter().zip(b).all(|((_, ta), (_, tb))| match (ta, tb) {
                (SnapshotTensor::F32(sa, da), SnapshotTensor::F32(sb, db)) => {
                    sa == sb
                        && da.len() == db.len()
                        && da.iter().zip(db).all(|(x, y)| x.to_bits() == y.to_bits())
                }
                (SnapshotTensor::I64(sa, da), SnapshotTensor::I64(sb, db)) => sa == sb && da == db,
                (SnapshotTensor::Bool(sa, da), SnapshotTensor::Bool(sb, db)) => {
                    sa == sb && da == db
                }
                _ => false,
            })
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn incremental_stateful_decode_matches_batch_decode() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to the verified April bundle");
        let mut engine = AprilPocketTts::load(Path::new(&dir), 1).expect("load April bundle");
        let style = crate::pocket::load_voice_style(&Path::new(&dir).join("reference_sample.wav"))
            .expect("load reference voice");

        // Generate one real latent sequence (the RNG makes repeat synths
        // differ, so both decode paths must consume the SAME latents).
        let prepared =
            prepare_april_prompt("The relay deploy finished and every check passed cleanly.")
                .expect("prepare prompt");
        let mut flow_state = engine
            .conditioned_flow_state(&style)
            .expect("condition voice");
        let token_ids = engine
            .tokenizer
            .encode(prepared.text.as_str(), false)
            .expect("tokenize")
            .get_ids()
            .iter()
            .copied()
            .map(i64::from)
            .collect::<Vec<_>>();
        let token_count = token_ids.len();
        let text_embeddings = engine.text_embeddings(token_ids).expect("text embeddings");
        engine
            .run_flow_main_prefix(&text_embeddings, &mut flow_state)
            .expect("prefix");
        let max_frames = estimate_max_frames(token_count, engine.bundle.frame_rate);
        let latents = engine
            .generate_latents(max_frames, prepared.frames_after_eos, &mut flow_state)
            .expect("generate latents");
        let frame_count = latents.len() / engine.bundle.latent_dim;
        assert!(
            frame_count > DECODER_CHUNK_FRAMES,
            "need a multi-chunk case"
        );

        // Batch: the production decode (fresh state, 12-frame steps).
        let batch = engine.decode_latents(&latents).expect("batch decode");

        // Incremental chunkings: 12-frame deltas through one carried Mimi
        // state must be bit-exact (the production batch path itself steps by
        // DECODER_CHUNK_FRAMES=12 through one state). Sub-12 chunkings are
        // measured for the record but are NOT exact — the decoder has
        // intra-chunk lookahead — so streaming must emit at >= 12 frames.
        for delta_frames in [6usize, 4, 2, 1] {
            let mut state =
                initialize_state(&engine.bundle.mimi_state_manifest).expect("mimi state");
            let mut streamed = Vec::new();
            for chunk in latents.chunks(delta_frames * engine.bundle.latent_dim) {
                streamed.extend(
                    engine
                        .decode_frames(chunk, &mut state)
                        .expect("delta decode"),
                );
            }
            assert_eq!(batch.len(), streamed.len(), "sample count must match");
            let max_diff = batch
                .iter()
                .zip(&streamed)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            let rms_batch = (batch.iter().map(|s| s * s).sum::<f32>() / batch.len() as f32).sqrt();
            let rms_err = (batch
                .iter()
                .zip(&streamed)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                / batch.len() as f32)
                .sqrt();
            eprintln!(
                "delta_frames={delta_frames}: max|diff|={max_diff:.6} rms_err={rms_err:.6} snr_db={:.1}",
                20.0 * (rms_batch / rms_err.max(1e-12)).log10()
            );
        }
        let mut state = initialize_state(&engine.bundle.mimi_state_manifest).expect("mimi state");
        let mut streamed = Vec::new();
        for chunk in latents.chunks(DECODER_CHUNK_FRAMES * engine.bundle.latent_dim) {
            streamed.extend(
                engine
                    .decode_frames(chunk, &mut state)
                    .expect("delta decode"),
            );
        }

        assert_eq!(batch.len(), streamed.len(), "sample count must match");
        let max_diff = batch
            .iter()
            .zip(&streamed)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff <= 1.0e-4,
            "incremental decode diverged from batch decode: max |diff| = {max_diff}"
        );
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn tokenizer_matches_sentencepiece_reference_including_unknown_words() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to the verified April bundle");
        let tokenizer =
            load_tokenizer(&Path::new(&dir).join("tokenizer.model")).expect("load April tokenizer");
        let cases: &[(&str, &[u32])] = &[
            ("Yep.", &[2462, 263]),
            ("Hello there.", &[2994, 310, 263]),
            (
                "quizzaciously xyzzy.",
                &[
                    260, 1157, 1818, 362, 1814, 323, 260, 568, 327, 1818, 327, 263,
                ],
            ),
            ("I'm listening.", &[268, 264, 283, 260, 604, 273, 263]),
        ];
        for (text, expected) in cases {
            let encoding = tokenizer.encode(*text, false).expect("tokenize");
            assert_eq!(encoding.get_ids(), *expected, "{text}");
        }
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn loader_splits_oversized_prompts_at_bundle_token_limit() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to the verified April bundle");
        let engine = AprilPocketTts::load(Path::new(&dir), 1).expect("load April bundle");
        let text = "This deliberately long sentence repeats ordinary English words so the exact SentencePiece token limit is exercised without relying on punctuation, and it keeps adding more material until the prompt must be divided into multiple independently safe generation chunks before the recurrent state cache can be exhausted.";
        let prepared = prepare_april_prompt(text).expect("prepare prompt");
        let chunks = engine.split_prompt(&prepared).expect("split prompt");

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| {
            engine.prepared_token_count(chunk).expect("tokenize chunk")
                <= engine.bundle.max_token_per_chunk
        }));
        assert_eq!(chunks.concat(), prepared.text);
    }

    #[test]
    #[ignore = "requires BUZZ_POCKET_TEST_MODEL_DIR"]
    fn gary_provost_long_sentence_respects_bundle_token_limit() {
        let dir = std::env::var("BUZZ_POCKET_TEST_MODEL_DIR")
            .expect("set BUZZ_POCKET_TEST_MODEL_DIR to the verified April bundle");
        let engine = AprilPocketTts::load(Path::new(&dir), 1).expect("load April bundle");
        let text = "And sometimes, when I am certain the reader is rested, I will engage him with a sentence of considerable length, a sentence that burns with energy and builds with all the impetus of a crescendo, the roll of the drums, the crash of the cymbals–sounds that say listen to this, it is important.";
        let prepared = prepare_april_prompt(text).expect("prepare prompt");
        let chunks = engine.split_prompt(&prepared).expect("split long sentence");
        let token_counts: Vec<_> = chunks
            .iter()
            .map(|chunk| engine.prepared_token_count(chunk).expect("count tokens"))
            .collect();

        assert!(token_counts
            .iter()
            .all(|&count| count <= engine.bundle.max_token_per_chunk));
        assert_eq!(chunks.concat(), prepared.text);
        assert!(chunks.len() > 1);
        assert!(chunks[..chunks.len() - 1].iter().all(|chunk| {
            chunk
                .trim_end()
                .chars()
                .last()
                .is_some_and(|ch| ['.', '!', '?', ',', ';', ':', '—', '–'].contains(&ch))
        }));
    }
}
