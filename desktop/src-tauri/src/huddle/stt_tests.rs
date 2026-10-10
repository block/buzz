use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    flush_to_stt, has_enough_voiced_audio, run_stt_receive_loop, spawn_openai_decoder,
    vad_flush_allowed, DecodeOutcome, HumanFloor, OpenAiSttConfig, SpeechDecoder, SttAudioInput,
    SttAudioOrigin, SttLoopInput, VadEndpoint, VadFrameAction, MIN_VOICED_FRAMES,
    SILENCE_FLUSH_FRAMES, VAD_FRAME_SAMPLES, VAD_ONSET_FRAMES, VAD_PRE_ROLL_FRAMES,
};

struct FailingDecoder;

impl SpeechDecoder for FailingDecoder {
    fn decode(&self, _speech: &[f32]) -> Result<DecodeOutcome, String> {
        Err("endpoint unavailable".into())
    }
}

#[derive(Clone, Copy)]
enum WorkerExit {
    Shutdown,
    SenderDisconnect,
}

fn assert_worker_exit_releases_floor(exit: WorkerExit) {
    let human_floor = HumanFloor::new();
    let shutdown = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = mpsc::channel();
    let acquired = Arc::new(Barrier::new(2));
    let worker_floor = human_floor.clone();
    let worker_shutdown = Arc::clone(&shutdown);
    let worker_acquired = Arc::clone(&acquired);
    let worker = std::thread::spawn(move || {
        run_stt_receive_loop(
            audio_rx,
            &worker_shutdown,
            worker_floor.clone(),
            |input, local_barge_in_state| {
                if matches!(input, SttLoopInput::Batch(_)) && !worker_floor.is_blocked() {
                    local_barge_in_state.acquire(&worker_floor, true, false);
                    worker_acquired.wait();
                }
            },
        );
    });

    audio_tx
        .send(SttAudioInput {
            pcm_bytes: Vec::new(),
            origin: SttAudioOrigin::Local,
        })
        .expect("worker receiver is open");
    acquired.wait();
    assert!(human_floor.is_blocked());
    match exit {
        WorkerExit::Shutdown => {
            shutdown.store(true, std::sync::atomic::Ordering::Release);
        }
        WorkerExit::SenderDisconnect => drop(audio_tx),
    }
    worker.join().expect("worker exits cleanly");

    let replacement_epoch = human_floor.epoch();
    assert!(
        human_floor.permits(replacement_epoch),
        "fresh TTS authorization must proceed after worker exit"
    );
    assert!(human_floor.enter_local(true, false));
}

#[test]
fn worker_shutdown_releases_local_floor_for_replacement() {
    assert_worker_exit_releases_floor(WorkerExit::Shutdown);
}

#[test]
fn worker_channel_disconnect_releases_local_floor_for_replacement() {
    assert_worker_exit_releases_floor(WorkerExit::SenderDisconnect);
}

fn frame(value: f32) -> Vec<f32> {
    vec![value; VAD_FRAME_SAMPLES]
}

#[test]
fn short_vad_blips_do_not_reach_the_recognizer() {
    assert!(!has_enough_voiced_audio(1));
    assert!(!has_enough_voiced_audio(MIN_VOICED_FRAMES - 1));
    assert!(has_enough_voiced_audio(MIN_VOICED_FRAMES));
}

#[test]
fn confirmed_onset_prepends_pre_roll_once() {
    let mut endpoint = VadEndpoint::new();
    for value in 0..VAD_PRE_ROLL_FRAMES - VAD_ONSET_FRAMES {
        assert_eq!(
            endpoint.process_frame(frame(value as f32), 0.0, true, true, SILENCE_FLUSH_FRAMES),
            VadFrameAction::None
        );
    }
    for value in 0..VAD_ONSET_FRAMES {
        let action = endpoint.process_frame(
            frame(100.0 + value as f32),
            0.9,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
        if value + 1 == VAD_ONSET_FRAMES {
            assert_eq!(action, VadFrameAction::ConfirmedOnset);
        } else {
            assert_eq!(action, VadFrameAction::None);
        }
    }

    assert_eq!(
        endpoint.speech_buf.len(),
        VAD_PRE_ROLL_FRAMES * VAD_FRAME_SAMPLES
    );
    assert_eq!(endpoint.speech_buf[0], 0.0);
    assert_eq!(endpoint.speech_buf[VAD_FRAME_SAMPLES], 1.0);
    assert_eq!(endpoint.pre_roll.len(), 0);
    endpoint.process_frame(frame(200.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    assert_eq!(
        endpoint.speech_buf.len(),
        (VAD_PRE_ROLL_FRAMES + 1) * VAD_FRAME_SAMPLES
    );
}

#[test]
fn onset_requires_consecutive_high_frames() {
    let mut endpoint = VadEndpoint::new();
    for probability in [0.9, 0.9, 0.2, 0.9, 0.9] {
        assert_eq!(
            endpoint.process_frame(frame(1.0), probability, true, true, SILENCE_FLUSH_FRAMES),
            VadFrameAction::None
        );
    }
    assert_eq!(
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::ConfirmedOnset
    );
}

#[test]
fn offset_hysteresis_preserves_borderline_speech() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(
        endpoint.process_frame(frame(2.0), 0.4, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::Speech
    );
    assert_eq!(endpoint.silence_frames, 0);
}

#[test]
fn below_offset_threshold_starts_silence() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(
        endpoint.process_frame(frame(0.0), 0.3, true, true, SILENCE_FLUSH_FRAMES),
        VadFrameAction::FirstSilence
    );
    assert_eq!(endpoint.silence_frames, 1);
}

#[test]
fn short_segment_reaches_the_visible_drop_path() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let mut action = VadFrameAction::None;
    for _ in 0..SILENCE_FLUSH_FRAMES {
        action = endpoint.process_frame(frame(0.0), 0.0, true, true, SILENCE_FLUSH_FRAMES);
    }
    assert_eq!(action, VadFrameAction::Flush);
    assert!(!has_enough_voiced_audio(endpoint.voiced_frames));
    assert!(!endpoint.speech_buf.is_empty());
}

#[test]
fn silence_flush_retains_only_hangover_audio() {
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(1.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let speech_len = endpoint.speech_buf.len();
    for index in 1..=SILENCE_FLUSH_FRAMES {
        let action = endpoint.process_frame(frame(0.0), 0.0, true, true, SILENCE_FLUSH_FRAMES);
        if index == SILENCE_FLUSH_FRAMES {
            assert_eq!(action, VadFrameAction::Flush);
        }
    }
    assert_eq!(
        endpoint.speech_buf.len(),
        speech_len + 6 * VAD_FRAME_SAMPLES
    );
}

#[test]
fn flush_boundary_never_double_includes_audio() {
    const SEGMENT_N_MARKER: f32 = 777.0;
    let mut endpoint = VadEndpoint::new();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(
            frame(SEGMENT_N_MARKER),
            0.9,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
    }
    for _ in 0..SILENCE_FLUSH_FRAMES {
        endpoint.process_frame(
            frame(SEGMENT_N_MARKER),
            0.0,
            true,
            true,
            SILENCE_FLUSH_FRAMES,
        );
    }
    endpoint.reset_segment();

    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(2.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let leaked = endpoint
        .speech_buf
        .iter()
        .filter(|sample| **sample == SEGMENT_N_MARKER)
        .count();
    assert_eq!(leaked, 0, "segment N audio leaked into segment N+1");
}

#[test]
fn reset_prevents_pre_roll_from_leaking_between_segments() {
    const SEGMENT_N_MARKER: f32 = 777.0;
    let mut endpoint = VadEndpoint::new();
    endpoint.pre_roll.push_back(frame(SEGMENT_N_MARKER));
    endpoint.reset_segment();
    for _ in 0..VAD_ONSET_FRAMES {
        endpoint.process_frame(frame(2.0), 0.9, true, true, SILENCE_FLUSH_FRAMES);
    }
    let leaked = endpoint
        .speech_buf
        .iter()
        .filter(|sample| **sample == SEGMENT_N_MARKER)
        .count();
    assert_eq!(leaked, 0, "segment N pre-roll leaked into segment N+1");
}

#[test]
fn held_push_to_talk_never_silence_flushes() {
    // Pure VAD mode: silence always ends the utterance.
    assert!(vad_flush_allowed(false, false, false));
    // Shortcut configured, nothing transmitting: nothing to flush anyway,
    // but the pause path stays closed.
    assert!(!vad_flush_allowed(true, false, false));
    // Shortcut held: "I am not done talking" — never flush on silence,
    // regardless of the manual mic state.
    assert!(!vad_flush_allowed(true, false, true));
    assert!(!vad_flush_allowed(true, true, true));
    // Manually open mic with the shortcut up: normal VAD behavior.
    assert!(vad_flush_allowed(true, true, false));
}

#[test]
fn decoder_failure_does_not_publish_transcript() {
    let (text_tx, mut text_rx) = tokio::sync::mpsc::channel(1);

    flush_to_stt(
        &[0.25; 16_000],
        MIN_VOICED_FRAMES,
        &FailingDecoder,
        &text_tx,
    );

    assert!(text_rx.try_recv().is_err());
}

#[test]
fn slow_openai_decode_does_not_block_later_utterance_capture() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let server = thread::spawn(move || {
        for text in ["ilk bölüm", "ikinci bölüm"] {
            let (mut stream, _) = listener.accept().expect("accept request");
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .expect("read timeout");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = stream.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            assert!(!request.is_empty());
            thread::sleep(Duration::from_millis(100));
            let body = format!(r#"{{"text":"{text}"}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write response");
        }
    });

    let config = OpenAiSttConfig::new(
        format!("http://{address}/v1/audio/transcriptions"),
        "test-key".into(),
        "whisper-large-v3-turbo-asr-fp16".into(),
        "tr".into(),
        String::new(),
    )
    .expect("valid loopback config");
    let (text_tx, mut text_rx) = tokio::sync::mpsc::channel(2);
    let shutdown = Arc::new(AtomicBool::new(false));
    let decoder =
        spawn_openai_decoder(config, text_tx, Arc::clone(&shutdown)).expect("decoder worker");

    let started = Instant::now();
    assert_eq!(
        decoder.decode(&[0.25; 16_000]).expect("queue first"),
        DecodeOutcome::Queued
    );
    assert_eq!(
        decoder.decode(&[0.25; 16_000]).expect("queue second"),
        DecodeOutcome::Queued
    );
    assert!(started.elapsed() < Duration::from_millis(50));
    assert_eq!(text_rx.blocking_recv().as_deref(), Some("ilk bölüm"));
    assert_eq!(text_rx.blocking_recv().as_deref(), Some("ikinci bölüm"));

    shutdown.store(true, Ordering::Release);
    server.join().expect("server thread");
}
