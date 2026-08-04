//! Acceptance test for the dictation STT configuration.
//!
//! `start_dictation` constructs [`SttPipeline`] with `tts_active` pinned
//! false, `tts_cancel` = None, and `ptt_active` = None — the combination that
//! selects continuous VAD mode. That choice is invisible at compile time: get
//! it wrong (e.g. pass a `ptt_active` flag that is never set) and the pipeline
//! runs happily while emitting nothing forever. This test pins it down by
//! feeding real speech through the real model and asserting text comes back.

use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};

use crate::huddle::{models, stt::SttPipeline};

/// Decode a 16-bit PCM mono WAV into f32 samples in [-1, 1].
///
/// Minimal parser: the fixtures ship with the model and are known-good
/// canonical WAV, so this walks the chunk list only far enough to find `data`.
fn decode_wav_i16_mono(bytes: &[u8]) -> Vec<f32> {
    assert_eq!(&bytes[0..4], b"RIFF", "not a RIFF file");
    assert_eq!(&bytes[8..12], b"WAVE", "not a WAVE file");

    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if chunk_id == b"data" {
            let end = (body + chunk_size).min(bytes.len());
            return bytes[body..end]
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                .collect();
        }
        // Chunks are word-aligned.
        offset = body + chunk_size + (chunk_size & 1);
    }
    panic!("no data chunk in WAV");
}

/// Nearest-neighbour 16 kHz → 48 kHz upsample (each sample repeated 3×).
///
/// The pipeline's input contract is 48 kHz (it resamples back down to 16 kHz
/// internally via rubato). Repeating samples is a crude but adequate inverse
/// for a fixture that started life at 16 kHz — good enough to exercise the
/// real VAD and inference path.
fn upsample_16k_to_48k(samples: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(samples.len() * 3);
    for &s in samples {
        out.push(s);
        out.push(s);
        out.push(s);
    }
    out
}

fn f32_to_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Hardware-gated (`#[ignore]`): loads the real Parakeet model from
/// `~/.buzz/models/`. Run with:
///   cargo test -p buzz-desktop dictation_pipeline_transcribes_speech \
///     -- --ignored --nocapture
#[test]
#[ignore = "loads a real model; run manually with --ignored"]
fn dictation_pipeline_transcribes_speech() {
    let Some(model_dir) = models::stt_model_dir() else {
        panic!("STT model not installed — run the desktop app once to download it");
    };
    let wav_path = model_dir.join("test_wavs").join("1.wav");
    let wav = std::fs::read(&wav_path).expect("read bundled test wav");
    let pcm48 = upsample_16k_to_48k(&decode_wav_i16_mono(&wav));

    // Exactly the configuration start_dictation uses.
    let (pipeline, mut text_rx) =
        SttPipeline::new(model_dir, Arc::new(AtomicBool::new(false)), None, None)
            .expect("spawn dictation stt pipeline");

    // Feed in 100 ms batches (4800 samples at 48 kHz), matching the worklet.
    const BATCH_SAMPLES: usize = 4800;
    for batch in pcm48.chunks(BATCH_SAMPLES) {
        pipeline
            .push_audio(f32_to_le_bytes(batch))
            .expect("push audio batch");
        // The queue is bounded and drops on overflow; pace the feed so the
        // worker keeps up and the VAD sees a realistic arrival rate.
        std::thread::sleep(Duration::from_millis(20));
    }
    // Trailing silence so the VAD closes the utterance and flushes to the
    // recognizer — without this the speech buffer never finalizes.
    let silence = vec![0.0f32; BATCH_SAMPLES];
    for _ in 0..40 {
        pipeline
            .push_audio(f32_to_le_bytes(&silence))
            .expect("push silence batch");
        std::thread::sleep(Duration::from_millis(20));
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut transcript = String::new();
    while Instant::now() < deadline {
        match text_rx.try_recv() {
            Ok(text) => {
                transcript.push_str(&text);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    println!("dictation transcript: {transcript:?}");
    assert!(
        !transcript.trim().is_empty(),
        "dictation pipeline produced no transcript — the continuous-VAD \
         configuration in start_dictation is not emitting text"
    );
}
