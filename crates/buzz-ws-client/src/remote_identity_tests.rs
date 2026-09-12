use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn identity(keys: &Keys) -> ManagedIdentity {
    ManagedIdentity {
        pubkey: keys.public_key().to_hex(),
        relay_ws_url: "wss://buzz.example".into(),
        relay_http_url: "https://buzz.example".into(),
    }
}

#[derive(Default)]
struct Authorization {
    invalid: AtomicBool,
    calls: AtomicUsize,
    fail: bool,
}
impl RemoteAuthorization for Authorization {
    fn check_active(&self) -> Result<(), String> {
        if self.invalid.load(Ordering::Acquire) {
            Err("login changed".into())
        } else {
            Ok(())
        }
    }
    fn credentials(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteCredentials, String>> + Send + '_>> {
        Box::pin(async {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.check_active()?;
            if self.fail {
                Err("refresh failed".into())
            } else {
                Ok(RemoteCredentials::new("test-only-bearer".into()))
            }
        })
    }
}

// Inject only a loopback URL into the production HTTP transport. Its redirect,
// response bound, request construction and verification paths are unchanged.
async fn server_reply(
    body: String,
    status: &str,
    extra_headers: &str,
    invalidate: Option<Arc<Authorization>>,
) -> (RemoteIdentityClient, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let extra_headers = extra_headers.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
            assert!(request.len() < 256 * 1024);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        if let Some(owner) = invalidate {
            owner.invalid.store(true, Ordering::Release);
        }
        let response = format!("HTTP/1.1 {status}\r\n{extra_headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        // Rejection may close a large-response connection before the server finishes.
        let _ = stream.write_all(response.as_bytes()).await;
        String::from_utf8(request).unwrap()
    });
    (
        RemoteIdentityClient {
            client: http_client().unwrap(),
            base: url::Url::parse(&format!("http://{address}/cash-app/goose/")).unwrap(),
        },
        task,
    )
}

fn unsigned(keys: &Keys) -> UnsignedEvent {
    EventBuilder::new(Kind::from(9), "hello 🐝\nexact")
        .custom_created_at(Timestamp::from(1000))
        .tags([
            Tag::parse(["h", "channel"]).unwrap(),
            Tag::parse(["x", "a", "b"]).unwrap(),
        ])
        .build(keys.public_key())
}

#[test]
fn release_destination_and_identity_validation_fails_closed() {
    for base in [
        "http://signer.example/cash-app/goose/",
        "https://user:pass@signer.example/api/",
        "https://@signer.example/api/",
        "https://signer.example:443/api/",
        "https://signer.example:444/api/",
        "https://signer.example/api/?",
        "https://signer.example/api/#",
        "https://signer.example/api/\\",
        "https://signer.example/api",
        "https://signer.example/",
        "https://signer.example/api/../goose/",
        "https://signer.example/api/%2e/goose/",
        "https://signer.example/api//goose/",
        "https://signer.example/api/v1/buzz/identity/sign/",
        "https://signer.example/api/\n",
    ] {
        assert!(RemoteIdentityClient::new(base).is_err(), "accepted {base}");
    }
    assert!(RemoteIdentityClient::new("https://signer.example/cash-app/goose/").is_ok());
    let keys = Keys::generate();
    let mut wrong = identity(&keys);
    wrong.relay_ws_url = "wss://other.example".into();
    assert!(wrong.validate().is_err());
    for pubkey in ["0".repeat(64), "a".repeat(63), "A".repeat(64)] {
        wrong = identity(&keys);
        wrong.pubkey = pubkey;
        assert!(wrong.validate().is_err());
    }
    for token in ["", "bad\r\nheader", "bad token"] {
        assert!(RemoteCredentials::new(token.into()).headers().is_err());
    }
    assert!(
        RemoteCredentials::new("token".into()).headers().unwrap()["authorization"].is_sensitive()
    );
}

#[tokio::test]
async fn ensure_is_separate_enrollment_with_empty_body_and_no_identity_selectors() {
    let expected = identity(&Keys::generate());
    let (client, request) = server_reply(
        serde_json::to_string(&expected).unwrap(),
        "200 OK",
        "",
        None,
    )
    .await;
    assert_eq!(
        client
            .ensure(&RemoteCredentials::new("test-only-bearer".into()))
            .await
            .unwrap(),
        expected
    );
    let request = request.await.unwrap();
    assert!(request.starts_with("POST /cash-app/goose/v1/buzz/identity/ensure HTTP/1.1"));
    assert_eq!(request.split("\r\n\r\n").nth(1).unwrap(), "{}");
    assert!(request.contains("authorization: Bearer test-only-bearer"));
    assert!(!request.to_lowercase().contains("cookie:"));
}

#[tokio::test]
async fn sign_uses_neutral_exact_event_interface_without_ensure_or_publish() {
    let keys = Keys::generate();
    let input = unsigned(&keys);
    let expected = input.clone().sign_with_keys(&keys).unwrap();
    let (client, request) = server_reply(
        serde_json::json!({"event": expected}).to_string(),
        "200 OK",
        "",
        None,
    )
    .await;
    let owner = Arc::new(Authorization::default());
    let snapshot = RemoteEventSigner::new(client, &identity(&keys), owner.clone()).unwrap();
    let signer: &dyn EventSigner = &snapshot;
    let result = signer.sign(input.clone()).await.unwrap();
    assert_eq!(result.id, expected.id);
    result.verify().unwrap();
    assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
    let request = request.await.unwrap();
    assert!(request.starts_with("POST /cash-app/goose/v1/buzz/identity/sign HTTP/1.1"));
    assert!(request.contains("authorization: Bearer test-only-bearer"));
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"purpose":"publish", "event": {"kind":9, "created_at":1000, "tags":[["h","channel"],["x","a","b"]], "content":"hello 🐝\nexact"}})
    );
    assert!(!request.contains("enterprise-signer"));
}

#[tokio::test]
async fn rejects_valid_signatures_with_any_template_change_and_invalid_crypto() {
    let keys = Keys::generate();
    let input = unsigned(&keys);
    let mut variants = Vec::new();
    let mut changed = input.clone();
    changed.content.push('!');
    variants.push(changed);
    let mut changed = input.clone();
    changed.kind = Kind::from(7);
    variants.push(changed);
    let mut changed = input.clone();
    changed.created_at = Timestamp::from(1001);
    variants.push(changed);
    let mut changed = input.clone();
    changed.tags = nostr::Tags::from_list(vec![Tag::parse(["h", "other"]).unwrap()]);
    variants.push(changed);
    let mut replies: Vec<_> = variants
        .into_iter()
        .map(|mut e| {
            e.id = None;
            serde_json::to_value(e.sign_with_keys(&keys).unwrap()).unwrap()
        })
        .collect();
    let other = Keys::generate();
    replies.push(serde_json::to_value(unsigned(&other).sign_with_keys(&other).unwrap()).unwrap());
    let correct = input.clone().sign_with_keys(&keys).unwrap();
    let mut forged = serde_json::to_value(&correct).unwrap();
    forged["sig"] = serde_json::json!("0".repeat(128));
    replies.push(forged);
    let mut forged = serde_json::to_value(&correct).unwrap();
    forged["id"] = serde_json::json!("0".repeat(64));
    replies.push(forged);
    for event in replies {
        let (client, request) = server_reply(
            serde_json::json!({"event": event}).to_string(),
            "200 OK",
            "",
            None,
        )
        .await;
        let signer =
            RemoteEventSigner::new(client, &identity(&keys), Arc::new(Authorization::default()))
                .unwrap();
        assert!(signer.sign(input.clone()).await.is_err());
        request.await.unwrap();
    }
}

#[tokio::test]
async fn refuses_wrong_author_and_failed_credentials_before_network_without_fallback() {
    let keys = Keys::generate();
    let owner = Arc::new(Authorization {
        fail: true,
        ..Default::default()
    });
    let client = RemoteIdentityClient::new("https://signer.invalid/cash-app/goose/").unwrap();
    let signer = RemoteEventSigner::new(client, &identity(&keys), owner.clone()).unwrap();
    assert!(signer.sign(unsigned(&Keys::generate())).await.is_err());
    assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        signer.sign(unsigned(&keys)).await.unwrap_err(),
        "refresh failed"
    );
    owner.invalid.store(true, Ordering::Release);
    assert_eq!(
        signer.sign(unsigned(&keys)).await.unwrap_err(),
        "login changed"
    );
    assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(signer.public_key(), keys.public_key());
}

#[tokio::test]
async fn invalidated_snapshot_discards_in_flight_signature() {
    let keys = Keys::generate();
    let event = unsigned(&keys).sign_with_keys(&keys).unwrap();
    let owner = Arc::new(Authorization::default());
    let (client, request) = server_reply(
        serde_json::json!({"event": event}).to_string(),
        "200 OK",
        "",
        Some(owner.clone()),
    )
    .await;
    let signer = RemoteEventSigner::new(client, &identity(&keys), owner).unwrap();
    assert_eq!(
        signer.sign(unsigned(&keys)).await.unwrap_err(),
        "login changed"
    );
    request.await.unwrap();
}

#[tokio::test]
async fn denial_malformed_and_oversize_response_propagate_without_local_fallback() {
    let keys = Keys::generate();
    for (status, body) in [
        ("403 Forbidden", "{}".into()),
        ("200 OK", "not json".into()),
        ("200 OK", "x".repeat(256 * 1024 + 1)),
    ] {
        let (client, request) = server_reply(body, status, "", None).await;
        let signer =
            RemoteEventSigner::new(client, &identity(&keys), Arc::new(Authorization::default()))
                .unwrap();
        assert!(signer.sign(unsigned(&keys)).await.is_err());
        request.await.unwrap();
    }
}

#[tokio::test]
async fn actual_http_client_refuses_redirect_without_sending_bearer_to_target() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let headers = format!("Location: http://{}/leak\r\n", target.local_addr().unwrap());
    let (client, request) =
        server_reply("{}".into(), "307 Temporary Redirect", &headers, None).await;
    assert!(client
        .ensure(&RemoteCredentials::new("test-only-bearer".into()))
        .await
        .is_err());
    request.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
}

#[test]
fn managed_community_allows_backend_port_but_never_changes_release_destination_rules() {
    let keys = Keys::generate();
    let mut managed = identity(&keys);
    managed.relay_http_url = "https://buzz.example:8443".into();
    managed.relay_ws_url = "wss://buzz.example:8443".into();
    assert!(managed.validate().is_ok());
    assert!(RemoteIdentityClient::new("https://signer.example:8443/cash-app/goose/").is_err());
    for invalid in [
        "http://buzz.example:8443",
        "https://user@buzz.example:8443",
        "https://buzz.example:8443/?q",
        "https://buzz.example:8443/#fragment",
        "https://buzz.example:8443/a/..",
        "https://buzz.example:8443//",
        "https://buzz.example:0",
    ] {
        managed.relay_http_url = invalid.into();
        assert!(managed.validate().is_err(), "{invalid}");
    }
}

#[test]
fn remote_proofs_are_exactly_scoped_before_credentials_or_network() {
    let keys = Keys::generate();
    let signer = RemoteEventSigner::new(
        RemoteIdentityClient::new("https://signer.example/cash-app/goose/").unwrap(),
        &identity(&keys),
        Arc::new(Authorization::default()),
    )
    .unwrap();
    for target in [
        "http://buzz.example/events",
        "https://other.example/events",
        "https://buzz.example:8443/events",
        "https://user@buzz.example/events",
        "https://buzz.example/events#fragment",
    ] {
        assert!(signer
            .validate_proof_scope(27235, &[vec!["u".into(), target.into()]])
            .is_err());
    }
    assert!(signer
        .validate_proof_scope(
            27235,
            &[vec!["u".into(), "https://buzz.example/events".into()]]
        )
        .is_ok());
    assert!(signer
        .validate_proof_scope(24242, &[vec!["server".into(), "buzz.example".into()]])
        .is_ok());
    assert!(signer
        .validate_proof_scope(24242, &[vec!["server".into(), "buzz.example:8443".into()]])
        .is_err());
    assert!(signer
        .validate_proof_scope(22242, &[vec!["relay".into(), "wss://other.example".into()]])
        .is_err());
    assert!(signer
        .validate_proof_scope(22242, &[vec!["relay".into(), "wss://buzz.example".into()]])
        .is_ok());
}

#[tokio::test]
async fn production_sign_rejects_wrong_proof_scope_before_authorization_or_network() {
    let keys = Keys::generate();
    let auth = Arc::new(Authorization::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = RemoteIdentityClient {
        client: http_client().unwrap(),
        base: url::Url::parse(&format!("http://{}/api/", listener.local_addr().unwrap())).unwrap(),
    };
    let signer = RemoteEventSigner::new(client, &identity(&keys), auth.clone()).unwrap();
    for (kind, tags) in [
        (27235, vec![vec!["u", "https://other.example/events"]]),
        (24242, vec![vec!["server", "other.example"]]),
        (22242, vec![vec!["relay", "wss://other.example"]]),
    ] {
        let input = EventBuilder::new(Kind::from(kind), "")
            .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
            .build(keys.public_key());
        assert!(signer.sign(input).await.is_err());
    }
    assert_eq!(auth.calls.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
