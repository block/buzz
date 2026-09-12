use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn identity(keys: &Keys) -> EnterpriseSession {
    EnterpriseSession {
        pubkey: keys.public_key().to_hex(),
        relay_ws_url: "wss://buzz.example".to_owned(),
        relay_http_url: "https://buzz.example".to_owned(),
    }
}

#[test]
fn configuration_and_credentials_fail_closed() {
    let keys = Keys::generate();
    for url in [
        "http://signer.example",
        "https://user:pass@signer.example",
        "https://signer.example/?token=x",
        "https://signer.example/#x",
    ] {
        assert!(EnterpriseSigner::new(url, identity(&keys)).is_err());
    }
    let mut wrong = identity(&keys);
    wrong.relay_ws_url = "wss://other.example".to_owned();
    assert!(EnterpriseSigner::new("https://signer.example", wrong).is_err());
    assert!(EnterpriseCredentials::new("".to_owned()).headers().is_err());
    assert!(EnterpriseCredentials::new("bad\r\nheader".to_owned())
        .headers()
        .is_err());
    let headers = EnterpriseCredentials::new("token".to_owned())
        .headers()
        .unwrap();
    assert!(headers["authorization"].is_sensitive());
    assert!(headers["authorization"].is_sensitive());
}

async fn server_reply(
    body: String,
    status: &str,
    expected: EnterpriseSession,
) -> (EnterpriseSigner, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
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
        stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    // Test-only loopback transport; production construction always enforces HTTPS.
    let signer = EnterpriseSigner {
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        base: format!("http://{address}"),
        expected,
    };
    (signer, task)
}

fn template() -> EnterpriseTemplate {
    EnterpriseTemplate {
        kind: 9,
        created_at: 1000,
        tags: vec![vec!["h".to_owned(), "channel".to_owned()]],
        content: "hello".to_owned(),
    }
}
fn signed(keys: &Keys, content: &str) -> Event {
    EventBuilder::new(Kind::from(9), content)
        .custom_created_at(Timestamp::from(1000))
        .tags([Tag::parse(["h", "channel"]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
async fn verifies_real_signatures_template_and_account_and_sends_explicit_credentials() {
    let keys = Keys::generate();
    let event = signed(&keys, "hello");
    let (signer, request) = server_reply(
        serde_json::json!({"event": event}).to_string(),
        "200 OK",
        identity(&keys),
    )
    .await;
    let credentials = EnterpriseCredentials::new("test-token-do-not-use".to_owned());
    assert_eq!(
        signer.sign(&template(), &credentials).await.unwrap().id,
        event.id
    );
    let request = request.await.unwrap();
    assert!(!request.contains("x-bb-session-credential:"));
    let authorization = request
        .lines()
        .find_map(|line| line.strip_prefix("authorization: "))
        .unwrap();
    let (scheme, token) = authorization.split_once(' ').unwrap();
    assert_eq!(scheme, "Bearer");
    assert_eq!(token, "test-token-do-not-use");
    assert!(!request.to_lowercase().contains("cookie:"));
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["purpose"], "publish");
    assert!(body["event"].get("pubkey").is_none());
    for invalid in [signed(&keys, "changed"), signed(&Keys::generate(), "hello")] {
        let (signer, request) = server_reply(
            serde_json::json!({"event": invalid}).to_string(),
            "200 OK",
            identity(&keys),
        )
        .await;
        assert!(signer.sign(&template(), &credentials).await.is_err());
        request.await.unwrap();
    }
    let mut forged = serde_json::to_value(event).unwrap();
    forged["sig"] = serde_json::json!("0".repeat(128));
    let (signer, request) = server_reply(
        serde_json::json!({"event": forged}).to_string(),
        "200 OK",
        identity(&keys),
    )
    .await;
    assert!(signer.sign(&template(), &credentials).await.is_err());
    request.await.unwrap();
}

#[tokio::test]
async fn rejects_denial_large_response_and_session_switch() {
    let keys = Keys::generate();
    let credentials = EnterpriseCredentials::new("test-token-do-not-use".to_owned());
    for (status, body) in [
        ("403 Forbidden", "{}".to_owned()),
        ("200 OK", "x".repeat(256 * 1024 + 1)),
        (
            "200 OK",
            serde_json::to_string(&identity(&Keys::generate())).unwrap(),
        ),
    ] {
        let (signer, request) = server_reply(body, status, identity(&keys)).await;
        assert!(signer.session(&credentials).await.is_err());
        request.await.unwrap();
    }
}
