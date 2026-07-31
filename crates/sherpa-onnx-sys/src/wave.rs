#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::os::raw::c_char;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SherpaOnnxWave {
    /// Samples normalized to [-1, 1]
    pub samples: *const f32,
    pub sample_rate: i32,
    pub num_samples: i32,
}

extern "C" {
    /// Read a WAV file. Returns NULL on error.
    pub fn SherpaOnnxReadWave(filename: *const c_char) -> *const SherpaOnnxWave;

    /// Free memory allocated by SherpaOnnxReadWave
    pub fn SherpaOnnxFreeWave(wave: *const SherpaOnnxWave);

    /// Write a WAV file. Returns 1 on success, 0 on failure.
    pub fn SherpaOnnxWriteWave(
        samples: *const f32,
        n: i32,
        sample_rate: i32,
        filename: *const c_char,
    ) -> i32;
}
