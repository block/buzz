//! Background delivery of queued channel messages to community relays.
//!
//! ```text
//!   messages (state = queued, next_attempt_at <= now)
//!        │ claim_due_messages: lease rows, bump attempts
//!        ▼
//!   MessageSigner::signer_for(target) ── the author of the message
//!        │
//!        ▼
//!   build kind-9 event ── created_at = the delivery's received_at, so the id
//!        │                is the same on every retry and the relay dedupes
//!        ▼
//!   RelayClient::submit ──▶ Accepted  → state accepted
//!                       ──▶ Rejected  → state failed
//!                       ──▶ Retry     → next_attempt_at = backoff(attempts),
//!                                       or failed once the budget is spent
//! ```
use crate::config::RelayScheme;
use crate::metrics::{record_forward, record_forwarder_failure, ForwardResult};
use crate::relay_client::{RelayClient, SubmitResult};
use crate::signer::{MessageSigner, SignerError, SigningTarget};
use crate::store::{ClaimedMessage, ForwardOutcome, Store, StoreError};
use chrono::{DateTime, Duration, Utc};
use nostr::{Event, EventBuilder, Kind, NostrSigner, Tag, Timestamp, Url};
use std::sync::Arc;
use tokio::task::JoinSet;

/// NIP-29 channel message kind.
pub const CHANNEL_MESSAGE_KIND: Kind = Kind::Custom(9);
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

/// Delay before the next attempt after `attempts` tries so far.
pub fn next_attempt(attempts: i32, now: DateTime<Utc>) -> DateTime<Utc> {
    let index = usize::try_from(attempts.max(1) - 1).unwrap_or(usize::MAX);
    now + BACKOFF[index.min(BACKOFF.len() - 1)]
}

/// Build the signed kind-9 event for a claimed message.
///
/// `created_at` is pinned to the delivery's receive time so every retry by
/// the same author produces the same event id; the relay then treats a resend
/// after a lost acknowledgement as a duplicate rather than a second message.
pub async fn build_message_event(
    signer: &Arc<dyn NostrSigner>,
    message: &ClaimedMessage,
) -> Result<Event, nostr::event::builder::Error> {
    let created_at = u64::try_from(message.received_at.timestamp()).unwrap_or(0);
    EventBuilder::new(CHANNEL_MESSAGE_KIND, &message.content)
        .tag(Tag::custom(
            nostr::TagKind::h(),
            [message.channel_id.to_string()],
        ))
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign(signer)
        .await
}

/// Relay submission endpoint for a community.
pub fn relay_url(scheme: RelayScheme, community_host: &str) -> Option<Url> {
    let url = Url::parse(&format!("{}://{community_host}/events", scheme.as_str())).ok()?;
    (url.host_str() == Some(community_host)).then_some(url)
}

/// Drains queued messages to their community relays.
pub struct Forwarder {
    pub store: Arc<dyn Store>,
    /// Transport to the community relays.
    pub client: Arc<dyn RelayClient>,
    /// Picks the identity that authors each message.
    pub signer: Arc<dyn MessageSigner>,
    /// Scheme used to reach community hosts.
    pub scheme: RelayScheme,
    /// Messages claimed and sent per poll.
    pub concurrency: usize,
    /// How long after the delivery's receipt a message keeps retrying.
    pub retry_max_age: Duration,
    /// Injectable clock so tests can drive the backoff schedule.
    pub now: fn() -> DateTime<Utc>,
}

impl Forwarder {
    /// Claim one batch and send it. Returns how many messages were claimed.
    pub async fn run_once(&self) -> Result<usize, StoreError> {
        let now = (self.now)();
        let batch = self
            .store
            .claim_due_messages(now, CLAIM_LEASE, self.concurrency.max(1))
            .await?;
        let claimed = batch.len();
        let mut tasks = JoinSet::new();
        for message in batch {
            let store = Arc::clone(&self.store);
            let client = Arc::clone(&self.client);
            let signer = Arc::clone(&self.signer);
            let scheme = self.scheme;
            let retry_max_age = self.retry_max_age;
            let now_fn = self.now;
            tasks.spawn(async move {
                let outcome =
                    attempt(&*client, &*signer, scheme, retry_max_age, now_fn, &message).await;
                if let Err(error) = store
                    .complete_message(&message.key, outcome, now_fn())
                    .await
                {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway could not record send outcome");
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

fn retry_or_fail(
    message: &ClaimedMessage,
    retry_max_age: Duration,
    now: DateTime<Utc>,
    event_id: Option<String>,
    error: String,
    elapsed: Option<f64>,
) -> ForwardOutcome {
    if message.received_at + retry_max_age <= now {
        record_forward(ForwardResult::Failed, elapsed);
        return ForwardOutcome::Failed {
            event_id,
            error: format!("retry budget exhausted: {error}"),
        };
    }
    record_forward(ForwardResult::Retry, elapsed);
    ForwardOutcome::Retry {
        event_id,
        error,
        next_attempt_at: next_attempt(message.attempts, now),
    }
}

async fn attempt(
    client: &dyn RelayClient,
    signer: &dyn MessageSigner,
    scheme: RelayScheme,
    retry_max_age: Duration,
    now: fn() -> DateTime<Utc>,
    message: &ClaimedMessage,
) -> ForwardOutcome {
    let failed = |event_id: Option<String>, error: String| {
        record_forward(ForwardResult::Failed, None);
        ForwardOutcome::Failed { event_id, error }
    };
    let Some(url) = relay_url(scheme, &message.community_host) else {
        return failed(None, "community host is not a valid relay URL".to_owned());
    };
    let target = SigningTarget {
        community_host: &message.community_host,
        integration_id: message.key.integration_id,
        subscription_id: message.key.subscription_id,
        channel_id: message.channel_id,
    };
    let author = match signer.signer_for(&target).await {
        Ok(author) => author,
        Err(error @ SignerError::Unavailable(_)) => {
            return retry_or_fail(message, retry_max_age, now(), None, error.to_string(), None)
        }
        Err(error @ SignerError::Unconfigured(_)) => return failed(None, error.to_string()),
    };
    let event = match build_message_event(&author, message).await {
        Ok(event) => event,
        Err(error) => {
            let error = format!("event signing failed: {error}");
            return retry_or_fail(message, retry_max_age, now(), None, error, None);
        }
    };
    let event_id = event.id.to_hex();
    let started = std::time::Instant::now();
    let result = client.submit(&url, &event, &author).await;
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
        SubmitResult::Retry(error) => retry_or_fail(
            message,
            retry_max_age,
            now(),
            Some(event_id),
            error,
            Some(elapsed),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::GatewayKeySigner;
    use crate::store::{
        DeliveryState, FanoutOutcome, MemoryStore, MessageKey, Mutation, NewDelivery, NewMessage,
        Operation, Provider, SubscriptionStore,
    };
    use async_trait::async_trait;
    use chrono::TimeZone;
    use nostr::{Keys, PublicKey};
    use std::sync::Mutex;
    use uuid::Uuid;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap()
    }

    fn claimed(attempts: i32) -> ClaimedMessage {
        ClaimedMessage {
            key: MessageKey {
                integration_id: Uuid::nil(),
                delivery_id: "d-1".to_owned(),
                subscription_id: Uuid::nil(),
            },
            community_host: "acme.communities.example.org".to_owned(),
            channel_id: Uuid::from_u128(7),
            content: "github: push".to_owned(),
            received_at: t0(),
            attempts,
        }
    }

    #[tokio::test]
    async fn message_event_is_deterministic_and_targets_the_channel() {
        let author: Arc<dyn NostrSigner> = Arc::new(Keys::generate());
        let a = build_message_event(&author, &claimed(1)).await.unwrap();
        let b = build_message_event(&author, &claimed(3)).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, CHANNEL_MESSAGE_KIND);
        assert_eq!(a.content, "github: push");
        assert_eq!(a.created_at.as_secs(), t0().timestamp() as u64);
        let tags: Vec<Vec<String>> = a.tags.iter().map(|t| t.clone().to_vec()).collect();
        assert_eq!(
            tags,
            vec![vec!["h".to_owned(), Uuid::from_u128(7).to_string()]]
        );
        assert!(a.verify().is_ok());
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

    type Seen = (Url, Event, PublicKey);

    struct FakeRelay {
        responses: Mutex<Vec<SubmitResult>>,
        seen: Mutex<Vec<Seen>>,
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
        async fn submit(
            &self,
            url: &Url,
            event: &Event,
            signer: &Arc<dyn NostrSigner>,
        ) -> SubmitResult {
            let request_signer = signer.get_public_key().await.unwrap();
            self.seen
                .lock()
                .unwrap()
                .push((url.clone(), event.clone(), request_signer));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                SubmitResult::Retry("no scripted response".to_owned())
            } else {
                responses.remove(0)
            }
        }
    }

    struct NoSigner(fn() -> SignerError);

    #[async_trait]
    impl MessageSigner for NoSigner {
        async fn signer_for(
            &self,
            _target: &SigningTarget<'_>,
        ) -> Result<Arc<dyn NostrSigner>, SignerError> {
            Err((self.0)())
        }
    }

    /// A store holding one fanned-out, queued message.
    async fn seeded_store() -> (Arc<MemoryStore>, MessageKey) {
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
                body: b"{}".to_vec(),
                received_at: t0(),
            })
            .await
            .unwrap();
        let subscription_id = Uuid::new_v4();
        let message = NewMessage {
            subscription_id,
            channel_id: Uuid::from_u128(7),
            content: Ok("github: push".to_owned()),
        };
        store
            .complete_fanout(
                integration_id,
                "d-1",
                FanoutOutcome::Fanned(vec![message]),
                t0(),
            )
            .await
            .unwrap();
        let key = MessageKey {
            integration_id,
            delivery_id: "d-1".to_owned(),
            subscription_id,
        };
        (store, key)
    }

    fn forwarder(
        store: Arc<MemoryStore>,
        relay: Arc<FakeRelay>,
        signer: Arc<dyn MessageSigner>,
        now: fn() -> DateTime<Utc>,
    ) -> Forwarder {
        Forwarder {
            store,
            client: relay,
            signer,
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
    async fn accepted_message_is_authored_and_authorized_by_the_signer() {
        let (store, key) = seeded_store().await;
        let relay = FakeRelay::new(vec![SubmitResult::Accepted]);
        let author = Keys::generate();
        let signer = Arc::new(GatewayKeySigner::new(author.clone()));
        let f = forwarder(store.clone(), relay.clone(), signer, one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        assert_eq!(f.run_once().await.unwrap(), 0);
        let row = store.message(&key).unwrap();
        assert_eq!(row.state, DeliveryState::Accepted);
        assert_eq!(row.attempts, 1);
        assert!(row.next_attempt_at.is_none());
        let seen = relay.seen.lock().unwrap();
        let (url, event, request_signer) = &seen[0];
        assert_eq!(url.as_str(), "https://acme.communities.example.org/events");
        assert_eq!(event.pubkey, author.public_key());
        assert_eq!(*request_signer, author.public_key());
        assert_eq!(row.event_id.as_deref(), Some(event.id.to_hex().as_str()));
    }

    #[tokio::test]
    async fn retry_keeps_message_queued_with_same_event_id_then_accepts() {
        let (store, key) = seeded_store().await;
        let relay = FakeRelay::new(vec![
            SubmitResult::Retry("relay returned 503".to_owned()),
            SubmitResult::Accepted,
        ]);
        let signer: Arc<dyn MessageSigner> = Arc::new(GatewayKeySigner::new(Keys::generate()));
        let f = forwarder(
            store.clone(),
            relay.clone(),
            signer.clone(),
            one_second_later,
        );
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.message(&key).unwrap();
        assert_eq!(row.state, DeliveryState::Queued);
        assert_eq!(row.last_error.as_deref(), Some("relay returned 503"));
        assert_eq!(
            row.next_attempt_at,
            Some(one_second_later() + Duration::seconds(1))
        );
        let first_event_id = row.event_id.clone().unwrap();

        assert_eq!(f.run_once().await.unwrap(), 0, "not due yet");
        fn later() -> DateTime<Utc> {
            t0() + Duration::minutes(5)
        }
        let f = forwarder(store.clone(), relay.clone(), signer, later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.message(&key).unwrap();
        assert_eq!(row.state, DeliveryState::Accepted);
        assert_eq!(row.attempts, 2);
        assert_eq!(row.last_error, None);
        let seen = relay.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].1.id.to_hex(), first_event_id);
    }

    #[tokio::test]
    async fn terminal_outcomes_fail_the_message() {
        fn two_days_later() -> DateTime<Utc> {
            t0() + Duration::days(2)
        }
        let gateway =
            || -> Arc<dyn MessageSigner> { Arc::new(GatewayKeySigner::new(Keys::generate())) };
        // (name, relay response, signer, clock, stored error, relay submissions)
        type Case = (
            &'static str,
            SubmitResult,
            Arc<dyn MessageSigner>,
            fn() -> DateTime<Utc>,
            &'static str,
            usize,
        );
        let cases: Vec<Case> = vec![
            (
                "relay rejects",
                SubmitResult::Rejected("restricted: not a member".to_owned()),
                gateway(),
                one_second_later,
                "restricted: not a member",
                1,
            ),
            (
                "retry budget spent",
                SubmitResult::Retry("relay returned 503".to_owned()),
                gateway(),
                two_days_later,
                "retry budget exhausted: relay returned 503",
                1,
            ),
            (
                "no identity for the target",
                SubmitResult::Accepted,
                Arc::new(NoSigner(|| {
                    SignerError::Unconfigured("no member".to_owned())
                })),
                one_second_later,
                "no signer for this target: no member",
                0,
            ),
        ];
        for (name, response, signer, now, error, submits) in cases {
            let (store, key) = seeded_store().await;
            let relay = FakeRelay::new(vec![response]);
            let f = forwarder(store.clone(), relay.clone(), signer, now);
            assert_eq!(f.run_once().await.unwrap(), 1, "{name}");
            let row = store.message(&key).unwrap();
            assert_eq!(row.state, DeliveryState::Failed, "{name}");
            assert_eq!(row.last_error.as_deref(), Some(error), "{name}");
            assert_eq!(relay.seen.lock().unwrap().len(), submits, "{name}");
            assert_eq!(f.run_once().await.unwrap(), 0, "{name}");
        }
    }

    #[tokio::test]
    async fn unavailable_signer_retries_without_contacting_the_relay() {
        let (store, key) = seeded_store().await;
        let relay = FakeRelay::new(vec![]);
        let signer = Arc::new(NoSigner(|| {
            SignerError::Unavailable("vault down".to_owned())
        }));
        let f = forwarder(store.clone(), relay.clone(), signer, one_second_later);
        assert_eq!(f.run_once().await.unwrap(), 1);
        let row = store.message(&key).unwrap();
        assert_eq!(row.state, DeliveryState::Queued);
        assert_eq!(
            row.last_error.as_deref(),
            Some("signer unavailable: vault down")
        );
        assert!(relay.seen.lock().unwrap().is_empty());
    }
}
