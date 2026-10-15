//! Real persistent-WS receive loop -> real HTTP submit admission regressions.
use super::*;
use crate::relay_admission::{reset_rate_limit_gate, TEST_SERIAL};
use axum::{routing::post, Json, Router};

async fn http_relay() -> (
    String,
    mpsc::Receiver<(nostr::Event, std::time::Instant)>,
    tokio::task::JoinHandle<()>,
) {
    let (sent, received) = mpsc::channel(4);
    let router = Router::new()
        .route(
            "/query",
            post(|| async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({"error": "rate-limited: quota exceeded; retry in 1s"})),
                )
            }),
        )
        .route(
            "/events",
            post(move |Json(event): Json<nostr::Event>| {
                let sent = sent.clone();
                async move {
                    let received_at = std::time::Instant::now();
                    let id = event.id.to_hex();
                    assert!(event.verify().is_ok());
                    sent.send((event, received_at)).await.unwrap();
                    Json(serde_json::json!({"event_id": id, "accepted": true, "message": ""}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}"), received, server)
}

fn reply(keys: &Keys) -> nostr::Event {
    EventBuilder::new(nostr::Kind::Custom(9), "startup reply")
        .tags([
            nostr::Tag::parse(["h", "5b130804-d759-40ad-a564-d64cc907fa8e"]).unwrap(),
            nostr::Tag::parse(["e", &"a".repeat(64), "", "reply"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

async fn closed_then_http_submit(message: &str, shared_unavailable: bool) {
    let _serial = TEST_SERIAL.lock().await;
    reset_rate_limit_gate();
    let (ws_url, mut frames, commands) = stub_relay().await;
    let keys = Keys::generate();
    let (session, mut events) = start(ws_url, keys.clone(), None).await;
    session
        .set_subscriptions(vec![
            probe_subscription(),
            Subscription {
                id: "barrier".into(),
                filter: serde_json::json!({"kinds": [1], "limit": 0}),
            },
        ])
        .await;
    assert_eq!(next_req(&mut frames, "probe REQ").await, PROBE_ID);
    assert_eq!(next_req(&mut frames, "barrier REQ").await, "barrier");
    commands
        .send(StubCommand::Closed(PROBE_ID.into(), message.into()))
        .await
        .unwrap();
    // An ordered frame on an unaffected persistent subscription proves CLOSED
    // went through the receive loop. No test-side gate activation or sleeps.
    let barrier = EventBuilder::text_note("barrier")
        .sign_with_keys(&keys)
        .unwrap();
    commands
        .send(StubCommand::Event(
            "barrier".into(),
            serde_json::to_value(barrier).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap()
            .subscription_id,
        "barrier"
    );

    let (http_url, mut submitted, server) = http_relay().await;
    let state = crate::app_state::build_app_state();
    let event = reply(&keys);
    let submit = crate::relay::submit_signed_event_at_with_keys(&event, &state, &http_url, &keys);
    let outcome = tokio::time::timeout(Duration::from_secs(1), submit).await;
    session.shutdown();
    server.abort();
    reset_rate_limit_gate();
    if shared_unavailable {
        assert!(
            outcome.is_err(),
            "shared admission outage must still damp HTTP"
        );
        assert!(
            submitted.try_recv().is_err(),
            "HTTP must not dispatch during the shared outage"
        );
    } else {
        let response = outcome
            .expect("WS quota must not withhold HTTP submission")
            .unwrap();
        assert!(response.accepted);
        assert_eq!(response.event_id, event.id.to_hex());
        assert_eq!(submitted.recv().await.unwrap().0, event);
    }
    assert!(
        frames.try_recv().is_err(),
        "the limited WS subscription must not reopen early"
    );
}

#[tokio::test]
async fn persistent_ws_quota_does_not_withhold_http_reply() {
    closed_then_http_submit("rate-limited: quota exceeded; retry in 50s", false).await;
}

#[tokio::test]
async fn persistent_ws_concurrency_does_not_withhold_http_reply() {
    closed_then_http_submit("rate-limited: too many concurrent requests", false).await;
}

#[tokio::test]
async fn persistent_ws_shared_unavailable_still_withholds_http_reply() {
    closed_then_http_submit("rate-limited: shared admission unavailable", true).await;
}

#[tokio::test]
async fn http_429_still_withholds_http_reply_then_accepts_it() {
    let _serial = TEST_SERIAL.lock().await;
    reset_rate_limit_gate();
    let (http_url, mut submitted, server) = http_relay().await;
    let state = crate::app_state::build_app_state();
    let keys = Keys::generate();
    let event = reply(&keys);
    let error = crate::relay::query_relay_at(&state, &http_url, &[serde_json::json!({"limit": 1})])
        .await
        .unwrap_err();
    assert_eq!(error, "relay rate-limited: retry in 1s");
    let before = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        crate::relay::submit_signed_event_at_with_keys(&event, &state, &http_url, &keys),
    )
    .await;
    server.abort();
    reset_rate_limit_gate();
    let response = outcome.unwrap().unwrap();
    let (received, received_at) = submitted.recv().await.unwrap();
    assert!(
        received_at.duration_since(before) >= Duration::from_millis(900),
        "HTTP submit must honour its own cooldown"
    );
    assert!(response.accepted);
    assert_eq!(received, event);
}
