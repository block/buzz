#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTransducerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub joiner: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineParaformerModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineNemoEncDecCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineWhisperModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub task: *const c_char,
    pub tail_paddings: i32,
    pub enable_token_timestamps: i32,
    pub enable_segment_timestamps: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineCanaryModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub src_lang: *const c_char,
    pub tgt_lang: *const c_char,
    pub use_pnc: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineFireRedAsrModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineMoonshineModelConfig {
    pub preprocessor: *const c_char,
    pub encoder: *const c_char,
    pub uncached_decoder: *const c_char,
    pub cached_decoder: *const c_char,
    pub merged_decoder: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineTdnnModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineLMConfig {
    pub model: *const c_char,
    pub scale: c_float,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineSenseVoiceModelConfig {
    pub model: *const c_char,
    pub language: *const c_char,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineDolphinModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineZipformerCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineWenetCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineOmnilingualAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineFunASRNanoModelConfig {
    pub encoder_adaptor: *const c_char,
    pub llm: *const c_char,
    pub embedding: *const c_char,
    pub tokenizer: *const c_char,
    pub system_prompt: *const c_char,
    pub user_prompt: *const c_char,
    pub max_new_tokens: i32,
    pub temperature: c_float,
    pub top_p: c_float,
    pub seed: i32,
    pub language: *const c_char,
    pub itn: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineMedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineFireRedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineQwen3ASRModelConfig {
    pub conv_frontend: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub tokenizer: *const c_char,
    pub max_total_len: i32,
    pub max_new_tokens: i32,
    pub temperature: c_float,
    pub top_p: c_float,
    pub seed: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineCohereTranscribeModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub use_punct: i32,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineModelConfig {
    pub transducer: OfflineTransducerModelConfig,
    pub paraformer: OfflineParaformerModelConfig,
    pub nemo_ctc: OfflineNemoEncDecCtcModelConfig,
    pub whisper: OfflineWhisperModelConfig,
    pub tdnn: OfflineTdnnModelConfig,

    pub tokens: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
    pub model_type: *const c_char,
    pub modeling_unit: *const c_char,
    pub bpe_vocab: *const c_char,
    pub telespeech_ctc: *const c_char,

    pub sense_voice: OfflineSenseVoiceModelConfig,
    pub moonshine: OfflineMoonshineModelConfig,
    pub fire_red_asr: OfflineFireRedAsrModelConfig,
    pub dolphin: OfflineDolphinModelConfig,
    pub zipformer_ctc: OfflineZipformerCtcModelConfig,
    pub canary: OfflineCanaryModelConfig,
    pub wenet_ctc: OfflineWenetCtcModelConfig,
    pub omnilingual: OfflineOmnilingualAsrCtcModelConfig,
    pub medasr: OfflineMedAsrCtcModelConfig,
    pub funasr_nano: OfflineFunASRNanoModelConfig,
    pub fire_red_asr_ctc: OfflineFireRedAsrCtcModelConfig,
    pub qwen3_asr: OfflineQwen3ASRModelConfig,
    pub cohere_transcribe: OfflineCohereTranscribeModelConfig,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineRecognizerConfig {
    pub feat_config: super::online_asr::FeatureConfig,
    pub model_config: OfflineModelConfig,
    pub lm_config: OfflineLMConfig,

    pub decoding_method: *const c_char,
    pub max_active_paths: i32,
    pub hotwords_file: *const c_char,
    pub hotwords_score: c_float,
    pub rule_fsts: *const c_char,
    pub rule_fars: *const c_char,
    pub blank_penalty: c_float,
    pub hr: super::online_asr::HomophoneReplacerConfig,
}

#[repr(C)]
pub struct OfflineRecognizer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OfflineStream {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateOfflineRecognizer(
        config: *const OfflineRecognizerConfig,
    ) -> *const OfflineRecognizer;

    pub fn SherpaOnnxDestroyOfflineRecognizer(recognizer: *const OfflineRecognizer);

    pub fn SherpaOnnxCreateOfflineStream(
        recognizer: *const OfflineRecognizer,
    ) -> *const OfflineStream;

    pub fn SherpaOnnxCreateOfflineStreamWithHotwords(
        recognizer: *const OfflineRecognizer,
        hotwords: *const c_char,
    ) -> *const OfflineStream;

    pub fn SherpaOnnxDestroyOfflineStream(stream: *const OfflineStream);

    pub fn SherpaOnnxAcceptWaveformOffline(
        stream: *const OfflineStream,
        sample_rate: i32,
        samples: *const f32,
        n: i32,
    );

    pub fn SherpaOnnxOfflineStreamSetOption(
        stream: *const OfflineStream,
        key: *const c_char,
        value: *const c_char,
    );

    pub fn SherpaOnnxOfflineStreamGetOption(
        stream: *const OfflineStream,
        key: *const c_char,
    ) -> *const c_char;

    pub fn SherpaOnnxOfflineStreamHasOption(
        stream: *const OfflineStream,
        key: *const c_char,
    ) -> i32;

    pub fn SherpaOnnxDecodeOfflineStream(
        recognizer: *const OfflineRecognizer,
        stream: *const OfflineStream,
    );

    pub fn SherpaOnnxDecodeMultipleOfflineStreams(
        recognizer: *const OfflineRecognizer,
        streams: *const *const OfflineStream,
        n: i32,
    );

    pub fn SherpaOnnxGetOfflineStreamResultAsJson(stream: *const OfflineStream) -> *const c_char;

    pub fn SherpaOnnxDestroyOfflineStreamResultJson(s: *const c_char);
}
