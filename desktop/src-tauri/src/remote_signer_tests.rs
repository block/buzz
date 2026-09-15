use super::*;
use crate::active_user_signer::ActiveUserSigner;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

const CREDENTIAL: &str = "opaque-bbidentity-session-fixture";

fn keys() -> Keys {
    Keys::parse("1000000000000000000000000000000000000000000000000000000000000001").unwrap()
}

fn template() -> EventBuilder {
    EventBuilder::new(Kind::Custom(40002), "hello \"🐝\"\n\\\t")
        .tags([
            Tag::parse(["h", "test-community"]).unwrap(),
            Tag::parse(["p", &keys().public_key().to_hex(), "", "label"]).unwrap(),
            Tag::parse(["x", "duplicate"]).unwrap(),
            Tag::parse(["x", "duplicate"]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(1_700_000_000))
}

fn unsigned() -> UnsignedEvent {
    template().build(keys().public_key())
}

fn signed() -> Value {
    serde_json::to_value(template().sign_with_keys(&keys()).unwrap()).unwrap()
}

struct Request {
    headers: String,
    body: Value,
}

async fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
        assert!(bytes.len() < 8192);
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let len = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(|s| s.parse::<usize>().unwrap())
        })
        .unwrap();
    assert!(len < 600_000);
    let mut body = vec![0; len];
    stream.read_exact(&mut body).await.unwrap();
    Request {
        headers,
        body: serde_json::from_slice(&body).unwrap(),
    }
}

struct Server {
    base: String,
    request: oneshot::Receiver<Request>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(response: String) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, request) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        let _ = tx.send(request);
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    Server {
        base,
        request,
        task,
    }
}

fn response(status: u16, body: impl AsRef<str>) -> String {
    let body = body.as_ref();
    format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

fn signer(base: &str) -> RemoteSigner {
    RemoteSigner::new(
        base,
        RemoteSignerSession::new(CREDENTIAL, keys().public_key()).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn real_http_exact_contract_and_active_user_signer_consistency() {
    for prefix in ["/cash-app/goose", "/api/goose", "/api/goose/"] {
        let expected_event = signed();
        let mut server = server(response(200, expected_event.to_string())).await;
        let remote = Arc::new(signer(&format!("{}{prefix}", server.base)));
        assert_eq!(remote.get_public_key().await.unwrap(), keys().public_key());
        assert_eq!(
            remote.backend(),
            SignerBackend::Custom("bbidentity-http".into())
        );
        let active = ActiveUserSigner::new(remote).await.unwrap();
        assert_eq!(active.public_key(), keys().public_key());
        let event = active.sign_event(template()).await.unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, active.public_key());
        assert_eq!(serde_json::to_value(event).unwrap(), expected_event);
        let request = (&mut server.request).await.unwrap();
        assert!(request.headers.starts_with(&format!(
            "POST {}/v1/buzz/identity/sign HTTP/1.1\r\n",
            prefix.trim_end_matches('/')
        )));
        let headers = request.headers.to_ascii_lowercase();
        assert!(headers.contains(&format!("x-bb-session-credential: {CREDENTIAL}\r\n")));
        assert!(headers.contains("content-type: application/json\r\n"));
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("cookie:"));
        let mut expected = signed();
        for field in ["pubkey", "id", "sig"] {
            expected.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(request.body, expected);
    }
}

#[tokio::test]
async fn rejects_each_changed_template_field_even_when_validly_signed() {
    let original = unsigned();
    for field in ["pubkey", "created_at", "kind", "tags", "content"] {
        let other_keys = Keys::generate();
        let mut changed = original.clone();
        changed.id = None;
        match field {
            "pubkey" => changed.pubkey = other_keys.public_key(),
            "created_at" => changed.created_at = Timestamp::from(1_700_000_001),
            "kind" => changed.kind = Kind::TextNote,
            "tags" => changed.tags = nostr::Tags::new(),
            "content" => changed.content.push('!'),
            _ => unreachable!(),
        }
        let local_keys = keys();
        let event = changed
            .sign_with_keys(if field == "pubkey" {
                &other_keys
            } else {
                &local_keys
            })
            .unwrap();
        event.verify().unwrap();
        let server = server(response(200, event.as_json())).await;
        assert_eq!(
            signer(&server.base).sign(original.clone()).await,
            Err(RemoteSignerError::EventMismatch),
            "{field}"
        );
    }
}

#[tokio::test]
async fn rejects_bad_id_and_bip340_signature_independently() {
    for field in ["id", "sig"] {
        let mut event = signed();
        event[field] = json!("0".repeat(if field == "id" { 64 } else { 128 }));
        let server = server(response(200, event.to_string())).await;
        assert_eq!(
            signer(&server.base).sign(unsigned()).await,
            Err(RemoteSignerError::InvalidSignature),
            "{field}"
        );
    }
}

#[tokio::test]
async fn rejects_malformed_wire_events_including_kind_truncation() {
    let mut bodies = vec![
        "not json".to_string(),
        "{}".to_string(),
        json!({"event": signed()}).to_string(),
    ];
    for (field, value) in [
        ("kind", json!(105538)), // 40002 + 65536 must not wrap to a valid kind
        ("kind", json!(40002.5)),
        ("created_at", json!(-1)),
        ("created_at", json!("1700000000")),
        ("tags", json!([["x", null]])),
        ("tags", json!([[123]])),
        ("content", Value::Null),
        ("id", json!("not-hex")),
        ("sig", json!("00")),
        ("pubkey", json!("bad")),
    ] {
        let mut event = signed();
        event[field] = value;
        bodies.push(event.to_string());
    }
    bodies.push(format!("{} trailing", signed()));
    bodies.push(signed().to_string().replacen("{", "{\"kind\":40002,", 1));
    for body in bodies {
        let server = server(response(200, body.clone())).await;
        assert_eq!(
            signer(&server.base).sign(unsigned()).await,
            Err(RemoteSignerError::MalformedResponse),
            "{body}"
        );
    }
}

#[tokio::test]
async fn classifies_http_and_transport_errors_without_echoing_secrets() {
    for (status, expected) in [
        (401, RemoteSignerError::Authentication(401)),
        (403, RemoteSignerError::Authentication(403)),
        (500, RemoteSignerError::TransientStatus(500)),
        (503, RemoteSignerError::TransientStatus(503)),
        (429, RemoteSignerError::TransientStatus(429)),
        (400, RemoteSignerError::HttpStatus(400)),
    ] {
        let server = server(response(status, CREDENTIAL)).await;
        let signer = signer(&server.base);
        let error = signer.sign(unsigned()).await.unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error} {error:?} {signer:?}").contains(CREDENTIAL));
        assert!(!format!("{signer:?}").contains(&server.base));
    }
    let server = server(String::new()).await;
    assert_eq!(
        signer(&server.base).sign(unsigned()).await,
        Err(RemoteSignerError::Transport)
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(
        signer(&base).sign(unsigned()).await,
        Err(RemoteSignerError::Transport)
    );
}

#[tokio::test]
async fn never_follows_redirects_or_forwards_credential() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for code in [301, 302, 303, 307, 308] {
        let server = server(format!(
            "HTTP/1.1 {code} Redirect\r\nLocation: http://{}/stolen\r\nContent-Length: 0\r\n\r\n",
            destination.local_addr().unwrap()
        ))
        .await;
        assert_eq!(
            signer(&server.base).sign(unsigned()).await,
            Err(RemoteSignerError::HttpStatus(code))
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn no_network_for_wrong_request_author_or_unsupported_crypto() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let signer = signer(&format!("http://{}", listener.local_addr().unwrap()));
    let pk = Keys::generate().public_key();
    assert_eq!(
        signer.sign(template().build(pk)).await,
        Err(RemoteSignerError::EventMismatch)
    );
    let mut invalid_id = unsigned();
    invalid_id.id = Some(nostr::EventId::from_hex(&"00".repeat(32)).unwrap());
    assert_eq!(
        signer.sign(invalid_id).await,
        Err(RemoteSignerError::EventMismatch)
    );
    let unsupported = RemoteSignerError::UnsupportedCapability.to_string();
    assert_eq!(
        signer
            .nip04_encrypt(&pk, "secret")
            .await
            .unwrap_err()
            .to_string(),
        unsupported
    );
    assert_eq!(
        signer
            .nip04_decrypt(&pk, "secret")
            .await
            .unwrap_err()
            .to_string(),
        unsupported
    );
    assert_eq!(signer.get_public_key().await.unwrap(), keys().public_key());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pins_credentials_in_old_signer_when_another_session_is_constructed() {
    let mut server = server(response(200, signed().to_string())).await;
    let old = signer(&server.base);
    let new = RemoteSigner::new(
        &server.base,
        RemoteSignerSession::new("replacement-session", Keys::generate().public_key()).unwrap(),
    )
    .unwrap();
    assert_ne!(
        old.get_public_key().await.unwrap(),
        new.get_public_key().await.unwrap()
    );
    old.sign(unsigned()).await.unwrap();
    let request = (&mut server.request).await.unwrap();
    assert!(request.headers.contains(CREDENTIAL));
    assert!(!request.headers.contains("replacement-session"));
}

#[tokio::test]
async fn bounds_content_length_and_chunked_response_without_event_size_policy() {
    for body in [
        "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".to_string(),
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            20_000,
            "x".repeat(20_000)
        ),
    ] {
        let server = server(body).await;
        assert_eq!(
            signer(&server.base).sign(unsigned()).await,
            Err(RemoteSignerError::ResponseTooLarge)
        );
    }
}

// Hold a real HTTP request open, then witness the client's release of its socket.
async fn hanging_server(send_headers: bool) -> (String, oneshot::Receiver<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, started) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        if send_headers {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
        }
        let _ = tx.send(());
        let mut byte = [0];
        assert_eq!(
            stream.read(&mut byte).await.unwrap(),
            0,
            "request must be released"
        );
    });
    (base, started, task)
}

#[tokio::test]
async fn cancellation_releases_request_before_response_and_during_body() {
    for send_headers in [false, true] {
        let (base, started, server) = hanging_server(send_headers).await;
        let retained = Arc::new(signer(&base));
        let client = Arc::clone(&retained);
        let pending = tokio::spawn(async move { client.sign(unsigned()).await });
        started.await.unwrap();
        // Let the body-pending case reach response consumption before cancellation.
        tokio::time::sleep(Duration::from_millis(20)).await;
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.get_public_key().await.unwrap(),
            keys().public_key()
        );
    }
}

#[tokio::test]
async fn concrete_30_second_deadline_bounds_headers_and_body() {
    async fn check(send_headers: bool) {
        let (base, started, server) = hanging_server(send_headers).await;
        let client = signer(&base);
        let start = tokio::time::Instant::now();
        let pending = tokio::spawn(async move { client.sign(unsigned()).await });
        started.await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(35), pending)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(error, Err(RemoteSignerError::Timeout));
        assert!(start.elapsed() >= REQUEST_TIMEOUT);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    tokio::join!(check(false), check(true));
}

#[test]
fn credential_validation_and_redacted_debug() {
    for credential in ["", " ", " padded", "line\nbreak", "a\rb"] {
        let error = RemoteSignerSession::new(credential, keys().public_key()).unwrap_err();
        assert_eq!(error, RemoteSignerError::Configuration);
    }
    let session = RemoteSignerSession::new(CREDENTIAL, keys().public_key()).unwrap();
    assert!(session.credential.is_sensitive());
    assert!(!format!("{session:?}").contains(CREDENTIAL));
    assert!(RemoteSigner::new("https://user:secret@example.com", session).is_err());
}

#[test]
#[ignore = "run explicitly with independent build-setting expectations"]
fn compiled_signer_config_matches_expected() {
    let mode = std::env::var("BUZZ_TEST_EXPECTED_SIGNER_MODE").unwrap();
    let base = std::env::var("BUZZ_TEST_EXPECTED_SIGNER_API_BASE").ok();
    assert_eq!(
        build_signer_config().unwrap(),
        SignerConfig::parse(Some(&mode), base.as_deref()).unwrap()
    );
}

#[path = "remote_signer/capabilities_tests.rs"]
mod capabilities;
