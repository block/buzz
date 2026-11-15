use super::*;
use serde_json::json;

/// Measure packing separately from ingestion, with no timing gate.
#[test]
#[ignore = "manual observer packing measurement"]
fn publish_queue_drain_probe() {
    let count: usize = std::env::var("BUZZ_OBSERVER_PROBE_EVENTS")
        .unwrap_or_else(|_| "10000".into())
        .parse()
        .unwrap();
    let mut queue = crate::ObserverPublishQueue::default();
    for seq in 1..=count as u64 {
        queue.ingest(ObserverEvent {
            seq,
            timestamp: "2026-11-01T00:00:00Z".into(),
            kind: "marker".into(),
            agent_index: Some(0),
            channel_id: Some("channel".into()),
            session_id: Some("session".into()),
            turn_id: Some("turn".into()),
            started_at: None,
            payload: json!({"text": "hello 🐝\n\"world\"", "seq": seq}),
        });
    }
    assert_eq!(queue.events.len(), count, "workload must fit the queue");
    let start = std::time::Instant::now();
    let mut frames = Vec::new();
    while let Some(frame) = queue.next_frame() {
        frames.push(frame);
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let seqs: Vec<_> = frames
        .iter()
        .flat_map(|frame| {
            frame.payload["events"].as_array().map_or_else(
                || vec![frame.seq],
                |events| {
                    events
                        .iter()
                        .map(|event| event["seq"].as_u64().unwrap())
                        .collect()
                },
            )
        })
        .collect();
    assert_eq!(seqs, (1..=count as u64).collect::<Vec<_>>());
    assert!(frames
        .iter()
        .all(|frame| crate::serialized_len(frame) <= crate::OBSERVER_MAX_PLAINTEXT_LEN));
    assert_eq!(queue.pending_bytes, 0);
    println!(
        "{}",
        json!({"events": count, "frames": frames.len(), "elapsedMs": elapsed_ms})
    );
}
