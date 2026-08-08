use super::{
    describe_connect_failure, pairing_relay_from_nip11, probe_pairing_relay,
    resolve_pairing_relay_url, PairingRelay,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// Build the transport error tungstenite hands back when the upgrade request
/// is answered with an HTTP status instead of a 101.
fn http_error(status: u16) -> tokio_tungstenite::tungstenite::Error {
    let response = tokio_tungstenite::tungstenite::http::Response::builder()
        .status(status)
        .body(None)
        .expect("build HTTP error response");
    tokio_tungstenite::tungstenite::Error::Http(Box::new(response))
}

#[test]
fn legacy_path_404_explains_the_missing_pairing_endpoint() {
    let message = describe_connect_failure(
        &PairingRelay::LegacyPath,
        "wss://relay.example.com/pair",
        &http_error(404),
    );

    // The URL that was actually dialled, because the old message named none.
    assert!(message.contains("wss://relay.example.com/pair"));
    // Why this URL was chosen at all.
    assert!(message.contains("NIP-43"));
    assert!(message.contains("pairing_relay_url"));
    // Both levers that fix it: neither was discoverable without a source dive.
    assert!(message.contains("BUZZ_PAIRING_RELAY_URL"));
    assert!(message.contains("buzz-pair-relay"));
}

#[test]
fn legacy_path_non_404_keeps_the_transport_error_and_gains_the_url() {
    // A routed /pair whose sidecar is down answers 502, which is a different
    // problem with a different fix, so it must not claim nothing is serving
    // the path.
    let message = describe_connect_failure(
        &PairingRelay::LegacyPath,
        "wss://relay.example.com/pair",
        &http_error(502),
    );

    assert!(message.contains("wss://relay.example.com/pair"));
    assert!(!message.contains("BUZZ_PAIRING_RELAY_URL"));
}

#[test]
fn configured_pairing_relay_404_reports_the_url_it_was_given() {
    // The split-domain case: the operator advertised a pairing_relay_url and
    // it is wrong. Naming it is the whole fix; guessing at /pair advice here
    // would point at a file that has nothing to do with the failure.
    let message = describe_connect_failure(
        &PairingRelay::Configured("wss://pairing.example.com".to_string()),
        "wss://pairing.example.com",
        &http_error(404),
    );

    assert!(message.contains("wss://pairing.example.com"));
    assert!(!message.contains("BUZZ_PAIRING_RELAY_URL"));
}

#[test]
fn main_relay_failure_keeps_the_transport_error() {
    let message = describe_connect_failure(
        &PairingRelay::MainRelay,
        "wss://relay.example.com",
        &http_error(500),
    );

    assert!(message.starts_with("WebSocket connection failed:"));
    assert!(message.contains("wss://relay.example.com"));
}
