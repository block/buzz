use super::*;

fn response(key: PublicKey, now: u64) -> AdapterResponse {
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"nip-fi+jwt","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "iss":"https://adapter.example", "sub":"opaque-employee", "aud":"buzz",
            "nostr_pubkey":key.to_hex(), "iat":now, "exp":now + 120
        }))
        .unwrap(),
    );
    AdapterResponse {
        assertion: format!("{header}.{claims}.fixture"),
        nostr_pubkey: key.to_hex(),
        expires_at: now + 100,
        agent_credential: None,
    }
}

#[test]
fn adapter_metadata_matches_local_signer_and_effective_deadline() {
    let key = Keys::generate().public_key();
    assert!(response(key, 10).into_assertion(key, 10).is_ok());
    assert!(response(key, 10)
        .into_assertion(Keys::generate().public_key(), 10)
        .is_err());
    let mut token = response(key, 10);
    token.expires_at = 131;
    assert!(token.into_assertion(key, 10).is_err());
    let mut token = response(key, 10);
    token.assertion = token.assertion.replacen(
        &URL_SAFE_NO_PAD.encode(br#"{"typ":"nip-fi+jwt","alg":"ES256"}"#),
        &URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#),
        1,
    );
    assert!(token.into_assertion(key, 10).is_err());
    assert!(response(key, 20).into_assertion(key, 10).is_err());
    assert!(response(key, 10).into_assertion(key, 120).is_err());
}

#[tokio::test]
async fn real_adapter_exchange_has_exact_body_possession_and_no_redirect_forwarding() {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Router,
    };
    let keys = Keys::generate();
    let key = keys.public_key();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let expected = format!("{base}/assertions");
    let app = Router::new().route("/assertions", post(move |headers: HeaderMap, body: String| {
        let expected = expected.clone();
        async move {
            assert_eq!(headers["x-bb-session-credential"], "fixture-session");
            assert!(!headers.contains_key(IDENTITY_HEADER));
            let proof: nostr::Event = serde_json::from_slice(&STANDARD.decode(headers["authorization"].to_str().unwrap().strip_prefix("Nostr ").unwrap()).unwrap()).unwrap();
            proof.verify().unwrap();
            assert_eq!(proof.pubkey, key);
            for tag in [["u", expected.as_str()], ["method", "POST"], ["payload", &hex::encode(Sha256::digest(body.as_bytes()))]] {
                assert!(proof.tags.iter().any(|t| t.as_slice() == tag));
            }
            let token = response(key, unix_now().unwrap());
            axum::Json(serde_json::json!({"assertion":token.assertion,"nostr_pubkey":token.nostr_pubkey,"expires_at":token.expires_at}))
        }
    })).route("/redirect", post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "http://127.0.0.1:9/never")]).into_response() }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let token = exchange(
        &client,
        &format!("{base}/assertions"),
        "fixture-session",
        &keys,
        "wss://relay.example",
        None,
    )
    .await
    .unwrap();
    assert!(token.into_assertion(key, unix_now().unwrap()).is_ok());
    assert!(exchange(
        &client,
        &format!("{base}/redirect"),
        "fixture-session",
        &keys,
        "wss://relay.example",
        None
    )
    .await
    .is_err());
    server.abort();
}

#[tokio::test]
async fn key_specific_sessions_renew_without_extending_existing_socket_lease() {
    let session = std::sync::Arc::new(IdentitySession::new(&["https://relay.example"]).unwrap());
    let human = Keys::generate().public_key();
    let agent = Keys::generate().public_key();
    let now = unix_now().unwrap();
    session
        .install(0, Assertion::new("a.b.c", human, now + 1, now).unwrap())
        .unwrap();
    session
        .install(0, Assertion::new("d.e.f", agent, now + 100, now).unwrap())
        .unwrap();
    assert_eq!(
        session
            .header("https://relay.example/events", agent, now)
            .unwrap()
            .unwrap(),
        "Bearer d.e.f"
    );
    let lease = session.lease_ended("wss://relay.example", human);
    tokio::pin!(lease);
    tokio::select! {
        _ = &mut lease => panic!("premature expiry"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {},
    }
    session
        .install(0, Assertion::new("g.h.i", human, now + 100, now).unwrap())
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut lease)
        .await
        .unwrap();
    session.invalidate().unwrap();
    assert!(session
        .header("https://relay.example/events", agent, now)
        .is_err());
    assert!(session
        .install(0, Assertion::new("a.b.c", human, now + 100, now).unwrap())
        .is_err());
}

#[tokio::test]
async fn socket_wrapper_ends_idle_stream_at_effective_expiry() {
    use futures_util::StreamExt;
    let (client_io, server_io) = tokio::io::duplex(1024);
    let client = tokio_tungstenite::WebSocketStream::from_raw_socket(
        client_io,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let _server = tokio_tungstenite::WebSocketStream::from_raw_socket(
        server_io,
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        None,
    )
    .await;
    let mut socket = crate::identity_socket::IdentitySocket::admitted(
        client,
        Some(unix_now().unwrap()),
        "wss://relay.example".into(),
        Keys::generate(),
    )
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.next())
            .await
            .unwrap()
            .is_none()
    );
}
