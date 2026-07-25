use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode, Uri};
use axum::routing::post;
use axum::Router;
use buzz_client::{BuzzClient, BuzzClientConfig, BuzzIdentity, ClientError, RetryPolicy};
use nostr::{Event, EventBuilder, Keys, Kind};
use serde_json::{json, Value};

type Responder = dyn Fn(usize, &str, &Value) -> StubResponse + Send + Sync;

#[derive(Clone)]
struct StubState {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    responder: Arc<Responder>,
}

struct CapturedRequest {
    path: String,
    headers: HeaderMap,
    body: Value,
}

struct StubResponse {
    status: StatusCode,
    body: String,
    delay: Duration,
}

impl StubResponse {
    fn json(status: StatusCode, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }
}

async fn stub_handler(
    State(state): State<StubState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    let attempt = {
        let mut requests = state.requests.lock().unwrap();
        requests.push(CapturedRequest {
            path: uri.to_string(),
            headers,
            body: body.clone(),
        });
        requests.len()
    };
    let response = (state.responder)(attempt, &uri.to_string(), &body);
    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    Response::builder()
        .status(response.status)
        .header("content-type", "application/json")
        .body(Body::from(response.body))
        .unwrap()
}

async fn stub_relay<F>(responder: F) -> (String, Arc<Mutex<Vec<CapturedRequest>>>)
where
    F: Fn(usize, &str, &Value) -> StubResponse + Send + Sync + 'static,
{
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = StubState {
        requests: requests.clone(),
        responder: Arc::new(responder),
    };
    let app = Router::new()
        .route("/{*path}", post(stub_handler).get(stub_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), requests)
}

fn test_client(relay_url: &str, customize: impl FnOnce(&mut BuzzClientConfig)) -> BuzzClient {
    let mut config = BuzzClientConfig::new(relay_url);
    customize(&mut config);
    BuzzClient::new(
        config,
        BuzzIdentity::from_keys(Keys::generate(), None).unwrap(),
    )
    .unwrap()
}

fn event_value(sequence: u64, created_at: u64) -> Value {
    json!({
        "id": format!("{sequence:064x}"),
        "created_at": created_at,
    })
}

fn signed_event(kind: Kind) -> Event {
    EventBuilder::new(kind, "test")
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

#[tokio::test]
async fn public_and_authenticated_gets_keep_distinct_header_contracts() {
    let (relay_url, requests) = stub_relay(|_, path, body| {
        assert!(matches!(path, "/info" | "/moderation/reports?status=open"));
        assert_eq!(body, &Value::Null);
        StubResponse::json(StatusCode::OK, json!({"ok": true}))
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    assert_eq!(client.get_public("/info").await.unwrap(), r#"{"ok":true}"#);
    assert_eq!(
        client
            .get_authenticated("/moderation/reports?status=open")
            .await
            .unwrap(),
        r#"{"ok":true}"#
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/info");
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(
        requests[0].headers.get("accept").unwrap(),
        "application/nostr+json"
    );
    assert_eq!(requests[1].path, "/moderation/reports?status=open");
    assert!(requests[1].headers.contains_key("authorization"));
}

#[tokio::test]
async fn generic_json_post_uses_authenticated_transport() {
    let (relay_url, requests) = stub_relay(|_, path, body| {
        assert_eq!(path, "/api/invites");
        assert_eq!(body, &json!({"ttl_secs": 60}));
        StubResponse::json(StatusCode::OK, json!({"token": "invite-token"}))
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    let response = client
        .post_json_value("/api/invites", &json!({"ttl_secs": 60}))
        .await
        .unwrap();

    assert_eq!(response, json!({"token": "invite-token"}));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[0].body, json!({"ttl_secs": 60}));
}

#[tokio::test]
async fn generic_paths_must_be_root_relative() {
    let client = test_client("https://relay.example", |_| {});

    assert!(matches!(
        client.get_authenticated("https://attacker.example").await,
        Err(ClientError::Protocol(_))
    ));
    assert!(matches!(
        client
            .post_json_value("//attacker.example/path", &Value::Null)
            .await,
        Err(ClientError::Protocol(_))
    ));
}

#[tokio::test]
async fn query_serializes_multiple_filters_and_returns_event_values() {
    let event = EventBuilder::text_note("hello")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let expected_id = event.id;
    let event_json = serde_json::to_value(event).unwrap();
    let (relay_url, requests) = stub_relay(move |_, path, _| {
        assert_eq!(path, "/query");
        StubResponse::json(StatusCode::OK, json!([event_json]))
    })
    .await;
    let client = test_client(&relay_url, |_| {});
    let filters = [json!({"kinds": [9]}), json!({"authors": ["a".repeat(64)]})];

    let events = client.query_values(&filters).await.unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["id"], expected_id.to_hex());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/query");
    assert_eq!(requests[0].body, json!(filters));
    assert!(requests[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn count_returns_unsigned_count() {
    let (relay_url, requests) =
        stub_relay(|_, _, _| StubResponse::json(StatusCode::OK, json!({"count": 42}))).await;
    let client = test_client(&relay_url, |_| {});
    let filters = [json!({"kinds": [9, 45001]})];

    assert_eq!(client.count(&filters).await.unwrap(), 42);
    assert_eq!(requests.lock().unwrap()[0].body, json!(filters));
}

#[tokio::test]
async fn pagination_advances_equal_timestamp_cursor_without_duplicates() {
    let first_page: Vec<_> = (501..=1000)
        .rev()
        .map(|sequence| event_value(sequence, 42))
        .collect();
    let boundary_id = format!("{:064x}", 501);
    let (relay_url, requests) = stub_relay(move |attempt, _, _| {
        if attempt == 1 {
            StubResponse::json(StatusCode::OK, json!(first_page))
        } else {
            StubResponse::json(
                StatusCode::OK,
                json!([
                    event_value(501, 42),
                    event_value(500, 42),
                    event_value(499, 42)
                ]),
            )
        }
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    let events = client
        .query_paginated(json!({"kinds": [9]}), None)
        .await
        .unwrap();

    assert_eq!(events.len(), 502);
    let unique_ids: std::collections::HashSet<_> = events
        .iter()
        .map(|event| event["id"].as_str().unwrap())
        .collect();
    assert_eq!(unique_ids.len(), events.len());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body[0]["limit"], 500);
    assert_eq!(requests[1].body[0]["until"], 42);
    assert_eq!(requests[1].body[0]["before_id"], boundary_id);
}

#[tokio::test]
async fn malformed_success_bodies_are_typed_errors() {
    let (relay_url, _) = stub_relay(|_, _, _| StubResponse {
        status: StatusCode::OK,
        body: "not-json".to_string(),
        delay: Duration::ZERO,
    })
    .await;
    let client = test_client(&relay_url, |_| {});
    assert!(matches!(
        client.query_values(&[json!({"kinds": [9]})]).await,
        Err(ClientError::Serialization(_))
    ));

    let (relay_url, _) =
        stub_relay(|_, _, _| StubResponse::json(StatusCode::OK, json!({"not": "an array"}))).await;
    let client = test_client(&relay_url, |_| {});
    assert!(matches!(
        client.query_values(&[json!({"kinds": [9]})]).await,
        Err(ClientError::Protocol(_))
    ));

    let (relay_url, _) =
        stub_relay(|_, _, _| StubResponse::json(StatusCode::OK, json!({"count": "many"}))).await;
    let client = test_client(&relay_url, |_| {});
    assert!(matches!(
        client.count(&[json!({"kinds": [9]})]).await,
        Err(ClientError::Protocol(_))
    ));
}

#[tokio::test]
async fn pagination_respects_finite_and_zero_limits() {
    let (relay_url, requests) =
        stub_relay(|_, _, _| StubResponse::json(StatusCode::OK, json!([event_value(1, 42)]))).await;
    let client = test_client(&relay_url, |_| {});

    let events = client
        .query_paginated(json!({"kinds": [9]}), Some(1))
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(requests.lock().unwrap()[0].body[0]["limit"], 1);

    let request_count = requests.lock().unwrap().len();
    assert!(client
        .query_paginated(json!({"kinds": [9]}), Some(0))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(requests.lock().unwrap().len(), request_count);
}

#[tokio::test]
async fn authentication_failures_are_not_retried() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
        let (relay_url, requests) =
            stub_relay(move |_, _, _| StubResponse::json(status, json!({"error": "not allowed"})))
                .await;
        let client = test_client(&relay_url, |_| {});
        let error = client
            .query_values(&[json!({"kinds": [9]})])
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ClientError::Relay {
                status: code,
                ref message,
                ..
            } if code == status.as_u16() && message == "not allowed"
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rate_limits_retry_with_and_without_hints() {
    for message in ["rate-limited: retry in 0s", "rate-limited without a hint"] {
        let (relay_url, requests) = stub_relay(move |attempt, _, _| {
            if attempt == 1 {
                StubResponse::json(StatusCode::TOO_MANY_REQUESTS, json!({"error": message}))
            } else {
                StubResponse::json(StatusCode::OK, json!([]))
            }
        })
        .await;
        let client = test_client(&relay_url, |config| {
            config.retry_policy.max_retry_delay = Duration::ZERO;
        });
        assert!(client.query_values(&[json!({"kinds": [9]})]).await.is_ok());
        assert_eq!(requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn relay_retry_hint_is_defensively_capped() {
    let (relay_url, _) = stub_relay(|_, _, _| {
        StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "rate-limited: retry in 999999s"}),
        )
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.retry_policy = RetryPolicy {
            max_attempts: 1,
            max_retry_delay: Duration::from_secs(7),
        };
    });
    assert!(matches!(
        client.query_values(&[json!({"kinds": [9]})]).await,
        Err(ClientError::Relay {
            status: 429,
            retry_after: Some(delay),
            ..
        }) if delay == Duration::from_secs(7)
    ));
}

#[tokio::test]
async fn gateway_failures_are_retried() {
    for status in [
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ] {
        let (relay_url, requests) = stub_relay(move |attempt, _, _| {
            if attempt == 1 {
                StubResponse::json(status, json!({"error": "transient"}))
            } else {
                StubResponse::json(StatusCode::OK, json!([]))
            }
        })
        .await;
        let client = test_client(&relay_url, |config| {
            config.retry_policy.max_retry_delay = Duration::ZERO;
        });
        assert!(client.query_values(&[json!({"kinds": [9]})]).await.is_ok());
        assert_eq!(requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn request_timeouts_are_distinct_and_bounded() {
    let (relay_url, requests) = stub_relay(|_, _, _| StubResponse {
        status: StatusCode::OK,
        body: "[]".to_string(),
        delay: Duration::from_millis(100),
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.request_timeout = Duration::from_millis(10);
        config.retry_policy.max_attempts = 1;
    });

    assert!(matches!(
        client.query_values(&[json!({"kinds": [9]})]).await,
        Err(ClientError::Timeout)
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn response_body_transfer_failures_are_retried() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let server_attempts = attempts.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let attempt = server_attempts.fetch_add(1, Ordering::SeqCst) + 1;
            let mut request = vec![0; 4096];
            let _ = stream.read(&mut request).await;
            if attempt == 1 {
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n[",
                    )
                    .await
                    .unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n[]",
                    )
                    .await
                    .unwrap();
                break;
            }
        }
    });

    let client = test_client(&format!("http://{address}"), |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });
    assert!(client.query_values(&[json!({"kinds": [9]})]).await.is_ok());
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stored_event_submission_returns_typed_acknowledgement() {
    let event = signed_event(Kind::TextNote);
    let event_id = event.id.to_hex();
    let response_id = event_id.clone();
    let (relay_url, requests) = stub_relay(move |_, path, _| {
        assert_eq!(path, "/events");
        StubResponse::json(
            StatusCode::OK,
            json!({
                "event_id": response_id,
                "accepted": true,
                "message": "stored"
            }),
        )
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    let response = client.submit_event(event).await.unwrap();

    assert_eq!(response.event_id, event_id);
    assert!(response.accepted);
    assert_eq!(response.message, "stored");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn explicit_event_rejection_is_typed() {
    let event = signed_event(Kind::TextNote);
    let event_id = event.id.to_hex();
    let response_id = event_id.clone();
    let (relay_url, _) = stub_relay(move |_, _, _| {
        StubResponse::json(
            StatusCode::OK,
            json!({
                "event_id": response_id,
                "accepted": false,
                "message": "duplicate"
            }),
        )
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    assert!(matches!(
        client.submit_event(event).await,
        Err(ClientError::Rejected {
            event_id: rejected_id,
            ref message,
        }) if rejected_id == event_id && message == "duplicate"
    ));
}

#[tokio::test]
async fn stored_events_retry_safe_transient_failures() {
    let event = signed_event(Kind::TextNote);
    let response_id = event.id.to_hex();
    let (relay_url, requests) = stub_relay(move |attempt, _, _| {
        if attempt == 1 {
            StubResponse::json(StatusCode::BAD_GATEWAY, json!({"error": "transient"}))
        } else {
            StubResponse::json(
                StatusCode::OK,
                json!({
                    "event_id": response_id,
                    "accepted": true,
                    "message": ""
                }),
            )
        }
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });

    assert!(client.submit_event(event).await.is_ok());
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn exhausted_ambiguous_stored_delivery_is_unknown() {
    let event = signed_event(Kind::TextNote);
    let event_id = event.id.to_hex();
    let (relay_url, requests) = stub_relay(|_, _, _| {
        StubResponse::json(StatusCode::BAD_GATEWAY, json!({"error": "transient"}))
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });

    assert!(matches!(
        client.submit_event(event).await,
        Err(ClientError::DeliveryUnknown {
            event_id: unknown_id,
            ..
        }) if unknown_id == event_id
    ));
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn moderation_retries_only_canonical_pre_ingest_rate_limit() {
    let event = signed_event(Kind::Custom(9040));
    let response_id = event.id.to_hex();
    let (relay_url, requests) = stub_relay(move |attempt, _, _| {
        if attempt == 1 {
            StubResponse::json(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "rate-limited: retry in 0s"}),
            )
        } else {
            StubResponse::json(
                StatusCode::OK,
                json!({
                    "event_id": response_id,
                    "accepted": true,
                    "message": ""
                }),
            )
        }
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });

    assert!(client.submit_event(event).await.is_ok());
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn ambiguous_moderation_statuses_are_not_retried() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ] {
        let event = signed_event(Kind::Custom(9041));
        let event_id = event.id.to_hex();
        let (relay_url, requests) = stub_relay(move |_, _, _| {
            StubResponse::json(status, json!({"error": "proxy response"}))
        })
        .await;
        let client = test_client(&relay_url, |config| {
            config.retry_policy.max_retry_delay = Duration::ZERO;
        });

        assert!(matches!(
            client.submit_event(event).await,
            Err(ClientError::DeliveryUnknown {
                event_id: unknown_id,
                ..
            }) if unknown_id == event_id
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn moderation_timeout_is_delivery_unknown_without_retry() {
    let event = signed_event(Kind::Custom(9042));
    let event_id = event.id.to_hex();
    let (relay_url, requests) = stub_relay(|_, _, _| StubResponse {
        status: StatusCode::OK,
        body: "{}".to_string(),
        delay: Duration::from_millis(100),
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.request_timeout = Duration::from_millis(10);
    });

    assert!(matches!(
        client.submit_event(event).await,
        Err(ClientError::DeliveryUnknown {
            event_id: unknown_id,
            ..
        }) if unknown_id == event_id
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn malformed_acknowledgement_is_delivery_unknown() {
    let event = signed_event(Kind::TextNote);
    let event_id = event.id.to_hex();
    let (relay_url, _) =
        stub_relay(|_, _, _| StubResponse::json(StatusCode::OK, json!({"accepted": true}))).await;
    let client = test_client(&relay_url, |_| {});

    assert!(matches!(
        client.submit_event(event).await,
        Err(ClientError::DeliveryUnknown {
            event_id: unknown_id,
            ..
        }) if unknown_id == event_id
    ));
}

#[tokio::test]
async fn definitive_moderation_rejection_remains_relay_error() {
    let event = signed_event(Kind::Custom(9043));
    let (relay_url, requests) = stub_relay(|_, _, _| {
        StubResponse::json(StatusCode::FORBIDDEN, json!({"error": "not permitted"}))
    })
    .await;
    let client = test_client(&relay_url, |_| {});

    assert!(matches!(
        client.submit_event(event).await,
        Err(ClientError::Relay {
            status: 403,
            ref message,
            ..
        }) if message == "not permitted"
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn moderation_connect_failure_remains_safe_to_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = test_client(&format!("http://{address}"), |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });

    assert!(matches!(
        client
            .submit_event(signed_event(Kind::Custom(9044)))
            .await,
        Err(ClientError::Network(ref error)) if error.is_connect()
    ));
}

#[tokio::test]
async fn canonical_stored_rate_limit_remains_safe_after_exhaustion() {
    let (relay_url, requests) = stub_relay(|_, _, _| {
        StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "rate-limited: retry in 0s"}),
        )
    })
    .await;
    let client = test_client(&relay_url, |config| {
        config.retry_policy.max_retry_delay = Duration::ZERO;
    });

    assert!(matches!(
        client.submit_event(signed_event(Kind::TextNote)).await,
        Err(ClientError::Relay { status: 429, .. })
    ));
    assert_eq!(requests.lock().unwrap().len(), 3);
}
