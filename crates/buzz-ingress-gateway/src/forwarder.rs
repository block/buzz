//! Background delivery of queued webhooks to community relays.
//!
//! ```text
//!   deliveries (state = queued, next_attempt_at <= now)
//!        │ claim_due: lease rows, bump attempts
//!        ▼
//!   build kind-20701 event ── created_at = received_at, so the id is the
//!        │                    same on every retry and the relay dedupes
//!        ▼
//!   RelayClient::submit ──▶ Accepted  → state accepted, payload dropped
//!                       ──▶ Rejected  → state failed
//!                       ──▶ Retry     → next_attempt_at = backoff(attempts),
//!                                       or failed once the budget is spent
//! ```
use crate::config::RelayScheme;
use crate::metrics::{record_forward, record_forwarder_failure, ForwardResult};
use crate::relay_client::{RelayClient, SubmitResult};
use crate::store::{ClaimedDelivery, ForwardOutcome, Store, StoreError};
use chrono::{DateTime, Duration, Utc};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp, Url};
use serde::Serialize;
use serde_json::value::RawValue;
use std::sync::Arc;
use thiserror::Error;
use tokio::task::JoinSet;

/// Event kind carrying one provider webhook into a community.
pub const WEBHOOK_CALL_KIND: Kind = Kind::Custom(20701);
const CONTENT_VERSION: u32 = 1;
const BACKOFF: [Duration; 6] = [
    Duration::seconds(1),
    Duration::seconds(5),
    Duration::seconds(30),
    Duration::minutes(2),
    Duration::minutes(10),
    Duration::minutes(30),
];
const CLAIM_LEASE: Duration = Duration::seconds(60);
const IDLE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// Why a claimed delivery could not be turned into an event.
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("stored payload is not JSON")]
    Payload,
    #[error("event signing failed: {0}")]
    Sign(#[from] nostr::event::builder::Error),
}

#[derive(Serialize)]
struct CallContent<'a> {
    version: u32,
    #[serde(rename = "type")]
    event_type: &'a str,
    delivery_id: &'a str,
    payload: &'a RawValue,
}

/// Delay before the next attempt after `attempts` tries so far.
pub fn next_attempt(attempts: i32, now: DateTime<Utc>) -> DateTime<Utc> {
    let index = usize::try_from(attempts.max(1) - 1).unwrap_or(usize::MAX);
    now + BACKOFF[index.min(BACKOFF.len() - 1)]
}

/// Build the signed webhook-call event for a claimed delivery.
///
/// `created_at` is pinned to the delivery's receive time so every retry
/// produces the same event id; the relay then treats a redelivery after a
/// lost acknowledgement as a duplicate rather than a second webhook.
pub fn build_call_event(keys: &Keys, delivery: &ClaimedDelivery) -> Result<Event, BuildError> {
    let payload = std::str::from_utf8(&delivery.body)
        .ok()
        .and_then(|text| serde_json::from_str::<&RawValue>(text).ok())
        .ok_or(BuildError::Payload)?;
    let content = serde_json::to_string(&CallContent {
        version: CONTENT_VERSION,
        event_type: &delivery.event_type,
        delivery_id: &delivery.delivery_id,
        payload,
    })
    .map_err(|_| BuildError::Payload)?;
    let created_at = u64::try_from(delivery.received_at.timestamp()).unwrap_or(0);
    Ok(EventBuilder::new(WEBHOOK_CALL_KIND, content)
        // A UUID, not an event ID, so not an `e` tag (NIP-01 gives `e` one
        // meaning across kinds). Subscriptions and records use the same name.
        .tag(Tag::custom(
            TagKind::custom("integration"),
            [delivery.integration_id.to_string()],
        ))
        .tag(Tag::hashtag(format!(
            "provider:{}",
            delivery.provider.as_str()
        )))
        .tag(Tag::custom(
            TagKind::custom("delivery"),
            [delivery.delivery_id.clone()],
        ))
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(keys)?)
}

/// Relay submission endpoint for a community.
pub fn relay_url(scheme: RelayScheme, community_host: &str) -> Option<Url> {
    let url = Url::parse(&format!("{}://{community_host}/events", scheme.as_str())).ok()?;
    (url.host_str() == Some(community_host)).then_some(url)
}

/// Drains queued deliveries to their community relays.
pub struct Forwarder {
    pub store: Arc<dyn Store>,
    /// Transport to the community relays.
    pub client: Arc<dyn RelayClient>,
    /// Gateway signing key; every webhook-call event carries its pubkey.
    pub keys: Keys,
    /// Scheme used to reach community hosts.
    pub scheme: RelayScheme,
    /// Deliveries claimed and sent per poll.
    pub concurrency: usize,
    /// How long after receipt a delivery keeps retrying before it fails.
    pub retry_max_age: Duration,
    /// Injectable clock so tests can drive the backoff schedule.
    pub now: fn() -> DateTime<Utc>,
}

impl Forwarder {
    /// Claim one batch and forward it. Returns how many deliveries were claimed.
    pub async fn run_once(&self) -> Result<usize, StoreError> {
        let now = (self.now)();
        let batch = self
            .store
            .claim_due(now, CLAIM_LEASE, self.concurrency.max(1))
            .await?;
        let claimed = batch.len();
        let mut tasks = JoinSet::new();
        for delivery in batch {
            let store = Arc::clone(&self.store);
            let client = Arc::clone(&self.client);
            let keys = self.keys.clone();
            let scheme = self.scheme;
            let retry_max_age = self.retry_max_age;
            let now_fn = self.now;
            tasks.spawn(async move {
                let outcome =
                    attempt(&*client, &keys, scheme, retry_max_age, now_fn, &delivery).await;
                if let Err(error) = store
                    .complete_forward(
                        delivery.integration_id,
                        &delivery.delivery_id,
                        outcome,
                        now_fn(),
                    )
                    .await
                {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway could not record forward outcome");
                }
            });
        }
        while tasks.join_next().await.is_some() {}
        Ok(claimed)
    }

    /// Poll the store until the task is aborted.
    pub async fn run(self) {
        loop {
            match self.run_once().await {
                Ok(0) => tokio::time::sleep(IDLE_POLL).await,
                Ok(_) => {}
                Err(error) => {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway forwarder poll failed");
                    tokio::time::sleep(IDLE_POLL).await;
                }
            }
        }
    }
}

async fn attempt(
    client: &dyn RelayClient,
    keys: &Keys,
    scheme: RelayScheme,
    retry_max_age: Duration,
    now: fn() -> DateTime<Utc>,
    delivery: &ClaimedDelivery,
) -> ForwardOutcome {
    let event = match build_call_event(keys, delivery) {
        Ok(event) => event,
        Err(error) => {
            record_forward(ForwardResult::Failed, None);
            return ForwardOutcome::Failed {
                event_id: None,
                error: error.to_string(),
            };
        }
    };
    let event_id = event.id.to_hex();
    let Some(url) = relay_url(scheme, &delivery.community_host) else {
        record_forward(ForwardResult::Failed, None);
        return ForwardOutcome::Failed {
            event_id: Some(event_id),
            error: "community host is not a valid relay URL".to_owned(),
        };
    };
    let started = std::time::Instant::now();
    let result = client.submit(&url, &event).await;
    let elapsed = started.elapsed().as_secs_f64();
    match result {
        SubmitResult::Accepted => {
            record_forward(ForwardResult::Accepted, Some(elapsed));
            ForwardOutcome::Accepted { event_id }
        }
        SubmitResult::Rejected(error) => {
            record_forward(ForwardResult::Failed, Some(elapsed));
            ForwardOutcome::Failed {
                event_id: Some(event_id),
                error,
            }
        }
        SubmitResult::Retry(error) => {
            let now = now();
            if delivery.received_at + retry_max_age <= now {
                record_forward(ForwardResult::Failed, Some(elapsed));
                return ForwardOutcome::Failed {
                    event_id: Some(event_id),
                    error: format!("retry budget exhausted: {error}"),
                };
            }
            record_forward(ForwardResult::Retry, Some(elapsed));
            ForwardOutcome::Retry {
                event_id: Some(event_id),
                error,
                next_attempt_at: next_attempt(delivery.attempts, now),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{DeliveryState, MemoryStore, Mutation, NewDelivery, Operation, Provider};
    use async_trait::async_trait;
    use chrono::TimeZone;
    use std::sync::Mutex;
    use uuid::Uuid;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap()
    }

    fn claimed(attempts: i32) -> ClaimedDelivery {
        ClaimedDelivery {
            integration_id: Uuid::nil(),
            delivery_id: "d-1".to_owned(),
            community_host: "acme.communities.example.org".to_owned(),
            provider: Provider::Github,
            event_type: "push".to_owned(),
            body: br#"{"ref":"refs/heads/main"}"#.to_vec(),
            received_at: t0(),
            attempts,
            event_id: None,
        }
    }

    #[test]
    fn event_is_deterministic_and_carries_routing_tags() {
        let keys = Keys::generate();
        let a = build_call_event(&keys, &claimed(1)).unwrap();
        let b = build_call_event(&keys, &claimed(3)).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, WEBHOOK_CALL_KIND);
        assert_eq!(a.created_at.as_secs(), t0().timestamp() as u64);
        let tags: Vec<Vec<String>> = a.tags.iter().map(|t| t.clone().to_vec()).collect();
        assert!(tags.contains(&vec!["integration".to_owned(), Uuid::nil().to_string()]));
        assert!(!tags.iter().any(|t| t[0] == "e"));
        assert!(tags.contains(&vec!["t".to_owned(), "provider:github".to_owned()]));
        assert!(tags.contains(&vec!["delivery".to_owned(), "d-1".to_owned()]));
        let content: serde_json::Value = serde_json::from_str(&a.content).unwrap();
        assert_eq!(content["version"], 1);
        assert_eq!(content["type"], "push");
        assert_eq!(content["delivery_id"], "d-1");
        assert_eq!(content["payload"]["ref"], "refs/heads/main");
        assert!(a.verify().is_ok());
    }

    #[test]
    fn non_json_payload_cannot_be_built() {
        let mut d = claimed(1);
        d.body = b"not json".to_vec();
        assert!(matches!(
            build_call_event(&Keys::generate(), &d),
            Err(BuildError::Payload)
        ));
    }

    #[test]
    fn backoff_grows_then_plateaus() {
        let base = t0();
        assert_eq!(next_attempt(1, base), base + Duration::seconds(1));
        assert_eq!(next_attempt(2, base), base + Duration::seconds(5));
        assert_eq!(next_attempt(6, base), base + Duration::minutes(30));
        assert_eq!(next_attempt(40, base), base + Duration::minutes(30));
        assert_eq!(next_attempt(0, base), base + Duration::seconds(1));
    }

    #[test]
    fn relay_url_uses_scheme_and_host() {
        assert_eq!(
            relay_url(RelayScheme::Https, "acme.communities.example.org")
                .unwrap()
                .as_str(),
            "https://acme.communities.example.org/events"
        );
        assert!(relay_url(RelayScheme::Http, "").is_none());
        assert!(relay_url(RelayScheme::Http, "a.example.org/x").is_none());
        assert!(relay_url(RelayScheme::Http, "a.example.org:1").is_none());
        assert!(relay_url(RelayScheme::Http, "u@a.example.org").is_none());
    }

    struct FakeRelay {
        responses: Mutex<Vec<SubmitResult>>,
        seen: Mutex<Vec<(Url, Event)>>,
    }

    impl FakeRelay {
        fn new(responses: Vec<SubmitResult>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses),
                seen: Mutex::default(),
            })
        }
    }

    #[async_trait]
    impl RelayClient for FakeRelay {
        async fn submit(&self, url: &Url, event: &Event) -> SubmitResult {
            self.seen.lock().unwrap().push((url.clone(), event.clone()));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                SubmitResult::Retry("no scripted response".to_owned())
            } else {
                responses.remove(0)
            }
        }
    }

    async fn seeded_store() -> (Arc<MemoryStore>, Uuid) {
        let store = Arc::new(MemoryStore::default());
        let integration_id = Uuid::new_v4();
        store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id,
                    revision: 1,
                    mutation: Mutation::PutIntegration {
                        community_host: "acme.communities.example.org".to_owned(),
                        provider: Provider::Github,
                    },
                },
                t0(),
            )
            .await
            .unwrap();
        store
            .insert_delivery(NewDelivery {
                integration_id,
                delivery_id: "d-1".to_owned(),
                event_type: "push".to_owned(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: br#"{"ref":"refs/heads/main"}"#.to_vec(),
                received_at: t0(),
            })
            .await
            .unwrap();
        (store, integration_id)
    }

    fn forwarder(
        store: Arc<MemoryStore>,
        relay: Arc<FakeRelay>,
        now: fn() -> DateTime<Utc>,
    ) -> Forwarder {
        forwarder_with_keys(store, relay, now, Keys::generate())
    }

    fn forwarder_with_keys(
        store: Arc<MemoryStore>,
        relay: Arc<FakeRelay>,
        now: fn() -> DateTime<Utc>,
        keys: Keys,
    ) -> Forwarder {
        Forwarder {
            store,
            client: relay,
            keys,
            scheme: RelayScheme::Https,
            concurrency: 4,
            retry_max_age: Duration::hours(24),
            now,
        }
    }

    fn one_second_later() -> DateTime<Utc> {
        t0() + Duration::seconds(1)
    }

    #[tokio::test]
    async fn accepted_delivery_drops_payload_and_records_event_id() {
        let (store, integration_id) = seeded_store().await;
        let relay = FakeRelay::new(vec![SubmitResult::Accepted]);
        let f = forwarder(store.clone(), relay.clone(), one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        assert_eq!(f.run_once().await.unwrap(), 0);
        let row = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(row.state, DeliveryState::Accepted);
        assert!(row.delivery.body.is_empty());
        assert_eq!(row.attempts, 1);
        assert!(row.next_attempt_at.is_none());
        let seen = relay.seen.lock().unwrap();
        assert_eq!(
            seen[0].0.as_str(),
            "https://acme.communities.example.org/events"
        );
        assert_eq!(
            row.event_id.as_deref(),
            Some(seen[0].1.id.to_hex().as_str())
        );
    }

    #[tokio::test]
    async fn retry_keeps_row_queued_with_same_event_id_then_accepts() {
        let (store, integration_id) = seeded_store().await;
        let relay = FakeRelay::new(vec![
            SubmitResult::Retry("relay returned 503".to_owned()),
            SubmitResult::Accepted,
        ]);
        let f = forwarder(store.clone(), relay.clone(), one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(row.state, DeliveryState::Queued);
        assert_eq!(row.attempts, 1);
        assert_eq!(row.last_error.as_deref(), Some("relay returned 503"));
        assert_eq!(
            row.next_attempt_at,
            Some(one_second_later() + Duration::seconds(1))
        );
        assert!(!row.delivery.body.is_empty());
        let first_event_id = row.event_id.clone().unwrap();

        assert_eq!(f.run_once().await.unwrap(), 0, "not due yet");
        fn later() -> DateTime<Utc> {
            t0() + Duration::minutes(5)
        }
        let f = forwarder_with_keys(store.clone(), relay.clone(), later, f.keys.clone());
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(row.state, DeliveryState::Accepted);
        assert_eq!(row.attempts, 2);
        assert_eq!(row.last_error, None);
        let seen = relay.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].1.id, seen[1].1.id);
        assert_eq!(seen[1].1.id.to_hex(), first_event_id);
    }

    #[tokio::test]
    async fn rejected_delivery_fails_without_retry() {
        let (store, integration_id) = seeded_store().await;
        let relay = FakeRelay::new(vec![SubmitResult::Rejected("invalid: bad tag".to_owned())]);
        let f = forwarder(store.clone(), relay, one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(row.state, DeliveryState::Failed);
        assert_eq!(row.last_error.as_deref(), Some("invalid: bad tag"));
        assert!(row.event_id.is_some());
        assert!(row.completed_at.is_some());
        assert_eq!(f.run_once().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn exhausted_retry_budget_fails_the_delivery() {
        let (store, integration_id) = seeded_store().await;
        let relay = FakeRelay::new(vec![SubmitResult::Retry("relay returned 503".to_owned())]);
        fn two_days_later() -> DateTime<Utc> {
            t0() + Duration::days(2)
        }
        let f = forwarder(store.clone(), relay, two_days_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(row.state, DeliveryState::Failed);
        assert!(row
            .last_error
            .as_deref()
            .unwrap()
            .starts_with("retry budget exhausted"));
    }

    #[tokio::test]
    async fn unparseable_payload_fails_without_contacting_relay() {
        let (store, integration_id) = seeded_store().await;
        store
            .insert_delivery(NewDelivery {
                integration_id,
                delivery_id: "d-2".to_owned(),
                event_type: "push".to_owned(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: b"not json".to_vec(),
                received_at: t0(),
            })
            .await
            .unwrap();
        let relay = FakeRelay::new(vec![SubmitResult::Accepted]);
        let f = forwarder(store.clone(), relay.clone(), one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 2);
        assert_eq!(
            store.delivery(integration_id, "d-2").unwrap().state,
            DeliveryState::Failed
        );
        assert_eq!(
            store.delivery(integration_id, "d-1").unwrap().state,
            DeliveryState::Accepted
        );
        assert_eq!(relay.seen.lock().unwrap().len(), 1);
    }
}
