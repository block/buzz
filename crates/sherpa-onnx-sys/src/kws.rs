#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_float};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct KeywordResult {
    pub keyword: *const c_char,
    pub tokens: *const c_char,
    pub tokens_arr: *const *const c_char,
    pub count: i32,
    pub timestamps: *mut c_float,
    pub start_time: c_float,
    pub json: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct KeywordSpotterConfig {
    pub feat_config: super::online_asr::FeatureConfig,
    pub model_config: super::online_asr::OnlineModelConfig,
    pub max_active_paths: i32,
    pub num_trailing_blanks: i32,
    pub keywords_score: c_float,
    pub keywords_threshold: c_float,
    pub keywords_file: *const c_char,
    pub keywords_buf: *const c_char,
    pub keywords_buf_size: i32,
}

#[repr(C)]
pub struct KeywordSpotter {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateKeywordSpotter(
        config: *const KeywordSpotterConfig,
    ) -> *const KeywordSpotter;

    pub fn SherpaOnnxDestroyKeywordSpotter(spotter: *const KeywordSpotter);

    pub fn SherpaOnnxCreateKeywordStream(
        spotter: *const KeywordSpotter,
    ) -> *const super::online_asr::OnlineStream;

    pub fn SherpaOnnxCreateKeywordStreamWithKeywords(
        spotter: *const KeywordSpotter,
        keywords: *const c_char,
    ) -> *const super::online_asr::OnlineStream;

    pub fn SherpaOnnxIsKeywordStreamReady(
        spotter: *const KeywordSpotter,
        stream: *const super::online_asr::OnlineStream,
    ) -> i32;

    pub fn SherpaOnnxDecodeKeywordStream(
        spotter: *const KeywordSpotter,
        stream: *const super::online_asr::OnlineStream,
    );

    pub fn SherpaOnnxResetKeywordStream(
        spotter: *const KeywordSpotter,
        stream: *const super::online_asr::OnlineStream,
    );

    pub fn SherpaOnnxDecodeMultipleKeywordStreams(
        spotter: *const KeywordSpotter,
        streams: *const *const super::online_asr::OnlineStream,
        n: i32,
    );

    pub fn SherpaOnnxGetKeywordResult(
        spotter: *const KeywordSpotter,
        stream: *const super::online_asr::OnlineStream,
    ) -> *const KeywordResult;

    pub fn SherpaOnnxDestroyKeywordResult(r: *const KeywordResult);

    pub fn SherpaOnnxGetKeywordResultAsJson(
        spotter: *const KeywordSpotter,
        stream: *const super::online_asr::OnlineStream,
    ) -> *const c_char;

    pub fn SherpaOnnxFreeKeywordResultJson(s: *const c_char);
}
