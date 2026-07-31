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

struct CachedVoice {
    samples_ptr: usize,
    samples_len: usize,
    sample_rate: i32,
    embeddings: Vec<f32>,
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
        })
    }

    pub(crate) fn split_prompt(
        &self,
        prepared: &AprilPreparedPrompt,
    ) -> Result<Vec<String>, String> {
        if self.token_count(&prepared.text)? <= self.bundle.max_token_per_chunk {
            return Ok(vec![prepared.text.clone()]);
        }

        let mut chunks = Vec::new();
        let mut current = String::new();
        for word in prepared.text.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if self.prepared_token_count(&candidate)? <= self.bundle.max_token_per_chunk {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }

            if self.prepared_token_count(word)? <= self.bundle.max_token_per_chunk {
                current = word.to_string();
                continue;
            }

            let mut fragment = String::new();
            for ch in word.chars() {
                let candidate = format!("{fragment}{ch}");
                if !fragment.is_empty()
                    && self.prepared_token_count(&candidate)? > self.bundle.max_token_per_chunk
                {
                    chunks.push(std::mem::take(&mut fragment));
                }
                fragment.push(ch);
            }
            current = fragment;
        }
        if !current.is_empty() {
            chunks.push(current);
        }

        chunks
            .into_iter()
            .map(|text| {
                let chunk = prepare_april_prompt(&text)
                    .ok_or_else(|| "Pocket TTS prompt chunk became empty".to_string())?;
                let token_count = self.token_count(&chunk.text)?;
                if token_count > self.bundle.max_token_per_chunk {
                    return Err(format!(
                        "Pocket TTS prompt chunk has {token_count} tokens; maximum is {}",
                        self.bundle.max_token_per_chunk
                    ));
                }
                Ok(chunk.text)
            })
            .collect()
    }

    pub(crate) fn synth_chunk(
        &mut self,
        prepared: &AprilPreparedPrompt,
        style: &VoiceStyle,
    ) -> Result<Vec<f32>, String> {
        let voice_embeddings = self.voice_embeddings(style)?;
        let mut flow_state = self.condition_voice(&voice_embeddings)?;
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
        let max_frames = estimate_max_frames(token_count, self.bundle.frame_rate);
        let latents =
            self.generate_latents(max_frames, prepared.frames_after_eos, &mut flow_state)?;
        self.decode_latents(&latents)
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
        let key = (
            style.samples.as_ptr() as usize,
            style.samples.len(),
            style.sample_rate,
        );
        if let Some(cached) = &self.cached_voice {
            if (cached.samples_ptr, cached.samples_len, cached.sample_rate) == key {
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
            samples_ptr: key.0,
            samples_len: key.1,
            sample_rate: key.2,
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
            engine.token_count(chunk).expect("tokenize chunk") <= engine.bundle.max_token_per_chunk
        }));
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
            .map(|chunk| engine.token_count(chunk).expect("count tokens"))
            .collect();

        assert_eq!(
            chunks,
            [
                "And sometimes, when I am certain the reader is rested, I will engage him with a sentence of considerable length, a sentence that burns with energy and builds with all the.",
                "Impetus of a crescendo, the roll of the drums, the crash of the cymbals–sounds that say listen to this, it is important.",
            ]
        );
        assert_eq!(token_counts, [48, 44]);
    }
}
