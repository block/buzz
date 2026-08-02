//! Focused tests for Pocket TTS callback streaming and playback queueing.

use super::*;
use std::sync::mpsc;

fn streaming_test_chunk(value: f32, len: usize) -> Vec<f32> {
    vec![value; len]
}

#[test]
fn cumulative_pcm_returns_only_new_samples() {
    let mut delta = CumulativePcmDelta::default();
    let first = vec![0.25; 1000];
    let mut second = first.clone();
    second.extend(vec![0.5; 800]);

    assert_eq!(delta.next(&first).expect("first batch"), Some(&first[..]));
    assert_eq!(
        delta.next(&second).expect("second batch"),
        Some(&second[1000..])
    );
}

#[test]
fn cumulative_pcm_ignores_equal_length_repeats_between_model_chunks() {
    let mut delta = CumulativePcmDelta::default();
    let samples = vec![0.25; 1000];

    assert!(delta.next(&samples).expect("first batch").is_some());
    assert_eq!(delta.next(&samples).expect("repeated batch"), None);
}

#[test]
fn cumulative_pcm_rejects_decreasing_lengths() {
    let mut delta = CumulativePcmDelta::default();
    delta.next(&vec![0.25; 1000]).expect("first batch");

    let error = delta
        .next(&vec![0.25; 800])
        .expect_err("decreasing cumulative length must fail");

    assert!(error.contains("decreased from 1000 to 800"));
}

#[test]
fn cumulative_multi_chunk_callbacks_reconstruct_final_pcm_once() {
    let first = streaming_test_chunk(0.25, 1000);
    let second = streaming_test_chunk(0.5, 1000);
    let complete = [first.as_slice(), second.as_slice()].concat();
    let cumulative = [first.clone(), first.clone(), complete.clone()];
    let mut delta = CumulativePcmDelta::default();
    let mut assembler = PocketStreamAssembler::default();
    let mut queued = Vec::<Vec<f32>>::new();

    for callback in cumulative {
        if let Some(samples) = delta.next(&callback).expect("valid cumulative callback") {
            assembler
                .push(samples, queued.is_empty(), |buffer| {
                    queued.push(buffer);
                    Ok(())
                })
                .expect("queue callback delta");
        }
    }
    assembler
        .finish(&complete, 2400, queued.is_empty(), |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("finish stream");

    assert_eq!(assembler.callback_count, 2);
    assert_eq!(assembler.queued_samples, complete.len());
    let output = queued.concat();
    let speech = &output[CHUNK_LEAD_IN_SAMPLES..CHUNK_LEAD_IN_SAMPLES + complete.len()];
    assert!(speech[..1000].iter().all(|sample| *sample == 0.25));
    assert!(speech[1000..1000 + (1000 - FADE_OUT_SAMPLES)]
        .iter()
        .all(|sample| *sample == 0.5));
}

#[test]
fn pocket_streaming_queues_before_generation_finishes() {
    let mut assembler = PocketStreamAssembler::default();
    let (sender, receiver) = mpsc::channel();

    assembler
        .push(&streaming_test_chunk(0.25, 1000), true, |buffer| {
            sender
                .send(buffer)
                .map_err(|error| format!("send test PCM: {error}"))
        })
        .expect("stream first callback");

    let queued = receiver
        .try_recv()
        .expect("playback queue receives PCM before finish");
    assert_eq!(
        queued.len(),
        CHUNK_LEAD_IN_SAMPLES + 1000 - STREAM_TAIL_SAMPLES
    );
}

#[test]
fn pocket_streaming_preserves_chunk_lead_in_while_playback_is_active() {
    let mut assembler = PocketStreamAssembler::default();
    let mut queued = Vec::<Vec<f32>>::new();

    assembler
        .push(&streaming_test_chunk(0.25, 1000), false, |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("stream first callback behind active playback");

    assert_eq!(queued.len(), 1);
    assert!(queued[0][..CHUNK_LEAD_IN_SAMPLES]
        .iter()
        .all(|sample| *sample == 0.0));
    assert_eq!(queued[0][CHUNK_LEAD_IN_SAMPLES], 0.25);
}

#[test]
fn pocket_streaming_preserves_quiet_speech_after_leading_silence() {
    let mut assembler = PocketStreamAssembler::default();
    let mut samples = vec![0.0; 5000];
    samples.extend(streaming_test_chunk(0.005, 500));
    samples.extend(streaming_test_chunk(0.5, 1000));
    let mut queued = Vec::<Vec<f32>>::new();

    assembler
        .push(&samples, true, |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("stream quiet onset");
    assembler
        .finish(&samples, 2400, queued.is_empty(), |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("finish stream");

    let output = queued.concat();
    let speech = &output[CHUNK_LEAD_IN_SAMPLES..];
    let quiet_start = speech
        .iter()
        .position(|sample| *sample == 0.005)
        .expect("quiet speech onset must be preserved");
    assert!(speech[quiet_start..quiet_start + 500]
        .iter()
        .all(|sample| *sample == 0.005));
    assert!(speech[quiet_start + 500..].contains(&0.5));
}

#[test]
fn pocket_streaming_cancellation_stops_queueing() {
    let mut assembler = PocketStreamAssembler::default();
    let queued = Vec::<Vec<f32>>::new();

    let error = assembler
        .push(&streaming_test_chunk(0.25, 1000), true, |buffer| {
            let _ = buffer;
            Err("Pocket TTS streaming cancelled".to_string())
        })
        .expect_err("cancelled append must stop generation");

    assert_eq!(error, "Pocket TTS streaming cancelled");
    assert_eq!(assembler.queued_samples, 0);
    assert!(queued.is_empty());
}

#[test]
fn pocket_streaming_surfaces_playback_queue_errors() {
    let mut assembler = PocketStreamAssembler::default();

    let error = assembler
        .push(&streaming_test_chunk(0.25, 1000), true, |_| {
            Err("audio device disconnected".to_string())
        })
        .expect_err("queue failure must be returned");

    assert_eq!(error, "audio device disconnected");
}

#[test]
fn pocket_streaming_rejects_callback_contract_mismatch() {
    let mut assembler = PocketStreamAssembler::default();
    assembler
        .push(&streaming_test_chunk(0.25, 1000), true, |_| Ok(()))
        .expect("callback");

    let error = assembler
        .finish(&streaming_test_chunk(0.25, 1200), 2400, false, |_| Ok(()))
        .expect_err("mismatched callback samples must not be duplicated");

    assert!(error.contains("callback contract mismatch"));
}

#[test]
fn pocket_streaming_rearms_lead_in_after_playback_underrun() {
    let mut assembler = PocketStreamAssembler::default();
    let mut queued = Vec::<Vec<f32>>::new();

    assembler
        .push(&streaming_test_chunk(0.25, 1000), true, |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("queue first decoder block");
    assembler
        .push(&streaming_test_chunk(0.5, 1000), true, |buffer| {
            queued.push(buffer);
            Ok(())
        })
        .expect("queue decoder block after simulated drain");

    assert_eq!(queued.len(), 2);
    assert!(queued[1][..CHUNK_LEAD_IN_SAMPLES]
        .iter()
        .all(|sample| *sample == 0.0));
    assert_eq!(queued[1][CHUNK_LEAD_IN_SAMPLES], 0.25);
}
