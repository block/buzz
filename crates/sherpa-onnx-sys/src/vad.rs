use std::os::raw::{c_char, c_float};

#[repr(C)]
pub struct SileroVadModelConfig {
    pub model: *const c_char,
    pub threshold: c_float,
    pub min_silence_duration: c_float,
    pub min_speech_duration: c_float,
    pub window_size: i32,
    pub max_speech_duration: c_float,
}

#[repr(C)]
pub struct TenVadModelConfig {
    pub model: *const c_char,
    pub threshold: c_float,
    pub min_silence_duration: c_float,
    pub min_speech_duration: c_float,
    pub window_size: i32,
    pub max_speech_duration: c_float,
}

#[repr(C)]
pub struct VadModelConfig {
    pub silero_vad: SileroVadModelConfig,
    pub sample_rate: i32,
    pub num_threads: i32,
    pub provider: *const c_char,
    pub debug: i32,
    pub ten_vad: TenVadModelConfig,
}

#[repr(C)]
pub struct CircularBuffer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SpeechSegment {
    pub start: i32,
    pub samples: *mut f32,
    pub n: i32,
}

#[repr(C)]
pub struct VoiceActivityDetector {
    _private: [u8; 0],
}

extern "C" {
    pub fn SherpaOnnxCreateCircularBuffer(capacity: i32) -> *const CircularBuffer;
    pub fn SherpaOnnxDestroyCircularBuffer(buffer: *const CircularBuffer);
    pub fn SherpaOnnxCircularBufferPush(buffer: *const CircularBuffer, p: *const f32, n: i32);
    pub fn SherpaOnnxCircularBufferGet(
        buffer: *const CircularBuffer,
        start_index: i32,
        n: i32,
    ) -> *const f32;
    pub fn SherpaOnnxCircularBufferFree(p: *const f32);
    pub fn SherpaOnnxCircularBufferPop(buffer: *const CircularBuffer, n: i32);
    pub fn SherpaOnnxCircularBufferSize(buffer: *const CircularBuffer) -> i32;
    pub fn SherpaOnnxCircularBufferHead(buffer: *const CircularBuffer) -> i32;
    pub fn SherpaOnnxCircularBufferReset(buffer: *const CircularBuffer);

    pub fn SherpaOnnxCreateVoiceActivityDetector(
        config: *const VadModelConfig,
        buffer_size_in_seconds: c_float,
    ) -> *const VoiceActivityDetector;
    pub fn SherpaOnnxDestroyVoiceActivityDetector(p: *const VoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorAcceptWaveform(
        p: *const VoiceActivityDetector,
        samples: *const f32,
        n: i32,
    );
    pub fn SherpaOnnxVoiceActivityDetectorEmpty(p: *const VoiceActivityDetector) -> i32;
    pub fn SherpaOnnxVoiceActivityDetectorDetected(p: *const VoiceActivityDetector) -> i32;
    pub fn SherpaOnnxVoiceActivityDetectorPop(p: *const VoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorClear(p: *const VoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorFront(
        p: *const VoiceActivityDetector,
    ) -> *const SpeechSegment;
    pub fn SherpaOnnxDestroySpeechSegment(p: *const SpeechSegment);
    pub fn SherpaOnnxVoiceActivityDetectorReset(p: *const VoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorFlush(p: *const VoiceActivityDetector);
}
