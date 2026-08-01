use std::collections::BTreeSet;
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::{timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn signed_event(content: &str) -> Event {
    EventBuilder::new(Kind::TextNote, content)
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

fn mutate(event: &Event, field: &str, value: Value) -> Event {
    let mut json = serde_json::to_value(event).unwrap();
    json[field] = value;
    Event::from_json(json.to_string()).unwrap()
}

fn raw_event_with_layout(event: &Event, id: &str) -> String {
    let mut value = serde_json::to_value(event).unwrap();
    value["id"] = json!(id);
    format!(
        concat!(
            "{{\n",
            "  \"sig\" : {},\n",
            "  \"content\" : {},\n",
            "  \"tags\" : {},\n",
            "  \"kind\" : {},\n",
            "  \"created_at\" : {},\n",
            "  \"pubkey\" : {},\n",
            "  \"id\" : {}\n",
            "}}"
        ),
        value["sig"],
        value["content"],
        value["tags"],
        value["kind"],
        value["created_at"],
        value["pubkey"],
        value["id"],
    )
}

async fn relay_sending(frames: Vec<Value>) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].as_str().unwrap();
        for frame in frames {
            let frame = match frame {
                Value::Array(mut parts) if parts.get(1) == Some(&json!("$subscription")) => {
                    parts[1] = json!(subscription_id);
                    Value::Array(parts)
                }
                other => other,
            };
            websocket
                .send(Message::Text(frame.to_string().into()))
                .await
                .unwrap();
        }
        request
    });
    (format!("ws://{address}"), handle)
}

async fn relay_sending_raw_event(raw_event: String) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].as_str().unwrap();
        websocket
            .send(Message::Text(
                format!(r#"["EVENT","{subscription_id}",{raw_event}]"#).into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["EOSE", subscription_id]).to_string().into(),
            ))
            .await
            .unwrap();
        request
    });
    (format!("ws://{address}"), handle)
}

async fn run_buzz(relay: &str, event_id: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command
        .kill_on_drop(true)
        .env_remove("BUZZ_RELAY_URL")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .args([
            "events",
            "get-verified",
            "--relay",
            relay,
            "--event",
            event_id,
        ]);
    command.output().await.unwrap()
}

async fn run_buzz_with_private_key(relay: &str, event_id: &str, private_key: &str) -> Output {
    run_buzz_with_identity(relay, event_id, private_key, None).await
}

async fn run_buzz_with_identity(
    relay: &str,
    event_id: &str,
    private_key: &str,
    auth_tag: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command
        .kill_on_drop(true)
        .env_remove("BUZZ_RELAY_URL")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .args([
            "--private-key",
            private_key,
            "events",
            "get-verified",
            "--relay",
            relay,
            "--event",
            event_id,
        ]);
    if let Some(auth_tag) = auth_tag {
        command.env("BUZZ_AUTH_TAG", auth_tag);
    }
    command.output().await.unwrap()
}

fn error_category(output: &Output) -> String {
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    error["error"].as_str().unwrap().to_string()
}

fn assert_failure(output: &Output, exit_code: i32, category: &str) {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(output.stdout.is_empty(), "failure wrote to stdout");
    assert_eq!(error_category(output), category);
}

#[tokio::test]
async fn emits_only_raw_signed_fields_after_exact_verified_fetch() {
    let event = signed_event("verified output");
    let raw_event = raw_event_with_layout(&event, &event.id.to_hex());
    let (relay, server) = relay_sending_raw_event(raw_event.clone()).await;

    let output = run_buzz(&relay, &event.id.to_hex()).await;
    let request = server.await.unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, format!("{raw_event}\n").as_bytes());
    let emitted: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(emitted, serde_json::to_value(&event).unwrap());
    let keys: BTreeSet<&str> = emitted
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "content",
            "created_at",
            "id",
            "kind",
            "pubkey",
            "sig",
            "tags"
        ])
    );
    assert_eq!(
        request,
        json!([
            "REQ",
            "buzz-get-verified-0",
            {"ids": [event.id.to_hex()], "limit": 2}
        ])
    );
}

#[tokio::test]
async fn uppercase_raw_event_id_is_rejected_instead_of_normalized() {
    let event = signed_event("uppercase raw ID");
    let raw_event = raw_event_with_layout(&event, &event.id.to_hex().to_ascii_uppercase());
    let (relay, server) = relay_sending_raw_event(raw_event).await;

    let output = run_buzz(&relay, &event.id.to_hex()).await;
    server.await.unwrap();

    assert_failure(&output, 4, "relay_mismatch");
}

#[tokio::test]
async fn malformed_raw_signature_is_not_a_generic_protocol_error() {
    let event = signed_event("malformed raw signature");
    let mut raw_event = serde_json::to_value(&event).unwrap();
    raw_event["sig"] = json!("not-a-signature");
    let (relay, server) = relay_sending_raw_event(raw_event.to_string()).await;

    let output = run_buzz(&relay, &event.id.to_hex()).await;
    server.await.unwrap();

    assert_failure(&output, 4, "signature_invalid");
}

#[tokio::test]
async fn optional_nip42_challenge_does_not_make_public_read_require_a_key() {
    let event = signed_event("public read");
    let frames = vec![
        json!(["AUTH", "optional-challenge"]),
        json!(["EVENT", "$subscription", event]),
        json!(["EOSE", "$subscription"]),
    ];
    let (relay, server) = relay_sending(frames).await;

    let output = run_buzz(&relay, &event.id.to_hex()).await;
    server.await.unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let emitted: Event = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(emitted, event);
}

#[tokio::test]
async fn optional_challenge_does_not_transmit_ambient_identity_or_auth_tag() {
    let event = signed_event("public read with ambient identity");
    let event_id = event.id;
    let agent_keys = Keys::generate();
    let private_key = agent_keys.secret_key().to_secret_hex();
    let owner_keys = Keys::generate();
    let auth_tag =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=1")
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "optional-challenge"]).to_string().into(),
            ))
            .await
            .unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        assert_eq!(request[0], "REQ");
        let subscription_id = request[1].as_str().unwrap();
        websocket
            .send(Message::Text(
                json!(["EVENT", subscription_id, event]).to_string().into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["EOSE", subscription_id]).to_string().into(),
            ))
            .await
            .unwrap();

        let next = websocket.next().await.unwrap().unwrap();
        let next: Value = serde_json::from_str(next.to_text().unwrap()).unwrap();
        assert_eq!(next[0], "CLOSE", "optional challenge elicited AUTH: {next}");
    });

    let output =
        run_buzz_with_identity(&relay, &event_id.to_hex(), &private_key, Some(&auth_tag)).await;
    server.await.unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn zero_and_multiple_results_fail_without_stdout() {
    let requested = signed_event("requested");

    let (empty_relay, empty_server) = relay_sending(vec![json!(["EOSE", "$subscription"])]).await;
    let empty = run_buzz(&empty_relay, &requested.id.to_hex()).await;
    empty_server.await.unwrap();
    assert_failure(&empty, 1, "not_found");

    let frames = vec![
        json!(["EVENT", "$subscription", requested]),
        json!(["EVENT", "$subscription", requested]),
        json!(["EOSE", "$subscription"]),
    ];
    let (multiple_relay, multiple_server) = relay_sending(frames).await;
    let multiple = run_buzz(&multiple_relay, &requested.id.to_hex()).await;
    multiple_server.await.unwrap();
    assert_failure(&multiple, 4, "ambiguous");
}

#[tokio::test]
async fn second_event_fails_immediately_without_waiting_for_eose() {
    let event = signed_event("immediate ambiguity");
    let event_id = event.id;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].as_str().unwrap();
        for _ in 0..2 {
            websocket
                .send(Message::Text(
                    json!(["EVENT", subscription_id, event]).to_string().into(),
                ))
                .await
                .unwrap();
        }
        std::future::pending::<()>().await;
    });

    let output = timeout(Duration::from_secs(2), run_buzz(&relay, &event_id.to_hex()))
        .await
        .expect("command waited for EOSE after the second event");

    assert_failure(&output, 4, "ambiguous");
    server.abort();
}

#[tokio::test]
async fn wrong_event_mutated_content_and_invalid_signature_are_distinct() {
    let requested = signed_event("requested");
    let wrong = signed_event("wrong");
    let id_mismatch = mutate(&requested, "content", json!("mutated"));
    let invalid_signature = mutate(&requested, "sig", json!("0".repeat(128)));

    for (returned, category) in [
        (wrong, "relay_mismatch"),
        (id_mismatch, "id_mismatch"),
        (invalid_signature, "signature_invalid"),
    ] {
        let frames = vec![
            json!(["EVENT", "$subscription", returned]),
            json!(["EOSE", "$subscription"]),
        ];
        let (relay, server) = relay_sending(frames).await;
        let output = run_buzz(&relay, &requested.id.to_hex()).await;
        server.await.unwrap();
        assert_failure(&output, 4, category);
    }
}

#[tokio::test]
async fn subscription_mismatch_and_transport_failure_are_distinct() {
    let requested = signed_event("requested");
    let (relay, server) = relay_sending(vec![json!(["EOSE", "wrong-subscription"])]).await;
    let mismatch = run_buzz(&relay, &requested.id.to_hex()).await;
    server.await.unwrap();
    assert_failure(&mismatch, 4, "relay_mismatch");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = format!("ws://{}", listener.local_addr().unwrap());
    drop(listener);
    let transport = run_buzz(&unavailable, &requested.id.to_hex()).await;
    assert_failure(&transport, 2, "transport_error");
}

#[tokio::test]
async fn websocket_upgrade_401_and_403_are_non_retryable_auth_errors() {
    let event = signed_event("upgrade denial");

    for (status, reason) in [(401, "Unauthorized"), (403, "Forbidden")] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let relay = format!("ws://{address}");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let bytes = stream.read(&mut request).await.unwrap();
            assert!(bytes > 0, "client did not attempt a WebSocket upgrade");
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let output = run_buzz(&relay, &event.id.to_hex()).await;
        server.await.unwrap();
        assert_failure(&output, 3, "auth_error");
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["retryable"], false);
    }
}

#[tokio::test]
async fn command_local_relay_is_required_even_when_default_env_is_set() {
    let event = signed_event("no fallback");
    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .env("BUZZ_RELAY_URL", "ws://127.0.0.1:1")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .args(["events", "get-verified", "--event", &event.id.to_hex()])
        .output()
        .await
        .unwrap();

    assert_failure(&output, 1, "user_error");
}

#[tokio::test]
async fn uppercase_event_id_is_rejected_before_any_relay_connection() {
    let event = signed_event("canonical ID");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());

    let output = run_buzz(&relay, &event.id.to_hex().to_ascii_uppercase()).await;

    assert_failure(&output, 1, "user_error");
    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "uppercase ID unexpectedly reached the relay"
    );
}

async fn authenticated_relay(
    event: Event,
    challenge_first: bool,
    challenge: String,
    required_reason: String,
) -> (String, tokio::task::JoinHandle<()>) {
    let event_id = event.id;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        if challenge_first {
            websocket
                .send(Message::Text(json!(["AUTH", challenge]).to_string().into()))
                .await
                .unwrap();
        }

        let first_request = websocket.next().await.unwrap().unwrap();
        let first_request: Value = serde_json::from_str(first_request.to_text().unwrap()).unwrap();
        let first_subscription = first_request[1].as_str().unwrap();

        websocket
            .send(Message::Text(
                json!(["CLOSED", first_subscription, required_reason])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        if !challenge_first {
            websocket
                .send(Message::Text(json!(["AUTH", challenge]).to_string().into()))
                .await
                .unwrap();
        }

        let auth = websocket.next().await.unwrap().unwrap();
        let auth: Value = serde_json::from_str(auth.to_text().unwrap()).unwrap();
        assert_eq!(auth[0], "AUTH");
        let auth_event: Event = serde_json::from_value(auth[1].clone()).unwrap();
        auth_event.verify().unwrap();
        assert!(auth_event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["challenge", challenge.as_str()]));

        websocket
            .send(Message::Text(
                json!(["OK", auth_event.id.to_hex(), true, ""])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let second_request = websocket.next().await.unwrap().unwrap();
        let second_request: Value =
            serde_json::from_str(second_request.to_text().unwrap()).unwrap();
        assert_eq!(second_request[1], "buzz-get-verified-1");
        assert_eq!(second_request[2]["ids"], json!([event_id.to_hex()]));
        let second_subscription = second_request[1].as_str().unwrap();
        websocket
            .send(Message::Text(
                json!(["EVENT", second_subscription, event])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["EOSE", second_subscription]).to_string().into(),
            ))
            .await
            .unwrap();
    });

    (relay, server)
}

#[tokio::test]
async fn nip42_buffers_challenge_and_auth_required_closure_in_either_order() {
    let event = signed_event("authenticated read");
    let event_id = event.id;
    let keys = Keys::generate();
    let private_key = keys.secret_key().to_secret_hex();

    for required_reason in ["auth-required", "auth-required: not authenticated"] {
        for challenge_first in [true, false] {
            let (relay, server) = authenticated_relay(
                event.clone(),
                challenge_first,
                "challenge".into(),
                required_reason.into(),
            )
            .await;
            let output = run_buzz_with_private_key(&relay, &event_id.to_hex(), &private_key).await;
            server.await.unwrap();

            assert!(output.status.success());
            assert!(output.stderr.is_empty());
            let emitted: Event = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(emitted.id, event_id);
        }
    }
}

#[tokio::test]
async fn authenticated_exact_event_may_carry_the_connections_nip_oa_authority() {
    let agent_keys = Keys::generate();
    let private_key = agent_keys.secret_key().to_secret_hex();
    let owner_keys = Keys::generate();
    let auth_tag_json =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=1")
            .unwrap();
    let auth_tag = buzz_sdk::nip_oa::parse_auth_tag(&auth_tag_json).unwrap();
    let event = EventBuilder::new(Kind::TextNote, "delegated exact event")
        .tags([auth_tag.clone()])
        .sign_with_keys(&agent_keys)
        .unwrap();
    let event_id = event.id;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "production-shape-challenge"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let first_request = websocket.next().await.unwrap().unwrap();
        let first_request: Value = serde_json::from_str(first_request.to_text().unwrap()).unwrap();
        let first_subscription = first_request[1].clone();
        websocket
            .send(Message::Text(
                json!(["NOTICE", "auth-required: authenticate before subscribing"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!([
                    "CLOSED",
                    first_subscription,
                    "auth-required: not authenticated"
                ])
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        let auth_message = websocket.next().await.unwrap().unwrap();
        let auth: Value = serde_json::from_str(auth_message.to_text().unwrap()).unwrap();
        let auth_event: Event = serde_json::from_value(auth[1].clone()).unwrap();
        assert!(auth_event.tags.iter().any(|tag| tag == &auth_tag));
        websocket
            .send(Message::Text(
                json!(["OK", auth_event.id.to_hex(), true, ""])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let second_request = websocket.next().await.unwrap().unwrap();
        let second_request: Value =
            serde_json::from_str(second_request.to_text().unwrap()).unwrap();
        assert_eq!(second_request[1], "buzz-get-verified-1");
        websocket
            .send(Message::Text(
                json!(["EVENT", second_request[1], event])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["EOSE", second_request[1]]).to_string().into(),
            ))
            .await
            .unwrap();
    });

    let output = run_buzz_with_identity(
        &relay,
        &event_id.to_hex(),
        &private_key,
        Some(&auth_tag_json),
    )
    .await;
    server.await.unwrap();

    assert!(output.status.success(), "{}", error_category(&output));
    assert!(output.stderr.is_empty());
    let emitted: Event = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(emitted.id, event_id);
}

async fn auth_sequence_relay(
    sequence: Vec<Value>,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].clone();

        for frame in sequence {
            let frame = match frame {
                Value::Array(mut parts) if parts.get(1) == Some(&json!("$subscription")) => {
                    parts[1] = subscription_id.clone();
                    Value::Array(parts)
                }
                other => other,
            };
            websocket
                .send(Message::Text(frame.to_string().into()))
                .await
                .unwrap();
        }

        let mut received = Vec::new();
        while let Ok(Some(Ok(message))) = timeout(Duration::from_secs(2), websocket.next()).await {
            match message {
                Message::Text(text) => {
                    received.push(serde_json::from_str(&text).unwrap());
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        received
    });
    (format!("ws://{address}"), server)
}

#[tokio::test]
async fn duplicate_pre_auth_challenges_are_ambiguous_in_either_order() {
    let event = signed_event("duplicate auth challenge");
    let keys = Keys::generate();
    let private_key = keys.secret_key().to_secret_hex();

    for (sequence, expected_auth_count) in [
        (
            vec![
                json!(["AUTH", "first-challenge"]),
                json!(["AUTH", "second-challenge"]),
                json!(["CLOSED", "$subscription", "auth-required: duplicate"]),
            ],
            0,
        ),
        (
            vec![
                json!(["CLOSED", "$subscription", "auth-required: duplicate"]),
                json!(["AUTH", "first-challenge"]),
                json!(["AUTH", "second-challenge"]),
            ],
            1,
        ),
    ] {
        let (relay, server) = auth_sequence_relay(sequence).await;
        let output = run_buzz_with_private_key(&relay, &event.id.to_hex(), &private_key).await;
        let received = server.await.unwrap();

        assert_failure(&output, 4, "ambiguous");
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        if expected_auth_count == 0 {
            assert!(error["message"]
                .as_str()
                .unwrap()
                .contains("multiple AUTH challenges"));
        } else {
            // The second challenge arrives after the signed AUTH event but
            // before relay OK. The output boundary starts at transmission.
            assert_eq!(
                error["message"],
                "ambiguous result: relay returned an ambiguous result after private authentication"
            );
        }
        let auth_messages: Vec<&Value> = received
            .iter()
            .filter(|message| message.get(0) == Some(&json!("AUTH")))
            .collect();
        assert_eq!(auth_messages.len(), expected_auth_count, "{received:?}");
        if let Some(auth) = auth_messages.first() {
            let event: Event = serde_json::from_value(auth[1].clone()).unwrap();
            assert!(event
                .tags
                .iter()
                .any(|tag| tag.as_slice() == ["challenge", "first-challenge"]));
        }
    }
}

#[tokio::test]
async fn auth_reason_lookalikes_never_trigger_credential_signing() {
    let event = signed_event("auth-required lookalikes");
    let keys = Keys::generate();
    let private_key = keys.secret_key().to_secret_hex();

    for reason in [
        "not-auth-required",
        "restricted: not-auth-required",
        "restricted auth-required response",
        "auth-required-suffix",
        "AUTH-REQUIRED",
    ] {
        let (relay, server) = auth_sequence_relay(vec![
            json!(["AUTH", "untrusted-reason-challenge"]),
            json!(["CLOSED", "$subscription", reason]),
        ])
        .await;
        let output = run_buzz_with_private_key(&relay, &event.id.to_hex(), &private_key).await;
        let received = server.await.unwrap();

        assert_failure(&output, 2, "relay_error");
        assert!(
            received
                .iter()
                .all(|message| message.get(0) != Some(&json!("AUTH"))),
            "lookalike reason {reason:?} elicited AUTH: {received:?}"
        );
    }
}

#[tokio::test]
async fn auth_challenge_limit_accepts_1024_bytes_and_refuses_1025_before_signing() {
    let event = signed_event("challenge size");
    let keys = Keys::generate();
    let private_key = keys.secret_key().to_secret_hex();
    let exact = "x".repeat(1024);
    let (relay, server) = authenticated_relay(
        event.clone(),
        false,
        exact,
        "auth-required: oversized-boundary-control".into(),
    )
    .await;

    let output = run_buzz_with_private_key(&relay, &event.id.to_hex(), &private_key).await;
    server.await.unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let oversized = "x".repeat(1025);
    let (relay, server) = auth_sequence_relay(vec![
        json!(["CLOSED", "$subscription", "auth-required: oversized"]),
        json!(["AUTH", oversized]),
    ])
    .await;
    let output = run_buzz_with_private_key(&relay, &event.id.to_hex(), &private_key).await;
    let received = server.await.unwrap();

    assert_failure(&output, 2, "relay_error");
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["message"].as_str().unwrap().contains("1025 bytes"));
    assert!(
        received
            .iter()
            .all(|message| message.get(0) != Some(&json!("AUTH"))),
        "oversized challenge was signed before refusal: {received:?}"
    );
}

#[tokio::test]
async fn reflection_before_auth_ok_cannot_escape_through_stderr() {
    let event = signed_event("malformed auth echo");
    let event_id = event.id;
    let agent_keys = Keys::generate();
    let private_key = agent_keys.secret_key().to_secret_hex();
    let owner_keys = Keys::generate();
    let auth_tag =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=1")
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].clone();
        websocket
            .send(Message::Text(
                json!(["CLOSED", subscription_id, "auth-required"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "malformed-echo-challenge"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let auth_message = websocket.next().await.unwrap().unwrap();
        let auth_text = auth_message.to_text().unwrap().to_string();
        let auth: Value = serde_json::from_str(&auth_text).unwrap();
        assert_eq!(auth[0], "AUTH");

        // Deliberately invalid JSON that reflects the complete signed event.
        // The client must report only a static protocol category.
        websocket
            .send(Message::Text(format!("['AUTH',{}]", auth[1]).into()))
            .await
            .unwrap();
        auth_text
    });

    let output =
        run_buzz_with_identity(&relay, &event_id.to_hex(), &private_key, Some(&auth_tag)).await;
    let auth_text = server.await.unwrap();

    assert_failure(&output, 2, "relay_error");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let auth: Value = serde_json::from_str(&auth_text).unwrap();
    let auth_event_json = auth[1].to_string();
    let signature = auth[1]["sig"].as_str().unwrap();
    for private_bytes in [
        auth_tag.as_str(),
        auth_text.as_str(),
        auth_event_json.as_str(),
        signature,
    ] {
        assert!(
            !stderr.contains(private_bytes),
            "post-AUTH error leaked private bytes {private_bytes:?}: {stderr}"
        );
    }
    let error: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(
        error["message"],
        "relay protocol error: relay sent an invalid response after private authentication"
    );
}

#[tokio::test]
async fn authenticated_closed_reflection_cannot_escape_through_cli_sinks() {
    let event = signed_event("authenticated CLOSED reflection");
    let event_id = event.id;
    let agent_keys = Keys::generate();
    let private_key = agent_keys.secret_key().to_secret_hex();
    let owner_keys = Keys::generate();
    let auth_tag =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=1")
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].clone();
        websocket
            .send(Message::Text(
                json!(["CLOSED", subscription_id, "auth-required"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "closed-reflection-challenge"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let auth_message = websocket.next().await.unwrap().unwrap();
        let auth_text = auth_message.to_text().unwrap().to_string();
        let auth: Value = serde_json::from_str(&auth_text).unwrap();
        let auth_event: Event = serde_json::from_value(auth[1].clone()).unwrap();
        websocket
            .send(Message::Text(
                json!(["OK", auth_event.id.to_hex(), true, ""])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let second_request = websocket.next().await.unwrap().unwrap();
        let second_request: Value =
            serde_json::from_str(second_request.to_text().unwrap()).unwrap();
        assert_eq!(second_request[1], "buzz-get-verified-1");
        websocket
            .send(Message::Text(
                json!(["CLOSED", second_request[1], format!("echo:{auth_text}")])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        auth_text
    });

    let output =
        run_buzz_with_identity(&relay, &event_id.to_hex(), &private_key, Some(&auth_tag)).await;
    let auth_text = server.await.unwrap();

    assert_failure(&output, 2, "relay_error");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let auth: Value = serde_json::from_str(&auth_text).unwrap();
    let event_signature = auth[1]["sig"].as_str().unwrap();
    let auth_tag_json: Value = serde_json::from_str(&auth_tag).unwrap();
    let authority_signature = auth_tag_json[3].as_str().unwrap();
    for private_bytes in [
        auth_tag.as_str(),
        auth_text.as_str(),
        event_signature,
        authority_signature,
    ] {
        assert!(
            !stdout.contains(private_bytes),
            "authenticated CLOSED reflected private bytes to stdout {private_bytes:?}: {stdout}"
        );
        assert!(
            !stderr.contains(private_bytes),
            "authenticated CLOSED reflected private bytes to stderr {private_bytes:?}: {stderr}"
        );
    }
}

#[derive(Clone, Copy)]
enum AuthFieldReflection {
    UppercaseEventSignature,
    SplitAuthoritySignature,
}

async fn authenticated_unknown_field_relay(
    event: Event,
    reflection: AuthFieldReflection,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = format!("ws://{address}");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        let subscription_id = request[1].clone();
        websocket
            .send(Message::Text(
                json!(["CLOSED", subscription_id, "auth-required"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "unknown-field-reflection-challenge"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let auth_message = websocket.next().await.unwrap().unwrap();
        let auth: Value = serde_json::from_str(auth_message.to_text().unwrap()).unwrap();
        let auth_event: Event = serde_json::from_value(auth[1].clone()).unwrap();
        websocket
            .send(Message::Text(
                json!(["OK", auth_event.id.to_hex(), true, ""])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        let second_request = websocket.next().await.unwrap().unwrap();
        let second_request: Value =
            serde_json::from_str(second_request.to_text().unwrap()).unwrap();
        let second_subscription = second_request[1].clone();
        let authority_signature = auth_event
            .tags
            .iter()
            .find_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().map(String::as_str) == Some("auth"))
                    .then(|| parts.last().cloned())
                    .flatten()
            })
            .unwrap();
        let (field_name, private_signature) = match reflection {
            AuthFieldReflection::UppercaseEventSignature => (
                auth_event.sig.to_string().to_ascii_uppercase(),
                auth_event.sig.to_string(),
            ),
            AuthFieldReflection::SplitAuthoritySignature => {
                let midpoint = authority_signature.len() / 2;
                (
                    format!(
                        "{}--{}",
                        &authority_signature[..midpoint],
                        &authority_signature[midpoint..]
                    ),
                    authority_signature,
                )
            }
        };
        let mut reflected_event = serde_json::to_value(&event).unwrap();
        reflected_event
            .as_object_mut()
            .unwrap()
            .insert(field_name, json!("relay-controlled"));
        websocket
            .send(Message::Text(
                json!(["EVENT", second_subscription, reflected_event])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        websocket
            .send(Message::Text(
                json!(["EOSE", second_subscription]).to_string().into(),
            ))
            .await
            .unwrap();
        private_signature
    });
    (relay, server)
}

async fn assert_post_auth_field_is_static(reflection: AuthFieldReflection) {
    let event = signed_event("transformed auth reflection");
    let event_id = event.id;
    let agent_keys = Keys::generate();
    let private_key = agent_keys.secret_key().to_secret_hex();
    let owner_keys = Keys::generate();
    let auth_tag =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=1")
            .unwrap();
    let (relay, server) = authenticated_unknown_field_relay(event, reflection).await;

    let output =
        run_buzz_with_identity(&relay, &event_id.to_hex(), &private_key, Some(&auth_tag)).await;
    let private_signature = server.await.unwrap();

    assert_failure(&output, 4, "relay_mismatch");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(
        error["message"],
        "relay mismatch: relay response did not match the exact-event request after private authentication"
    );
    let normalized_stderr = stderr.to_ascii_lowercase().replace("--", "");
    assert!(
        !normalized_stderr.contains(&private_signature.to_ascii_lowercase()),
        "post-AUTH relay-controlled field leaked transformed private bytes: {stderr}"
    );
}

#[tokio::test]
async fn uppercased_post_auth_field_cannot_escape_through_cli_errors() {
    assert_post_auth_field_is_static(AuthFieldReflection::UppercaseEventSignature).await;
}

#[tokio::test]
async fn split_post_auth_field_cannot_escape_through_cli_errors() {
    assert_post_auth_field_is_static(AuthFieldReflection::SplitAuthoritySignature).await;
}

async fn ping_stall_relay() -> (String, Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_seen = Arc::new(AtomicBool::new(false));
    let request_seen_by_server = Arc::clone(&request_seen);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = websocket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        assert_eq!(request[0], "REQ");
        request_seen_by_server.store(true, Ordering::SeqCst);

        loop {
            if websocket
                .send(Message::Ping(Vec::new().into()))
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    (format!("ws://{address}"), request_seen, server)
}

async fn handshake_stall_relay() -> (String, Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connected = Arc::new(AtomicBool::new(false));
    let connected_by_server = Arc::clone(&connected);
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        connected_by_server.store(true, Ordering::SeqCst);
        std::future::pending::<()>().await;
    });
    (format!("ws://{address}"), connected, server)
}

async fn auth_stall_relay() -> (String, Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let auth_seen = Arc::new(AtomicBool::new(false));
    let auth_seen_by_server = Arc::clone(&auth_seen);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        websocket
            .send(Message::Text(
                json!(["AUTH", "deadline-challenge"]).to_string().into(),
            ))
            .await
            .unwrap();

        let first = websocket.next().await.unwrap().unwrap();
        let first: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
        assert_eq!(first[0], "REQ");
        let subscription_id = first[1].clone();
        websocket
            .send(Message::Text(
                json!(["CLOSED", subscription_id, "auth-required: deadline"])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let second = websocket.next().await.unwrap().unwrap();
        let second: Value = serde_json::from_str(second.to_text().unwrap()).unwrap();
        assert_eq!(second[0], "AUTH");
        auth_seen_by_server.store(true, Ordering::SeqCst);
        std::future::pending::<()>().await;
    });
    (format!("ws://{address}"), auth_seen, server)
}

async fn deadline_bounded_output(
    output: impl std::future::Future<Output = Output>,
) -> (Duration, Output) {
    const TEST_CEILING: Duration = Duration::from_millis(16_500);
    let started = Instant::now();
    let output = timeout(TEST_CEILING, output)
        .await
        .expect("command exceeded the 15-second transaction deadline");
    (started.elapsed(), output)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_deadline_bounds_ping_connect_and_auth_stalls() {
    let event = signed_event("deadline");
    let event_id = event.id.to_hex();
    let keys = Keys::generate();
    let private_key = keys.secret_key().to_secret_hex();
    let (ping_relay, ping_request_seen, ping_server) = ping_stall_relay().await;
    let (connect_relay, connect_seen, connect_server) = handshake_stall_relay().await;
    let (auth_relay, auth_seen, auth_server) = auth_stall_relay().await;

    let (ping, connect, auth) = tokio::join!(
        deadline_bounded_output(run_buzz(&ping_relay, &event_id)),
        deadline_bounded_output(run_buzz(&connect_relay, &event_id)),
        deadline_bounded_output(run_buzz_with_private_key(
            &auth_relay,
            &event_id,
            &private_key
        )),
    );

    for (name, (elapsed, output)) in [("PING", ping), ("connect", connect), ("AUTH", auth)] {
        assert!(
            elapsed >= Duration::from_secs(14),
            "{name} stall ended vacuously after {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(16_500),
            "{name} stall exceeded deadline: {elapsed:?}"
        );
        assert_failure(&output, 2, "transport_error");
    }

    assert!(ping_request_seen.load(Ordering::SeqCst));
    assert!(connect_seen.load(Ordering::SeqCst));
    assert!(auth_seen.load(Ordering::SeqCst));
    ping_server.abort();
    connect_server.abort();
    auth_server.abort();
}
