use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};

const SIGNATURE_VERSION: &str = "v0";
const SIGNATURE_MAX_AGE: Duration = Duration::from_secs(5 * 60);
const CALLBACK_COMPLETION_TIMEOUT: Duration = Duration::from_millis(2_500);
const MAX_WEBHOOK_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SlackEvent {
    Message {
        event_id: String,
        team_id: String,
        channel_id: String,
        user_id: String,
        text: String,
        ts: String,
        thread_ts: Option<String>,
        is_ext_shared: Option<bool>,
    },
    ChannelIdChanged {
        event_id: String,
        team_id: String,
        old_channel_id: String,
        new_channel_id: String,
    },
    ChannelShared {
        event_id: String,
        team_id: String,
        channel_id: String,
    },
    ChannelUnshared {
        event_id: String,
        team_id: String,
        channel_id: String,
        is_ext_shared: bool,
    },
}

pub(crate) struct SlackDelivery {
    pub(crate) event: SlackEvent,
    pub(crate) completion: oneshot::Sender<Result<(), String>>,
}

#[derive(Clone)]
pub(crate) struct WebhookControl {
    ready: Arc<AtomicBool>,
}

impl WebhookControl {
    pub(crate) fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Release);
    }
}

#[derive(Clone)]
pub(crate) struct WebhookServerState {
    signing_secret: Arc<[u8]>,
    delivery_tx: mpsc::Sender<SlackDelivery>,
    ready: Arc<AtomicBool>,
}

impl WebhookServerState {
    pub(crate) fn new(
        signing_secret: String,
        delivery_tx: mpsc::Sender<SlackDelivery>,
    ) -> (Self, WebhookControl) {
        let ready = Arc::new(AtomicBool::new(false));
        (
            Self {
                signing_secret: Arc::from(signing_secret.into_bytes()),
                delivery_tx,
                ready: Arc::clone(&ready),
            },
            WebhookControl { ready },
        )
    }
}

pub(crate) async fn run_webhook_server(
    listener: tokio::net::TcpListener,
    state: WebhookServerState,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/slack/events", post(slack_events))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BYTES))
        .with_state(state);
    let listen_addr = listener
        .local_addr()
        .context("failed to read Slack webhook listener address")?;
    info!(%listen_addr, "Slack webhook listener ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown.borrow() {
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .context("Slack webhook server failed")
}

async fn healthz(State(state): State<WebhookServerState>) -> StatusCode {
    if state.ready.load(Ordering::Acquire) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn slack_events(
    State(state): State<WebhookServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = verify_signature(&state.signing_secret, &headers, &body, SystemTime::now())
    {
        warn!(reason = %error, "rejected Slack webhook signature");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match payload.get("type").and_then(Value::as_str) {
        Some("url_verification") => {
            let Some(challenge) = payload.get("challenge").and_then(Value::as_str) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            return Json(json!({ "challenge": challenge })).into_response();
        }
        Some("event_callback") => {}
        _ => return StatusCode::OK.into_response(),
    }

    let event = match parse_callback(&payload) {
        Ok(Some(event)) => event,
        Ok(None) => return StatusCode::OK.into_response(),
        Err(error) => {
            warn!(reason = %error, "rejected malformed Slack event callback");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    if !state.ready.load(Ordering::Acquire) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let (completion, done) = oneshot::channel();
    match state
        .delivery_tx
        .try_send(SlackDelivery { event, completion })
    {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            warn!("Slack delivery queue is full; asking Slack to retry");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }

    match tokio::time::timeout(CALLBACK_COMPLETION_TIMEOUT, done).await {
        Ok(Ok(Ok(()))) => StatusCode::OK.into_response(),
        Ok(Ok(Err(reason))) => {
            warn!(%reason, "Slack callback processing failed; asking Slack to retry");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        Ok(Err(_)) | Err(_) => {
            warn!("Slack callback processing did not complete in time; asking Slack to retry");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn verify_signature(
    signing_secret: &[u8],
    headers: &HeaderMap,
    body: &[u8],
    now: SystemTime,
) -> Result<()> {
    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|value| value.to_str().ok())
        .context("missing Slack request timestamp")?;
    let timestamp_secs = timestamp
        .parse::<u64>()
        .context("invalid Slack request timestamp")?;
    let now_secs = now
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    if now_secs.abs_diff(timestamp_secs) > SIGNATURE_MAX_AGE.as_secs() {
        anyhow::bail!("Slack request timestamp is outside the replay window");
    }

    let signature = headers
        .get("x-slack-signature")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("v0="))
        .context("missing Slack v0 signature")?;
    let signature = hex::decode(signature).context("invalid Slack signature encoding")?;

    let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret)
        .context("failed to initialize Slack signature verifier")?;
    mac.update(SIGNATURE_VERSION.as_bytes());
    mac.update(b":");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    mac.verify_slice(&signature)
        .context("Slack signature mismatch")
}

fn parse_callback(payload: &Value) -> Result<Option<SlackEvent>> {
    let event_id = required(payload, "event_id")?;
    let team_id = required(payload, "team_id")?;
    let event = payload
        .get("event")
        .context("event callback omitted event")?;
    let event_type = required(event, "type")?;

    let parsed = match event_type.as_str() {
        "message" => parse_message(payload, event_id, team_id, event)?,
        "channel_id_changed" => Some(SlackEvent::ChannelIdChanged {
            event_id,
            team_id,
            old_channel_id: required(event, "old_channel_id")?,
            new_channel_id: required(event, "new_channel_id")?,
        }),
        "channel_shared" => Some(SlackEvent::ChannelShared {
            event_id,
            team_id,
            channel_id: required(event, "channel")?,
        }),
        "channel_unshared" => Some(SlackEvent::ChannelUnshared {
            event_id,
            team_id,
            channel_id: required(event, "channel")?,
            is_ext_shared: required_bool(event, "is_ext_shared")?,
        }),
        _ => None,
    };
    Ok(parsed)
}

fn parse_message(
    payload: &Value,
    event_id: String,
    team_id: String,
    event: &Value,
) -> Result<Option<SlackEvent>> {
    if event.get("bot_id").is_some() || event.get("hidden").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let subtype = event.get("subtype").and_then(Value::as_str);
    if subtype.is_some_and(|value| value != "thread_broadcast") {
        return Ok(None);
    }

    Ok(Some(SlackEvent::Message {
        event_id,
        team_id,
        channel_id: required(event, "channel")?,
        user_id: required(event, "user")?,
        text: required_allow_empty(event, "text")?,
        ts: required(event, "ts")?,
        thread_ts: event
            .get("thread_ts")
            .and_then(Value::as_str)
            .map(str::to_owned),
        is_ext_shared: payload
            .get("is_ext_shared_channel")
            .and_then(Value::as_bool),
    }))
}

fn required(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("Slack callback omitted {field}"))
}

fn required_allow_empty(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("Slack callback omitted {field}"))
}

fn required_bool(value: &Value, field: &str) -> Result<bool> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .with_context(|| format!("Slack callback omitted {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::HeaderValue;

    fn signed_headers(secret: &[u8], timestamp: u64, body: &[u8]) -> HeaderMap {
        let timestamp = timestamp.to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-slack-request-timestamp",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(
            "x-slack-signature",
            HeaderValue::from_str(&signature).unwrap(),
        );
        headers
    }

    fn current_signed_headers(secret: &[u8], body: &[u8]) -> HeaderMap {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        signed_headers(secret, timestamp, body)
    }

    #[test]
    fn accepts_valid_signature_and_rejects_tampering() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let body = br#"{"type":"event_callback"}"#;
        let headers = signed_headers(b"secret", 1_700_000_000, body);
        verify_signature(b"secret", &headers, body, now).unwrap();
        assert!(verify_signature(b"secret", &headers, b"tampered", now).is_err());
    }

    #[test]
    fn rejects_replayed_signature() {
        let body = br#"{}"#;
        let headers = signed_headers(b"secret", 1_700_000_000, body);
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_301);
        let error = verify_signature(b"secret", &headers, body, now)
            .unwrap_err()
            .to_string();
        assert!(error.contains("replay window"), "{error}");
    }

    #[test]
    fn parses_external_thread_message() {
        let payload = json!({
            "type": "event_callback",
            "event_id": "Ev123",
            "team_id": "T12345678",
            "is_ext_shared_channel": true,
            "event": {
                "type": "message",
                "channel": "C12345678",
                "user": "U12345678",
                "text": "hello",
                "ts": "1700000000.000001",
                "thread_ts": "1699999999.000001"
            }
        });
        assert_eq!(
            parse_callback(&payload).unwrap(),
            Some(SlackEvent::Message {
                event_id: "Ev123".into(),
                team_id: "T12345678".into(),
                channel_id: "C12345678".into(),
                user_id: "U12345678".into(),
                text: "hello".into(),
                ts: "1700000000.000001".into(),
                thread_ts: Some("1699999999.000001".into()),
                is_ext_shared: Some(true),
            })
        );
    }

    #[test]
    fn parses_shared_channel_lifecycle_events() {
        let unshared = json!({
            "event_id": "Ev-unshared",
            "team_id": "T12345678",
            "event": {
                "type": "channel_unshared",
                "channel": "C12345678",
                "is_ext_shared": true
            }
        });
        assert_eq!(
            parse_callback(&unshared).unwrap(),
            Some(SlackEvent::ChannelUnshared {
                event_id: "Ev-unshared".into(),
                team_id: "T12345678".into(),
                channel_id: "C12345678".into(),
                is_ext_shared: true,
            })
        );

        let changed = json!({
            "event_id": "Ev-changed",
            "team_id": "T12345678",
            "event": {
                "type": "channel_id_changed",
                "old_channel_id": "C12345678",
                "new_channel_id": "C87654321"
            }
        });
        assert_eq!(
            parse_callback(&changed).unwrap(),
            Some(SlackEvent::ChannelIdChanged {
                event_id: "Ev-changed".into(),
                team_id: "T12345678".into(),
                old_channel_id: "C12345678".into(),
                new_channel_id: "C87654321".into(),
            })
        );
    }

    #[tokio::test]
    async fn handles_signed_url_verification_before_ready() {
        let (delivery_tx, _delivery_rx) = mpsc::channel(1);
        let (state, _control) = WebhookServerState::new("secret".into(), delivery_tx);
        let body =
            Bytes::from_static(br#"{"type":"url_verification","challenge":"slack-challenge"}"#);
        let response =
            slack_events(State(state), current_signed_headers(b"secret", &body), body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"challenge": "slack-challenge"})
        );
    }

    #[tokio::test]
    async fn asks_slack_to_retry_when_bridge_is_not_ready() {
        let (delivery_tx, _delivery_rx) = mpsc::channel(1);
        let (state, _control) = WebhookServerState::new("secret".into(), delivery_tx);
        let body = Bytes::from_static(
            br#"{
                "type":"event_callback",
                "event_id":"Ev123",
                "team_id":"T12345678",
                "event":{
                    "type":"message",
                    "channel":"C12345678",
                    "user":"U12345678",
                    "text":"hello",
                    "ts":"1700000000.000001"
                }
            }"#,
        );
        let response =
            slack_events(State(state), current_signed_headers(b"secret", &body), body).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn acknowledges_callback_after_durable_processing() {
        let (delivery_tx, mut delivery_rx) = mpsc::channel(1);
        let (state, control) = WebhookServerState::new("secret".into(), delivery_tx);
        control.set_ready(true);
        let body = Bytes::from_static(
            br#"{
                "type":"event_callback",
                "event_id":"Ev123",
                "team_id":"T12345678",
                "event":{
                    "type":"message",
                    "channel":"C12345678",
                    "user":"U12345678",
                    "text":"hello",
                    "ts":"1700000000.000001"
                }
            }"#,
        );

        let processing = tokio::spawn(async move {
            let delivery = delivery_rx.recv().await.unwrap();
            assert!(matches!(delivery.event, SlackEvent::Message { .. }));
            delivery.completion.send(Ok(())).unwrap();
        });
        let response =
            slack_events(State(state), current_signed_headers(b"secret", &body), body).await;
        processing.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn ignores_bot_and_edit_events() {
        let base = json!({
            "event_id": "Ev123",
            "team_id": "T12345678",
            "event": {
                "type": "message",
                "channel": "C12345678",
                "user": "U12345678",
                "text": "hello",
                "ts": "1700000000.000001",
                "bot_id": "B123"
            }
        });
        assert_eq!(parse_callback(&base).unwrap(), None);

        let mut edit = base;
        edit["event"]["bot_id"] = Value::Null;
        edit["event"]["subtype"] = Value::String("message_changed".into());
        assert_eq!(parse_callback(&edit).unwrap(), None);
    }
}
