//! Best-effort usage-event delivery for relay operators.
//!
//! The exporter is deliberately small: it emits one privacy-minimal JSON
//! object for each newly persisted client-submitted stream message (Nostr kind
//! 9). Relay-authored automation is intentionally excluded. Delivery never
//! blocks event ingestion; a bounded queue drops analytics work when its single
//! worker cannot keep up.

use std::time::Duration;

use buzz_core::kind::{event_kind_u32, KIND_STREAM_MESSAGE};
use nostr::Event;
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::warn;

const EVENT_TYPE_MESSAGE_SENT: &str = "message_sent";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Fixed queue depth. Usage analytics must not add backpressure to ingestion.
pub(crate) const QUEUE_CAPACITY: usize = 1024;

/// Minimal usage event sent to the configured webhook.
#[derive(Clone, Serialize)]
pub(crate) struct UsageEvent {
    schema_version: u8,
    #[serde(rename = "type")]
    event_type: &'static str,
    accepted_at_ms: i64,
    actor_id: String,
    community_id: String,
}

/// Build an event for a kind:9 message, or `None` for every other kind.
///
/// This reads only the event kind and author public key. Content, tags, event
/// identifiers, signatures, and channel or thread identifiers are not used.
pub(crate) fn message_sent(
    event: &Event,
    community_id: &str,
    accepted_at_ms: i64,
) -> Option<UsageEvent> {
    if event_kind_u32(event) != KIND_STREAM_MESSAGE {
        return None;
    }

    Some(UsageEvent {
        schema_version: 1,
        event_type: EVENT_TYPE_MESSAGE_SENT,
        accepted_at_ms,
        actor_id: event.pubkey.to_hex(),
        community_id: community_id.to_owned(),
    })
}

/// Non-blocking handle to the single usage-analytics delivery worker.
#[derive(Clone)]
pub(crate) struct UsageAnalyticsSender {
    tx: mpsc::Sender<UsageEvent>,
}

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

impl UsageAnalyticsSender {
    /// Start one worker with a bounded queue.
    pub(crate) fn spawn(endpoint: url::Url, capacity: usize) -> Result<Self, reqwest::Error> {
        let client = build_client()?;
        let (tx, mut rx) = mpsc::channel::<UsageEvent>(capacity);

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                post(&client, &endpoint, &event).await;
            }
        });

        Ok(Self { tx })
    }

    /// Enqueue without waiting. Returns `false` when the event was dropped.
    pub(crate) fn try_send(&self, event: UsageEvent) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                metrics::counter!(
                    "buzz_usage_analytics_events_dropped_total",
                    "reason" => "queue_full"
                )
                .increment(1);
                warn!("usage analytics event dropped: queue full");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                metrics::counter!(
                    "buzz_usage_analytics_events_dropped_total",
                    "reason" => "worker_stopped"
                )
                .increment(1);
                warn!("usage analytics event dropped: worker stopped");
                false
            }
        }
    }
}

async fn post(client: &reqwest::Client, endpoint: &url::Url, event: &UsageEvent) {
    match client.post(endpoint.clone()).json(event).send().await {
        Ok(response) if response.status().is_success() => {
            metrics::counter!(
                "buzz_usage_analytics_deliveries_total",
                "outcome" => "accepted"
            )
            .increment(1);
        }
        Ok(response) => {
            metrics::counter!(
                "buzz_usage_analytics_deliveries_total",
                "outcome" => "http_error"
            )
            .increment(1);
            warn!(
                status = response.status().as_u16(),
                "usage analytics webhook rejected an event"
            );
        }
        Err(error) => {
            metrics::counter!(
                "buzz_usage_analytics_deliveries_total",
                "outcome" => "transport_error"
            )
            .increment(1);
            warn!(
                error = %error.without_url(),
                "usage analytics webhook request failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use axum::{
        body::Bytes, extract::State, http::StatusCode, response::Redirect,
        routing::post as post_route, Router,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use tokio::sync::Semaphore;

    use super::*;

    const ACCEPTED_AT_MS: i64 = 1_752_000_000_000;
    const PRIVATE_CONTENT: &str = "private message for marker@example.invalid";
    const CHANNEL_ID: &str = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
    const THREAD_ID: &str = "aabbccddeeff00112233445566778899aabbccddeeff001122334455667788ff";
    const PROFILE_NAME: &str = "Synthetic Profile";
    const NIP05_IDENTIFIER: &str = "nip05-marker@example.invalid";
    const IP_ADDRESS: &str = "203.0.113.42";
    const DEVICE_ID: &str = "device-installation-abc123";
    const HOSTNAME: &str = "member.example.net";

    fn signed_event(keys: &Keys, kind: u32) -> Event {
        EventBuilder::new(Kind::from(kind as u16), PRIVATE_CONTENT)
            .tags([
                Tag::parse(["h", CHANNEL_ID]).expect("valid h tag"),
                Tag::parse(["e", THREAD_ID]).expect("valid e tag"),
                Tag::parse(["profile", PROFILE_NAME]).expect("valid profile tag"),
                Tag::parse(["nip05", NIP05_IDENTIFIER]).expect("valid NIP-05 tag"),
                Tag::parse(["ip", IP_ADDRESS]).expect("valid IP tag"),
                Tag::parse(["device", DEVICE_ID]).expect("valid device tag"),
                Tag::parse(["hostname", HOSTNAME]).expect("valid hostname tag"),
            ])
            .sign_with_keys(keys)
            .expect("sign event")
    }

    fn sample_event(keys: &Keys) -> UsageEvent {
        message_sent(
            &signed_event(keys, KIND_STREAM_MESSAGE),
            "community-opaque-1",
            ACCEPTED_AT_MS,
        )
        .expect("kind:9 produces usage event")
    }

    async fn serve(app: Router) -> url::Url {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind capture server");
        let endpoint = format!(
            "http://{}/events",
            listener.local_addr().expect("capture address")
        )
        .parse()
        .expect("capture URL");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        endpoint
    }

    async fn next<T>(rx: &mut mpsc::Receiver<T>, description: &str) -> T {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {description}"))
            .unwrap_or_else(|| panic!("channel closed waiting for {description}"))
    }

    #[test]
    fn payload_is_exact_and_excludes_message_details() {
        let keys = Keys::generate();
        let event = signed_event(&keys, KIND_STREAM_MESSAGE);
        let usage = message_sent(&event, "community-opaque-1", ACCEPTED_AT_MS)
            .expect("kind:9 produces usage event");

        assert_eq!(
            serde_json::to_value(&usage).expect("serialize"),
            serde_json::json!({
                "schema_version": 1,
                "type": "message_sent",
                "accepted_at_ms": 1_752_000_000_000i64,
                "actor_id": keys.public_key().to_hex(),
                "community_id": "community-opaque-1"
            })
        );

        let wire = serde_json::to_string(&usage).expect("serialize");
        for (label, forbidden) in [
            ("message content", PRIVATE_CONTENT),
            ("email address", "marker@example.invalid"),
            ("channel id", CHANNEL_ID),
            ("thread id", THREAD_ID),
            ("profile name", PROFILE_NAME),
            ("NIP-05 identifier", NIP05_IDENTIFIER),
            ("IP address", IP_ADDRESS),
            ("device id", DEVICE_ID),
            ("hostname", HOSTNAME),
            ("Nostr event id", event.id.to_hex().as_str()),
            ("signature", event.sig.to_string().as_str()),
        ] {
            assert!(!wire.contains(forbidden), "{label} must not be exported");
        }
    }

    #[test]
    fn only_kind_nine_produces_an_event() {
        use buzz_core::kind::{
            KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_V2, KIND_SYSTEM_MESSAGE, KIND_TEXT_NOTE,
        };

        let keys = Keys::generate();
        for kind in [
            0,
            KIND_TEXT_NOTE,
            7,
            8,
            10,
            KIND_STREAM_MESSAGE_V2,
            KIND_STREAM_MESSAGE_EDIT,
            KIND_SYSTEM_MESSAGE,
        ] {
            assert!(
                message_sent(&signed_event(&keys, kind), "community", ACCEPTED_AT_MS).is_none(),
                "kind:{kind} must not produce a usage event"
            );
        }
    }

    #[tokio::test]
    async fn full_queue_drops_without_blocking_and_worker_continues_after_failure() {
        struct Gate {
            arrived: mpsc::Sender<()>,
            hits: AtomicUsize,
            release: Semaphore,
        }

        async fn gated(State(gate): State<Arc<Gate>>, _body: Bytes) -> StatusCode {
            let hit = gate.hits.fetch_add(1, Ordering::SeqCst);
            let _ = gate.arrived.send(()).await;
            if hit == 0 {
                let _permit = gate.release.acquire().await.expect("gate open");
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::NO_CONTENT
            }
        }

        let (arrived_tx, mut arrived_rx) = mpsc::channel(4);
        let gate = Arc::new(Gate {
            arrived: arrived_tx,
            hits: AtomicUsize::new(0),
            release: Semaphore::new(0),
        });
        let endpoint = serve(
            Router::new()
                .route("/events", post_route(gated))
                .with_state(gate.clone()),
        )
        .await;
        let sender = UsageAnalyticsSender::spawn(endpoint, 1).expect("analytics client");
        let event = sample_event(&Keys::generate());

        assert!(sender.try_send(event.clone()));
        next(&mut arrived_rx, "first delivery").await;
        assert!(sender.try_send(event.clone()));
        assert!(!sender.try_send(event));

        gate.release.add_permits(1);
        next(&mut arrived_rx, "delivery after HTTP failure").await;
        assert_eq!(gate.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn client_refuses_redirects() {
        async fn destination(State(hits): State<Arc<AtomicUsize>>) -> StatusCode {
            hits.fetch_add(1, Ordering::SeqCst);
            StatusCode::NO_CONTENT
        }

        async fn redirect(State(destination): State<String>) -> Redirect {
            Redirect::temporary(&destination)
        }

        let hits = Arc::new(AtomicUsize::new(0));
        let destination = serve(
            Router::new()
                .route("/events", post_route(destination))
                .with_state(hits.clone()),
        )
        .await;
        let endpoint = serve(
            Router::new()
                .route("/events", post_route(redirect))
                .with_state(destination.to_string()),
        )
        .await;

        let client = build_client().expect("analytics client");
        post(&client, &endpoint, &sample_event(&Keys::generate())).await;

        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
}
