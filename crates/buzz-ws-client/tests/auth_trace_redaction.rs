use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, RelayUrl, Tag};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};

const RELAY_ENV: &str = "BUZZ_WS_AUTH_TRACE_TEST_RELAY";
const PRIVATE_KEY_ENV: &str = "BUZZ_WS_AUTH_TRACE_TEST_PRIVATE_KEY";
const AUTH_TAG_ENV: &str = "BUZZ_WS_AUTH_TRACE_TEST_TAG";

// Runs only in the explicitly spawned child process. Keeping the trace bridge
// out of the parent means the local test relay's inbound Tungstenite trace is
// not confused with the client-side boundary under test.
#[tokio::test]
#[ignore = "subprocess helper for auth_trace_hides_private_event_from_tungstenite"]
async fn auth_trace_client_helper() {
    assert_eq!(log::STATIC_MAX_LEVEL, log::LevelFilter::Info);
    let relay = std::env::var(RELAY_ENV).unwrap();
    let keys = Keys::parse(&std::env::var(PRIVATE_KEY_ENV).unwrap()).unwrap();
    let tag_parts: Vec<String> =
        serde_json::from_str(&std::env::var(AUTH_TAG_ENV).unwrap()).unwrap();
    let auth_tag = Tag::parse(tag_parts).unwrap();

    tracing_log::LogTracer::init().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let mut connection = NostrWsConnection::connect(&relay).await.unwrap();
    connection
        .authenticate(&keys, Some(&auth_tag))
        .await
        .unwrap();
    connection
        .send_raw(&json!([
            "REQ",
            "post-auth-diagnostic",
            {"kinds": [1], "limit": 1}
        ]))
        .await
        .unwrap();
    assert!(matches!(
        connection.next_event(Duration::from_secs(2)).await.unwrap(),
        RelayMessage::Event { .. }
    ));
    assert!(matches!(
        connection.next_event(Duration::from_secs(2)).await.unwrap(),
        RelayMessage::Eose { .. }
    ));
    connection.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "subprocess helper for malformed_auth_echo_is_sanitized_in_error_logs"]
async fn auth_error_log_client_helper() {
    assert_eq!(log::STATIC_MAX_LEVEL, log::LevelFilter::Info);
    let relay = std::env::var(RELAY_ENV).unwrap();
    let keys = Keys::parse(&std::env::var(PRIVATE_KEY_ENV).unwrap()).unwrap();
    let tag_parts: Vec<String> =
        serde_json::from_str(&std::env::var(AUTH_TAG_ENV).unwrap()).unwrap();
    let auth_tag = Tag::parse(tag_parts).unwrap();

    tracing_log::LogTracer::init().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let mut connection = NostrWsConnection::connect(&relay).await.unwrap();
    let error = connection
        .authenticate(&keys, Some(&auth_tag))
        .await
        .unwrap_err();
    tracing::error!(error = %error, "authentication failed");
    assert!(matches!(error, WsClientError::ReflectedAuthMaterial));
}

#[tokio::test]
async fn auth_trace_hides_private_event_from_tungstenite() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());
    let keys = Keys::generate();
    let auth_tag_parts = vec![
        "auth".to_string(),
        "private-owner-authority-9f8e7d6c".to_string(),
        "private-condition-kind-verified".to_string(),
        "private-reusable-signature-1a2b3c4d".to_string(),
    ];
    let auth_tag_json = serde_json::to_string(&auth_tag_parts).unwrap();

    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "auth_trace_client_helper",
            "--nocapture",
        ])
        .env(RELAY_ENV, &relay)
        .env(PRIVATE_KEY_ENV, keys.secret_key().to_secret_hex())
        .env(AUTH_TAG_ENV, &auth_tag_json)
        .env(
            "RUST_LOG",
            "tungstenite=trace,tokio_tungstenite=trace,buzz_ws_client=trace",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let (stream, _) = timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
    websocket
        .send(Message::Text(
            json!(["AUTH", "trace-capture-challenge"])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // A real Tungstenite server proves the hand-built client frame is masked,
    // valid UTF-8 JSON, and exactly the expected signed AUTH envelope.
    let auth_message = timeout(Duration::from_secs(2), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let auth_text = auth_message.to_text().unwrap().to_string();
    let auth_json: Value = serde_json::from_str(&auth_text).unwrap();
    assert_eq!(auth_json[0], "AUTH");
    let auth_event: Event = serde_json::from_value(auth_json[1].clone()).unwrap();
    auth_event.verify().unwrap();
    assert!(auth_event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == auth_tag_parts.as_slice()));

    websocket
        .send(Message::Text(
            json!(["OK", auth_event.id.to_hex(), true, ""])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // Tungstenite remains usable after the raw AUTH write: the authenticated
    // REQ and its EVENT/EOSE response complete on the original stream.
    let request = timeout(Duration::from_secs(2), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
    assert_eq!(request[0], "REQ");
    assert_eq!(request[1], "post-auth-diagnostic");
    let event = EventBuilder::new(Kind::TextNote, "post-auth response")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    websocket
        .send(Message::Text(
            json!(["EVENT", "post-auth-diagnostic", event])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    websocket
        .send(Message::Text(
            json!(["EOSE", "post-auth-diagnostic"]).to_string().into(),
        ))
        .await
        .unwrap();

    let output = timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}{stderr}");
    assert!(output.status.success(), "trace helper failed:\n{logs}");
    assert!(
        logs.contains("connected to relay") && logs.contains("post-auth-diagnostic"),
        "nonsecret tracing controls were vacuous:\n{logs}"
    );
    assert!(logs.contains("AUTH <redacted>"));

    let auth_event_json = auth_event.as_json();
    for private_bytes in auth_tag_parts.iter().skip(1).map(String::as_str).chain([
        auth_tag_json.as_str(),
        auth_text.as_str(),
        auth_event_json.as_str(),
    ]) {
        assert!(
            !logs.contains(private_bytes),
            "Tungstenite/log bridge leaked private AUTH bytes `{private_bytes}`:\n{logs}"
        );
    }
}

#[tokio::test]
async fn malformed_auth_echo_is_sanitized_in_error_logs() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());
    let keys = Keys::generate();
    let auth_tag_parts = vec![
        "auth".to_string(),
        "echo-private-owner-authority-9f8e7d6c".to_string(),
        "echo-private-condition-kind-verified".to_string(),
        "echo-private-reusable-signature-1a2b3c4d".to_string(),
    ];
    let auth_tag_json = serde_json::to_string(&auth_tag_parts).unwrap();

    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "auth_error_log_client_helper",
            "--nocapture",
        ])
        .env(RELAY_ENV, &relay)
        .env(PRIVATE_KEY_ENV, keys.secret_key().to_secret_hex())
        .env(AUTH_TAG_ENV, &auth_tag_json)
        .env(
            "RUST_LOG",
            "tungstenite=trace,tokio_tungstenite=trace,buzz_ws_client=trace,auth_trace_redaction=trace",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let (stream, _) = timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
    websocket
        .send(Message::Text(
            json!(["AUTH", "malformed-log-echo-challenge"])
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let auth_message = timeout(Duration::from_secs(2), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let auth_text = auth_message.to_text().unwrap().to_string();
    let auth_json: Value = serde_json::from_str(&auth_text).unwrap();
    let auth_event: Event = serde_json::from_value(auth_json[1].clone()).unwrap();
    auth_event.verify().unwrap();
    websocket
        .send(Message::Text(format!("['AUTH',{}]", auth_json[1]).into()))
        .await
        .unwrap();

    let output = timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{stdout}{stderr}");
    assert!(output.status.success(), "error-log helper failed:\n{logs}");
    assert!(
        logs.contains("relay reflected private authentication material"),
        "sanitized log control was vacuous:\n{logs}"
    );
    assert!(
        logs.contains("connected to relay") && logs.contains("AUTH <redacted>"),
        "nonsecret tracing controls were vacuous:\n{logs}"
    );

    let auth_event_json = auth_event.as_json();
    let signature = auth_json[1]["sig"].as_str().unwrap();
    for private_bytes in auth_tag_parts.iter().skip(1).map(String::as_str).chain([
        auth_tag_json.as_str(),
        auth_text.as_str(),
        auth_event_json.as_str(),
        signature,
    ]) {
        assert!(
            !stdout.contains(private_bytes),
            "post-AUTH reflection reached helper stdout: `{private_bytes}`:\n{stdout}"
        );
        assert!(
            !stderr.contains(private_bytes),
            "post-AUTH reflection reached dependency/application stderr logs: `{private_bytes}`:\n{stderr}"
        );
    }
}

#[tokio::test]
async fn direct_send_raw_auth_is_rejected_before_transmission() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());
    let relay_url = RelayUrl::parse(&relay).unwrap();
    let auth_event = EventBuilder::auth("direct-send-raw-challenge", relay_url)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let private_signature = auth_event.sig.to_string();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let Ok(Some(Ok(auth_message))) =
            timeout(Duration::from_millis(250), websocket.next()).await
        else {
            return false;
        };
        let auth: Value = serde_json::from_str(auth_message.to_text().unwrap()).unwrap();
        assert_eq!(auth[0], "AUTH");
        let signature = auth[1]["sig"].as_str().unwrap();
        websocket
            .send(Message::Text(
                json!(["NOTICE", format!("echo:{signature}")])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        true
    });

    let mut connection = NostrWsConnection::connect(&relay).await.unwrap();
    let result = connection.send_raw(&json!(["AUTH", auth_event])).await;
    let safely_rejected = matches!(
        &result,
        Err(WsClientError::AuthFailed(message))
            if message == "raw AUTH messages are not accepted; use authenticate"
    );
    assert!(
        safely_rejected,
        "direct send_raw AUTH was not rejected with the static API error"
    );
    if let Err(error) = result {
        assert!(!error.to_string().contains(&private_signature));
    }
    assert!(!connection.private_auth_started());
    assert!(
        !server.await.unwrap(),
        "rejected direct send_raw AUTH still reached the relay"
    );
}
