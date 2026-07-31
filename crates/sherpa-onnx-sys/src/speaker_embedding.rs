#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpeakerEmbeddingExtractorConfig {
    pub model: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
pub struct SpeakerEmbeddingExtractor {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SpeakerEmbeddingManager {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpeakerEmbeddingManagerSpeakerMatch {
    pub score: c_float,
    pub name: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SpeakerEmbeddingManagerBestMatchesResult {
    pub matches: *const SpeakerEmbeddingManagerSpeakerMatch,
    pub count: i32,
}

extern "C" {
    pub fn SherpaOnnxCreateSpeakerEmbeddingExtractor(
        config: *const SpeakerEmbeddingExtractorConfig,
    ) -> *const SpeakerEmbeddingExtractor;

    pub fn SherpaOnnxDestroySpeakerEmbeddingExtractor(p: *const SpeakerEmbeddingExtractor);

    pub fn SherpaOnnxSpeakerEmbeddingExtractorDim(p: *const SpeakerEmbeddingExtractor) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingExtractorCreateStream(
        p: *const SpeakerEmbeddingExtractor,
    ) -> *const crate::online_asr::OnlineStream;

    pub fn SherpaOnnxSpeakerEmbeddingExtractorIsReady(
        p: *const SpeakerEmbeddingExtractor,
        s: *const crate::online_asr::OnlineStream,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding(
        p: *const SpeakerEmbeddingExtractor,
        s: *const crate::online_asr::OnlineStream,
    ) -> *const c_float;

    pub fn SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding(v: *const c_float);

    pub fn SherpaOnnxCreateSpeakerEmbeddingManager(dim: i32) -> *const SpeakerEmbeddingManager;

    pub fn SherpaOnnxDestroySpeakerEmbeddingManager(p: *const SpeakerEmbeddingManager);

    pub fn SherpaOnnxSpeakerEmbeddingManagerAdd(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
        v: *const c_float,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerAddList(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
        v: *const *const c_float,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerAddListFlattened(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
        v: *const c_float,
        n: i32,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerRemove(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerSearch(
        p: *const SpeakerEmbeddingManager,
        v: *const c_float,
        threshold: c_float,
    ) -> *const c_char;

    pub fn SherpaOnnxSpeakerEmbeddingManagerFreeSearch(name: *const c_char);

    pub fn SherpaOnnxSpeakerEmbeddingManagerGetBestMatches(
        p: *const SpeakerEmbeddingManager,
        v: *const c_float,
        threshold: c_float,
        n: i32,
    ) -> *const SpeakerEmbeddingManagerBestMatchesResult;

    pub fn SherpaOnnxSpeakerEmbeddingManagerFreeBestMatches(
        r: *const SpeakerEmbeddingManagerBestMatchesResult,
    );

    pub fn SherpaOnnxSpeakerEmbeddingManagerVerify(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
        v: *const c_float,
        threshold: c_float,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerContains(
        p: *const SpeakerEmbeddingManager,
        name: *const c_char,
    ) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerNumSpeakers(p: *const SpeakerEmbeddingManager) -> i32;

    pub fn SherpaOnnxSpeakerEmbeddingManagerGetAllSpeakers(
        p: *const SpeakerEmbeddingManager,
    ) -> *const *const c_char;

    pub fn SherpaOnnxSpeakerEmbeddingManagerFreeAllSpeakers(names: *const *const c_char);
}
