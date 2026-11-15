use super::*;
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

struct SlowKind {
    started: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

impl From<SlowKind> for String {
    fn from(kind: SlowKind) -> Self {
        kind.started.send(()).unwrap();
        kind.resume.recv().unwrap();
        "slow".into()
    }
}

#[test]
fn concurrent_emitters_keep_live_and_replay_sequence_order() {
    let observer = ObserverHandle::in_process();
    let mut live = observer.subscribe();
    let context = context_for_turn(
        None,
        Some("session".into()),
        "turn".into(),
        "2026-11-01T00:00:00Z".into(),
    );
    // Keep the payload below the frame cap so this checks ordering separately
    // from the retention suite's oversized-payload fitting cases.
    let payload = json!({"text":"hello 🐝\n\"world\"", "body":"x".repeat(40_000)});
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let (observer_ref, context_ref, payload_ref) = (&observer, &context, &payload);
    let (mut handoff_live, mut handoff_snapshot) = std::thread::scope(move |scope| {
        scope.spawn(move || {
            observer_ref.emit(
                SlowKind {
                    started: started_tx,
                    resume: resume_rx,
                },
                Some(0),
                context_ref,
                payload_ref.clone(),
            )
        });
        // Pause one real emit during preparation, then let another publish.
        // No timing assertion: the channel controls this interleaving exactly.
        started_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        observer_ref.emit("fast", Some(1), context_ref, payload_ref.clone());
        let handoff_live = observer_ref.subscribe();
        let snapshot = observer_ref.snapshot();
        resume_tx.send(()).unwrap();
        (handoff_live, snapshot)
    });
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.len(), 2);
    for (index, kind) in ["fast", "slow"].into_iter().enumerate() {
        let event = live.try_recv().unwrap();
        assert_eq!(event.seq, index as u64 + 1);
        assert_eq!(event.kind, kind);
        assert!(chrono::DateTime::parse_from_rfc3339(&event.timestamp).is_ok());
        assert_eq!(event.payload, payload);
        assert_eq!(event.session_id, context.session_id);
        assert_eq!(event.turn_id, context.turn_id);
        assert_eq!(event.started_at, context.started_at);
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::to_value(&snapshot[index]).unwrap()
        );
    }
    assert!(matches!(live.try_recv(), Err(TryRecvError::Empty)));
    let high_water = handoff_snapshot.last().map_or(0, |event| event.seq);
    while let Ok(event) = handoff_live.try_recv() {
        if event.seq > high_water {
            handoff_snapshot.push(event);
        }
    }
    assert_eq!(
        serde_json::to_value(&handoff_snapshot).unwrap(),
        serde_json::to_value(&snapshot).unwrap(),
        "snapshot/live handoff must neither skip nor duplicate the slow event"
    );
    for n in 0..1_010 {
        observer.emit("marker", None, &context, json!({"n":n}));
    }
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.len(), 1_000);
    assert_eq!(snapshot.first().unwrap().payload["n"], 10);
    assert_eq!(snapshot.last().unwrap().payload["n"], 1_009);
}

/// Local measurement only: the production publish queue, with no timing gate.
#[test]
#[ignore = "manual observer fitting measurement"]
fn publish_queue_probe() {
    let count: usize = std::env::var("BUZZ_OBSERVER_PROBE_EVENTS")
        .unwrap_or_else(|_| "200".into())
        .parse()
        .unwrap();
    let payload_bytes: usize = std::env::var("BUZZ_OBSERVER_PROBE_BYTES")
        .unwrap_or_else(|_| "1048576".into())
        .parse()
        .unwrap();
    let fields: usize = std::env::var("BUZZ_OBSERVER_PROBE_FIELDS")
        .unwrap_or_else(|_| "64".into())
        .parse()
        .unwrap();
    assert!(fields > 0);
    let mut template = ObserverEvent {
        seq: 0,
        timestamp: "2026-11-01T00:00:00Z".into(),
        kind: "acp_read".into(),
        agent_index: None,
        channel_id: None,
        session_id: None,
        turn_id: None,
        started_at: None,
        payload: json!({"body": (0..fields).map(|n|
            "x".repeat(payload_bytes / fields + usize::from(n < payload_bytes % fields))
        ).collect::<Vec<_>>() }),
    };
    let mut queue = crate::ObserverPublishQueue::default();
    let start = std::time::Instant::now();
    for n in 0..count {
        template.seq = n as u64 + 1;
        queue.ingest(template.clone());
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{}",
        json!({"events": count, "payloadBytes": payload_bytes, "fields": fields,
        "elapsedMs": elapsed_ms, "queuedEvents": queue.events.len(), "queuedBytes": queue.pending_bytes})
    );
}
