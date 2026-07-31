use std::os::raw::c_float;

#[repr(C)]
pub struct SherpaOnnxLinearResampler {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SherpaOnnxResampleOut {
    pub samples: *const f32,
    pub n: i32,
}

extern "C" {
    pub fn SherpaOnnxCreateLinearResampler(
        samp_rate_in_hz: i32,
        samp_rate_out_hz: i32,
        filter_cutoff_hz: c_float,
        num_zeros: i32,
    ) -> *const SherpaOnnxLinearResampler;

    pub fn SherpaOnnxDestroyLinearResampler(p: *const SherpaOnnxLinearResampler);

    pub fn SherpaOnnxLinearResamplerReset(p: *const SherpaOnnxLinearResampler);

    pub fn SherpaOnnxLinearResamplerResample(
        p: *const SherpaOnnxLinearResampler,
        input: *const f32,
        input_dim: i32,
        flush: i32,
    ) -> *const SherpaOnnxResampleOut;

    pub fn SherpaOnnxLinearResamplerResampleFree(p: *const SherpaOnnxResampleOut);

    pub fn SherpaOnnxLinearResamplerResampleGetInputSampleRate(
        p: *const SherpaOnnxLinearResampler,
    ) -> i32;

    pub fn SherpaOnnxLinearResamplerResampleGetOutputSampleRate(
        p: *const SherpaOnnxLinearResampler,
    ) -> i32;
}
