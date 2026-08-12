use super::native_websocket_status::{is_loopback_url, nip11_url, parse_nip11_status_identity};
use super::*;
use crate::client_binding_status_session::CurrentProjection;
use futures_util::FutureExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use url::Url;

use buzz_core_pkg::client_binding_bootstrap::{
    CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_STATUS_SUB_ID,
};
use tauri::ipc::InvokeResponseBody;
use tokio::io::duplex;
use tokio_tungstenite::{tungstenite::protocol::Role, WebSocketStream};

fn silent_channel() -> Channel<serde_json::Value> {
    Channel::new(|_: InvokeResponseBody| Ok(()))
}

#[tokio::test]
async fn secure_websocket_reaches_tls_without_panicking() {
    install_crypto_provider();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let result =
        std::panic::AssertUnwindSafe(tokio_tungstenite::connect_async(format!("wss://{address}")))
            .catch_unwind()
            .await;

    assert!(result.is_ok(), "TLS setup must not panic");
    server.await.unwrap();
}

#[tokio::test]
async fn live_tcp_server_connect_send_and_disconnect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (received_tx, received_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let message = socket.next().await.unwrap().unwrap();
        received_tx.send(message).unwrap();
        while let Some(message) = socket.next().await {
            if matches!(message, Ok(Message::Close(_))) {
                break;
            }
        }
    });

    let manager = WebSocketManager::default();
    let id = open_connection(&manager, &format!("ws://{address}"), silent_channel())
        .await
        .unwrap();
    send_message(&manager, id, WebSocketMessage::Text("live-probe".into()))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), received_rx)
            .await
            .unwrap()
            .unwrap(),
        Message::Text("live-probe".into())
    );

    manager.disconnect(id).await;
    assert!(!manager.connections.lock().await.contains_key(&id));
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("live server should observe native socket shutdown")
        .unwrap();
}

#[tokio::test]
async fn eof_removes_connection() {
    let manager = WebSocketManager::default();
    let (client_io, server_io) = duplex(1024);
    let (client, server) = tokio::join!(
        WebSocketStream::from_raw_socket(client_io, Role::Client, None),
        WebSocketStream::from_raw_socket(server_io, Role::Server, None),
    );
    let (sender, receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(1, handle.clone());
    let task = tauri::async_runtime::spawn(run_connection(
        1,
        client,
        receiver,
        handle.cancel.clone(),
        silent_channel(),
        manager.clone(),
    ));
    *handle.task.lock().await = Some(task);

    drop(server);
    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.connections.lock().await.contains_key(&1) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("EOF should clean up its native connection ID");
}

#[tokio::test]
async fn disconnect_removes_and_drops_task_before_returning() {
    struct DropGuard(Arc<AtomicBool>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let manager = WebSocketManager::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let task_dropped = dropped.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (sender, _receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(Some(tauri::async_runtime::spawn(async move {
            let _guard = DropGuard(task_dropped);
            ready_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        }))),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(7, handle);
    ready_rx.await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), manager.disconnect(7))
        .await
        .expect("disconnect should abort an unresponsive task");
    assert!(!manager.connections.lock().await.contains_key(&7));
    assert!(dropped.load(Ordering::SeqCst));

    // Repeated teardown is intentionally a no-op.
    manager.disconnect(7).await;
}

#[tokio::test]
async fn teardown_gate_stays_closed_until_tasks_stop() {
    let manager = WebSocketManager::default();
    let gate = manager.connect_cancel.lock().await;
    let (sender, _receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(Some(tauri::async_runtime::spawn(async {
            std::future::pending::<()>().await;
        }))),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(1, handle);
    gate.cancel();
    let handles = {
        let mut connections = manager.connections.lock().await;
        connections
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>()
    };

    let shutdown = futures_util::future::join_all(
        handles.into_iter().map(WebSocketManager::disconnect_handle),
    );
    assert!(manager.connect_cancel.try_lock().is_err());
    shutdown.await;
    drop(gate);
    assert!(manager.connect_cancel.try_lock().is_ok());
}

#[tokio::test]
async fn disconnect_all_cancels_pending_connect_generation() {
    let manager = WebSocketManager::default();
    let pending = manager.current_connect_cancel().await;
    let generation = manager.projection_generation().await;

    manager.disconnect_all().await;

    assert!(pending.is_cancelled());
    assert!(!manager.current_connect_cancel().await.is_cancelled());
    assert_ne!(manager.projection_generation().await, generation);
}

#[tokio::test]
async fn one_connection_does_not_block_another_send_queue() {
    let manager = WebSocketManager::default();
    let (blocked_sender, blocked_receiver) = mpsc::channel(1);
    blocked_sender
        .send(SendRequest {
            message: Message::Text("blocked".into()),
            result: oneshot::channel().0,
        })
        .await
        .unwrap();
    let blocked = Arc::new(ConnectionHandle {
        sender: blocked_sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(1, blocked);

    let (healthy_sender, mut healthy_receiver) = mpsc::channel(1);
    let healthy = Arc::new(ConnectionHandle {
        sender: healthy_sender.clone(),
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(2, healthy);

    let (result, _) = oneshot::channel();
    tokio::time::timeout(
        Duration::from_millis(50),
        healthy_sender.send(SendRequest {
            message: Message::Text("healthy".into()),
            result,
        }),
    )
    .await
    .expect("a full queue on one connection must not block another")
    .unwrap();
    assert!(healthy_receiver.recv().await.is_some());
    drop(blocked_receiver);
}

#[test]
fn nip11_url_accepts_tls_or_loopback_only() {
    assert_eq!(
        nip11_url("wss://relay.example.test/community?view=1")
            .expect("secure relay URL is eligible")
            .as_str(),
        "https://relay.example.test/community?view=1"
    );
    assert_eq!(
        nip11_url("ws://localhost:3000/")
            .expect("localhost relay URL is eligible")
            .as_str(),
        "http://localhost:3000/"
    );
    assert!(nip11_url("ws://127.0.0.1:3000/").is_ok());
    assert!(nip11_url("ws://[::1]:3000/").is_ok());
    assert!(nip11_url("ws://relay.example.test/").is_err());
    assert!(nip11_url("http://localhost:3000/").is_err());

    assert!(is_loopback_url(
        &Url::parse("ws://LOCALHOST:3000/").expect("test URL")
    ));
    assert!(!is_loopback_url(
        &Url::parse("ws://localhost.example.test/").expect("test URL")
    ));
}

#[test]
fn nip11_status_extension_is_optional_and_exactly_bounded() {
    let relay = nostr::Keys::generate().public_key().to_hex();
    let compatible = serde_json::json!({
        "self": relay,
        "supported_extensions": ["buzz-client-binding-status-v1"],
        "client_status": {
            "version": 1,
            "status_kind": 24244,
            "bootstrap_kind": 24245,
            "max_lifetime_seconds": 300,
            "delivery": "authenticated-connection",
            "authoritative": false
        }
    });
    assert!(parse_nip11_status_identity(&serde_json::to_vec(&compatible).unwrap()).is_ok());

    for incompatible in [
        serde_json::json!({"self": relay}),
        serde_json::json!({
            "self": relay,
            "supported_extensions": ["buzz-client-binding-status-v1"],
            "client_status": {
                "version": 1,
                "status_kind": 24244,
                "bootstrap_kind": 24245,
                "max_lifetime_seconds": 301,
                "delivery": "authenticated-connection",
                "authoritative": false
            }
        }),
        serde_json::json!({
            "self": relay,
            "supported_extensions": ["buzz-client-binding-status-v1"],
            "client_status": {
                "version": 1,
                "status_kind": 24244,
                "bootstrap_kind": 24245,
                "max_lifetime_seconds": 300,
                "delivery": "authenticated-connection",
                "authoritative": true
            }
        }),
    ] {
        assert!(parse_nip11_status_identity(&serde_json::to_vec(&incompatible).unwrap()).is_err());
    }
}

#[test]
fn current_projection_ipc_is_exactly_epoch_free() {
    let projection = CurrentProjection {
        event_author_pubkey: "11".repeat(32),
        fresh_until: 1_234_567_890,
    };

    let serialized = serde_json::to_value(projection).expect("projection serializes");
    assert_eq!(
        serialized,
        serde_json::json!({
            "eventAuthorPubkey": "11".repeat(32),
            "freshUntil": 1_234_567_890_u64,
        })
    );
    assert!(serialized.get("connectionEpoch").is_none());
    assert!(serialized.get("connection_epoch").is_none());
}

fn test_epoch(suffix: u8) -> ClientBindingEpoch {
    ClientBindingEpoch::parse(&format!("11111111-1111-4111-8111-{suffix:012x}"))
        .expect("synthetic epoch")
}

fn test_handle(status_scope: Option<StatusScope>) -> Arc<ConnectionHandle> {
    let (sender, _receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(status_scope),
        fold_pause: None,
    })
}

fn test_status_scope(
    generation: u64,
    attempt: u64,
    relay: PublicKey,
    author: PublicKey,
    epoch: ClientBindingEpoch,
) -> StatusScope {
    StatusScope {
        relay_url: "ws://localhost:3000/".to_string(),
        relay_signer: relay,
        expected_author: author,
        epoch,
        projection_channel: silent_channel(),
        generation,
        attempt,
        challenge: None,
        auth_proven: false,
    }
}

#[tokio::test]
async fn only_proven_status_socket_owns_projection_and_old_handle_is_fenced() {
    let manager = WebSocketManager::default();
    let relay = nostr::Keys::generate().public_key();
    let author = nostr::Keys::generate().public_key();
    let (generation, old_attempt) = manager.begin_status_attempt().await.unwrap();
    let old_epoch = test_epoch(0x11);
    let old = test_handle(Some(test_status_scope(
        generation,
        old_attempt,
        relay,
        author,
        old_epoch.clone(),
    )));
    manager.connections.lock().await.insert(7, old.clone());
    manager
        .record_status_challenge(7, &old, "challenge-old")
        .await;
    let old_proof = manager
        .status_auth_proof(7, "challenge-old", "ws://localhost:3000/", author)
        .await
        .expect("exact native proof");

    assert!(manager
        .status_auth_proof(7, "challenge-old", "ws://wrong/", author)
        .await
        .is_err());

    let (new_generation, new_attempt) = manager.begin_status_attempt().await.unwrap();
    assert_eq!(new_generation, generation);
    assert!(new_attempt > old_attempt);
    let new_epoch = test_epoch(0x22);
    let new = test_handle(Some(test_status_scope(
        new_generation,
        new_attempt,
        relay,
        author,
        new_epoch.clone(),
    )));
    manager.connections.lock().await.insert(8, new.clone());
    manager
        .record_status_challenge(8, &new, "challenge-new")
        .await;
    let new_proof = manager
        .status_auth_proof(8, "challenge-new", "ws://localhost:3000/", author)
        .await
        .expect("replacement proof");
    manager
        .complete_status_auth(8, &new_proof)
        .await
        .expect("replacement owns projection");

    // An older eligible attempt cannot replace a newer owner even when its
    // exact proof was captured before the newer attempt completed.
    assert!(manager.complete_status_auth(7, &old_proof).await.is_err());

    let current = CurrentProjection {
        event_author_pubkey: author.to_hex(),
        fresh_until: unix_now() + 60,
    };
    manager
        .apply_projection_update(
            8,
            &new,
            &new_epoch,
            ProjectionUpdate::Current(current.clone()),
        )
        .await;
    assert_eq!(
        manager.projection.lock().await.current,
        Some(current.clone())
    );

    manager.clear_projection_if_owner(7, &old, &old_epoch).await;
    manager
        .apply_projection_update(
            7,
            &old,
            &old_epoch,
            ProjectionUpdate::Current(CurrentProjection {
                event_author_pubkey: "11".repeat(32),
                fresh_until: u64::MAX,
            }),
        )
        .await;
    assert_eq!(manager.projection.lock().await.current, Some(current));

    let read_only = test_handle(None);
    manager
        .connections
        .lock()
        .await
        .insert(9, read_only.clone());
    assert!(manager
        .status_auth_proof(9, "challenge", "ws://localhost:3000/", author)
        .await
        .is_err());

    manager.invalidate_projection().await;
    assert!(new.cancel.is_cancelled());
    assert!(manager.projection.lock().await.owner.is_none());
    assert!(manager.complete_status_auth(8, &new_proof).await.is_err());
    manager.suspend_projection().await;
    assert!(manager.status_head().await.is_none());
}

#[tokio::test]
async fn post_activation_rechallenge_revokes_projection_and_old_session() {
    for (id, epoch_suffix, rechallenge) in
        [(20, 0x51, "challenge"), (21, 0x52, "replacement-challenge")]
    {
        let manager = WebSocketManager::default();
        let relay = nostr::Keys::generate().public_key();
        let author = nostr::Keys::generate().public_key();
        let (generation, attempt) = manager.begin_status_attempt().await.unwrap();
        let epoch = test_epoch(epoch_suffix);
        let handle = test_handle(Some(test_status_scope(
            generation,
            attempt,
            relay,
            author,
            epoch.clone(),
        )));
        manager.connections.lock().await.insert(id, handle.clone());
        manager
            .record_status_challenge(id, &handle, "challenge")
            .await;
        let proof = manager
            .status_auth_proof(id, "challenge", "ws://localhost:3000/", author)
            .await
            .expect("initial status proof");
        manager
            .complete_status_auth(id, &proof)
            .await
            .expect("initial status activation");
        manager
            .apply_projection_update(
                id,
                &handle,
                &epoch,
                ProjectionUpdate::Current(CurrentProjection {
                    event_author_pubkey: author.to_hex(),
                    fresh_until: unix_now() + 60,
                }),
            )
            .await;
        assert!(manager.projection.lock().await.current.is_some());

        manager
            .record_status_challenge(id, &handle, rechallenge)
            .await;
        assert!(handle.status_scope.lock().await.is_none());
        assert!(manager.projection.lock().await.owner.is_none());
        assert!(manager.projection.lock().await.current.is_none());
        assert!(manager
            .status_auth_proof(id, rechallenge, "ws://localhost:3000/", author)
            .await
            .is_err());

        manager
            .apply_projection_update(
                id,
                &handle,
                &epoch,
                ProjectionUpdate::Current(CurrentProjection {
                    event_author_pubkey: author.to_hex(),
                    fresh_until: u64::MAX,
                }),
            )
            .await;
        assert!(manager.projection.lock().await.current.is_none());
    }
}

#[tokio::test]
async fn changed_pre_activation_challenge_removes_status_capability() {
    let manager = WebSocketManager::default();
    let relay = nostr::Keys::generate().public_key();
    let author = nostr::Keys::generate().public_key();
    let (generation, attempt) = manager.begin_status_attempt().await.unwrap();
    let handle = test_handle(Some(test_status_scope(
        generation,
        attempt,
        relay,
        author,
        test_epoch(0x53),
    )));
    manager.connections.lock().await.insert(22, handle.clone());
    manager.record_status_challenge(22, &handle, "first").await;
    manager
        .record_status_challenge(22, &handle, "changed")
        .await;

    assert!(handle.status_scope.lock().await.is_none());
    assert!(manager.projection.lock().await.owner.is_none());
    assert!(manager
        .status_auth_proof(22, "changed", "ws://localhost:3000/", author)
        .await
        .is_err());
}

#[tokio::test]
async fn overlapping_scope_mutations_keep_status_fail_closed() {
    let manager = WebSocketManager::default();
    let relay = nostr::Keys::generate().public_key();
    let author = nostr::Keys::generate().public_key();
    let (generation, attempt) = manager.begin_status_attempt().await.unwrap();
    let epoch = test_epoch(0x33);
    let handle = test_handle(Some(test_status_scope(
        generation, attempt, relay, author, epoch,
    )));
    manager.connections.lock().await.insert(10, handle.clone());
    manager
        .record_status_challenge(10, &handle, "challenge")
        .await;
    let proof = manager
        .status_auth_proof(10, "challenge", "ws://localhost:3000/", author)
        .await
        .expect("pre-mutation proof");

    let first_mutation = manager.begin_scope_mutation().await.unwrap();
    assert!(manager.status_head().await.is_none());
    assert!(handle.cancel.is_cancelled());
    let second_mutation = manager.begin_scope_mutation().await.unwrap();
    second_mutation.finish().await;
    assert!(manager.status_head().await.is_none());

    first_mutation.finish().await;
    assert!(manager.status_head().await.is_some());
    assert!(manager.complete_status_auth(10, &proof).await.is_err());
}

#[tokio::test]
async fn cancelled_scope_mutation_releases_its_exact_guard() {
    let manager = WebSocketManager::default();
    let task_manager = manager.clone();
    let (entered_tx, entered_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _mutation = task_manager.begin_scope_mutation().await.unwrap();
        entered_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    entered_rx.await.unwrap();
    assert!(manager.status_head().await.is_none());
    assert_eq!(manager.projection.lock().await.active_mutations.len(), 1);

    task.abort();
    assert!(task.await.is_err());
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if manager.status_head().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropped mutation guard releases without stranding status");
    assert!(manager.projection.lock().await.active_mutations.is_empty());
}

#[tokio::test]
async fn security_token_exhaustion_fails_closed() {
    let manager = WebSocketManager::default();
    manager.projection.lock().await.attempt_head = u64::MAX;
    assert!(manager.begin_status_attempt().await.is_none());
    assert!(manager.projection.lock().await.suspended);

    let manager = WebSocketManager::default();
    manager.projection.lock().await.generation = u64::MAX;
    manager.invalidate_projection().await;
    let projection = manager.projection.lock().await;
    assert!(projection.suspended);
    assert_eq!(projection.generation, u64::MAX);
    drop(projection);

    let manager = WebSocketManager::default();
    let handle = test_handle(Some(test_status_scope(
        0,
        1,
        nostr::Keys::generate().public_key(),
        nostr::Keys::generate().public_key(),
        test_epoch(0x33),
    )));
    manager.connections.lock().await.insert(12, handle.clone());
    {
        let mut projection = manager.projection.lock().await;
        projection.mutation_head = u64::MAX;
        projection.owner = Some(ProjectionOwner {
            id: 12,
            handle: handle.clone(),
            epoch: test_epoch(0x33),
            generation: 0,
            attempt: 1,
            presentation_token: 0,
            channel: silent_channel(),
        });
        projection.current = Some(CurrentProjection {
            event_author_pubkey: "11".repeat(32),
            fresh_until: unix_now() + 60,
        });
    }
    assert!(manager.begin_scope_mutation().await.is_err());
    let projection = manager.projection.lock().await;
    assert!(projection.suspended);
    assert!(projection.owner.is_none());
    assert!(projection.current.is_none());
    drop(projection);
    assert!(handle.cancel.is_cancelled());
    assert!(handle.status_scope.lock().await.is_none());

    let manager = WebSocketManager::default();
    let handle = test_handle(None);
    let epoch = test_epoch(0x34);
    let current = CurrentProjection {
        event_author_pubkey: "11".repeat(32),
        fresh_until: unix_now() + 60,
    };
    {
        let mut projection = manager.projection.lock().await;
        projection.owner = Some(ProjectionOwner {
            id: 12,
            handle: handle.clone(),
            epoch: epoch.clone(),
            generation: 0,
            attempt: 1,
            presentation_token: u64::MAX,
            channel: silent_channel(),
        });
        projection.current = Some(current.clone());
    }
    manager
        .apply_projection_update(12, &handle, &epoch, ProjectionUpdate::Current(current))
        .await;
    let projection = manager.projection.lock().await;
    assert!(projection.owner.is_none());
    assert!(projection.current.is_none());
}

#[tokio::test]
async fn early_monotonic_expiry_rechecks_wall_clock_and_rearms() {
    let manager = WebSocketManager::default();
    let handle = test_handle(None);
    let epoch = test_epoch(0x35);
    let fresh_until = unix_now() + 60;
    {
        let mut projection = manager.projection.lock().await;
        projection.owner = Some(ProjectionOwner {
            id: 13,
            handle: handle.clone(),
            epoch: epoch.clone(),
            generation: 0,
            attempt: 2,
            presentation_token: 3,
            channel: silent_channel(),
        });
        projection.current = Some(CurrentProjection {
            event_author_pubkey: "22".repeat(32),
            fresh_until,
        });
    }

    assert!(unix_now() < fresh_until, "test models a backward clock");
    let rearmed = manager
        .expire_projection_if_owner(13, &handle, &epoch, (0, 2, 3), fresh_until)
        .await;
    assert!(rearmed.is_some());
    assert!(manager.projection.lock().await.current.is_some());
}

#[tokio::test]
async fn status_expiry_sleep_keeps_deadline_across_delayed_first_poll() {
    let deadline = monotonic_deadline_after(Duration::from_millis(1));
    tokio::time::sleep(Duration::from_millis(10)).await;

    let sleep = status_expiry_sleep(deadline);
    assert_eq!(sleep.deadline(), deadline);
    sleep.await;

    let overflow_deadline = monotonic_deadline_after(Duration::MAX);
    assert!(overflow_deadline <= tokio::time::Instant::now());
}

#[tokio::test]
async fn projection_expires_while_reserved_fold_is_blocked() {
    let manager = WebSocketManager::default();
    let relay = nostr::Keys::generate().public_key();
    let author = nostr::Keys::generate().public_key();
    let (generation, attempt) = manager.begin_status_attempt().await.unwrap();
    let epoch = test_epoch(0x44);
    let pause = Arc::new(TestFoldPause::new());
    let (sender, receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(Some(test_status_scope(
            generation,
            attempt,
            relay,
            author,
            epoch.clone(),
        ))),
        fold_pause: Some(pause.clone()),
    });
    manager.connections.lock().await.insert(11, handle.clone());
    manager
        .record_status_challenge(11, &handle, "challenge")
        .await;
    let proof = manager
        .status_auth_proof(11, "challenge", "ws://localhost:3000/", author)
        .await
        .expect("exact native proof");
    manager
        .complete_status_auth(11, &proof)
        .await
        .expect("test socket owns projection");

    let fresh_until = unix_now() + 2;
    manager
        .apply_projection_update(
            11,
            &handle,
            &epoch,
            ProjectionUpdate::Current(CurrentProjection {
                event_author_pubkey: author.to_hex(),
                fresh_until,
            }),
        )
        .await;
    assert!(manager.projection.lock().await.current.is_some());

    let (client_io, server_io) = duplex(4096);
    let (client, mut server) = tokio::join!(
        WebSocketStream::from_raw_socket(client_io, Role::Client, None),
        WebSocketStream::from_raw_socket(server_io, Role::Server, None),
    );
    let task = tauri::async_runtime::spawn(run_connection_inner(
        11,
        client,
        receiver,
        handle.cancel.clone(),
        silent_channel(),
        manager.clone(),
        handle.clone(),
        Some(ClientBindingStatusSession::new(relay, author, epoch)),
    ));
    *handle.task.lock().await = Some(task);
    server
        .send(Message::Text(
            serde_json::json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID, "malformed"])
                .to_string()
                .into(),
        ))
        .await
        .expect("send reserved frame");

    let entered = tokio::time::timeout(Duration::from_secs(1), async {
        while !pause.entered.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let visible_after_entering_fold = manager.projection.lock().await.current.is_some();
    let expired = if entered {
        tokio::time::timeout(Duration::from_secs(3), async {
            while manager.projection.lock().await.current.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    } else {
        false
    };

    pause.release();
    server
        .send(Message::Close(None))
        .await
        .expect("close synthetic socket");
    let task = handle.task.lock().await.take().expect("registered task");
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("connection loop exits")
        .expect("connection task joins");

    assert!(entered, "reserved fold should reach its blocking section");
    assert!(
        visible_after_entering_fold,
        "projection should still be visible when the fold blocks"
    );
    assert!(
        expired,
        "deadline must clear while the fold remains blocked"
    );
    assert!(unix_now() >= fresh_until);
}

#[tokio::test]
async fn stale_task_cannot_remove_reused_connection_id() {
    let manager = WebSocketManager::default();
    let (old_sender, _old_receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let old = Arc::new(ConnectionHandle {
        sender: old_sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    let (new_sender, _new_receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let current = Arc::new(ConnectionHandle {
        sender: new_sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(9, old.clone());
    manager.connections.lock().await.insert(9, current.clone());

    manager.remove_if_current(9, &old).await;
    assert!(manager
        .connections
        .lock()
        .await
        .get(&9)
        .is_some_and(|handle| Arc::ptr_eq(handle, &current)));
    manager.remove_if_current(9, &current).await;
    assert!(!manager.connections.lock().await.contains_key(&9));
}

#[tokio::test]
async fn reserved_text_is_swallowed_but_binary_remains_raw_delivery() {
    let manager = WebSocketManager::default();
    let delivered = Arc::new(AtomicUsize::new(0));
    let delivered_for_channel = delivered.clone();
    let channel = Channel::new(move |_: InvokeResponseBody| {
        delivered_for_channel.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let (client_io, server_io) = duplex(4096);
    let (client, mut server) = tokio::join!(
        WebSocketStream::from_raw_socket(client_io, Role::Client, None),
        WebSocketStream::from_raw_socket(server_io, Role::Server, None),
    );
    let (sender, receiver) = mpsc::channel(SEND_QUEUE_CAPACITY);
    let handle = Arc::new(ConnectionHandle {
        sender,
        cancel: CancellationToken::new(),
        task: Mutex::new(None),
        status_scope: Mutex::new(None),
        fold_pause: None,
    });
    manager.connections.lock().await.insert(42, handle.clone());
    let task = tauri::async_runtime::spawn(run_connection(
        42,
        client,
        receiver,
        handle.cancel.clone(),
        channel,
        manager.clone(),
    ));
    *handle.task.lock().await = Some(task);

    server
        .send(Message::Text(
            serde_json::json!(["EVENT", CLIENT_BINDING_BOOTSTRAP_SUB_ID])
                .to_string()
                .into(),
        ))
        .await
        .expect("send reserved bootstrap frame");
    server
        .send(Message::Binary(
            serde_json::json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID, "malformed"])
                .to_string()
                .into_bytes()
                .into(),
        ))
        .await
        .expect("send reserved status frame");
    server
        .send(Message::Text("ordinary".into()))
        .await
        .expect("send ordinary frame");
    server
        .send(Message::Close(None))
        .await
        .expect("close synthetic socket");

    let task = handle
        .task
        .lock()
        .await
        .take()
        .expect("connection task is registered");
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("connection loop exits")
        .expect("connection task joins");
    assert_eq!(
        delivered.load(Ordering::SeqCst),
        3,
        "binary, ordinary text, and terminal close reach raw delivery"
    );
    assert!(!manager.connections.lock().await.contains_key(&42));
}

#[test]
fn reserved_classifier_never_intercepts_binary() {
    let reserved =
        serde_json::json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID, "malformed"]).to_string();
    assert!(reserved_text_message(&Message::Text(reserved.clone().into())).is_some());
    assert!(reserved_text_message(&Message::Binary(reserved.into_bytes().into())).is_none());
}

#[test]
fn auth_challenge_recording_requires_exact_frame_shape() {
    assert_eq!(
        nip42_challenge(&serde_json::json!(["AUTH", "exact"]).to_string()).as_deref(),
        Some("exact")
    );
    assert!(nip42_challenge(&serde_json::json!(["AUTH", "exact", "extra"]).to_string()).is_none());
    assert!(nip42_challenge("not-json").is_none());
}
