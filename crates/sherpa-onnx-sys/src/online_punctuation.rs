#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::c_char;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlinePunctuationModelConfig {
    pub cnn_bilstm: *const c_char,
    pub bpe_vocab: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OnlinePunctuationConfig {
    pub model: OnlinePunctuationModelConfig,
}

#[repr(C)]
pub struct OnlinePunctuation {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateOnlinePunctuation(
        config: *const OnlinePunctuationConfig,
    ) -> *const OnlinePunctuation;

    pub fn SherpaOnnxDestroyOnlinePunctuation(punctuation: *const OnlinePunctuation);

    pub fn SherpaOnnxOnlinePunctuationAddPunct(
        punctuation: *const OnlinePunctuation,
        text: *const c_char,
    ) -> *const c_char;

    pub fn SherpaOnnxOnlinePunctuationFreeText(text: *const c_char);
}
