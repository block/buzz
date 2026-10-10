use super::{
    pairing_connection_error, pairing_relay_from_nip11, probe_pairing_relay,
    resolve_pairing_relay_url, PairingEndpointSource, PairingRelay,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{http::Response, Error as WebSocketError};

fn websocket_http_error(status: u16) -> WebSocketError {
    WebSocketError::Http(Box::new(
        Response::builder()
            .status(status)
            .body(None)
            .expect("build test HTTP response"),
    ))
}

#[tokio::test]
async fn live_nip11_probe_discovers_configured_pairing_relay() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test NIP-11 server");
    let addr = listener.local_addr().expect("test server address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept NIP-11 request");
        let mut request = vec![0; 2048];
        let bytes_read = stream.read(&mut request).await.expect("read request");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("GET / HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept: application/nostr+json"));

        let body = r#"{"pairing_relay_url":"ws://127.0.0.1:5000"}"#;
        let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/nostr+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    assert_eq!(
        probe_pairing_relay(&format!("ws://{addr}")).await,
        PairingRelay::Configured("ws://127.0.0.1:5000".to_string())
    );
    server.await.expect("NIP-11 server task");
}

#[test]
fn configured_pairing_relay_takes_precedence_over_legacy_path() {
    let document = serde_json::json!({
        "pairing_relay_url": "wss://pairing.buzz.xyz",
        "supported_nips": [43]
    });

    assert_eq!(
        pairing_relay_from_nip11(&document),
        PairingRelay::Configured("wss://pairing.buzz.xyz".to_string())
    );
}

#[test]
fn invalid_pairing_relay_url_falls_back_to_legacy_path() {
    let document = serde_json::json!({
        "pairing_relay_url": "https://pairing.buzz.xyz",
        "supported_nips": [43]
    });

    assert_eq!(
        pairing_relay_from_nip11(&document),
        PairingRelay::LegacyPath
    );
}

#[test]
fn document_without_pairing_configuration_uses_main_relay() {
    let document = serde_json::json!({ "supported_nips": [1, 11] });

    assert_eq!(pairing_relay_from_nip11(&document), PairingRelay::MainRelay);
}

#[test]
fn configured_pairing_relay_resolves_to_configured_url() {
    let resolved = resolve_pairing_relay_url(
        "wss://flint.communities.buzz.xyz",
        PairingRelay::Configured("wss://pairing.buzz.xyz".to_string()),
    )
    .expect("resolve configured pairing relay");

    assert_eq!(resolved, "wss://pairing.buzz.xyz");
}

#[test]
fn legacy_pairing_relay_appends_pair_path() {
    let resolved = resolve_pairing_relay_url(
        "wss://flint.communities.buzz.xyz/community",
        PairingRelay::LegacyPath,
    )
    .expect("resolve legacy pairing relay");

    assert_eq!(resolved, "wss://flint.communities.buzz.xyz/community/pair");
}

#[test]
fn main_relay_pairing_uses_main_relay_url() {
    let resolved = resolve_pairing_relay_url(
        "wss://sprout-oss.stage.blox.sqprod.co",
        PairingRelay::MainRelay,
    )
    .expect("resolve main pairing relay");

    assert_eq!(resolved, "wss://sprout-oss.stage.blox.sqprod.co");
}

#[test]
fn legacy_pairing_404_explains_the_missing_pairing_service() {
    let error = websocket_http_error(404);
    let message = pairing_connection_error(
        "wss://relay.example.com/pair?token=do-not-display#fragment",
        PairingEndpointSource::LegacyPath,
        &error,
    );

    assert_eq!(
        message,
        "Pairing endpoint wss://relay.example.com/pair returned 404. This relay advertises \
         NIP-43 without a pairing_relay_url, but nothing serves /pair. Set \
         BUZZ_PAIRING_RELAY_URL or route /pair to buzz-pair-relay, then try again."
    );
    assert!(!message.contains("do-not-display"));
}

#[test]
fn configured_pairing_error_names_configured_endpoint_and_remedy() {
    let error = websocket_http_error(404);
    let message = pairing_connection_error(
        "wss://pairing.example.com/connect?token=do-not-display#fragment",
        PairingEndpointSource::Configured,
        &error,
    );

    assert_eq!(
        message,
        "Connection to configured pairing endpoint wss://pairing.example.com/connect failed: \
         HTTP error: 404 Not Found. Check the relay's pairing_relay_url and that the pairing \
         service is reachable, then try again."
    );
    assert!(!message.contains("do-not-display"));
    assert!(!message.contains("route /pair"));
}

#[test]
fn main_relay_pairing_404_keeps_the_generic_connection_error() {
    let error = websocket_http_error(404);

    assert_eq!(
        pairing_connection_error(
            "wss://pairing.example.com",
            PairingEndpointSource::MainRelay,
            &error,
        ),
        "WebSocket connection failed: HTTP error: 404 Not Found"
    );
}
