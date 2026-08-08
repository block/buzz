use super::{
    pairing_relay_from_nip11, probe_pairing_relay, resolve_pairing_relay_url, PairingRelay,
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
fn configured_pairing_relay_takes_precedence_over_membership_gate() {
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
fn membership_relay_without_pairing_url_is_not_pairable() {
    let document = serde_json::json!({
        "pairing_relay_url": "https://pairing.buzz.xyz",
        "supported_nips": [43]
    });

    assert_eq!(
        pairing_relay_from_nip11(&document),
        PairingRelay::MembershipWithoutPairingRelay
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
fn membership_without_pairing_relay_errors_instead_of_guessing_a_path() {
    // Regression: a membership-enforcing relay with no advertised pairing
    // relay must surface an actionable error, never fabricate a `/pair` URL
    // that every current buzz-relay 404s on (the relay serves WS only at `/`).
    let error = resolve_pairing_relay_url(
        "wss://flint.communities.buzz.xyz/community",
        PairingRelay::MembershipWithoutPairingRelay,
    )
    .expect_err("membership relay without a pairing relay is not pairable");

    assert!(
        !error.contains("/pair"),
        "must not fabricate a /pair URL: {error}"
    );
    assert!(
        error.contains("pairing relay"),
        "error should name the missing pairing relay: {error}"
    );
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
