use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_client::{
    BuzzClient, ClientConfig, ClientError, CommunityEndpoint, DeliveryOutcome, MessageAttachment,
    MessageKind, RetryPolicy, SendMessageRequest,
};
use nostr::signer::SignerBackend;
use nostr::util::BoxedFuture;
use nostr::{Event, JsonUtil, Keys, NostrSigner, PublicKey, SignerError, UnsignedEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

fn request() -> SendMessageRequest {
    SendMessageRequest {
        channel_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("channel UUID"),
        content: "retry me once, sign me once".into(),
        kind: MessageKind::Stream,
        reply_to: None,
        broadcast: false,
        attachments: Vec::new(),
        mentions: Vec::new(),
    }
}

fn retry_config() -> ClientConfig {
    ClientConfig::new(
        Duration::from_secs(5),
        Duration::from_secs(1),
        RetryPolicy::new(3, Duration::from_millis(1), Duration::from_millis(1))
            .expect("retry policy"),
    )
    .expect("client config")
}

#[derive(Debug)]
struct RefusingSigner {
    public_key: PublicKey,
}

impl NostrSigner for RefusingSigner {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("refusing-test-signer".into())
    }

    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async { Ok(self.public_key) })
    }

    fn sign_event(&self, _unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async { Err(SignerError::from("signing refused")) })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unavailable")) })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unavailable")) })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-44 unavailable")) })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _payload: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-44 unavailable")) })
    }
}

#[tokio::test]
async fn body_loss_retries_identical_event_with_fresh_request_auth() {
    let attempts = Arc::new(AtomicU32::new(0));
    let captured_bodies = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let captured_auth = Arc::new(Mutex::new(Vec::<String>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let attempts_for_server = attempts.clone();
    let bodies_for_server = captured_bodies.clone();
    let auth_for_server = captured_auth.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let attempt = attempts_for_server.fetch_add(1, Ordering::SeqCst) + 1;
            let mut bytes = vec![0_u8; 16 * 1024];
            let Ok(Ok(read)) =
                tokio::time::timeout(Duration::from_millis(500), stream.read(&mut bytes)).await
            else {
                continue;
            };
            bytes.truncate(read);
            let body_offset = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
                .unwrap_or(bytes.len());
            bodies_for_server
                .lock()
                .expect("body capture lock")
                .push(bytes[body_offset..].to_vec());
            let request_text = String::from_utf8_lossy(&bytes);
            let authorization = request_text
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                .unwrap_or_default()
                .to_string();
            auth_for_server
                .lock()
                .expect("auth capture lock")
                .push(authorization);

            if attempt < 3 {
                let partial = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"partial\":";
                let _ = stream.write_all(partial).await;
            } else {
                let body = r#"{"event_id":"ignored","accepted":true,"message":"stored"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        }
    });

    let keys = Keys::generate();
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(keys),
    )
    .config(retry_config())
    .build()
    .await
    .expect("client");
    let result = client.send_message(request()).await.expect("send result");
    let accepted = match result.delivery {
        DeliveryOutcome::Accepted(accepted) => accepted,
        other => panic!("expected acceptance, got {other:?}"),
    };

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    let bodies = captured_bodies.lock().expect("body capture lock");
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[1], bodies[2]);
    let event = Event::from_json(&bodies[0]).expect("submitted event JSON");
    event.verify().expect("event signature");
    assert_eq!(accepted.event_id, event.id);
    let auth = captured_auth.lock().expect("auth capture lock");
    assert!(auth.iter().all(|header| !header.is_empty()));
    assert_ne!(auth[0], auth[1]);
    assert_ne!(auth[1], auth[2]);
}

#[tokio::test]
async fn exhausted_proxy_failures_preserve_original_id_as_delivery_unknown() {
    let attempts = Arc::new(AtomicU32::new(0));
    let first_event_id = Arc::new(Mutex::new(None));
    let app_attempts = attempts.clone();
    let app_event_id = first_event_id.clone();
    let app = axum::Router::new().route(
        "/events",
        axum::routing::post(move |axum::Json(event): axum::Json<serde_json::Value>| {
            let app_attempts = app_attempts.clone();
            let app_event_id = app_event_id.clone();
            async move {
                app_attempts.fetch_add(1, Ordering::SeqCst);
                let mut event_id = app_event_id.lock().expect("event id lock");
                if event_id.is_none() {
                    *event_id = event["id"].as_str().map(str::to_string);
                }
                (
                    axum::http::StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({"error": "proxy failed"})),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .config(retry_config())
    .build()
    .await
    .expect("client");

    let result = client.send_message(request()).await.expect("send result");
    let unknown = match result.delivery {
        DeliveryOutcome::Unknown(unknown) => unknown,
        other => panic!("expected unknown delivery, got {other:?}"),
    };
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        unknown.event_id.to_hex(),
        first_event_id
            .lock()
            .expect("event id lock")
            .clone()
            .expect("captured event id")
    );
}

#[tokio::test]
async fn semantic_rejection_is_definitive_and_not_retried() {
    let attempts = Arc::new(AtomicU32::new(0));
    let app_attempts = attempts.clone();
    let app = axum::Router::new().route(
        "/events",
        axum::routing::post(move || {
            let app_attempts = app_attempts.clone();
            async move {
                app_attempts.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({"error": "invalid event"})),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .config(retry_config())
    .build()
    .await
    .expect("client");

    let result = client.send_message(request()).await.expect("send result");
    let rejected = match result.delivery {
        DeliveryOutcome::Rejected(rejected) => rejected,
        other => panic!("expected rejection, got {other:?}"),
    };
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(rejected.status, 422);
    assert_eq!(rejected.reason, "invalid event");
    assert!(!rejected.retry_safe);
}

#[tokio::test]
async fn malformed_success_response_is_delivery_unknown() {
    let app = axum::Router::new().route(
        "/events",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                "not valid JSON",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .config(retry_config())
    .build()
    .await
    .expect("client");

    let result = client.send_message(request()).await.expect("send result");
    let unknown = match result.delivery {
        DeliveryOutcome::Unknown(unknown) => unknown,
        other => panic!("expected unknown delivery, got {other:?}"),
    };
    assert!(unknown.reason.contains("invalid relay success response"));
}

#[tokio::test]
async fn signer_refusal_during_message_build_is_typed() {
    let keys = Keys::generate();
    let client = BuzzClient::builder(
        CommunityEndpoint::parse("https://buzz.example").expect("endpoint"),
        Arc::new(RefusingSigner {
            public_key: keys.public_key(),
        }),
    )
    .build()
    .await
    .expect("client construction");

    let error = client
        .send_message(request())
        .await
        .expect_err("signing refusal must fail");
    assert!(matches!(error, ClientError::Signer(_)));
}

#[tokio::test]
async fn upload_response_outside_community_authority_is_rejected() {
    let app = axum::Router::new().route(
        "/upload",
        axum::routing::put(|| async {
            axum::Json(serde_json::json!({
                "url": "https://media.example/asset.png",
                "sha256": "00",
                "size": 3,
                "type": "image/png"
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .build()
    .await
    .expect("client");
    let mut send = request();
    send.attachments.push(MessageAttachment {
        bytes: bytes::Bytes::from_static(b"png"),
        mime_type: "image/png".into(),
    });

    let error = client
        .send_message(send)
        .await
        .expect_err("foreign upload location must fail");
    assert!(matches!(error, ClientError::Endpoint(_)));
}
