use super::{
    pairing_relay_from_nip11, probe_pairing_relay, resolve_advertised_mobile_relay,
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

#[test]
fn private_mobile_relay_normalizes_tailnet_https_origin() {
    let resolved = resolve_advertised_mobile_relay(
        Some("  https://matthews-macbook-pro-1.tailf29f2c.ts.net  "),
        "ws://localhost:3000",
        "http://localhost:3000",
    )
    .expect("valid tailnet origin");

    assert_eq!(
        resolved.ws_url,
        "wss://matthews-macbook-pro-1.tailf29f2c.ts.net/"
    );
    assert_eq!(
        resolved.http_url,
        "https://matthews-macbook-pro-1.tailf29f2c.ts.net/"
    );
    assert!(resolved.is_private_tailnet);
}

#[test]
fn absent_private_mobile_relay_preserves_workspace_addresses() {
    let resolved =
        resolve_advertised_mobile_relay(None, "ws://localhost:3000", "http://localhost:3000")
            .expect("default workspace relay");

    assert_eq!(resolved.ws_url, "ws://localhost:3000");
    assert_eq!(resolved.http_url, "http://localhost:3000");
    assert!(!resolved.is_private_tailnet);
}

#[test]
fn private_mobile_relay_rejects_non_tailnet_or_non_origin_inputs() {
    for value in [
        "http://matthews-macbook-pro-1.tailf29f2c.ts.net",
        "https://example.com",
        "https://user@matthews-macbook-pro-1.tailf29f2c.ts.net",
        "https://matthews-macbook-pro-1.tailf29f2c.ts.net/path",
        "https://matthews-macbook-pro-1.tailf29f2c.ts.net?query=1",
        "https://matthews-macbook-pro-1.tailf29f2c.ts.net#fragment",
        "https://",
    ] {
        assert!(
            resolve_advertised_mobile_relay(
                Some(value),
                "ws://localhost:3000",
                "http://localhost:3000",
            )
            .is_err(),
            "must reject {value}"
        );
    }
}
