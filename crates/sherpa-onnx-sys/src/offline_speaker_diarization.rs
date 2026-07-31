#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineSpeakerSegmentationPyannoteModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineSpeakerSegmentationModelConfig {
    pub pyannote: OfflineSpeakerSegmentationPyannoteModelConfig,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct FastClusteringConfig {
    pub num_clusters: i32,
    pub threshold: c_float,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineSpeakerDiarizationConfig {
    pub segmentation: OfflineSpeakerSegmentationModelConfig,
    pub embedding: crate::speaker_embedding::SpeakerEmbeddingExtractorConfig,
    pub clustering: FastClusteringConfig,
    pub min_duration_on: c_float,
    pub min_duration_off: c_float,
}

#[repr(C)]
pub struct OfflineSpeakerDiarization {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OfflineSpeakerDiarizationResult {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct OfflineSpeakerDiarizationSegment {
    pub start: c_float,
    pub end: c_float,
    pub speaker: i32,
}

extern "C" {
    pub fn SherpaOnnxCreateOfflineSpeakerDiarization(
        config: *const OfflineSpeakerDiarizationConfig,
    ) -> *const OfflineSpeakerDiarization;

    pub fn SherpaOnnxDestroyOfflineSpeakerDiarization(sd: *const OfflineSpeakerDiarization);

    pub fn SherpaOnnxOfflineSpeakerDiarizationGetSampleRate(
        sd: *const OfflineSpeakerDiarization,
    ) -> i32;

    pub fn SherpaOnnxOfflineSpeakerDiarizationSetConfig(
        sd: *const OfflineSpeakerDiarization,
        config: *const OfflineSpeakerDiarizationConfig,
    );

    pub fn SherpaOnnxOfflineSpeakerDiarizationResultGetNumSpeakers(
        r: *const OfflineSpeakerDiarizationResult,
    ) -> i32;

    pub fn SherpaOnnxOfflineSpeakerDiarizationResultGetNumSegments(
        r: *const OfflineSpeakerDiarizationResult,
    ) -> i32;

    pub fn SherpaOnnxOfflineSpeakerDiarizationResultSortByStartTime(
        r: *const OfflineSpeakerDiarizationResult,
    ) -> *const OfflineSpeakerDiarizationSegment;

    pub fn SherpaOnnxOfflineSpeakerDiarizationDestroySegment(
        s: *const OfflineSpeakerDiarizationSegment,
    );

    pub fn SherpaOnnxOfflineSpeakerDiarizationProcess(
        sd: *const OfflineSpeakerDiarization,
        samples: *const c_float,
        n: i32,
    ) -> *const OfflineSpeakerDiarizationResult;

    pub fn SherpaOnnxOfflineSpeakerDiarizationDestroyResult(
        r: *const OfflineSpeakerDiarizationResult,
    );
}
