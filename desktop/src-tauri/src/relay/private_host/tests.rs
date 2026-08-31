use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    loop {
        let mut buf = [0; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let len: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + len {
                return (headers, bytes[end + 4..end + 4 + len].to_vec());
            }
        }
    }
}

fn state_and_keys() -> (AppState, Keys) {
    let state = crate::app_state::build_app_state();
    let keys = Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    (state, keys)
}

#[tokio::test]
async fn private_host_redirects_never_contact_target_or_disclose_filters() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for status in [307, 308] {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", origin.local_addr().unwrap());
        let target_url = format!("http://{}/private-leak", target.local_addr().unwrap());
        let (state, keys) = state_and_keys();
        let filters =
            [serde_json::json!({"kinds":[50002],"#p":[keys.public_key().to_hex()],"limit":1000})];
        let expected_body = serde_json::to_vec(&filters).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.unwrap();
            let (_, body) = read_request(&mut stream).await;
            assert_eq!(body, expected_body);
            stream.write_all(format!("HTTP/1.1 {status} Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            query_private_host_at_with_keys(&state, &base, &filters, &keys, None),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().contains(&status.to_string()));
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), target.accept())
                .await
                .is_err(),
            "redirect target must receive no request, including no filter body"
        );
    }
}

#[tokio::test]
async fn private_host_configured_auth_is_bound_to_exact_url_and_body() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", origin.local_addr().unwrap());
    let url = format!("{base}/query");
    let (state, keys) = state_and_keys();
    let signer = keys.public_key();
    let server = tokio::spawn(async move {
        let (mut stream, _) = origin.accept().await.unwrap();
        let (headers, body) = read_request(&mut stream).await;
        let header = |name: &str| {
            headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case(name).then(|| value.trim())
                })
                .unwrap()
        };
        assert_eq!(header("x-auth-tag"), "synthetic-configured-auth");
        let encoded = header("authorization").strip_prefix("Nostr ").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let auth = nostr::Event::from_json(bytes).unwrap();
        buzz_core_pkg::verify_event(&auth).unwrap();
        assert_eq!(auth.pubkey, signer);
        assert!(auth.tags.iter().any(|t| t.as_slice() == ["u", &url]));
        assert!(auth.tags.iter().any(|t| t.as_slice() == ["method", "POST"]));
        assert!(auth
            .tags
            .iter()
            .any(|t| t.as_slice() == ["payload", &hex::encode(Sha256::digest(&body))]));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await.unwrap();
    });
    assert!(query_private_host_at_with_keys(
        &state,
        &base,
        &[serde_json::json!({"kinds":[50000],"#p":[signer.to_hex()]})],
        &keys,
        Some("synthetic-configured-auth")
    )
    .await
    .unwrap()
    .is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn private_host_deadline_covers_stalled_headers_and_body() {
    // Exercise the actual 15s policy (no test-only shorter timeout).
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    assert_eq!(PRIVATE_HOST_REQUEST_TIMEOUT, Duration::from_secs(15));
    for send_headers in [false, true] {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", origin.local_addr().unwrap());
        let (state, keys) = state_and_keys();
        let server = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.unwrap();
            read_request(&mut stream).await;
            if send_headers {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 50\r\n\r\n[").await.unwrap();
            }
            std::future::pending::<()>().await;
        });
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            query_private_host_at_with_keys(
                &state,
                &base,
                &[serde_json::json!({"kinds":[50002]})],
                &keys,
                None,
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), "relay unreachable: request timed out");
        assert!(start.elapsed() >= Duration::from_secs(14));
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn private_host_locked_or_changed_identity_has_no_egress() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", origin.local_addr().unwrap());
    let (state, keys) = state_and_keys();
    for gate in [&state.keyring_locked, &state.identity_lost] {
        gate.store(true, std::sync::atomic::Ordering::Release);
        assert!(
            query_private_host_at_with_keys(&state, &base, &[], &keys, None)
                .await
                .is_err()
        );
        gate.store(false, std::sync::atomic::Ordering::Release);
    }
    assert!(
        query_private_host_at_with_keys(&state, &base, &[], &Keys::generate(), None)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), origin.accept())
            .await
            .is_err()
    );
}
