//! Bounded synthetic WebSocket fixtures; no relay service, proxy or real agent.
use super::tests::{next_test_frame, seed_test_subscription, test_channel_filter, test_ws_pair};
use super::*;

fn fixture_event(channel: Uuid, n: u64, kind: u16) -> Event {
    let keys =
        Keys::parse("0000000000000000000000000000000000000000000000000000000000000001").unwrap();
    EventBuilder::new(Kind::Custom(kind), format!("synthetic-{n}"))
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .custom_created_at(nostr::Timestamp::from(1_000 + n))
        .sign_with_keys(&keys)
        .unwrap()
}

async fn dispatch(
    client: &mut WsStream,
    state: &mut BgState,
    tx: &mpsc::Sender<Option<BuzzEvent>>,
    frame: Value,
) {
    let (control_tx, _control_rx) = mpsc::channel(1);
    assert!(
        handle_ws_message(
            Message::Text(frame.to_string().into()),
            client,
            tx,
            &control_tx,
            state,
            &Keys::generate(),
            "ws://127.0.0.1:1",
            "synthetic-agent",
            None,
        )
        .await
    );
}

#[tokio::test]
async fn repeated_overflow_recovers_only_affected_channel_after_capacity() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channels: Vec<_> = (0..18).map(|_| Uuid::new_v4()).collect();
    for ch in &channels {
        seed_test_subscription(&mut state, *ch);
    }
    let ch = channels[0];
    let sub = channel_sub_id(ch);
    let (tx, mut rx) = mpsc::channel(256);
    // Relay history is newest-first; the oldest dropped event must survive
    // a watermark already advanced by much newer successfully-enqueued events.
    let events: Vec<_> = (0..320).rev().map(|n| fixture_event(ch, n, 9)).collect();
    for event in &events {
        dispatch(&mut client, &mut state, &tx, json!(["EVENT", sub, event])).await;
        recovery::recover_one(&mut client, &mut state, &tx, "synthetic-agent").await;
    }
    assert_eq!(rx.len(), 256);
    assert_eq!(state.channel_dropped_since[&ch], 1_000);
    assert!(timeout(Duration::from_millis(30), server.next())
        .await
        .is_err());
    for event in &events[..256] {
        assert_eq!(rx.recv().await.unwrap().unwrap().event.id, event.id);
    }
    recovery::recover_one(&mut client, &mut state, &tx, "synthetic-agent").await;
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[0], "REQ");
    assert_eq!(req[1], sub);
    assert_eq!(req[2]["#h"], json!([ch.to_string()]));
    assert_eq!(req[2]["kinds"], json!([9]));
    assert_eq!(req[2]["since"], 995);
    // Concurrent timer ticks / duplicate arrivals cannot replace the replay.
    for _ in 0..40 {
        recovery::recover_one(&mut client, &mut state, &tx, "synthetic-agent").await;
    }
    assert!(timeout(Duration::from_millis(30), server.next())
        .await
        .is_err());
    for event in &events {
        dispatch(&mut client, &mut state, &tx, json!(["EVENT", sub, event])).await;
    }
    dispatch(&mut client, &mut state, &tx, json!(["EOSE", sub])).await;
    assert_eq!(rx.len(), 64, "delivered IDs must remain deduplicated");
    for event in &events[256..] {
        assert_eq!(rx.recv().await.unwrap().unwrap().event.id, event.id);
    }
    assert_eq!(state.channel_since(&ch), Some(1_319));
    recovery::recover_one(&mut client, &mut state, &tx, "synthetic-agent").await;
    assert!(timeout(Duration::from_millis(30), server.next())
        .await
        .is_err());
    let live = fixture_event(ch, 400, 9);
    dispatch(&mut client, &mut state, &tx, json!(["EVENT", sub, live])).await;
    assert_eq!(rx.recv().await.unwrap().unwrap().event.id, live.id);
    println!("18 subscriptions; 320 newest-first arrivals; 64 losses coalesced; 0 REQ while full; 1 targeted REQ; 320 unique deliveries + live");
}

#[tokio::test]
async fn socket_owner_services_ping_shutdown_and_coalesces_overflow_ticks() {
    let (client, mut server) = test_ws_pair().await;
    let (tx, mut rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let task = tokio::spawn(run_background_task(
        client,
        VecDeque::new(),
        tx,
        control_tx,
        cmd_rx,
        Keys::generate(),
        "ws://127.0.0.1:1".into(),
        "synthetic-agent".into(),
        None,
    ));
    let channels: Vec<_> = (0..18).map(|_| Uuid::new_v4()).collect();
    for ch in &channels {
        cmd_tx
            .send(RelayCommand::Subscribe {
                channel_id: *ch,
                filter: test_channel_filter(),
                replay_since: Some(1_000),
            })
            .await
            .unwrap();
    }
    let mut subscriptions = 0;
    while subscriptions < 18 {
        let frame = timeout(Duration::from_secs(2), server.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(text) => {
                let req: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(req[0], "REQ");
                server
                    .send(Message::Text(json!(["EOSE", req[1]]).to_string().into()))
                    .await
                    .unwrap();
                subscriptions += 1;
            }
            Message::Ping(payload) => server.send(Message::Pong(payload)).await.unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }
    let sub = channel_sub_id(channels[0]);
    for n in 0..40 {
        server
            .send(Message::Text(
                json!(["EVENT", sub, fixture_event(channels[0], n, 9)])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    server.send(Message::Ping(vec![42].into())).await.unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            match server.next().await.unwrap().unwrap() {
                Message::Ping(payload) => server.send(Message::Pong(payload)).await.unwrap(),
                Message::Pong(payload) => {
                    assert_eq!(payload.as_ref(), &[42]);
                    break;
                }
                other => panic!("no immediate all-channel recovery before ping: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    // Wait across a recovery tick with the consumer still full.
    assert!(socket_frame(&mut server, Duration::from_millis(5_100))
        .await
        .is_none());
    rx.recv().await.unwrap().unwrap();
    let frame = timeout(Duration::from_secs(6), server.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let req: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
    assert_eq!(req[1], sub);
    assert_eq!(req[2]["since"], 996);
    assert!(timeout(Duration::from_millis(100), server.next())
        .await
        .is_err());
    // Sustained lag: each replay is followed by another burst, without EOSE.
    // Recovery must stay paced, not permanently stall or sweep healthy channels.
    for round in 1..=3 {
        let started = tokio::time::Instant::now();
        for n in round * 40..(round + 1) * 40 {
            server
                .send(Message::Text(
                    json!(["EVENT", sub, fixture_event(channels[0], n, 9)])
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        server.send(Message::Ping(vec![43].into())).await.unwrap();
        let pong = timeout(Duration::from_secs(1), server.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(pong, Message::Pong(_)),
            "recovery preempted ping: {pong:?}"
        );
        rx.recv().await.unwrap().unwrap();
        let frame = timeout(Duration::from_secs(6), server.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let req: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(req[1], sub, "healthy channels must not be swept");
        assert!(
            started.elapsed() >= Duration::from_secs(4),
            "unpaced repeat"
        );
    }
    cmd_tx.send(RelayCommand::Shutdown).await.unwrap();
    timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    println!("socket-owner seam: 18 live REQs, 39 coalesced losses, PONG while full, zero recovery across full-capacity timer tick, four paced targeted REQs over sustained lag without EOSE, responsive shutdown");
}

#[tokio::test]
async fn recovery_is_fair_and_paced_even_with_new_loss_and_stale_eose() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channels = [Uuid::new_v4(), Uuid::new_v4()];
    for ch in channels {
        seed_test_subscription(&mut state, ch);
        state.channel_dropped_since.insert(ch, 600);
    }
    state.membership_sub_active = true;
    state.membership_dropped_since = Some(500);
    let (tx, _rx) = mpsc::channel(1);
    let mut visited = HashSet::new();
    for round in 0..9 {
        timeout(
            Duration::from_millis(100),
            recovery::ready(&tx, recovery::ready_at(&mut state)),
        )
        .await
        .unwrap();
        recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
        let req = next_test_frame(&mut server).await;
        let sub = req[1].as_str().unwrap();
        if round < 3 {
            assert!(visited.insert(sub.to_owned()), "starved intent");
        }
        if sub == MEMBERSHIP_NOTIF_SUB_ID {
            assert_eq!(req[2]["since"], 495);
            state.membership_dropped_since = Some(500);
        } else {
            let ch = channel_id_from_sub_id(sub).unwrap();
            assert_eq!(req[2]["since"], 595);
            state.channel_dropped_since.insert(ch, 600);
        }
        // Neither stale nor current EOSE creates completion state or erases loss.
        dispatch(&mut client, &mut state, &tx, json!(["EOSE", sub])).await;
        for _ in 0..30 {
            recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
        }
        assert!(timeout(Duration::from_millis(1), server.next())
            .await
            .is_err());
        advance_clock(recovery::RECOVERY_INTERVAL).await;
    }
    for ch in channels {
        state.active_subscriptions.remove(&ch);
        state.clear_channel_state(&ch);
        assert!(!state
            .recovery
            .last_attempt
            .contains_key(&channel_sub_id(ch)));
    }
    assert_eq!(state.recovery.last_attempt.len(), 1);
}

#[tokio::test]
async fn gate_headroom_failed_writes_and_reconnect_preserve_pending_attempts() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let ch = Uuid::new_v4();
    seed_test_subscription(&mut state, ch);
    state.channel_dropped_since.insert(ch, 700);
    let (tx, mut rx) = mpsc::channel(4);
    for _ in 0..3 {
        tx.try_send(None).unwrap();
    }
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert!(state.recovery.last_attempt.is_empty());
    rx.recv().await;
    state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_secs(10));
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert!(state.recovery.last_attempt.is_empty());
    advance_clock(Duration::from_secs(10)).await;
    // Close locally, so the actual production writer fails deterministically.
    client.close(None).await.unwrap();
    server.next().await;
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_eq!(state.channel_dropped_since[&ch], 700);
    let attempted = state.recovery.last_attempt.clone();
    for _ in 0..30 {
        recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    }
    assert_eq!(state.recovery.last_attempt, attempted);
    advance_clock(recovery::RECOVERY_INTERVAL).await;
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_ne!(
        state.recovery.last_attempt, attempted,
        "failed write must be retried"
    );
    assert_eq!(state.channel_dropped_since[&ch], 700);

    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    assert!(matches!(
        resubscribe_after_reconnect(&mut client, &mut cmd_rx, &mut state, "agent", true,).await,
        ResubscribeResult::Ok
    ));
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[1], channel_sub_id(ch));
    assert_eq!(req[2]["since"], 695);
    assert!(!state.channel_dropped_since.contains_key(&ch));
    // This is deliberately the baseline write-retirement contract, not a receipt.
}

async fn advance_clock(duration: Duration) {
    tokio::time::pause();
    tokio::time::advance(duration).await;
    tokio::time::resume();
}

#[tokio::test]
async fn blocked_recovery_write_is_bounded_and_retains_loss() {
    let (mut client, _stalled_server) = test_ws_pair().await;
    let mut state = BgState::new();
    let ch = Uuid::new_v4();
    seed_test_subscription(&mut state, ch);
    // Bounded 16MB JSON request exceeds loopback TCP buffering. The server does
    // not read it. This tests the real production write/timeout, not a mock sink.
    state.active_filters.get_mut(&ch).unwrap().kinds = Some(vec![9; 8_000_000]);
    state.channel_dropped_since.insert(ch, 700);
    let (tx, _rx) = mpsc::channel(1);
    let started = tokio::time::Instant::now();
    timeout(
        Duration::from_secs(15),
        recovery::recover_one(&mut client, &mut state, &tx, "agent"),
    )
    .await
    .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(WS_SEND_TIMEOUT_SECS));
    assert_eq!(state.channel_dropped_since[&ch], 700);
    let attempted = state.recovery.last_attempt.clone();
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_eq!(state.recovery.last_attempt, attempted);
}

// Keep the fixture responsive to independent client keepalives while checking
// recovery traffic. Wall-clock scheduling may deliver the initial ping late.
async fn socket_frame(
    server: &mut WebSocketStream<tokio::net::TcpStream>,
    duration: Duration,
) -> Option<Message> {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        match tokio::time::timeout_at(deadline, server.next()).await {
            Err(_) => return None,
            Ok(Some(Ok(Message::Ping(payload)))) => {
                server.send(Message::Pong(payload)).await.unwrap();
            }
            Ok(Some(Ok(frame))) => return Some(frame),
            other => panic!("unexpected socket state: {other:?}"),
        }
    }
}

#[path = "recovery_wake_tests.rs"]
mod wake;
