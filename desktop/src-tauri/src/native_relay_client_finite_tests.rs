//! Finite CLOSED recovery through the real native session and WebSocket loop.
use super::*;

fn fetch(
    session: &Arc<RelaySession>,
    timeout: Duration,
) -> tokio::task::JoinHandle<Result<Vec<Event>, String>> {
    let session = Arc::clone(session);
    tokio::spawn(async move {
        session
            .fetch_events(
                serde_json::json!({"kinds": [9], "#h": ["unread"], "since": 42, "limit": 1000}),
                timeout,
            )
            .await
    })
}

#[tokio::test]
async fn quota_retry_preserves_events_without_duplicates_and_keeps_archive_live() {
    let (url, mut frames, commands) = stub_relay().await;
    let (session, mut archive) = start(url, Keys::generate(), None).await;
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "archive").await, PROBE_ID);
    let pending = fetch(&session, Duration::from_secs(10));
    let id = next_req(&mut frames, "finite").await;
    let event = EventBuilder::text_note("retained")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let encoded = serde_json::to_value(&event).unwrap();
    commands
        .send(StubCommand::Event(id.clone(), encoded.clone()))
        .await
        .unwrap();
    let refused = Instant::now();
    commands
        .send(StubCommand::Closed(
            id.clone(),
            "rate-limited: quota exceeded; retry in 1s".into(),
        ))
        .await
        .unwrap();
    // Non-finite delivery is a receive-loop barrier, not a sleep or test-side retry setup.
    commands
        .send(StubCommand::Event(PROBE_ID.into(), encoded.clone()))
        .await
        .unwrap();
    assert_eq!(
        *tokio::time::timeout(Duration::from_secs(3), archive.recv())
            .await
            .unwrap()
            .unwrap()
            .event,
        event
    );
    assert!(
        !pending.is_finished(),
        "quota refusal must retain the finite operation"
    );
    assert_eq!(next_req(&mut frames, "quota retry").await, id);
    assert!(refused.elapsed() >= Duration::from_secs(1));
    commands
        .send(StubCommand::Event(id.clone(), encoded))
        .await
        .unwrap();
    commands.send(StubCommand::Eose(id.clone())).await.unwrap();
    assert_eq!(pending.await.unwrap().unwrap(), vec![event]);
    assert_eq!(
        next_frame(&mut frames, "finite CLOSE").await,
        Frame::Close(id)
    );
    session.shutdown();
}

#[tokio::test]
async fn zero_hint_exhausts_three_retries_as_an_error() {
    let (url, mut frames, commands) = stub_relay().await;
    let (session, _archive) = start(url, Keys::generate(), None).await;
    let pending = fetch(&session, Duration::from_secs(12));
    let id = next_req(&mut frames, "finite").await;
    for attempt in 0..4 {
        commands
            .send(StubCommand::Closed(
                id.clone(),
                "rate-limited: quota exceeded; retry in 0s".into(),
            ))
            .await
            .unwrap();
        if attempt < 3 {
            assert_eq!(next_req(&mut frames, "bounded retry").await, id);
        }
    }
    let error = pending.await.unwrap().unwrap_err();
    assert!(error.contains("rate-limited:"), "{error}");
    assert!(session.requests.lock().await.is_empty());
    session.shutdown();
}

#[tokio::test]
async fn terminal_refusal_and_deadline_do_not_become_empty_success() {
    for message in [
        "restricted: denied",
        "error: transient",
        "rate-limited: quota exceeded; retry in 10s",
        "rate-limited: quota exceeded",
        "rate-limited: quota exceeded; retry in 18446744073709551615s",
    ] {
        let (url, mut frames, commands) = stub_relay().await;
        let (session, _archive) = start(url, Keys::generate(), None).await;
        let pending = fetch(&session, Duration::from_millis(300));
        let id = next_req(&mut frames, "finite").await;
        commands
            .send(StubCommand::Closed(id, message.into()))
            .await
            .unwrap();
        let error = pending.await.unwrap().unwrap_err();
        if message.starts_with("rate-limited:") {
            assert!(error.contains("timed out"), "{error}");
        } else {
            assert!(error.contains(message), "{error}");
        }
        assert!(session.requests.lock().await.is_empty());
        assert!(session.state.lock().await.transient.is_empty());
        assert!(
            frames.try_recv().is_err(),
            "must not dispatch through denial/deadline"
        );
        session.shutdown();
    }
}

#[tokio::test]
async fn dropping_finite_caller_closes_request_without_stopping_archive() {
    let (url, mut frames, commands) = stub_relay().await;
    let (session, mut archive) = start(url, Keys::generate(), None).await;
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "archive").await, PROBE_ID);
    let pending = fetch(&session, Duration::from_secs(10));
    let id = next_req(&mut frames, "finite").await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(
        next_frame(&mut frames, "cancelled finite CLOSE").await,
        Frame::Close(id)
    );
    assert!(session.requests.lock().await.is_empty());
    assert!(session.state.lock().await.transient.is_empty());
    let event = EventBuilder::text_note("archive remains live")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    commands
        .send(StubCommand::Event(
            PROBE_ID.into(),
            serde_json::to_value(&event).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        *tokio::time::timeout(Duration::from_secs(3), archive.recv())
            .await
            .unwrap()
            .unwrap()
            .event,
        event
    );
    session.shutdown();
}

#[tokio::test]
async fn reconnect_preserves_quota_hold_and_partial_events_do_not_reset_budget() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (seen_tx, mut seen_rx) = mpsc::channel(8);
    let event = EventBuilder::text_note("partial replay")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let server = tokio::spawn(async move {
        let mut original: Option<serde_json::Value> = None;
        let mut refused = std::time::Instant::now();
        for connection in 0..2 {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text(
                serde_json::json!(["AUTH", "reconnect-test"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let mut requests = 0;
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                match frame[0].as_str() {
                    Some("AUTH") => ws
                        .send(Message::Text(
                            serde_json::json!(["OK", frame[1]["id"], true, ""])
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap(),
                    Some("REQ") => {
                        if let Some(expected) = &original {
                            assert_eq!(&frame, expected);
                            assert!(
                                refused.elapsed()
                                    >= Duration::from_secs(if connection == 1 && requests == 0 {
                                        2
                                    } else {
                                        1
                                    }),
                                "reconnect must not erase due time"
                            );
                        } else {
                            original = Some(frame.clone());
                        }
                        seen_tx.send(frame.clone()).await.unwrap();
                        ws.send(Message::Text(
                            serde_json::json!(["EVENT", frame[1], event])
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        let hint = if connection == 0 { 2 } else { 0 };
                        refused = std::time::Instant::now();
                        ws.send(Message::Text(
                            serde_json::json!([
                                "CLOSED",
                                frame[1],
                                format!("rate-limited: quota exceeded; retry in {hint}s")
                            ])
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                        requests += 1;
                        if connection == 0 {
                            ws.close(None).await.unwrap();
                            break;
                        }
                        if requests == 3 {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
    });
    let (session, _archive) = start(url, Keys::generate(), None).await;
    let pending = fetch(&session, Duration::from_secs(15));
    let error = tokio::time::timeout(Duration::from_secs(12), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("rate-limited:"), "{error}");
    let mut count = 0;
    while seen_rx.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(
        count, 4,
        "initial request plus three retries across both sockets"
    );
    assert!(session.requests.lock().await.is_empty());
    server.await.unwrap();
    session.shutdown();
}

#[tokio::test]
async fn caller_drop_during_authentication_is_reclaimed_without_restarting_connect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (connected_tx, connected) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        connected_tx.send(()).unwrap();
        release_rx.await.unwrap();
        ws.send(Message::Text(
            serde_json::json!(["AUTH", "held-auth"]).to_string().into(),
        ))
        .await
        .unwrap();
        let Some(Ok(Message::Text(text))) = ws.next().await else {
            panic!("same authentication attempt must survive wake")
        };
        let auth: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(auth[0], "AUTH");
        ws.send(Message::Text(
            serde_json::json!(["OK", auth[1]["id"], true, ""])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let next = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
        assert!(next.is_err(), "cancelled request must not be replayed");
    });
    let (session, _archive) = start(url, Keys::generate(), None).await;
    connected.await.unwrap();
    let pending = fetch(&session, Duration::from_secs(10));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !session.state.lock().await.transient.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if session.requests.lock().await.is_empty()
                && session.state.lock().await.transient.is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup must not require successful auth");
    release.send(()).unwrap();
    server.await.unwrap();
    session.shutdown();
}

#[tokio::test]
async fn cancellation_and_stale_eose_during_hold_cannot_complete_or_retry() {
    for shutdown in [false, true] {
        let (url, mut frames, commands) = stub_relay().await;
        let (session, mut archive) = start(url, Keys::generate(), None).await;
        session.set_subscriptions(vec![probe_subscription()]).await;
        assert_eq!(next_req(&mut frames, "archive").await, PROBE_ID);
        let pending = fetch(&session, Duration::from_secs(10));
        let id = next_req(&mut frames, "finite").await;
        commands
            .send(StubCommand::Closed(
                id.clone(),
                "rate-limited: quota exceeded; retry in 1s".into(),
            ))
            .await
            .unwrap();
        commands.send(StubCommand::Eose(id)).await.unwrap();
        let event = EventBuilder::text_note("barrier")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        commands
            .send(StubCommand::Event(
                PROBE_ID.into(),
                serde_json::to_value(event).unwrap(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), archive.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !pending.is_finished(),
            "stale EOSE must not finish refused history"
        );
        if shutdown {
            session.shutdown();
            assert!(pending.await.unwrap().unwrap_err().contains("cancelled"));
        } else {
            pending.abort();
            assert!(pending.await.unwrap_err().is_cancelled());
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if session.requests.lock().await.is_empty()
                    && session.state.lock().await.transient.is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1100), frames.recv())
                .await
                .ok()
                .flatten()
                .is_none(),
            "cancelled retry must never reopen"
        );
        session.shutdown();
    }
}

#[tokio::test]
async fn timeout_cleanup_remains_owned_when_caller_drops_under_state_contention() {
    let (url, mut frames, _commands) = stub_relay().await;
    let (session, _archive) = start(url, Keys::generate(), None).await;
    let pending = fetch(&session, Duration::from_millis(150));
    let id = next_req(&mut frames, "finite").await;
    let state = session.state.lock().await;
    // Hold the real cleanup lock beyond the caller's absolute deadline. Abort
    // only after cleanup has been runnable, then let the existing loop reclaim.
    tokio::time::sleep(Duration::from_millis(250)).await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    drop(state);
    assert_eq!(
        next_frame(&mut frames, "cancelled cleanup CLOSE").await,
        Frame::Close(id)
    );
    assert!(session.requests.lock().await.is_empty());
    assert!(session.state.lock().await.transient.is_empty());
    session.shutdown();
}

#[tokio::test]
async fn cancellation_during_registration_does_not_leave_transient_state() {
    let (url, mut frames, _commands) = stub_relay().await;
    let (session, _archive) = start(url, Keys::generate(), None).await;
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "archive").await, PROBE_ID);
    let state = session.state.lock().await;
    let pending = fetch(&session, Duration::from_secs(10));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !session.requests.lock().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    drop(state);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if session.requests.lock().await.is_empty()
                && session.state.lock().await.transient.is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(frames.try_recv().is_err());
    session.shutdown();
}
