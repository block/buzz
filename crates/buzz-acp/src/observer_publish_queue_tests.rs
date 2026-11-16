use super::*;

fn event(seq: u64, kind: &str, channel: Option<&str>) -> observer::ObserverEvent {
    observer::ObserverEvent {
        seq,
        timestamp: format!("2026-04-29T04:00:{:02}Z", seq.min(59)),
        kind: kind.to_string(),
        agent_index: Some(0),
        channel_id: channel.map(ToOwned::to_owned),
        session_id: Some("session-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        started_at: None,
        payload: serde_json::json!({ "seq": seq }),
    }
}

fn queue_of(events: Vec<observer::ObserverEvent>) -> ObserverPublishQueue {
    let mut queue = ObserverPublishQueue::default();
    for event in events {
        queue.ingest(event);
    }
    queue
}

/// Collect every frame the queue will produce, one publish slot at a time.
fn drain_frames(queue: &mut ObserverPublishQueue) -> Vec<observer::ObserverEvent> {
    let mut frames = Vec::new();
    while !queue.is_empty() {
        frames.push(queue.next_frame().expect("queue not empty"));
    }
    frames
}

/// Inner seqs of a frame, whether it is an envelope or an unwrapped
/// singleton.
fn frame_seqs(frame: &observer::ObserverEvent) -> Vec<u64> {
    match frame.payload.get("events").and_then(|v| v.as_array()) {
        Some(inner) => inner.iter().map(|e| e["seq"].as_u64().unwrap()).collect(),
        None => vec![frame.seq],
    }
}

/// Retained bytes computed by WALKING the entries, independently of the
/// queue's own accumulator. Cap regressions must assert on this, not on
/// `total_pending_bytes()` — asserting the counter against itself passed
/// while the process retained ~2x the budget (an earlier regression: each
/// pending coalescer entry holds the first chunk's text twice, in the
/// serialized skeleton AND the extracted `text` copy).
fn walked_retained_bytes(queue: &ObserverPublishQueue) -> usize {
    let fifo: usize = queue
        .events
        .iter()
        .map(|(_, _, event)| serialized_len(event))
        .sum();
    let coalescer: usize = queue
        .coalescer
        .pending
        .iter()
        .map(|pending| serialized_len(&pending.event) + pending.text.len())
        .sum();
    fifo + coalescer
}

/// The walker above is itself an instrument, and every cap test asks it
/// only for `<= CAP` — a blinded walker (missing an arm, or returning 0)
/// would satisfy all of them while hiding exactly the 2x overshoot it was
/// added to catch. Pin it two-sided: it must SEE
/// the double retention, and it must agree with the accumulator EXACTLY
/// while both stores are non-empty — neither may drift.
#[test]
fn walked_retained_bytes_agrees_with_the_accumulator_exactly() {
    fn chunk(seq: u64, message_id: &str, text: &str) -> observer::ObserverEvent {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": message_id,
                    "content": { "type": "text", "text": text },
                },
            },
        });
        e
    }

    let text = "w".repeat(7_000);
    let mut queue = ObserverPublishQueue::default();
    // One pending chunk: its text lives in the serialized skeleton AND
    // the extracted copy, so a walker blind to either arm reads short.
    queue.ingest(chunk(1, "message-a", &text));
    assert!(
        walked_retained_bytes(&queue) >= 2 * text.len(),
        "the walker must SEE the first chunk's text twice \
             (skeleton + extracted copy), got {}",
        walked_retained_bytes(&queue)
    );

    // Populate BOTH stores: the non-chunk event flushes message-a into
    // the FIFO and queues itself; fresh pending keys (plus a same-key
    // append) rebuild the coalescer side.
    queue.ingest(event(2, "tool_call", Some("chan-a")));
    queue.ingest(chunk(3, "message-b", &text));
    queue.ingest(chunk(4, "message-b", &text));
    queue.ingest(chunk(5, "message-c", &text));
    assert!(
        !queue.events.is_empty() && !queue.coalescer.pending.is_empty(),
        "both arms must be non-empty for the agreement check to bind"
    );
    assert_eq!(
        queue.total_pending_bytes(),
        walked_retained_bytes(&queue),
        "accumulator and entry-walk must agree exactly: neither may drift"
    );
}

/// Two or more pending events for one channel ship as a single batch
/// envelope whose payload carries every inner event in arrival order.
#[test]
fn multiple_events_ship_as_one_envelope_in_order() {
    let mut queue = queue_of(vec![
        event(1, "turn_started", Some("chan-a")),
        event(2, "acp_read", Some("chan-a")),
        event(3, "acp_write", Some("chan-a")),
    ]);

    let frame = queue.next_frame().expect("one frame");
    assert!(queue.is_empty(), "one channel, one publish slot");
    assert_eq!(frame.kind, OBSERVER_BATCH_KIND);
    assert_eq!(frame.seq, 3, "envelope mirrors the last inner event");
    assert_eq!(frame_seqs(&frame), [1, 2, 3], "arrival order preserved");
    let inner = frame.payload["events"].as_array().expect("events array");
    assert_eq!(inner[1]["kind"], "acp_read", "inner events keep their kind");
}

/// A single pending event is published unwrapped — no envelope, so
/// consumers that predate batching still understand quiet periods.
#[test]
fn a_single_event_stays_unwrapped() {
    let mut queue = queue_of(vec![event(7, "turn_started", Some("chan-a"))]);
    let frame = queue.next_frame().expect("one frame");
    assert!(queue.is_empty());
    assert_eq!(frame.kind, "turn_started");
    assert_eq!(frame.seq, 7);
}

/// An empty queue yields no frame — a tick with nothing pending must not
/// publish anything.
#[test]
fn empty_queue_yields_no_frame() {
    let mut queue = ObserverPublishQueue::default();
    assert!(queue.next_frame().is_none());
    assert!(queue.is_empty());
}

/// Frames never mix channels, and each channel's events keep their FIFO
/// order. Gathering is QUEUE-WIDE: the front event's channel collects its
/// events from anywhere in the queue (that is what keeps the drain rate
/// in bytes per slot under interleaving), so cross-channel frame order
/// MAY differ from arrival order — but a null-channel event is a barrier
/// nothing gathers across.
#[test]
fn frames_never_mix_channels_and_gather_queue_wide() {
    let mut queue = queue_of(vec![
        event(1, "acp_read", Some("chan-a")),
        event(2, "acp_write", Some("chan-a")),
        event(3, "acp_read", Some("chan-b")),
        event(4, "acp_read", Some("chan-a")),
        event(5, "acp_read", None),
    ]);

    let frames = drain_frames(&mut queue);
    assert_eq!(
        frames.len(),
        3,
        "gathered: [1,2,4]@a, [3]@b, [5]@None — one frame each"
    );
    for frame in &frames {
        let channels: HashSet<Option<String>> = match frame.payload.get("events") {
            Some(serde_json::Value::Array(inner)) => inner
                .iter()
                .map(|e| e["channelId"].as_str().map(ToOwned::to_owned))
                .collect(),
            _ => std::iter::once(frame.channel_id.clone()).collect(),
        };
        assert_eq!(channels.len(), 1, "a frame never mixes channels");
    }
    assert_eq!(
        frame_seqs(&frames[0]),
        [1, 2, 4],
        "chan-a gathers queue-wide, FIFO within the channel"
    );
    assert_eq!(frames[0].channel_id.as_deref(), Some("chan-a"));
    assert_eq!(frames[1].kind, "acp_read", "singleton stays unwrapped");
    assert_eq!(frames[1].channel_id.as_deref(), Some("chan-b"));
    assert_eq!(frames[2].channel_id, None);
}

/// A NULL-channel event is a barrier: channel events queued BEHIND it
/// must not gather into a frame ahead of it, so causally-global events
/// (`agent_panic`-class) keep their exact order against every channel.
/// The null event itself ships only its contiguous front run.
#[test]
fn null_channel_events_are_gather_barriers() {
    let mut queue = queue_of(vec![
        event(1, "acp_read", Some("chan-a")),
        event(2, "acp_read", Some("chan-b")),
        event(3, "agent_panic", None),
        event(4, "acp_write", Some("chan-a")),
    ]);

    let frames = drain_frames(&mut queue);
    let published: Vec<Vec<u64>> = frames.iter().map(frame_seqs).collect();
    assert_eq!(
        published,
        [vec![1], vec![2], vec![3], vec![4]],
        "seq 4 must not gather past the null barrier into frame 1"
    );
}

/// The drain-rate regression: with two channels strictly
/// alternating, a front-run packer degrades to ONE event per slot
/// (~275 B/s regardless of the 64KB frame budget). Queue-wide gathering
/// must drain an interleaved backlog in ~ceil(events / per-frame-fit)
/// slots per channel, not one slot per event.
#[test]
fn interleaved_channels_drain_at_bytes_per_slot_not_events_per_slot() {
    let mut events = Vec::new();
    for i in 0..100u64 {
        events.push(event(2 * i + 1, "acp_read", Some("chan-a")));
        events.push(event(2 * i + 2, "acp_read", Some("chan-b")));
    }
    let mut queue = queue_of(events);

    let frames = drain_frames(&mut queue);
    assert!(
        frames.len() <= 4,
        "200 tiny alternating events must gather into a few full frames, \
             got {} (front-run packing would need 200 slots)",
        frames.len()
    );
    for frame in &frames {
        assert!(serialized_len(frame) <= OBSERVER_MAX_PLAINTEXT_LEN);
    }
    // Within each channel, FIFO order survives the gather.
    let mut seqs_a = Vec::new();
    let mut seqs_b = Vec::new();
    for frame in &frames {
        match frame.channel_id.as_deref() {
            Some("chan-a") => seqs_a.extend(frame_seqs(frame)),
            Some("chan-b") => seqs_b.extend(frame_seqs(frame)),
            other => panic!("unexpected channel {other:?}"),
        }
    }
    assert!(seqs_a.windows(2).all(|w| w[0] < w[1]), "chan-a FIFO");
    assert!(seqs_b.windows(2).all(|w| w[0] < w[1]), "chan-b FIFO");
    assert_eq!(seqs_a.len() + seqs_b.len(), 200, "nothing lost");
}

/// A same-channel backlog that cannot fit one 64KB frame splits across
/// SUCCESSIVE publish slots — never multiple frames from one slot — with
/// every frame under the cap and no event lost or reordered.
#[test]
fn oversized_backlogs_split_across_publish_slots_under_the_cap() {
    let big_text = "x".repeat(30_000);
    let mut queue = queue_of(
        (1..=6)
            .map(|seq| {
                let mut e = event(seq, "acp_read", Some("chan-a"));
                e.payload = serde_json::json!({ "seq": seq, "text": big_text });
                e
            })
            .collect(),
    );

    let frames = drain_frames(&mut queue);
    assert!(
        frames.len() > 1,
        "six 30KB events cannot fit one 64KB frame"
    );
    let mut seen = Vec::new();
    for frame in &frames {
        assert!(
            serialized_len(frame) <= OBSERVER_MAX_PLAINTEXT_LEN,
            "every emitted frame must fit the plaintext cap"
        );
        seen.extend(frame_seqs(frame));
    }
    assert_eq!(
        seen,
        [1, 2, 3, 4, 5, 6],
        "no event lost or reordered by splitting"
    );
}

#[test]
fn batch_packing_matches_wire_bytes_at_the_exact_limit() {
    for count in [2, 3, 16] {
        for channel in [None, Some("channel\"🐝\n")] {
            for delta in [-1isize, 0, 1] {
                let mut events: Vec<_> = (0..count)
                    .map(|i| {
                        let mut e = event(9 + i, "marker", channel);
                        e.agent_index = (i % 2 == 0).then_some(123);
                        e.session_id = Some("session\"\\\n🐝".into());
                        e.turn_id = None;
                        e.started_at = (i % 2 == 1).then(|| "2026-11-01T00:00:00Z".into());
                        e.payload = serde_json::json!({"text": ""});
                        e
                    })
                    .collect();
                // Independent wire oracle, including the newest metadata,
                // escaped strings, optional fields, array commas and UTF-8.
                let wire_batch = |events: &[observer::ObserverEvent]| {
                    let mut value = serde_json::to_value(events.last().unwrap()).unwrap();
                    value["kind"] = serde_json::json!("batch");
                    value["payload"] = serde_json::json!({"events": events});
                    value
                };
                let overhead = serde_json::to_vec(&wire_batch(&events)).unwrap().len();
                let padding = (OBSERVER_MAX_PLAINTEXT_LEN as isize + delta) as usize - overhead;
                events.last_mut().unwrap().payload["text"] = serde_json::json!("x".repeat(padding));
                let expected = wire_batch(&events);
                assert_eq!(
                    serde_json::to_vec(&expected).unwrap().len(),
                    (OBSERVER_MAX_PLAINTEXT_LEN as isize + delta) as usize
                );
                let mut queue = queue_of(events.clone());
                let frames = drain_frames(&mut queue);
                assert_eq!(frames.len(), if delta <= 0 { 1 } else { 2 });
                if delta <= 0 {
                    assert_eq!(serde_json::to_value(&frames[0]).unwrap(), expected);
                }
                let mut delivered = Vec::new();
                for frame in frames {
                    assert!(
                        serde_json::to_vec(&frame).unwrap().len() <= OBSERVER_MAX_PLAINTEXT_LEN
                    );
                    if let Some(inner) = frame.payload["events"].as_array() {
                        delivered.extend(inner.iter().cloned());
                    } else {
                        delivered.push(serde_json::to_value(frame).unwrap());
                    }
                }
                assert_eq!(
                    delivered,
                    serde_json::to_value(events)
                        .unwrap()
                        .as_array()
                        .unwrap()
                        .clone()
                );
                assert_eq!(queue.pending_bytes, 0);
            }
        }
    }
}

/// The queue preserves the coalescer's ordering rule: a non-chunk event
/// force-flushes pending chunk text ahead of itself, so merged chunks can
/// never leapfrog a tool call that arrived after them.
#[test]
fn non_chunk_events_flush_pending_chunks_ahead_of_themselves() {
    fn chunk(seq: u64, text: &str) -> observer::ObserverEvent {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({
            "params": { "update": {
                "sessionUpdate": "agent_message_chunk",
                "messageId": "m1",
                "content": { "text": text },
            }}
        });
        e
    }

    let mut queue = ObserverPublishQueue::default();
    queue.ingest(chunk(1, "hello "));
    queue.ingest(chunk(2, "world"));
    queue.ingest(event(3, "tool_call", Some("chan-a")));

    let frame = queue.next_frame().expect("one frame");
    assert!(queue.is_empty());
    let inner = frame.payload["events"].as_array().expect("batch of 2");
    assert_eq!(inner.len(), 2, "two chunks coalesce into one event");
    assert_eq!(
        inner[0]["payload"]["params"]["update"]["content"]["text"], "hello world",
        "chunk text merged before the tool call"
    );
    assert_eq!(inner[1]["kind"], "tool_call");
    assert!(inner[0]["seq"].as_u64() < inner[1]["seq"].as_u64());
}

/// Chunks still pending inside the coalescer (no non-chunk flushed them)
/// are picked up by the publish slot itself, not stranded.
#[test]
fn a_publish_slot_flushes_pending_coalesced_chunks() {
    let mut e = event(1, "acp_read", Some("chan-a"));
    e.payload = serde_json::json!({
        "params": { "update": {
            "sessionUpdate": "agent_message_chunk",
            "messageId": "m1",
            "content": { "text": "buffered" },
        }}
    });
    let mut queue = ObserverPublishQueue::default();
    queue.ingest(e);
    assert!(!queue.is_empty(), "pending chunk counts as queued work");

    let frame = queue.next_frame().expect("chunk must ship");
    assert!(queue.is_empty());
    assert_eq!(
        frame.payload["params"]["update"]["content"]["text"],
        "buffered"
    );
}

/// The queue ceiling assertion: when sustained input outruns the one-frame
/// drain budget for longer than the queue's byte budget, the OLDEST events
/// drop with accounting — never silently — and everything that survives
/// publishes in order with nothing else lost.
#[test]
fn over_budget_floods_drop_oldest_with_accounting() {
    let big_text = "y".repeat(10_000);
    let total = 500usize; // ~5MB of ~10KB events > 4MiB budget
    let mut queue = ObserverPublishQueue::default();
    for seq in 1..=total as u64 {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({ "seq": seq, "text": big_text });
        queue.ingest(e);
    }

    assert!(
        queue.dropped_events > 0,
        "a 5MB backlog must overflow the 4MiB budget"
    );
    assert!(
        walked_retained_bytes(&queue) <= OBSERVER_PENDING_QUEUE_MAX_BYTES,
        "eviction must restore the byte budget (entry-walked), got {}",
        walked_retained_bytes(&queue)
    );

    let frames = drain_frames(&mut queue);
    let published: Vec<u64> = frames.iter().flat_map(frame_seqs).collect();
    let expected: Vec<u64> = (queue.dropped_events + 1..=total as u64).collect();
    assert_eq!(
        published, expected,
        "exactly the oldest `dropped_events` events are missing; the rest \
             publish in order"
    );
    assert_eq!(
        published.len() as u64 + queue.dropped_events,
        total as u64,
        "accounting: published + dropped == ingested"
    );
}

/// The coalescer-bypass regression: a flood of chunks with DISTINCT
/// messageIds never flushes on its own, so every chunk sits in the
/// coalescer's pending buffer. TRUE retained bytes — walked from the
/// entries, never the queue's own accumulator — MUST respect the byte
/// budget with event-level drop accounting. Pre-fix this retained ~25MB
/// against the 4 MiB cap with `pending_bytes == 0` and zero drops; the
/// accounting refinement caught the accumulator itself reading
/// under cap while true retention was 1.99x over.
#[test]
fn distinct_key_chunk_floods_are_bounded_by_the_byte_budget() {
    let big_text = "z".repeat(50_000);
    let total = 500u64; // ~25MB pending chunk text vs a 4MiB budget
    let mut queue = ObserverPublishQueue::default();
    for seq in 1..=total {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": format!("message-{seq}"),
                    "content": { "type": "text", "text": big_text },
                },
            },
        });
        queue.ingest(e);
    }

    let walked = walked_retained_bytes(&queue);
    assert!(
        walked <= OBSERVER_PENDING_QUEUE_MAX_BYTES,
        "TRUE retained bytes (walked from entries) must respect the cap, \
             got {walked}"
    );
    assert!(
        queue.total_pending_bytes() >= walked,
        "the accumulator must never under-count true retention \
             (accumulator {} < walked {walked})",
        queue.total_pending_bytes()
    );
    assert!(
        queue.dropped_events > 0,
        "a ~25MB distinct-key chunk flood must record drops"
    );
    // Event-level accounting: everything that survives publishes, and
    // survivors + dropped == ingested.
    let frames = drain_frames(&mut queue);
    let survived: u64 = frames.iter().map(|f| frame_seqs(f).len() as u64).sum();
    assert_eq!(
        survived + queue.dropped_events,
        total,
        "accounting: published + dropped == ingested"
    );
    // The survivors are the NEWEST events (drop-oldest).
    let last_frame_seqs = frame_seqs(frames.last().expect("frames"));
    assert_eq!(*last_frame_seqs.last().expect("seqs"), total);
}

/// The merged-chunk accounting regression: one coalescer entry can
/// represent MANY generated observer events (same-messageId chunks merge
/// in place), so evicting it must charge every merged source event to
/// `dropped_events`, not 1 per retained entry. Pre-fix, evicting an entry
/// that merged 50 chunks recorded `dropped_events == 1` and 49 generated
/// events vanished from the accounting.
#[test]
fn evicting_a_merged_chunk_entry_accounts_every_source_event() {
    fn chunk(seq: u64, message_id: &str, text: &str) -> observer::ObserverEvent {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": message_id,
                    "content": { "type": "text", "text": text },
                },
            },
        });
        e
    }

    let mut queue = ObserverPublishQueue::default();
    // 50 × 1KB chunks under ONE messageId merge into a single pending
    // coalescer entry — the oldest item anywhere in the queue.
    let merged_text = "m".repeat(1_000);
    let merged_sources = 50u64;
    for seq in 1..=merged_sources {
        queue.ingest(chunk(seq, "message-merged", &merged_text));
    }
    // Flood with distinct-key 50KB chunks until the byte budget evicts
    // the oldest entries — the merged entry goes first.
    let flood_text = "f".repeat(50_000);
    let flood = 100u64;
    for seq in 1..=flood {
        queue.ingest(chunk(
            merged_sources + seq,
            &format!("message-{seq}"),
            &flood_text,
        ));
    }

    assert!(
        walked_retained_bytes(&queue) <= OBSERVER_PENDING_QUEUE_MAX_BYTES,
        "eviction must restore the byte budget (entry-walked), got {}",
        walked_retained_bytes(&queue)
    );
    let frames = drain_frames(&mut queue);
    assert!(
        !frames
            .iter()
            .flat_map(frame_seqs)
            .any(|seq| seq <= merged_sources),
        "the merged entry (globally oldest) must have been evicted"
    );
    // Every survivor is an unmerged distinct-key chunk (1 source each),
    // so source-event accounting must close exactly: the merged entry's
    // eviction charges all 50 sources.
    let survived: u64 = frames.iter().map(|f| frame_seqs(f).len() as u64).sum();
    assert_eq!(
        survived + queue.dropped_events,
        merged_sources + flood,
        "accounting: published sources + dropped sources == ingested"
    );
}

/// The forced-flush probe covers the OTHER eviction arm. A
/// merged entry FLUSHED into the publish FIFO (by a non-chunk event) must
/// still charge every absorbed source on eviction — the FIFO stores the
/// per-entry count precisely so the ledger survives flush. The
/// coalescer-side regression above never exercises this arm; mutating the
/// FIFO eviction to `dropped += 1` survived all 687 tests until this one.
#[test]
fn evicting_a_flushed_merged_entry_from_the_fifo_accounts_every_source_event() {
    fn chunk(seq: u64, message_id: &str, text: &str) -> observer::ObserverEvent {
        let mut e = event(seq, "acp_read", Some("chan-a"));
        e.payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": message_id,
                    "content": { "type": "text", "text": text },
                },
            },
        });
        e
    }

    let mut queue = ObserverPublishQueue::default();
    // 50 × 1KB chunks merge under one messageId in the coalescer…
    let merged_text = "m".repeat(1_000);
    let merged_sources = 50u64;
    for seq in 1..=merged_sources {
        queue.ingest(chunk(seq, "message-merged", &merged_text));
    }
    // …then a non-chunk event force-flushes the merged entry into the
    // publish FIFO. From here eviction happens on the FIFO arm.
    queue.ingest(event(merged_sources + 1, "tool_call", Some("chan-a")));
    assert!(
        queue.coalescer.pending.is_empty(),
        "the non-chunk event must have flushed the merged entry"
    );
    assert_eq!(
        queue.events.front().expect("flushed entry queued").1,
        merged_sources,
        "the FIFO front must carry the merged source count"
    );

    // Distinct-key flood forces byte-budget eviction of the FIFO front.
    let flood_text = "f".repeat(50_000);
    let flood = 100u64;
    for seq in 1..=flood {
        queue.ingest(chunk(
            merged_sources + 1 + seq,
            &format!("message-{seq}"),
            &flood_text,
        ));
    }

    assert!(
        walked_retained_bytes(&queue) <= OBSERVER_PENDING_QUEUE_MAX_BYTES,
        "eviction must restore the byte budget (entry-walked), got {}",
        walked_retained_bytes(&queue)
    );
    let frames = drain_frames(&mut queue);
    assert!(
        !frames
            .iter()
            .flat_map(frame_seqs)
            .any(|seq| seq <= merged_sources),
        "the flushed merged entry (globally oldest) must have been evicted"
    );
    // Ledger in source units: survivors are unmerged (1 source each), the
    // evicted merged FIFO entry must charge all 50 sources.
    let survived: u64 = frames.iter().map(|f| frame_seqs(f).len() as u64).sum();
    let ingested = merged_sources + 1 + flood;
    assert_eq!(
        survived + queue.dropped_events,
        ingested,
        "accounting: published sources + dropped sources == ingested"
    );
}

/// Under the byte budget the queue is lossless: every ingested event
/// publishes exactly once.
#[test]
fn under_budget_backlogs_are_lossless() {
    let mut queue = queue_of(
        (1..=200)
            .map(|seq| event(seq, "acp_read", Some("chan-a")))
            .collect(),
    );
    let frames = drain_frames(&mut queue);
    let published: Vec<u64> = frames.iter().flat_map(frame_seqs).collect();
    assert_eq!(published, (1..=200).collect::<Vec<u64>>());
    assert_eq!(queue.dropped_events, 0);
}
