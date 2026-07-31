#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::c_char;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflinePunctuationModelConfig {
    pub ct_transformer: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflinePunctuationConfig {
    pub model: OfflinePunctuationModelConfig,
}

#[repr(C)]
pub struct OfflinePunctuation {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateOfflinePunctuation(
        config: *const OfflinePunctuationConfig,
    ) -> *const OfflinePunctuation;

    pub fn SherpaOnnxDestroyOfflinePunctuation(punct: *const OfflinePunctuation);

    pub fn SherpaOfflinePunctuationAddPunct(
        punct: *const OfflinePunctuation,
        text: *const c_char,
    ) -> *const c_char;

    pub fn SherpaOfflinePunctuationFreeText(text: *const c_char);
}
