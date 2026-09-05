use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_core_pkg::pairing::session::PairingSession;
use buzz_pair_relay_pkg::CONN_TIMEOUT;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tauri::Listener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tokio_util::sync::CancellationToken;

use super::{pairing_ws_task_on_socket, PairingMode, PairingTaskContext, PAIRING_HARD_TIMEOUT};

#[test]
fn pair_relay_conn_timeout_outlives_desktop_pairing_hard_timeout() {
    assert!(
        CONN_TIMEOUT > PAIRING_HARD_TIMEOUT,
        "CONN_TIMEOUT ({CONN_TIMEOUT:?}) must exceed desktop PAIRING_HARD_TIMEOUT ({PAIRING_HARD_TIMEOUT:?})"
    );
}

/// Drives the production pairing task against a controlled WebSocket peer.
/// Setup consumes 11 seconds, so moving `hard_timeout` below NIP-42/EOSE would
/// make the desktop deadline 141 seconds and let the simulated 140-second
/// relay close win instead.
#[tokio::test(start_paused = true)]
async fn desktop_timeout_starts_when_websocket_connects() {
    let relay_url = "ws://pairing.test".to_string();
    let (desktop_io, peer_io) = tokio::io::duplex(4096);
    let desktop_ws = WebSocketStream::from_raw_socket(desktop_io, Role::Client, None).await;
    let mut peer = WebSocketStream::from_raw_socket(peer_io, Role::Server, None).await;

    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();
    let (error_tx, mut error_rx) = mpsc::unbounded_channel();
    app_handle.listen("pairing-error", move |event| {
        let _ = error_tx.send(event.payload().to_string());
    });

    let (session, _) = PairingSession::new_source(relay_url.clone());
    let session = Arc::new(tokio::sync::Mutex::new(Some(session)));
    let generation = Arc::new(AtomicU64::new(1));
    let context = PairingTaskContext {
        mode: PairingMode::SendIdentity,
        generation,
        generation_fence: Arc::new(Mutex::new(())),
        task_generation: 1,
    };
    let (outbound_tx, outbound_rx) = mpsc::channel(1);
    let desktop = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        pairing_ws_task_on_socket(
            desktop_ws,
            &relay_url,
            &session,
            &context,
            &cancel,
            outbound_rx,
            &app_handle,
        )
        .await
    });

    // With no NIP-42 challenge, Desktop spends its three-second challenge
    // budget before proceeding to the subscription.
    tokio::time::advance(Duration::from_secs(3)).await;
    let req = peer.next().await.unwrap().unwrap();
    assert!(
        matches!(req, Message::Text(ref text) if text.contains("\"REQ\"") && text.contains("\"pair\"")),
        "expected desktop pairing subscription, got {req:?}"
    );

    tokio::time::advance(Duration::from_secs(8)).await;
    peer.send(Message::Text(r#"["EOSE","pair"]"#.into()))
        .await
        .unwrap();
    outbound_tx.send("test-ready".into()).await.unwrap();
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    let ready = match peer.next().await {
        Some(Ok(message)) => message,
        other => panic!(
            "desktop closed before entering post-EOSE loop: {other:?}; emitted={:?}",
            error_rx.try_recv()
        ),
    };
    assert!(
        matches!(ready, Message::Text(ref text) if text.as_str() == "test-ready"),
        "expected post-EOSE desktop message, got {ready:?}"
    );

    // 129 seconds from connect: the desktop remains active and no expiry was
    // emitted early.
    tokio::time::advance(Duration::from_secs(118)).await;
    tokio::task::yield_now().await;
    assert!(!desktop.is_finished());
    assert!(error_rx.try_recv().is_err());

    // Exactly 130 seconds from connect: the desktop must own expiry, ten
    // seconds before the controlled peer's relay-style close.
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    let expired_by_130 = desktop.is_finished();
    let timeout_payload = error_rx.try_recv().ok();

    // Simulate the relay's independent orphan-socket deadline. If the desktop
    // timer were started after the 11-second setup, this close would win at
    // 140 seconds and surface "relay connection closed" instead.
    tokio::time::advance(CONN_TIMEOUT - PAIRING_HARD_TIMEOUT).await;
    let _ = peer.close(None).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    let desktop_result = desktop.await.unwrap();

    assert!(
        expired_by_130,
        "pairing task did not expire 130 seconds after WebSocket establishment"
    );
    assert_eq!(desktop_result, Ok(()));
    let payload = match timeout_payload {
        Some(payload) => payload,
        None => error_rx
            .try_recv()
            .expect("pairing task did not emit an error before relay close"),
    };
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["message"], "Session timed out");
}

/// If the peer stops reading after the WebSocket handshake, a subscribe write
/// must not block past the desktop hard deadline.
#[tokio::test(start_paused = true)]
async fn stalled_subscribe_write_emits_session_timeout() {
    let relay_url = "ws://pairing.test".to_string();
    let (desktop_io, peer_io) = tokio::io::duplex(1);
    let desktop_ws = WebSocketStream::from_raw_socket(desktop_io, Role::Client, None).await;
    let _peer = WebSocketStream::from_raw_socket(peer_io, Role::Server, None).await;

    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();
    let (error_tx, mut error_rx) = mpsc::unbounded_channel();
    app_handle.listen("pairing-error", move |event| {
        let _ = error_tx.send(event.payload().to_string());
    });

    let (session, _) = PairingSession::new_source(relay_url.clone());
    let session = Arc::new(tokio::sync::Mutex::new(Some(session)));
    let generation = Arc::new(AtomicU64::new(1));
    let context = PairingTaskContext {
        mode: PairingMode::SendIdentity,
        generation,
        generation_fence: Arc::new(Mutex::new(())),
        task_generation: 1,
    };
    let (_outbound_tx, outbound_rx) = mpsc::channel(1);
    let cancel = CancellationToken::new();
    let desktop = tokio::spawn(async move {
        pairing_ws_task_on_socket(
            desktop_ws,
            &relay_url,
            &session,
            &context,
            &cancel,
            outbound_rx,
            &app_handle,
        )
        .await
    });

    // Let the pairing deadline wrapper start before advancing virtual time.
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }

    for _ in 0..PAIRING_HARD_TIMEOUT.as_secs() {
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
    }
    for _ in 0..64 {
        if desktop.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }

    if !desktop.is_finished() {
        desktop.abort();
        panic!(
            "pairing task did not finish within {PAIRING_HARD_TIMEOUT:?} after stalled subscribe write"
        );
    }
    let desktop_result = desktop.await.unwrap();
    assert_eq!(desktop_result, Ok(()));
    let payload: Value = serde_json::from_str(
        &error_rx
            .try_recv()
            .expect("expected Session timed out after stalled subscribe write"),
    )
    .unwrap();
    assert_eq!(payload["message"], "Session timed out");
}
