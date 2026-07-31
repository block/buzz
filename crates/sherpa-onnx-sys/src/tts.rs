#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float, c_void};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsVitsModelConfig {
    pub model: *const c_char,
    pub lexicon: *const c_char,
    pub tokens: *const c_char,
    pub data_dir: *const c_char,
    pub noise_scale: c_float,
    pub noise_scale_w: c_float,
    pub length_scale: c_float,
    pub dict_dir: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsMatchaModelConfig {
    pub acoustic_model: *const c_char,
    pub vocoder: *const c_char,
    pub lexicon: *const c_char,
    pub tokens: *const c_char,
    pub data_dir: *const c_char,
    pub noise_scale: c_float,
    pub length_scale: c_float,
    pub dict_dir: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsKokoroModelConfig {
    pub model: *const c_char,
    pub voices: *const c_char,
    pub tokens: *const c_char,
    pub data_dir: *const c_char,
    pub length_scale: c_float,
    pub dict_dir: *const c_char,
    pub lexicon: *const c_char,
    pub lang: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsKittenModelConfig {
    pub model: *const c_char,
    pub voices: *const c_char,
    pub tokens: *const c_char,
    pub data_dir: *const c_char,
    pub length_scale: c_float,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsZipvoiceModelConfig {
    pub tokens: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub vocoder: *const c_char,
    pub data_dir: *const c_char,
    pub lexicon: *const c_char,
    pub feat_scale: c_float,
    pub t_shift: c_float,
    pub target_rms: c_float,
    pub guidance_scale: c_float,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsPocketModelConfig {
    pub lm_flow: *const c_char,
    pub lm_main: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub text_conditioner: *const c_char,
    pub vocab_json: *const c_char,
    pub token_scores_json: *const c_char,
    pub voice_embedding_cache_capacity: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsSupertonicModelConfig {
    pub duration_predictor: *const c_char,
    pub text_encoder: *const c_char,
    pub vector_estimator: *const c_char,
    pub vocoder: *const c_char,
    pub tts_json: *const c_char,
    pub unicode_indexer: *const c_char,
    pub voice_style: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsModelConfig {
    pub vits: OfflineTtsVitsModelConfig,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
    pub matcha: OfflineTtsMatchaModelConfig,
    pub kokoro: OfflineTtsKokoroModelConfig,
    pub kitten: OfflineTtsKittenModelConfig,
    pub zipvoice: OfflineTtsZipvoiceModelConfig,
    pub pocket: OfflineTtsPocketModelConfig,
    pub supertonic: OfflineTtsSupertonicModelConfig,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTtsConfig {
    pub model: OfflineTtsModelConfig,
    pub rule_fsts: *const c_char,
    pub max_num_sentences: i32,
    pub rule_fars: *const c_char,
    pub silence_scale: c_float,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SherpaOnnxGeneratedAudio {
    pub samples: *const f32,
    pub n: i32,
    pub sample_rate: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SherpaOnnxGenerationConfig {
    pub silence_scale: c_float,
    pub speed: c_float,
    pub sid: i32,
    pub reference_audio: *const f32,
    pub reference_audio_len: i32,
    pub reference_sample_rate: i32,
    pub reference_text: *const c_char,
    pub num_steps: i32,
    pub extra: *const c_char,
}

pub type SherpaOnnxGeneratedAudioProgressCallbackWithArg = Option<
    unsafe extern "C" fn(samples: *const f32, n: i32, progress: c_float, arg: *mut c_void) -> i32,
>;

#[repr(C)]
pub struct SherpaOnnxOfflineTts {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateOfflineTts(
        config: *const OfflineTtsConfig,
    ) -> *const SherpaOnnxOfflineTts;

    pub fn SherpaOnnxDestroyOfflineTts(tts: *const SherpaOnnxOfflineTts);

    pub fn SherpaOnnxOfflineTtsSampleRate(tts: *const SherpaOnnxOfflineTts) -> i32;

    pub fn SherpaOnnxOfflineTtsNumSpeakers(tts: *const SherpaOnnxOfflineTts) -> i32;

    pub fn SherpaOnnxOfflineTtsGenerateWithConfig(
        tts: *const SherpaOnnxOfflineTts,
        text: *const c_char,
        config: *const SherpaOnnxGenerationConfig,
        callback: SherpaOnnxGeneratedAudioProgressCallbackWithArg,
        arg: *mut c_void,
    ) -> *const SherpaOnnxGeneratedAudio;

    pub fn SherpaOnnxDestroyOfflineTtsGeneratedAudio(p: *const SherpaOnnxGeneratedAudio);
}
