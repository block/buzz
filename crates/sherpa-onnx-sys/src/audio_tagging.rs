#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineZipformerAudioTaggingModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AudioTaggingModelConfig {
    pub zipformer: OfflineZipformerAudioTaggingModelConfig,
    pub ced: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AudioTaggingConfig {
    pub model: AudioTaggingModelConfig,
    pub labels: *const c_char,
    pub top_k: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AudioEvent {
    pub name: *const c_char,
    pub index: i32,
    pub prob: c_float,
}

#[repr(C)]
pub struct AudioTagging {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateAudioTagging(config: *const AudioTaggingConfig) -> *const AudioTagging;

    pub fn SherpaOnnxDestroyAudioTagging(tagger: *const AudioTagging);

    pub fn SherpaOnnxAudioTaggingCreateOfflineStream(
        tagger: *const AudioTagging,
    ) -> *const crate::offline_asr::OfflineStream;

    pub fn SherpaOnnxAudioTaggingCompute(
        tagger: *const AudioTagging,
        s: *const crate::offline_asr::OfflineStream,
        top_k: i32,
    ) -> *const *const AudioEvent;

    pub fn SherpaOnnxAudioTaggingFreeResults(p: *const *const AudioEvent);
}
