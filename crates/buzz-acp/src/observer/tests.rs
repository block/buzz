use super::*;
use buzz_core::observer::OBSERVER_MAX_PLAINTEXT_LEN;
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

const REPLAY_MAX_BYTES: usize = 4 * 1024 * 1024;

fn bytes(event: &ObserverEvent) -> usize {
    serde_json::to_vec(event).unwrap().len()
}

#[test]
fn small_events_preserve_payload_context_and_count_limited_replay() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let context = context_for_turn(
        Some(uuid::Uuid::new_v4()),
        Some("session".into()),
        "turn".into(),
        "2026-11-01T00:00:00Z".into(),
    );
    let payload = json!({"text": "hello 🐝\n\"world\"", "count": 3});
    observer.emit("turn_started", Some(2), &context, payload.clone());
    let first = live.try_recv().unwrap();
    assert_eq!(first.payload, payload);
    assert_eq!(first.kind, "turn_started");
    assert_eq!(first.agent_index, Some(2));
    assert_eq!(first.channel_id, context.channel_id);
    assert_eq!(first.session_id, context.session_id);
    assert_eq!(first.turn_id, context.turn_id);
    assert_eq!(first.started_at, context.started_at);
    assert_eq!(first.seq, 1);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&observer.snapshot()[0]).unwrap()
    );

    for n in 0..1_020 {
        observer.emit("marker", None, &context, json!({"n": n}));
    }
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.len(), 1_000);
    assert_eq!(snapshot.first().unwrap().payload["n"], 20);
    assert_eq!(snapshot.last().unwrap().payload["n"], 1_019);
}

#[test]
fn replay_evicts_oldest_by_serialized_bytes_and_remains_usable() {
    let observer = ObserverHandle::in_process();
    let context = ObserverContext::default();
    // Individually under the frame cap, collectively beyond the replay budget.
    // Escaping makes the wire size substantially larger than the raw strings.
    for n in 0..180 {
        observer.emit(
            "acp_read",
            None,
            &context,
            json!({"n": n, "text": "\n🐝".repeat(8_000)}),
        );
    }
    let snapshot = observer.snapshot();
    let retained: usize = snapshot.iter().map(bytes).sum();
    assert!(retained <= REPLAY_MAX_BYTES, "retained {retained} bytes");
    assert!(snapshot.len() < 180);
    assert_eq!(snapshot.last().unwrap().payload["n"], 179);
    assert!(snapshot
        .windows(2)
        .all(|pair| pair[1].seq == pair[0].seq + 1));
    // Eviction is greedy, keeping the largest newest suffix that fits.
    assert!(retained + bytes(snapshot.first().unwrap()) > REPLAY_MAX_BYTES);
    drop(snapshot);

    observer.emit("turn_completed", None, &context, json!({"done": true}));
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.last().unwrap().kind, "turn_completed");
    assert!(snapshot.iter().map(bytes).sum::<usize>() <= REPLAY_MAX_BYTES);
}

#[test]
fn oversized_payloads_are_fitted_before_replay_and_stalled_broadcast() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let context = context_for(None, Some("session".into()), Some("turn".into()));
    // Exercise a string leaf, structure that cannot be leaf-trimmed, and large
    // object keys. Both consumers must see the same bounded representation.
    let payloads = [
        json!({"body": "🐝\n".repeat(40_000)}),
        json!(vec![0; 80_000]),
        json!({"x".repeat(100_000): "value"}),
    ];
    let original_sizes: Vec<_> = payloads
        .iter()
        .map(|payload| serde_json::to_vec(payload).unwrap().len())
        .collect();
    for payload in payloads {
        observer.emit("acp_read", Some(0), &context, payload);
    }
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.len(), 3);
    for (replay, original_size) in snapshot.into_iter().zip(original_sizes) {
        let event = live.try_recv().unwrap();
        assert!(bytes(&event) <= OBSERVER_MAX_PLAINTEXT_LEN);
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::to_value(&replay).unwrap()
        );
        assert_eq!(event.session_id, context.session_id);
        assert_eq!(event.turn_id, context.turn_id);
        assert!(serde_json::to_string(&event.payload)
            .unwrap()
            .contains("elided"));
        if let Some(original_bytes) = event.payload.get("originalBytes") {
            assert_eq!(original_bytes, original_size);
        }
    }
}

#[test]
fn stalled_subscriber_has_a_bounded_recent_window_and_reports_lag() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    for n in 0..1_100 {
        observer.emit(
            "acp_read",
            None,
            &ObserverContext::default(),
            json!({"n": n, "body": "x".repeat(70_000)}),
        );
    }
    // Tokio rounds the 1,000 requested slots up to 1,024. Even a subscriber
    // that never drains must not keep raw oversized events in those slots.
    let mut lagged = 0;
    let mut received = Vec::new();
    loop {
        match live.try_recv() {
            Ok(event) => received.push(event),
            Err(TryRecvError::Lagged(count)) => lagged += count,
            Err(TryRecvError::Empty) => break,
            Err(error) => panic!("unexpected receive error: {error}"),
        }
    }
    assert!(lagged > 0);
    assert_eq!(lagged as usize + received.len(), 1_100);
    assert!(received.len() <= 1_024);
    assert!(received
        .iter()
        .all(|event| bytes(event) <= OBSERVER_MAX_PLAINTEXT_LEN));
    assert!(received.iter().map(bytes).sum::<usize>() <= 1_024 * OBSERVER_MAX_PLAINTEXT_LEN);
    assert_eq!(received.last().unwrap().payload["n"], 1_099);
    assert!(received
        .windows(2)
        .all(|pair| pair[1].seq == pair[0].seq + 1));
}

#[test]
fn oversized_envelope_is_not_retained_and_later_events_still_deliver() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let context = context_for(None, Some("s".repeat(100_000)), None);
    observer.emit("acp_read", None, &context, json!({"body": "small"}));
    assert!(observer.snapshot().is_empty());
    assert!(matches!(live.try_recv(), Err(TryRecvError::Empty)));
    observer.emit(
        "turn_completed",
        None,
        &ObserverContext::default(),
        json!({}),
    );
    assert_eq!(live.try_recv().unwrap().seq, 2);
    assert_eq!(observer.snapshot().len(), 1);
}

#[test]
fn concurrent_emitters_keep_live_and_replay_sequence_order() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for producer in 0..4 {
            let (observer, start) = (&observer, &start);
            scope.spawn(move || {
                start.wait();
                for n in 0..50 {
                    observer.emit(
                        "acp_read", None, &ObserverContext::default(),
                        json!({"producer": producer, "n": n, "body": "x".repeat(1_000 + producer * 40_000)}),
                    );
                }
            });
        }
    });
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.len(), 200);
    for (index, replay) in snapshot.iter().enumerate() {
        let event = live.try_recv().unwrap();
        assert_eq!(event.seq, index as u64 + 1);
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
    }
    assert!(matches!(live.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn publication_metadata_growth_keeps_live_events_bounded() {
    struct PausedKind {
        ready: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    }
    impl From<PausedKind> for String {
        fn from(kind: PausedKind) -> Self {
            kind.ready.send(()).unwrap();
            kind.resume.recv().unwrap();
            "boundary".into()
        }
    }
    let observer = ObserverHandle::in_process();
    let context = ObserverContext::default();
    for _ in 0..8 {
        observer.emit("seed", None, &context, json!({}));
    }
    let mut live = observer.subscribe();
    let mut template = ObserverEvent {
        seq: 9,
        timestamp: "2000-01-01T00:00:00.000000000+00:00".into(),
        kind: "boundary".into(),
        agent_index: None,
        channel_id: None,
        session_id: None,
        turn_id: None,
        started_at: None,
        payload: json!({"body":""}),
    };
    template.payload = json!({"body":"x".repeat(OBSERVER_MAX_PLAINTEXT_LEN - bytes(&template))});
    assert_eq!(bytes(&template), OBSERVER_MAX_PLAINTEXT_LEN);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            observer.emit(
                PausedKind {
                    ready: ready_tx,
                    resume: resume_rx,
                },
                None,
                &context,
                template.payload.clone(),
            );
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        observer.emit("fast", None, &context, json!({}));
        resume_tx.send(()).unwrap();
    });
    let fast = live.try_recv().unwrap();
    let boundary = live.try_recv().unwrap();
    assert_eq!((fast.seq, boundary.seq), (9, 10));
    assert_eq!(fast.kind, "fast");
    assert_eq!(boundary.kind, "boundary");
    assert!(bytes(&boundary) <= OBSERVER_MAX_PLAINTEXT_LEN);
    assert_ne!(boundary.payload, template.payload);
    let snapshot = observer.snapshot();
    assert_eq!(
        serde_json::to_value(snapshot.last().unwrap()).unwrap(),
        serde_json::to_value(boundary).unwrap()
    );
    assert_eq!(
        observer.inner.buffer.lock().unwrap().bytes,
        snapshot.iter().map(bytes).sum::<usize>()
    );
}

/// Local measurement only: no timing threshold and no relay or real agent.
#[test]
#[ignore = "manual observer retention and emit-time measurement"]
fn retention_probe() {
    let count: usize = std::env::var("BUZZ_OBSERVER_PROBE_EVENTS")
        .unwrap_or_else(|_| "1000".into())
        .parse()
        .unwrap();
    let payload_bytes: usize = std::env::var("BUZZ_OBSERVER_PROBE_BYTES")
        .unwrap_or_else(|_| "1048576".into())
        .parse()
        .unwrap();
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let body = "x".repeat(payload_bytes);
    let start = std::time::Instant::now();
    for n in 0..count {
        observer.emit(
            "acp_read",
            Some(0),
            &ObserverContext::default(),
            json!({"n": n, "body": body}),
        );
    }
    let emit_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let snapshot = observer.snapshot();
    let replay_events = snapshot.len();
    let replay_bytes: usize = snapshot.iter().map(bytes).sum();
    drop(snapshot);
    let (mut live_events, mut live_bytes, mut lagged) = (0, 0, 0);
    loop {
        match live.try_recv() {
            Ok(event) => {
                live_events += 1;
                live_bytes += bytes(&event);
            }
            Err(TryRecvError::Lagged(count)) => lagged += count,
            Err(TryRecvError::Empty) => break,
            Err(error) => panic!("unexpected receive error: {error}"),
        }
    }
    println!(
        "{}",
        json!({"events": count, "payloadBytes": payload_bytes, "emitMs": emit_ms,
        "replayEvents": replay_events, "replayBytes": replay_bytes,
        "liveEvents": live_events, "liveBytes": live_bytes, "lagged": lagged})
    );
}
