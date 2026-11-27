use super::*;
use crate::signer::GatewayKeySigner;
use crate::store::{
    DeliveryState, Evaluated, FanoutOutcome, IntegrationConfig, MemoryStore, MessageKey, Mutation,
    NewDelivery, NewMessage, Operation, Provider, SubscriptionStore,
};
use async_trait::async_trait;
use chrono::TimeZone;
use nostr::{Event, Keys, PublicKey};
use std::sync::Mutex;
use uuid::Uuid;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap()
}

fn claimed(attempts: i32, subscription_id: Uuid) -> ClaimedMessage {
    ClaimedMessage {
        target: None,
        key: MessageKey {
            integration_id: Uuid::nil(),
            delivery_id: "d-1".to_owned(),
            subscription_id,
        },
        community_host: "acme.communities.example.org".to_owned(),
        channel_id: Uuid::from_u128(7),
        content: "github: push".to_owned(),
        received_at: t0() - Duration::hours(1),
        attempts,
        signed_at: t0(),
        event_id: None,
    }
}

#[tokio::test]
async fn message_event_is_deterministic_per_row_and_targets_the_channel() {
    let author: Arc<dyn NostrSigner> = Arc::new(Keys::generate());
    let pubkey = author.get_public_key().await.unwrap();
    let a = build_message_event(&author, &claimed(1, Uuid::nil()))
        .await
        .unwrap();
    let b = build_message_event(&author, &claimed(3, Uuid::nil()))
        .await
        .unwrap();
    assert_eq!(a.id, b.id, "every attempt of one row signs the same event");
    assert_eq!(a.id, message_event_id(pubkey, &claimed(1, Uuid::nil())));
    let twin = build_message_event(&author, &claimed(1, Uuid::from_u128(1)))
        .await
        .unwrap();
    assert_ne!(a.id, twin.id, "same text, channel and second, another row");
    assert_eq!(a.kind, CHANNEL_MESSAGE_KIND);
    assert_eq!(a.content, "github: push");
    assert_eq!(a.created_at.as_secs(), t0().timestamp() as u64);
    let tags: Vec<Vec<String>> = a.tags.iter().map(|t| t.clone().to_vec()).collect();
    assert_eq!(
        tags,
        vec![
            vec!["h".to_owned(), Uuid::from_u128(7).to_string()],
            vec![
                MESSAGE_TAG.to_owned(),
                message_tag(&claimed(1, Uuid::nil()))
            ],
        ]
    );
    assert_eq!(message_tag(&claimed(1, Uuid::nil())).len(), 64);
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
fn relay_url_uses_scheme_host_and_endpoint() {
    assert_eq!(
        relay_url(RelayScheme::Https, "acme.communities.example.org", "events")
            .unwrap()
            .as_str(),
        "https://acme.communities.example.org/events"
    );
    assert_eq!(
        relay_url(RelayScheme::Http, "a.example.org", "query")
            .unwrap()
            .as_str(),
        "http://a.example.org/query"
    );
    for bad in ["", "a.example.org/x", "a.example.org:1", "u@a.example.org"] {
        assert!(
            relay_url(RelayScheme::Http, bad, "events").is_none(),
            "{bad}"
        );
    }
}

type Seen = (Url, Event, PublicKey);

struct FakeRelay {
    responses: Mutex<Vec<SubmitResult>>,
    lookups: Mutex<Vec<LookupResult>>,
    seen: Mutex<Vec<Seen>>,
    asked: Mutex<Vec<(Url, Vec<String>)>>,
}

impl FakeRelay {
    fn new(responses: Vec<SubmitResult>) -> Arc<Self> {
        Self::with_lookups(responses, vec![])
    }

    fn with_lookups(responses: Vec<SubmitResult>, lookups: Vec<LookupResult>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            lookups: Mutex::new(lookups),
            seen: Mutex::default(),
            asked: Mutex::default(),
        })
    }
}

fn next<T>(scripted: &Mutex<Vec<T>>, otherwise: T) -> T {
    let mut scripted = scripted.lock().unwrap();
    if scripted.is_empty() {
        otherwise
    } else {
        scripted.remove(0)
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
        next(
            &self.responses,
            SubmitResult::Retry("no scripted response".to_owned()),
        )
    }

    async fn find(
        &self,
        url: &Url,
        ids: &[String],
        _channel_id: Uuid,
        _signer: &Arc<dyn NostrSigner>,
    ) -> LookupResult {
        self.asked.lock().unwrap().push((url.clone(), ids.to_vec()));
        next(
            &self.lookups,
            LookupResult::Retry("no scripted lookup".to_owned()),
        )
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
                    config: IntegrationConfig::default(),
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
    let claim = store
        .claim_due(t0(), Duration::seconds(60), 1)
        .await
        .unwrap();
    let subscription_id = Uuid::new_v4();
    let message = NewMessage {
        group: None,
        subscription_id,
        channel_id: Uuid::from_u128(7),
        content: Ok("github: push".to_owned()),
    };
    store
        .complete_fanout(
            integration_id,
            "d-1",
            claim[0].attempts,
            FanoutOutcome::Fanned {
                messages: vec![message],
                evaluated: Vec::<Evaluated>::new(),
            },
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
    assert_eq!(
        event.created_at.as_secs(),
        one_second_later().timestamp() as u64,
        "signed when first claimed, not when received"
    );
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
    assert!(relay.asked.lock().unwrap().is_empty(), "stamp still fresh");
}

#[tokio::test]
async fn an_expired_stamp_is_renewed_only_after_the_relay_lacks_the_earlier_send() {
    fn outage_over() -> DateTime<Utc> {
        t0() + Duration::minutes(40)
    }
    // (name, lookup answer, final state, relay submissions)
    let cases = [
        ("earlier send landed", LookupResult::Found(String::new()), 1),
        ("earlier send lost", LookupResult::Missing, 2),
    ];
    for (name, lookup, submits) in cases {
        let (store, key) = seeded_store().await;
        let relay = FakeRelay::with_lookups(
            vec![
                SubmitResult::Retry("relay request timed out".to_owned()),
                SubmitResult::Accepted,
            ],
            vec![],
        );
        let signer: Arc<dyn MessageSigner> = Arc::new(GatewayKeySigner::new(Keys::generate()));
        let f = forwarder(
            store.clone(),
            relay.clone(),
            signer.clone(),
            one_second_later,
        );
        f.run_once().await.unwrap();
        let first = relay.seen.lock().unwrap()[0].1.id.to_hex();
        let lookup = match lookup {
            LookupResult::Found(_) => LookupResult::Found(first.clone()),
            other => other,
        };
        relay.lookups.lock().unwrap().push(lookup);

        // Forty minutes on, the relay would refuse the first timestamp.
        let f = forwarder(store.clone(), relay.clone(), signer, outage_over);
        assert_eq!(f.run_once().await.unwrap(), 1, "{name}");
        let asked = relay.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "{name}");
        assert_eq!(
            asked[0].0.as_str(),
            "https://acme.communities.example.org/query"
        );
        assert_eq!(asked[0].1, vec![first.clone()], "{name}");
        f.run_once().await.unwrap();

        let row = store.message(&key).unwrap();
        assert_eq!(row.state, DeliveryState::Accepted, "{name}");
        let seen = relay.seen.lock().unwrap();
        assert_eq!(seen.len(), submits, "{name}");
        if submits == 1 {
            assert_eq!(row.event_id.as_deref(), Some(first.as_str()), "{name}");
        } else {
            let resent = &seen[1].1;
            assert_ne!(resent.id.to_hex(), first, "{name}");
            assert_eq!(
                resent.created_at.as_secs(),
                outage_over().timestamp() as u64
            );
            assert_eq!(row.event_id, Some(resent.id.to_hex()), "{name}");
        }
    }
}

#[tokio::test]
async fn a_claim_taken_over_after_its_lease_cannot_settle() {
    let (store, key) = seeded_store().await;
    let first = store
        .claim_due_messages(t0(), Duration::seconds(60), 1)
        .await
        .unwrap()
        .remove(0);
    let second = store
        .claim_due_messages(t0() + Duration::seconds(61), Duration::seconds(60), 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!((first.attempts, second.attempts), (1, 2));
    let failed = ForwardOutcome::Failed {
        event_id: None,
        error: "late".to_owned(),
    };
    store.complete_message(&first, failed, t0()).await.unwrap();
    let row = store.message(&key).unwrap();
    assert_eq!(
        row.state,
        DeliveryState::Queued,
        "the stale worker is fenced"
    );
    assert_eq!(row.signed_at, Some(t0()), "the stamp survives the takeover");
    let accepted = ForwardOutcome::Accepted {
        event_id: "e".repeat(64),
    };
    store
        .complete_message(&second, accepted, t0())
        .await
        .unwrap();
    assert_eq!(store.message(&key).unwrap().state, DeliveryState::Accepted);
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
            SubmitResult::Rejected("restricted: not a member\0".to_owned()),
            gateway(),
            one_second_later,
            "restricted: not a member\u{FFFD}",
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
