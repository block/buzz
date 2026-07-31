#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineTransducerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub joiner: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineParaformerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineZipformer2CtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineNemoCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineToneCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineModelConfig {
    pub transducer: OnlineTransducerModelConfig,
    pub paraformer: OnlineParaformerModelConfig,
    pub zipformer2_ctc: OnlineZipformer2CtcModelConfig,

    pub tokens: *const c_char,
    pub num_threads: i32,
    pub provider: *const c_char,
    pub debug: i32,

    pub model_type: *const c_char,

    // cjkchar | bpe | cjkchar+bpe
    pub modeling_unit: *const c_char,

    pub bpe_vocab: *const c_char,

    pub tokens_buf: *const u8,
    pub tokens_buf_size: i32,

    pub nemo_ctc: OnlineNemoCtcModelConfig,
    pub t_one_ctc: OnlineToneCtcModelConfig,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct FeatureConfig {
    pub sample_rate: i32,
    pub feature_dim: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineCtcFstDecoderConfig {
    pub graph: *const c_char,
    pub max_active: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct HomophoneReplacerConfig {
    pub dict_dir: *const c_char,
    pub lexicon: *const c_char,
    pub rule_fsts: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlineRecognizerConfig {
    pub feat_config: FeatureConfig,
    pub model_config: OnlineModelConfig,

    // greedy_search | modified_beam_search
    pub decoding_method: *const c_char,

    pub max_active_paths: i32,

    pub enable_endpoint: i32,

    pub rule1_min_trailing_silence: c_float,
    pub rule2_min_trailing_silence: c_float,
    pub rule3_min_utterance_length: c_float,

    pub hotwords_file: *const c_char,
    pub hotwords_score: c_float,

    pub ctc_fst_decoder_config: OnlineCtcFstDecoderConfig,

    pub rule_fsts: *const c_char,
    pub rule_fars: *const c_char,

    pub blank_penalty: c_float,

    pub hotwords_buf: *const u8,
    pub hotwords_buf_size: i32,

    pub hr: HomophoneReplacerConfig,
}

#[repr(C)]
pub struct OnlineRecognizer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OnlineStream {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateOnlineRecognizer(
        config: *const OnlineRecognizerConfig,
    ) -> *const OnlineRecognizer;

    pub fn SherpaOnnxDestroyOnlineRecognizer(recognizer: *const OnlineRecognizer);

    pub fn SherpaOnnxCreateOnlineStream(recognizer: *const OnlineRecognizer)
        -> *const OnlineStream;

    pub fn SherpaOnnxCreateOnlineStreamWithHotwords(
        recognizer: *const OnlineRecognizer,
        hotwords: *const c_char,
    ) -> *const OnlineStream;

    pub fn SherpaOnnxDestroyOnlineStream(stream: *const OnlineStream);

    pub fn SherpaOnnxOnlineStreamAcceptWaveform(
        stream: *const OnlineStream,
        sample_rate: i32,
        samples: *const f32,
        n: i32,
    );

    pub fn SherpaOnnxIsOnlineStreamReady(
        recognizer: *const OnlineRecognizer,
        stream: *const OnlineStream,
    ) -> i32;

    pub fn SherpaOnnxDecodeOnlineStream(
        recognizer: *const OnlineRecognizer,
        stream: *const OnlineStream,
    );

    pub fn SherpaOnnxDecodeMultipleOnlineStreams(
        recognizer: *const OnlineRecognizer,
        streams: *const *const OnlineStream,
        n: i32,
    );

    pub fn SherpaOnnxGetOnlineStreamResultAsJson(
        recognizer: *const OnlineRecognizer,
        stream: *const OnlineStream,
    ) -> *const c_char;

    pub fn SherpaOnnxDestroyOnlineStreamResultJson(s: *const c_char);

    pub fn SherpaOnnxOnlineStreamReset(
        recognizer: *const OnlineRecognizer,
        stream: *const OnlineStream,
    );

    pub fn SherpaOnnxOnlineStreamInputFinished(stream: *const OnlineStream);

    pub fn SherpaOnnxOnlineStreamSetOption(
        stream: *const OnlineStream,
        key: *const c_char,
        value: *const c_char,
    );

    pub fn SherpaOnnxOnlineStreamGetOption(
        stream: *const OnlineStream,
        key: *const c_char,
    ) -> *const c_char;

    pub fn SherpaOnnxOnlineStreamHasOption(stream: *const OnlineStream, key: *const c_char) -> i32;

    pub fn SherpaOnnxOnlineStreamIsEndpoint(
        recognizer: *const OnlineRecognizer,
        stream: *const OnlineStream,
    ) -> i32;
}
