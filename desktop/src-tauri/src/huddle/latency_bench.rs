//! Ad-hoc baseline latency bench for the STT -> fake LLM -> TTS pipeline.
//!
//! Drives the REAL production machinery:
//!   - `SttPipeline::new` (rubato 48k->16k, earshot VAD, 300 ms silence flush,
//!     Parakeet TDT-CTC 110M int8 via sherpa-onnx, 1 thread)
//!   - `TtsPipeline::new_with_voice` (warmup synth, chunker, synth_chunk,
//!     rodio persistent Player, 20 ms lead-in)
//!
//! with a fake LLM in place of the relay/agent leg.
//!
//! Audio is fed in real-time 100 ms batches (mirroring the AudioWorklet
//! cadence) so VAD endpointing behaves exactly like production.
//!
//! Timestamps captured per turn:
//!   t_speech_end   last voiced sample delivered to push_audio (wall clock,
//!                  derived from the WAV's last voiced sample + feed pacing)
//!   t_transcript   text_rx yields the transcript
//!   t_speak        fake-LLM reply handed to TtsPipeline::speak
//!   t_first_audio  tts_active rising edge = first player.append accepted
//!
//! Run:
//!   BUZZ_BENCH_WAV=<48k f32 mono wav> cargo test --release -p buzz-desktop \
//!     --lib huddle::latency_bench -- --ignored --nocapture

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::stt::SttPipeline;
use super::tts::TtsPipeline;

/// Read a mono 32-bit-float WAV (as produced by `afconvert -d LEF32@48000`).
/// Minimal parser: walks RIFF chunks, asserts fmt = IEEE float mono 48 kHz.
fn read_wav_f32_48k(path: &str) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read wav");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut pos = 12usize;
    let mut fmt_ok = false;
    let mut data: Option<(usize, usize)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        match id {
            b"fmt " => {
                let format = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
                let rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
                assert_eq!(format, 3, "expected IEEE float wav");
                assert_eq!(channels, 1, "expected mono");
                assert_eq!(rate, 48_000, "expected 48 kHz");
                assert_eq!(bits, 32);
                fmt_ok = true;
            }
            b"data" => data = Some((body, len)),
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    assert!(fmt_ok, "fmt chunk missing");
    let (off, len) = data.expect("data chunk missing");
    bytes[off..off + len]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Index (in samples) one past the last sample whose |amplitude| exceeds the
/// threshold — "when the user stopped speaking" on the feed timeline.
fn last_voiced_sample(samples: &[f32], threshold: f32) -> usize {
    samples
        .iter()
        .rposition(|s| s.abs() > threshold)
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Poll a tokio mpsc receiver from sync context for up to `timeout`.
/// 1 ms poll keeps timestamp error negligible against ~100 ms scales.
fn tokio_recv_with_timeout(
    rx: &mut tokio::sync::mpsc::Receiver<String>,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(t) = rx.try_recv() {
            return Some(t);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct TurnResult {
    label: &'static str,
    transcript: String,
    stt_ms: f64,
    llm_ms: f64,
    tts_ms: f64,
    e2e_ms: f64,
}

#[test]
#[ignore = "ad-hoc latency baseline; needs models in ~/.buzz/models and an audio output device"]
fn baseline_stt_fake_llm_tts_first_audio() {
    let home = dirs::home_dir().expect("home");
    let stt_dir = home.join(".buzz/models/parakeet-tdt-ctc-110m-en");
    let tts_dir = home.join(".buzz/models/pocket-tts");
    assert!(
        stt_dir.join("model.int8.onnx").exists(),
        "parakeet model missing"
    );
    assert!(tts_dir.join("bundle.json").exists(), "pocket model missing");

    let wav_path = std::env::var("BUZZ_BENCH_WAV").expect("set BUZZ_BENCH_WAV");
    let samples_48k = read_wav_f32_48k(&wav_path);
    let speech_end_sample = last_voiced_sample(&samples_48k, 0.015);
    let audio_dur_s = samples_48k.len() as f64 / 48_000.0;
    let speech_end_s = speech_end_sample as f64 / 48_000.0;
    eprintln!(
        "bench: utterance {wav_path}: {audio_dur_s:.2} s total, speech ends at {speech_end_s:.2} s"
    );

    let llm_delay_ms: u64 = std::env::var("BUZZ_BENCH_LLM_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // ── Bring up the real pipelines, exactly as maybe_start_* do ────────────
    let tts_active = Arc::new(AtomicBool::new(false));
    let tts_cancel = Arc::new(AtomicBool::new(false));

    let t = Instant::now();
    let tts = TtsPipeline::new_with_voice(
        tts_dir,
        Arc::clone(&tts_active),
        Arc::clone(&tts_cancel),
        super::human_floor::HumanFloor::new(),
        "eve",
        None, // default output device
        None, // no Tauri app handle
    )
    .expect("tts pipeline");
    eprintln!(
        "bench: TTS pipeline ready (engine load + warmup + audio prime) in {:.0} ms",
        t.elapsed().as_secs_f64() * 1e3
    );

    let t = Instant::now();
    let (stt, mut text_rx) = SttPipeline::new(
        stt_dir,
        None,
        None,
        super::human_floor::HumanFloor::new(),
        None,
    )
    .expect("stt pipeline");
    // Recognizer loads inside the worker thread; give it time, then verify
    // liveness via a first throwaway feed below.
    std::thread::sleep(Duration::from_secs(2));
    assert!(!stt.is_finished(), "stt worker died during init");
    eprintln!(
        "bench: STT pipeline spawned ({:.0} ms incl. settle sleep)",
        t.elapsed().as_secs_f64() * 1e3
    );

    // Fake LLM replies: short / medium / long, cycled across turns.
    let replies: [(&'static str, &'static str); 3] = [
        ("reply_short", "Let me check."),
        ("reply_medium", "Got it. The relay deploy finished about two minutes ago and all checks passed."),
        ("reply_long", "Here's where things stand. The relay deploy finished cleanly and every health check is green. Two pods restarted during rollout, which is expected, and message latency is back to normal."),
    ];
    let turns: usize = std::env::var("BUZZ_BENCH_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);

    // 100 ms batches at 48 kHz, matching the AudioWorklet push cadence.
    const BATCH: usize = 4_800;
    let mut results: Vec<TurnResult> = Vec::new();

    let stt = Arc::new(stt);
    for turn in 0..turns {
        let (label, reply) = replies[turn % replies.len()];

        // Feed the utterance in real time from a separate thread (the
        // AudioWorklet role), then trailing silence so the 300 ms VAD flush
        // fires. The main thread meanwhile timestamps transcript arrival —
        // recv must NOT be serialized behind the silence feed, or the
        // measurement floor becomes the feed loop instead of the STT path.
        let feeder_stt = Arc::clone(&stt);
        let feeder_samples = samples_48k.clone();
        let feed_start = Instant::now();
        let feeder = std::thread::spawn(move || {
            let mut cursor = 0usize;
            while cursor < feeder_samples.len() {
                let end = (cursor + BATCH).min(feeder_samples.len());
                let bytes: Vec<u8> = feeder_samples[cursor..end]
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect();
                feeder_stt.push_audio(bytes).expect("push");
                cursor = end;
                // Pace to real time.
                let target = feed_start + Duration::from_millis((cursor / 48) as u64);
                let now = Instant::now();
                if target > now {
                    std::thread::sleep(target - now);
                }
            }
            // Trailing silence: 1 s guarantees the 300 ms flush window closes.
            let silence = vec![0u8; BATCH * 4];
            for _ in 0..10 {
                feeder_stt
                    .push_audio(silence.clone())
                    .expect("push silence");
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let t_speech_end = feed_start + Duration::from_secs_f64(speech_end_s);

        // Transcript arrival. An utterance with an intra-sentence pause can
        // VAD-split into multiple segments; keep the LAST one delivered so the
        // turn aligns with the true end of speech. The extra "is another
        // segment coming?" wait below is a HARNESS artifact (prod forwards
        // every segment immediately) and is excluded from all timings.
        let mut transcript = text_rx
            .blocking_recv()
            .expect("stt channel closed before transcript");
        let mut t_transcript = Instant::now();
        let mut segments = 1usize;
        loop {
            match text_rx.try_recv() {
                Ok(t) => {
                    transcript = t;
                    t_transcript = Instant::now();
                    segments += 1;
                }
                Err(_) => {
                    if feeder.is_finished() {
                        // Feed done (incl. 1 s trailing silence): any final
                        // segment has already flushed and decoded. One short
                        // grace poll covers a decode still in flight.
                        match tokio_recv_with_timeout(&mut text_rx, Duration::from_millis(500)) {
                            Some(t) => {
                                transcript = t;
                                t_transcript = Instant::now();
                                segments += 1;
                            }
                            None => break,
                        }
                    } else {
                        // Feeder still delivering audio — a later segment may
                        // arrive any time until the feed (plus flush window)
                        // completes. Keep waiting; do NOT break early or the
                        // tail segment leaks into the next turn.
                        if let Some(t) =
                            tokio_recv_with_timeout(&mut text_rx, Duration::from_millis(100))
                        {
                            transcript = t;
                            t_transcript = Instant::now();
                            segments += 1;
                        }
                    }
                }
            }
        }
        feeder.join().expect("feeder");

        // Fake LLM. Applied AFTER the harness-only segment wait; llm_ms is the
        // configured delay, so the harness wait never leaks into any timing.
        if llm_delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(llm_delay_ms));
        }
        let t_speak = Instant::now();
        tts.speak(reply.to_string()).expect("speak");

        // First audio: tts_active rising edge == first accepted player append.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !tts_active.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "no first audio within 30 s");
            std::thread::sleep(Duration::from_micros(500));
        }
        let t_first_audio = Instant::now();

        let stt_ms = (t_transcript - t_speech_end).as_secs_f64() * 1e3;
        // llm_ms is exactly the configured fake-LLM delay; tts is measured
        // from speak() to first accepted append. e2e composes the three real
        // legs so the harness-only segment wait (between t_transcript and the
        // fake-LLM sleep) never inflates the pipeline number.
        let llm_ms = llm_delay_ms as f64;
        let tts_ms = (t_first_audio - t_speak).as_secs_f64() * 1e3;
        let e2e_ms = stt_ms + llm_ms + tts_ms;
        eprintln!(
            "bench turn {turn} [{label}]: stt={stt_ms:.0}ms llm={llm_ms:.0}ms tts_first_audio={tts_ms:.0}ms e2e={e2e_ms:.0}ms segments={segments} transcript={transcript:?}"
        );
        results.push(TurnResult {
            label,
            transcript,
            stt_ms,
            llm_ms,
            tts_ms,
            e2e_ms,
        });

        // Wait for playback to drain + prod cooldown before the next turn.
        while tts_active.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Summary JSON for the write-up.
    println!("[");
    for (i, r) in results.iter().enumerate() {
        let comma = if i + 1 < results.len() { "," } else { "" };
        println!(
            "  {{\"turn\":{i},\"label\":\"{}\",\"stt_ms\":{:.1},\"llm_ms\":{:.1},\"tts_first_audio_ms\":{:.1},\"e2e_ms\":{:.1},\"transcript\":{:?}}}{comma}",
            r.label, r.stt_ms, r.llm_ms, r.tts_ms, r.e2e_ms, r.transcript
        );
    }
    println!("]");

    stt.shutdown();
    tts.shutdown();
}
