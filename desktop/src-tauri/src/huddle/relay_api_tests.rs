use super::*;

#[test]
fn relay_auth_errors_preserve_stable_codes_for_ui_mapping() {
    for (code, message) in [
        ("room_full", "room participant capacity reached"),
        ("room_ended", "huddle has ended"),
        ("huddle_relay_draining", "relay is draining; reconnect"),
        (
            "huddle_owner_unreachable",
            "could not reach the huddle owner",
        ),
        ("unsupported_version", "unsupported audio protocol version"),
        ("upgrade_required", "audio protocol upgrade required"),
    ] {
        let payload = serde_json::json!({
            "type": "error",
            "code": code,
            "message": message,
        });
        let error = format_audio_relay_error(&payload);
        assert_eq!(error.code(), Some(code));
        assert_eq!(
            error.to_string(),
            format!("audio relay auth error [{code}]: {message}")
        );
    }
}

#[test]
fn audio_send_queue_drops_oldest_frame_when_full() {
    let queue = AudioSendQueue::default();
    for value in 0..=AUDIO_SEND_QUEUE_DEPTH as u8 {
        queue.push_latest(vec![value]);
    }
    let frames = queue
        .state
        .lock()
        .expect("queue")
        .frames
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(frames, vec![vec![1], vec![2], vec![3], vec![4]]);
}

#[tokio::test]
async fn audio_send_queue_close_wakes_waiter_and_rejects_new_frames() {
    let queue = std::sync::Arc::new(AudioSendQueue::default());
    let waiting_queue = std::sync::Arc::clone(&queue);
    let waiter = tokio::spawn(async move { waiting_queue.pop().await });
    tokio::task::yield_now().await;

    queue.close();
    assert_eq!(waiter.await.expect("waiter"), None);
    queue.push_latest(vec![1]);
    assert_eq!(queue.pop().await, None);
}

#[tokio::test]
async fn audio_send_queue_drains_before_reporting_closed() {
    let queue = AudioSendQueue::default();
    queue.push_latest(vec![1]);
    queue.close();

    assert_eq!(queue.pop().await, Some(vec![1]));
    assert_eq!(queue.pop().await, None);
}

#[tokio::test]
async fn wire_send_failure_is_preserved_for_pipeline_owner() {
    struct FailingSink;
    impl futures_util::Sink<WsMsg> for FailingSink {
        type Error = &'static str;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Err("socket closed"))
        }
        fn start_send(self: std::pin::Pin<&mut Self>, _item: WsMsg) -> Result<(), Self::Error> {
            Err("socket closed")
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    let queue = std::sync::Arc::new(AudioSendQueue::default());
    queue.push_latest(vec![1]);
    let result = wire_send_loop(
        queue,
        std::sync::Arc::new(tokio::sync::Mutex::new(FailingSink)),
    )
    .await;
    assert!(
        result.is_err_and(|error| error == "audio send: socket closed"),
        "the reconnect owner must receive the socket send failure"
    );
}

#[test]
fn tts_upsampling_doubles_rate_with_linear_midpoints() {
    assert_eq!(
        upsample_tts_24k_to_48k(&[0.0, 1.0, -1.0]),
        vec![0.0, 0.5, 1.0, 0.0, -1.0, -1.0]
    );
}

#[test]
fn tts_queue_rejects_cancelled_versions_and_pads_twenty_ms_frames() {
    let mut queue = std::collections::VecDeque::new();
    queue_tts_broadcast_packet(
        &mut queue,
        super::super::tts::TtsBroadcastPacket {
            epoch: 1,
            speaker_generation: 7,
            samples_24k: vec![0.25; 480],
        },
        1,
        7,
    );
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].samples_48k.len(), 960);

    queue_tts_broadcast_packet(
        &mut queue,
        super::super::tts::TtsBroadcastPacket {
            epoch: 1,
            speaker_generation: 7,
            samples_24k: vec![0.5; 480],
        },
        2,
        7,
    );
    assert_eq!(queue.len(), 1, "cancelled epoch must not enqueue");
}

#[test]
fn authenticated_roster_parsing_checks_routing_bounds() {
    let peer = serde_json::json!({"pubkey": "agent", "peer_index": 7, "epoch": 3});
    assert_eq!(parse_audio_roster_peer(&peer), Some((7, "agent".into(), 3)));
    let legacy = serde_json::json!({"pubkey": "human", "peer_index": 8});
    assert_eq!(
        parse_audio_roster_peer(&legacy),
        Some((8, "human".into(), 0))
    );
    for field in ["peer_index", "epoch"] {
        let mut invalid = peer.clone();
        invalid[field] = 256.into();
        assert_eq!(parse_audio_roster_peer(&invalid), None, "{field}");
    }
}

// Drop records are set by the actual spawned children, not by the supervisor.
// Awaiting supervisor completion must imply every child has released ownership.
struct ChildDrop {
    finished: Arc<std::sync::atomic::AtomicUsize>,
    floor: Option<super::super::human_floor::HumanFloor>,
}
impl Drop for ChildDrop {
    fn drop(&mut self) {
        if let Some(floor) = &self.floor {
            // Model a final write during child teardown. Cleanup must run AFTER
            // this, not merely schedule an abort and clear the shared state.
            floor.enter_remote(9);
        }
        self.finished
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn supervisor_joins_children_and_clears_only_its_connection_on_every_exit() {
    for exit in [
        "encode",
        "send",
        "receive",
        "encode-panic",
        "send-panic",
        "receive-panic",
        "cancel",
    ] {
        let root = super::super::human_floor::HumanFloor::new();
        let old = root.for_audio_connection();
        let replacement = root.for_audio_connection();
        old.enter_remote(7);
        replacement.enter_remote(7); // same routing index, different socket
        root.enter_local(true, true);
        let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let encode_drop = ChildDrop {
            finished: Arc::clone(&finished),
            floor: None,
        };
        let send_drop = ChildDrop {
            finished: Arc::clone(&finished),
            floor: None,
        };
        let recv_drop = ChildDrop {
            finished: Arc::clone(&finished),
            floor: Some(old.clone()),
        };
        let (encode_tx, encode_rx) = tokio::sync::oneshot::channel::<bool>();
        let (send_tx, send_rx) = tokio::sync::oneshot::channel::<bool>();
        let (recv_tx, recv_rx) = tokio::sync::oneshot::channel::<bool>();
        let encode = tokio::spawn(async move {
            let _guard = encode_drop;
            assert!(!encode_rx.await.unwrap(), "encode panic");
        });
        let send = tokio::spawn(async move {
            let _guard = send_drop;
            assert!(!send_rx.await.unwrap(), "send panic");
            Err("socket failed".into())
        });
        let recv = tokio::spawn(async move {
            let _guard = recv_drop;
            assert!(!recv_rx.await.unwrap(), "receive panic");
        });
        let cancel = CancellationToken::new();
        match exit {
            "encode" => encode_tx.send(false).unwrap(),
            "send" => send_tx.send(false).unwrap(),
            "receive" => recv_tx.send(false).unwrap(),
            "encode-panic" => encode_tx.send(true).unwrap(),
            "send-panic" => send_tx.send(true).unwrap(),
            "receive-panic" => recv_tx.send(true).unwrap(),
            _ => cancel.cancel(),
        }
        let queue = AudioSendQueue::default();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            supervise_audio_tasks(encode, send, recv, &queue, &cancel, &old),
        )
        .await
        .expect("supervisor must terminate");
        assert_eq!(
            result.is_err(),
            exit == "send" || exit.ends_with("panic"),
            "{exit}: {result:?}"
        );
        assert_eq!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "{exit}: unjoined child"
        );
        assert!(
            queue.state.lock().unwrap().closed,
            "{exit}: queue left open"
        );
        // Old cleanup must preserve both replacement and local ownership.
        replacement.leave_remote(7);
        assert!(root.is_blocked(), "{exit}: local floor was erased");
        root.leave_local();
        assert!(!root.is_blocked(), "{exit}: old remote floor leaked");
        replacement.enter_remote(7);
        old.clear_remote();
        assert!(
            root.is_blocked(),
            "{exit}: stale cleanup erased replacement"
        );
        replacement.clear_remote();
        assert!(!root.is_blocked());
    }
}

#[tokio::test]
async fn supervisor_waits_for_receiver_drop_when_other_children_already_finished() {
    let root = super::super::human_floor::HumanFloor::new();
    let floor = root.for_audio_connection();
    let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let drop_guard = ChildDrop {
        finished: Arc::clone(&finished),
        floor: Some(floor.clone()),
    };
    let encode = tokio::spawn(async {});
    let send = tokio::spawn(async { Ok(()) });
    let recv = tokio::spawn(async move {
        let _guard = drop_guard;
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    assert!(encode.is_finished() && send.is_finished());
    supervise_audio_tasks(
        encode,
        send,
        recv,
        &AudioSendQueue::default(),
        &CancellationToken::new(),
        &floor,
    )
    .await
    .unwrap();
    assert_eq!(
        finished.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "receiver must have dropped before supervisor returns"
    );
    assert!(
        !root.is_blocked(),
        "cleanup follows the receiver's last write"
    );
}
