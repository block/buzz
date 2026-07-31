#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::c_char;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpokenLanguageIdentificationWhisperConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub tail_paddings: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpokenLanguageIdentificationConfig {
    pub whisper: SpokenLanguageIdentificationWhisperConfig,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
pub struct SpokenLanguageIdentification {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpokenLanguageIdentificationResult {
    pub lang: *const c_char,
}

extern "C" {
    pub fn SherpaOnnxCreateSpokenLanguageIdentification(
        config: *const SpokenLanguageIdentificationConfig,
    ) -> *const SpokenLanguageIdentification;

    pub fn SherpaOnnxDestroySpokenLanguageIdentification(slid: *const SpokenLanguageIdentification);

    pub fn SherpaOnnxSpokenLanguageIdentificationCreateOfflineStream(
        slid: *const SpokenLanguageIdentification,
    ) -> *const super::offline_asr::OfflineStream;

    pub fn SherpaOnnxSpokenLanguageIdentificationCompute(
        slid: *const SpokenLanguageIdentification,
        stream: *const super::offline_asr::OfflineStream,
    ) -> *const SpokenLanguageIdentificationResult;

    pub fn SherpaOnnxDestroySpokenLanguageIdentificationResult(
        r: *const SpokenLanguageIdentificationResult,
    );
}
