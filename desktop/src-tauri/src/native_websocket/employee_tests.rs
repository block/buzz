use super::tests::recording_channel;
use super::*;

#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback's error type.
async fn employee_upgrade_keeps_native_evidence_out_of_ipc_and_closes_on_logout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (observed_tx, observed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut observed = None;
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                let values = request
                    .headers()
                    .get_all("Nostr-Federated-Identity")
                    .iter()
                    .map(|value| value.to_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                observed = Some(values);
                Ok(response)
            },
        )
        .await
        .unwrap();
        observed_tx.send(observed.unwrap()).unwrap();
        socket
            .send(Message::Text("public frame".into()))
            .await
            .unwrap();
        while let Some(message) = socket.next().await {
            if matches!(message, Ok(Message::Close(_))) {
                break;
            }
        }
    });
    let token = CancellationToken::new();
    let mut header = reqwest::header::HeaderValue::from_static("Bearer private.native.assertion");
    header.set_sensitive(true);
    let evidence = crate::builderlab::EmployeeEvidence {
        header,
        cancel: token.clone(),
        expires_at: chrono::Utc::now().timestamp() + 120,
    };
    let manager = WebSocketManager::default();
    let (channel, deliveries) = recording_channel();
    let id = open_connection(
        &manager,
        &format!("ws://{address}"),
        channel,
        Some(evidence),
    )
    .await
    .unwrap();
    assert_eq!(
        observed_rx.await.unwrap(),
        vec!["Bearer private.native.assertion"]
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while deliveries.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    token.cancel();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.connections.lock().await.contains_key(&id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let renderer = deliveries.lock().unwrap().join("");
    assert!(renderer.contains("public frame"));
    assert!(!renderer.contains("private.native.assertion"));
}

#[tokio::test]
async fn employee_relay_reflection_is_refused_before_the_renderer_channel() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket
            .send(Message::Text("reflected private.native.assertion".into()))
            .await
            .unwrap();
        while socket.next().await.is_some() {}
    });
    let evidence = crate::builderlab::EmployeeEvidence {
        header: reqwest::header::HeaderValue::from_static("Bearer private.native.assertion"),
        cancel: CancellationToken::new(),
        expires_at: chrono::Utc::now().timestamp() + 120,
    };
    let manager = WebSocketManager::default();
    let (channel, deliveries) = recording_channel();
    let id = open_connection(
        &manager,
        &format!("ws://{address}"),
        channel,
        Some(evidence),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.connections.lock().await.contains_key(&id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let renderer = deliveries.lock().unwrap().join("");
    assert!(renderer.contains("Invalid employee relay response"));
    assert!(!renderer.contains("private.native.assertion"));
    server.abort();
}
