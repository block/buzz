use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn inert_pipeline(cancel: Arc<AtomicBool>) -> TtsPipeline {
    let (text_tx, text_rx) = std::sync::mpsc::sync_channel(TEXT_QUEUE_DEPTH);
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let thread = std::thread::spawn(move || {
        while !worker_shutdown.load(Ordering::Acquire) {
            let _ = text_rx.recv_timeout(RECV_TIMEOUT);
        }
    });
    TtsPipeline {
        text_tx,
        tts_active: Arc::new(AtomicBool::new(false)),
        shutdown,
        cancel,
        voice_cancel: Arc::new(AtomicBool::new(false)),
        voice: Arc::new(std::sync::Mutex::new("reference_sample".to_string())),
        voice_generation: Arc::new(AtomicU64::new(1)),
        speaker_generations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        active_speaker: Arc::new(std::sync::Mutex::new(None)),
        speaker_cancel: Arc::new(std::sync::Mutex::new(None)),
        playback_probe: PlaybackProbe::new(),
        voice_change_ack: Arc::new(std::sync::Mutex::new(None)),
        thread: Some(thread),
    }
}

#[test]
fn selecting_a_voice_raises_only_the_internal_cancel_and_retains_the_engine_handle() {
    let cancel = Arc::new(AtomicBool::new(false));
    let pipeline = inert_pipeline(Arc::clone(&cancel));

    let _acknowledged = pipeline.select_voice("eve");

    assert!(!cancel.load(Ordering::Acquire));
    assert!(pipeline.voice_cancel.load(Ordering::Acquire));
    assert_eq!(
        pipeline
            .voice
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_str(),
        "eve"
    );
}

#[test]
fn reconciling_an_unpublished_pipeline_does_not_cancel_its_first_message() {
    let cancel = Arc::new(AtomicBool::new(false));
    let pipeline = inert_pipeline(Arc::clone(&cancel));

    pipeline.select_voice_before_publish("eve");

    assert!(!cancel.load(Ordering::Acquire));
    assert_eq!(
        pipeline
            .voice
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_str(),
        "eve"
    );
}

#[test]
fn received_text_reconciles_a_voice_changed_while_the_worker_was_waiting() {
    let model_dir = tempfile::tempdir().expect("temp model dir");
    let bundled_voice =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/pocket-voices/eve.wav");
    std::fs::copy(
        &bundled_voice,
        model_dir.path().join("reference_sample.wav"),
    )
    .expect("Mary test voice");
    std::fs::copy(&bundled_voice, model_dir.path().join("eve.wav")).expect("Eve test voice");

    let selected_voice = Arc::new(std::sync::Mutex::new("reference_sample".to_string()));
    let mut style =
        load_voice_style(&model_dir.path().join("reference_sample.wav")).expect("initial style");
    let waiting = Arc::new(std::sync::Barrier::new(2));
    let (text_tx, text_rx) = std::sync::mpsc::channel();
    let worker_voice = Arc::clone(&selected_voice);
    let worker_waiting = Arc::clone(&waiting);
    let worker_model_dir = model_dir.path().to_path_buf();
    let worker = std::thread::spawn(move || {
        let mut voice_name = "reference_sample".to_string();
        worker_waiting.wait();
        let text = text_rx.recv().expect("first queued text");
        assert!(reconcile_selected_voice(
            &worker_model_dir,
            &worker_voice,
            &mut voice_name,
            &mut style,
        ));
        (text, voice_name)
    });

    waiting.wait();
    *selected_voice.lock().expect("selected voice") = "eve".to_string();
    text_tx
        .send("first message".to_string())
        .expect("queue first message");

    assert_eq!(
        worker.join().expect("worker"),
        ("first message".to_string(), "eve".to_string())
    );
}

#[test]
fn corrupt_selected_voice_falls_back_to_mary() {
    let model_dir = tempfile::tempdir().expect("temp model dir");
    let bundled_voice =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/pocket-voices/eve.wav");
    std::fs::copy(bundled_voice, model_dir.path().join("reference_sample.wav"))
        .expect("Mary test voice");
    std::fs::write(model_dir.path().join("eve.wav"), b"not a wave")
        .expect("corrupt selected voice");

    let selected_voice = std::sync::Mutex::new("eve".to_string());
    let mut voice_name = "reference_sample".to_string();
    let mut style =
        load_voice_style(&model_dir.path().join("reference_sample.wav")).expect("Mary style");

    assert!(reconcile_selected_voice(
        model_dir.path(),
        &selected_voice,
        &mut voice_name,
        &mut style,
    ));
    assert_eq!(voice_name, DEFAULT_VOICE);
    assert_eq!(
        selected_voice
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_str(),
        DEFAULT_VOICE
    );
}

#[test]
fn an_in_hand_post_change_message_survives_cancellation() {
    let selected_voice = Arc::new(std::sync::Mutex::new("reference_sample".to_string()));
    let voice_generation = AtomicU64::new(1);
    let barge_in = AtomicBool::new(false);
    let voice_cancel = Arc::new(AtomicBool::new(false));
    let voice_change_ack = Arc::new(std::sync::Mutex::new(None));
    let (text_tx, text_rx) = std::sync::mpsc::sync_channel(1);
    let mut acknowledged = begin_voice_change(
        &selected_voice,
        &voice_generation,
        &voice_cancel,
        &voice_change_ack,
        "eve",
    )
    .expect("voice changed");
    assert!(voice_cancel.load(Ordering::Acquire));
    assert!(matches!(
        acknowledged.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    acknowledge_voice_change(&voice_change_ack, &voice_cancel);
    assert!(matches!(
        acknowledged.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    text_tx
        .send(QueuedText {
            generation: voice_generation.load(Ordering::Acquire),
            route_id: 1,
            speaker_pubkey: None,
            speaker_generation: 0,
            voice_reference: None,
            text: "new message".to_string(),
        })
        .expect("new message");
    let mut current_text = Some(text_rx.recv().expect("in-hand new message"));

    let shutdown = AtomicBool::new(false);
    let active = AtomicBool::new(true);
    let mut deferred_text = VecDeque::from([
        QueuedText {
            generation: 1,
            route_id: 2,
            speaker_pubkey: None,
            speaker_generation: 0,
            voice_reference: None,
            text: "old message".to_string(),
        },
        QueuedText {
            generation: voice_generation.load(Ordering::Acquire),
            route_id: 3,
            speaker_pubkey: None,
            speaker_generation: 0,
            voice_reference: None,
            text: "later new message".to_string(),
        },
    ]);
    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));
    acknowledge_voice_change(&voice_change_ack, &voice_cancel);
    acknowledged.blocking_recv().expect("voice change ack");

    assert_eq!(
        deferred_text
            .pop_front()
            .expect("preserved post-change message")
            .text,
        "new message"
    );
    assert_eq!(
        deferred_text
            .pop_front()
            .expect("later post-change message")
            .text,
        "later new message"
    );
    assert!(text_rx.try_recv().is_err());
}

#[test]
fn superseding_voice_change_removes_earlier_deferred_messages() {
    let selected_voice = std::sync::Mutex::new("reference_sample".to_string());
    let voice_generation = AtomicU64::new(1);
    let barge_in = AtomicBool::new(false);
    let voice_cancel = AtomicBool::new(false);
    let voice_change_ack = Arc::new(std::sync::Mutex::new(None));
    let (_text_tx, text_rx) = std::sync::mpsc::channel();
    let shutdown = AtomicBool::new(false);
    let active = AtomicBool::new(true);
    let mut deferred_text = VecDeque::new();
    let mut current_text = None;

    let first = begin_voice_change(
        &selected_voice,
        &voice_generation,
        &voice_cancel,
        &voice_change_ack,
        "eve",
    )
    .expect("first voice change");
    deferred_text.push_back(QueuedText {
        generation: voice_generation.load(Ordering::Acquire),
        route_id: 4,
        speaker_pubkey: None,
        speaker_generation: 0,
        voice_reference: None,
        text: "message for Eve".to_string(),
    });
    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));
    acknowledge_voice_change(&voice_change_ack, &voice_cancel);
    first.blocking_recv().expect("first acknowledgement");

    let _second = begin_voice_change(
        &selected_voice,
        &voice_generation,
        &voice_cancel,
        &voice_change_ack,
        "reference_sample",
    )
    .expect("second voice change");
    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));

    assert!(deferred_text.is_empty());
}

#[test]
fn barge_in_clears_deferred_voice_change_messages() {
    let barge_in = AtomicBool::new(true);
    let voice_cancel = AtomicBool::new(false);
    let shutdown = AtomicBool::new(false);
    let active = AtomicBool::new(true);
    let voice_change_ack = Arc::new(std::sync::Mutex::new(None));
    let (_text_tx, text_rx) = std::sync::mpsc::channel();
    let mut deferred_text = VecDeque::from([QueuedText {
        generation: 2,
        route_id: 5,
        speaker_pubkey: None,
        speaker_generation: 0,
        voice_reference: None,
        text: "deferred message".to_string(),
    }]);
    let mut current_text = None;

    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));

    assert!(deferred_text.is_empty());
}

#[test]
fn barge_in_during_a_voice_change_clears_post_change_messages() {
    let selected_voice = std::sync::Mutex::new("reference_sample".to_string());
    let voice_generation = AtomicU64::new(1);
    let barge_in = AtomicBool::new(false);
    let voice_cancel = AtomicBool::new(false);
    let voice_change_ack = Arc::new(std::sync::Mutex::new(None));
    let (_text_tx, text_rx) = std::sync::mpsc::channel();
    let shutdown = AtomicBool::new(false);
    let active = AtomicBool::new(true);
    let mut deferred_text = VecDeque::new();
    let mut current_text = None;

    let _acknowledged = begin_voice_change(
        &selected_voice,
        &voice_generation,
        &voice_cancel,
        &voice_change_ack,
        "eve",
    )
    .expect("voice change");
    deferred_text.push_back(QueuedText {
        generation: voice_generation.load(Ordering::Acquire),
        route_id: 6,
        speaker_pubkey: None,
        speaker_generation: 0,
        voice_reference: None,
        text: "post-change message".to_string(),
    });
    barge_in.store(true, Ordering::Release);

    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));
    assert!(deferred_text.is_empty());
}

#[test]
fn a_sender_captured_before_voice_change_is_stale_even_if_it_sends_after_drain() {
    let selected_voice = std::sync::Mutex::new("reference_sample".to_string());
    let voice_generation = Arc::new(AtomicU64::new(1));
    let barge_in = AtomicBool::new(false);
    let voice_cancel = AtomicBool::new(false);
    let voice_change_ack = Arc::new(std::sync::Mutex::new(None));
    let (text_tx, text_rx) = std::sync::mpsc::sync_channel(1);
    let old_sender = TtsTextSender {
        text_tx,
        generation: voice_generation.load(Ordering::Acquire),
        speaker_generations: Arc::new(std::sync::Mutex::new(HashMap::new())),
    };
    let shutdown = AtomicBool::new(false);
    let active = AtomicBool::new(true);
    let mut deferred_text = VecDeque::new();
    let mut current_text = None;

    let _acknowledged = begin_voice_change(
        &selected_voice,
        &voice_generation,
        &voice_cancel,
        &voice_change_ack,
        "eve",
    )
    .expect("voice change");
    assert!(handle_cancel_or_shutdown(
        (&barge_in, &voice_cancel),
        &shutdown,
        &active,
        (&text_rx, &mut deferred_text, &mut current_text),
        &voice_change_ack,
        None,
        None,
    ));
    old_sender
        .send(
            7,
            "agent".to_string(),
            0,
            "reference_sample".to_string(),
            "late old message".to_string(),
        )
        .expect("late send");
    let late = text_rx.recv().expect("late queued text");

    assert!(late.generation < voice_generation.load(Ordering::Acquire));
    assert_eq!(late.voice_reference.as_deref(), Some("reference_sample"));
}
