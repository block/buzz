use super::{
    pairing_relay_from_nip11, probe_pairing_relay, resolve_pairing_relay_url, PairingRelay,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve_nip11_document(
    body: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
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

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/nostr+json\r\nContent-Length: \
             {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });
    (addr, server)
}

#[tokio::test]
async fn live_nip11_probe_discovers_configured_pairing_relay() {
    let (addr, server) =
        serve_nip11_document(r#"{"pairing_relay_url":"ws://127.0.0.1:5000"}"#).await;

    assert_eq!(
        probe_pairing_relay(&format!("ws://{addr}"))
            .await
            .expect("discover configured pairing relay"),
        PairingRelay::Configured("ws://127.0.0.1:5000".to_string())
    );
    server.await.expect("NIP-11 server task");
}

#[tokio::test]
async fn live_nip11_probe_rejects_membership_relay_without_pairing_url() {
    let (addr, server) = serve_nip11_document(r#"{"supported_nips":[1,11,43]}"#).await;

    let error = probe_pairing_relay(&format!("ws://{addr}"))
        .await
        .expect_err("NIP-43 alone must not invent a /pair endpoint");
    assert!(error.contains("membership-gated relay"));
    assert!(error.contains("buzz-pair-relay"));
    assert!(error.contains("BUZZ_PAIRING_RELAY_URL"));
    server.await.expect("NIP-11 server task");
}

#[test]
fn configured_pairing_relay_takes_precedence_over_nip43_membership() {
    let document = serde_json::json!({
        "pairing_relay_url": "wss://pairing.buzz.xyz",
        "supported_nips": [43]
    });

    assert_eq!(
        pairing_relay_from_nip11(&document).expect("valid pairing relay configuration"),
        PairingRelay::Configured("wss://pairing.buzz.xyz".to_string())
    );
}

#[test]
fn invalid_pairing_relay_url_returns_actionable_configuration_error() {
    for value in ["https://pairing.buzz.xyz", "ws://"] {
        let document = serde_json::json!({
            "pairing_relay_url": value,
            "supported_nips": [43]
        });

        let error = pairing_relay_from_nip11(&document)
            .expect_err("invalid explicit pairing URL must be rejected");
        assert!(error.contains("valid ws:// or wss:// URL with a host"));
        assert!(error.contains("BUZZ_PAIRING_RELAY_URL"));
    }
}

#[test]
fn document_without_pairing_configuration_uses_main_relay() {
    let document = serde_json::json!({ "supported_nips": [1, 11] });

    assert_eq!(
        pairing_relay_from_nip11(&document).expect("non-NIP-43 main relay fallback"),
        PairingRelay::MainRelay
    );
}

#[test]
fn configured_pairing_relay_resolves_to_configured_url() {
    let resolved = resolve_pairing_relay_url(
        "wss://flint.communities.buzz.xyz",
        PairingRelay::Configured("wss://pairing.buzz.xyz".to_string()),
    );

    assert_eq!(resolved, "wss://pairing.buzz.xyz");
}

#[test]
fn same_host_pair_path_is_preserved_when_explicitly_advertised() {
    let document = serde_json::json!({
        "pairing_relay_url": "wss://flint.communities.buzz.xyz/pair",
        "supported_nips": [43]
    });
    let discovered = pairing_relay_from_nip11(&document).expect("explicit same-host pairing relay");
    let resolved = resolve_pairing_relay_url("wss://flint.communities.buzz.xyz", discovered);

    assert_eq!(resolved, "wss://flint.communities.buzz.xyz/pair");
}

#[test]
fn main_relay_pairing_uses_main_relay_url() {
    let resolved = resolve_pairing_relay_url(
        "wss://sprout-oss.stage.blox.sqprod.co",
        PairingRelay::MainRelay,
    );

    assert_eq!(resolved, "wss://sprout-oss.stage.blox.sqprod.co");
}
