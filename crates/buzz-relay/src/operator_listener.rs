//! Deployment-global operator-listener mention matching and notification delivery.

use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use tokio::{sync::mpsc, task::JoinSet};
use tracing::{error, warn};
use uuid::Uuid;

use crate::{handlers::req::event_visible_to_reader, nip98::nip98_header, state::AppState};

use reqwest::StatusCode;

const CLAIM_SECS: i64 = 30;
const MAX_IN_FLIGHT: usize = 10;
const IDLE_POLL_FLOOR: Duration = Duration::from_millis(250);
const IDLE_POLL_CEILING: Duration = Duration::from_secs(2);
const CLAIM_FAILURE_DELAY: Duration = Duration::from_secs(2);
const OUTBOX_REAP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const REGISTRATION_REAP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Notification sent to the configured operator-listener endpoint.
///
/// Version 2 adds the full signed `event` to version 1's metadata.
#[derive(Debug, Serialize)]
struct MentionNotification<'a> {
    v: u8,
    pubkey: String,
    community_host: String,
    event_id: String,
    event_kind: i32,
    event_created_at: i64,
    event: &'a nostr::Event,
}

/// Reap stale deliveries every five minutes and expired registrations daily.
pub async fn run_reaper(state: Arc<AppState>) {
    tokio::join!(
        async {
            loop {
                if let Err(error) = state.db.delete_expired_operator_listener_pubkeys().await {
                    warn!(%error, "operator-listener registration cleanup failed");
                }
                tokio::time::sleep(REGISTRATION_REAP_INTERVAL).await;
            }
        },
        async {
            loop {
                if let Err(error) = state.db.reap_operator_listener_deliveries().await {
                    warn!(%error, "operator-listener outbox reap failed");
                }
                tokio::time::sleep(OUTBOX_REAP_INTERVAL).await;
            }
        }
    );
}

/// Continuously deliver operator-listener notification rows with bounded concurrency and retries.
pub async fn run_delivery_worker(state: Arc<AppState>) {
    let http = match build_delivery_http_client(state.config.operator_listener_timeout) {
        Ok(http) => http,
        Err(error) => {
            error!(%error, "operator-listener HTTP client initialization failed");
            return;
        }
    };
    // Rows committed on this pod come straight here; the poll covers the
    // rest. `None` only if another worker in this process already subscribed.
    let committed = state.db.subscribe_operator_listener_commits(MAX_IN_FLIGHT);
    let admission = buzz_db::operator_listener::DeliveryAdmission {
        require_membership: state.config.require_relay_membership,
        allow_owner_membership: state.config.allow_nip_oa_auth,
        require_allowlist: state.config.pubkey_allowlist_enabled,
    };
    run_delivery_loop(
        committed,
        |claim| {
            let state = Arc::clone(&state);
            async move {
                let lease_until = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
                match claim {
                    Claim::Due(limit) => {
                        state
                            .db
                            .claim_operator_listener_deliveries(limit as i64, lease_until)
                            .await
                    }
                    Claim::Committed(ids) => {
                        state
                            .db
                            .claim_operator_listener_deliveries_by_id(&ids, lease_until)
                            .await
                    }
                }
                .map_err(|error| error.to_string())
            }
        },
        |delivery| {
            let state = Arc::clone(&state);
            let http = http.clone();
            async move {
                let store = DbDeliveryStore {
                    db: &state.db,
                    admission,
                };
                deliver_one(
                    &state.config.operator_listener_delivery_urls,
                    &state.relay_keypair,
                    &store,
                    &ReqwestDeliveryTransport { http: &http },
                    delivery,
                )
                .await;
            }
        },
    )
    .await;
}

/// What the delivery loop asks the outbox for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Claim {
    /// Up to this many due rows, oldest first: the poll.
    Due(usize),
    /// These rows, just committed on this pod, if no one has claimed them.
    Committed(Vec<Uuid>),
}

/// Deliver rows as they commit on this pod, poll for everything else, and
/// claim whenever a delivery slot is free, so one slow endpoint never holds
/// up the deliveries behind it.
///
/// Committed rows share the in-flight limit with the poll: they are only
/// received while a slot is free, and the bounded hand-off drops rows to the
/// poll when it is full. Both paths claim through the outbox, so a row is sent
/// by exactly one of them.
async fn run_delivery_loop<C, CF, D, DF>(
    mut committed: Option<mpsc::Receiver<Uuid>>,
    mut claim: C,
    deliver: D,
) where
    C: FnMut(Claim) -> CF,
    CF: Future<Output = Result<Vec<buzz_db::operator_listener::ClaimedDelivery>, String>>,
    D: Fn(buzz_db::operator_listener::ClaimedDelivery) -> DF,
    DF: Future<Output = ()> + Send + 'static,
{
    let mut in_flight = JoinSet::new();
    let mut idle_delay = IDLE_POLL_FLOOR;
    let mut poll = true;
    // Committed hand-offs must not keep pushing the next poll back, or
    // retries and other pods' rows would wait on them.
    let mut next_poll = tokio::time::Instant::now();
    loop {
        let capacity = MAX_IN_FLIGHT - in_flight.len();
        if poll && capacity > 0 {
            match claim(Claim::Due(capacity)).await {
                Ok(claimed) if !claimed.is_empty() => {
                    for delivery in claimed {
                        in_flight.spawn(deliver(delivery));
                    }
                    idle_delay = IDLE_POLL_FLOOR;
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    error!(%error, "operator-listener delivery claim failed");
                    idle_delay = CLAIM_FAILURE_DELAY;
                }
            }
        }
        if poll {
            poll = false;
            next_poll = tokio::time::Instant::now() + idle_delay;
        }
        tokio::select! {
            Some(joined) = in_flight.join_next() => {
                // Refill every freed slot with one claim, not one per row.
                for joined in std::iter::once(joined).chain(std::iter::from_fn(|| in_flight.try_join_next())) {
                    if let Err(error) = joined {
                        error!(%error, "operator-listener delivery task failed");
                    }
                }
                poll = true;
            }
            Some(first) = next_committed(&mut committed), if capacity > 0 => {
                let mut ids = vec![first];
                if let Some(receiver) = committed.as_mut() {
                    while ids.len() < capacity {
                        let Ok(id) = receiver.try_recv() else { break };
                        ids.push(id);
                    }
                }
                match claim(Claim::Committed(ids)).await {
                    Ok(claimed) => {
                        for delivery in claimed {
                            in_flight.spawn(deliver(delivery));
                        }
                    }
                    // The rows stay pending for the poll.
                    Err(error) => warn!(%error, "operator-listener committed delivery claim failed"),
                }
            }
            () = tokio::time::sleep_until(next_poll) => {
                idle_delay = (idle_delay * 2).min(IDLE_POLL_CEILING);
                poll = true;
            }
        }
    }
}

async fn next_committed(committed: &mut Option<mpsc::Receiver<Uuid>>) -> Option<Uuid> {
    match committed {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

fn build_delivery_http_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

#[async_trait::async_trait]
trait DeliveryTransport: Sync {
    async fn post(
        &self,
        url: &url::Url,
        authorization: &str,
        body: Vec<u8>,
    ) -> Result<StatusCode, String>;
}

#[async_trait::async_trait]
trait DeliveryStore: Sync {
    async fn load(
        &self,
        delivery: &buzz_db::operator_listener::ClaimedDelivery,
    ) -> Result<Option<buzz_core::StoredEvent>, String>;
    async fn release(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String>;
    async fn complete(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String>;
    async fn retry(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String>;
    async fn fail(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String>;
}

struct DbDeliveryStore<'a> {
    db: &'a buzz_db::Db,
    admission: buzz_db::operator_listener::DeliveryAdmission,
}

#[async_trait::async_trait]
impl DeliveryStore for DbDeliveryStore<'_> {
    async fn load(
        &self,
        delivery: &buzz_db::operator_listener::ClaimedDelivery,
    ) -> Result<Option<buzz_core::StoredEvent>, String> {
        self.db
            .load_operator_listener_delivery_event(delivery, self.admission)
            .await
            .map_err(|error| error.to_string())
    }

    async fn release(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String> {
        self.db
            .release_operator_listener_delivery(id, claim_id, next)
            .await
            .map_err(|error| error.to_string())
    }

    async fn complete(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String> {
        self.db
            .complete_operator_listener_delivery(id, claim_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn retry(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String> {
        self.db
            .retry_operator_listener_delivery(id, claim_id, next)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fail(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String> {
        self.db
            .fail_operator_listener_delivery(id, claim_id)
            .await
            .map_err(|error| error.to_string())
    }
}

struct ReqwestDeliveryTransport<'a> {
    http: &'a reqwest::Client,
}

#[async_trait::async_trait]
impl DeliveryTransport for ReqwestDeliveryTransport<'_> {
    async fn post(
        &self,
        url: &url::Url,
        authorization: &str,
        body: Vec<u8>,
    ) -> Result<StatusCode, String> {
        self.http
            .post(url.clone())
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map(|response| response.status())
            .map_err(|error| error.to_string())
    }
}

async fn deliver_one<T: DeliveryTransport, S: DeliveryStore>(
    routes: &HashMap<String, url::Url>,
    relay_keypair: &nostr::Keys,
    store: &S,
    transport: &T,
    delivery: buzz_db::operator_listener::ClaimedDelivery,
) {
    let listener_hex = hex::encode(&delivery.listener_pubkey);
    let Some(url) = routes.get(&listener_hex) else {
        warn!(
            delivery=%delivery.id,
            listener=%listener_hex,
            "operator-listener delivery has no route on this pod; releasing claim"
        );
        if let Err(error) = store
            .release(
                delivery.id,
                delivery.claim_id,
                Utc::now() + TimeDelta::seconds(CLAIM_SECS),
            )
            .await
        {
            error!(%error, delivery=%delivery.id, "failed to release unroutable operator-listener delivery");
        }
        metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "unroutable")
            .increment(1);
        return;
    };
    let stored = match store.load(&delivery).await {
        Ok(Some(stored)) if event_visible_to_reader(&stored.event, &delivery.target_pubkey) => {
            stored
        }
        // Retry rather than drop: a mention can commit just before the
        // target is added to the channel.
        Ok(_) => {
            retry_or_fail(store, &delivery, "target cannot read event".to_owned()).await;
            return;
        }
        Err(error) => {
            retry_or_fail(store, &delivery, format!("event load failed: {error}")).await;
            return;
        }
    };
    let body = match serde_json::to_vec(&MentionNotification {
        v: 2,
        pubkey: hex::encode(&delivery.target_pubkey),
        community_host: delivery.community_host.clone(),
        event_id: hex::encode(&delivery.event_id),
        event_kind: delivery.event_kind,
        event_created_at: delivery.event_created_at.timestamp(),
        event: &stored.event,
    }) {
        Ok(body) => body,
        Err(error) => {
            fail_permanently(
                store,
                &delivery,
                &format!("notification encoding failed: {error}"),
            )
            .await;
            return;
        }
    };
    let auth = match nip98_header(relay_keypair, url.as_str(), &body) {
        Ok(auth) => auth,
        Err(error) => {
            fail_permanently(
                store,
                &delivery,
                &format!("notification auth failed: {error}"),
            )
            .await;
            return;
        }
    };
    let response = transport.post(url, &auth, body).await;
    match response {
        Ok(status) if status.is_success() => {
            match store.complete(delivery.id, delivery.claim_id).await {
                Ok(true) => {}
                Ok(false) => warn!(
                    delivery=%delivery.id,
                    "operator-listener delivery completion lost its claim"
                ),
                Err(error) => error!(
                    delivery=%delivery.id,
                    %error,
                    "failed to persist operator-listener delivery completion"
                ),
            }
            metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "accepted")
                .increment(1);
        }
        Ok(status)
            if status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error() =>
        {
            retry_or_fail(store, &delivery, format!("HTTP {status}")).await
        }
        Ok(status) => {
            fail_permanently(store, &delivery, &format!("HTTP {status}")).await;
        }
        Err(error) => retry_or_fail(store, &delivery, error.to_string()).await,
    }
}

async fn fail_permanently<S: DeliveryStore>(
    store: &S,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: &str,
) {
    error!(delivery=%delivery.id, %reason, "operator-listener delivery failed permanently");
    if let Err(error) = store.fail(delivery.id, delivery.claim_id).await {
        error!(delivery=%delivery.id, %error, "failed to delete terminal operator-listener delivery");
    }
    metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "failed")
        .increment(1);
}

async fn retry_or_fail<S: DeliveryStore>(
    store: &S,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: String,
) {
    if delivery.attempt >= buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS {
        fail_permanently(store, delivery, &format!("retries exhausted: {reason}")).await;
        return;
    }
    let delay = 2_i64.pow((delivery.attempt - 1).clamp(0, 7) as u32);
    warn!(
        delivery=%delivery.id,
        attempt=delivery.attempt,
        retry_in_seconds=delay,
        %reason,
        "operator-listener delivery failed; retrying"
    );
    if let Err(error) = store
        .retry(
            delivery.id,
            delivery.claim_id,
            Utc::now() + TimeDelta::seconds(delay),
        )
        .await
    {
        error!(delivery=%delivery.id, %error, "failed to persist operator-listener retry");
    }
    metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "retry").increment(1);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::{TcpListener, TcpStream},
    };

    use super::*;
    use buzz_core::tenant::CommunityId;
    use nostr::{EventBuilder, Keys, Kind};

    #[test]
    fn reapers_use_their_expected_intervals() {
        assert_eq!(OUTBOX_REAP_INTERVAL, Duration::from_secs(5 * 60));
        assert_eq!(
            REGISTRATION_REAP_INTERVAL,
            Duration::from_secs(24 * 60 * 60)
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum StoreCall {
        Release(Uuid, Uuid, DateTime<Utc>),
        Complete(Uuid, Uuid),
        Retry(Uuid, Uuid, DateTime<Utc>),
        Fail(Uuid, Uuid),
    }

    struct MockStore {
        calls: Mutex<Vec<StoreCall>>,
        loaded: Result<Option<buzz_core::StoredEvent>, String>,
    }

    impl Default for MockStore {
        fn default() -> Self {
            Self::loading(Ok(Some(stored_event(Kind::Custom(9)))))
        }
    }

    impl MockStore {
        fn loading(loaded: Result<Option<buzz_core::StoredEvent>, String>) -> Self {
            Self {
                calls: Mutex::default(),
                loaded,
            }
        }

        fn calls(&self) -> Vec<StoreCall> {
            self.calls.lock().expect("store calls lock").clone()
        }
    }

    fn stored_event(kind: Kind) -> buzz_core::StoredEvent {
        let event = EventBuilder::new(kind, "hello agent")
            .sign_with_keys(&Keys::generate())
            .expect("sign test event");
        buzz_core::StoredEvent::new(event, None)
    }

    #[async_trait::async_trait]
    impl DeliveryStore for MockStore {
        async fn load(
            &self,
            _delivery: &buzz_db::operator_listener::ClaimedDelivery,
        ) -> Result<Option<buzz_core::StoredEvent>, String> {
            self.loaded.clone()
        }

        async fn release(
            &self,
            id: Uuid,
            claim_id: Uuid,
            next: DateTime<Utc>,
        ) -> Result<bool, String> {
            self.calls
                .lock()
                .expect("store calls lock")
                .push(StoreCall::Release(id, claim_id, next));
            Ok(true)
        }

        async fn complete(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String> {
            self.calls
                .lock()
                .expect("store calls lock")
                .push(StoreCall::Complete(id, claim_id));
            Ok(true)
        }

        async fn retry(
            &self,
            id: Uuid,
            claim_id: Uuid,
            next: DateTime<Utc>,
        ) -> Result<bool, String> {
            self.calls
                .lock()
                .expect("store calls lock")
                .push(StoreCall::Retry(id, claim_id, next));
            Ok(true)
        }

        async fn fail(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String> {
            self.calls
                .lock()
                .expect("store calls lock")
                .push(StoreCall::Fail(id, claim_id));
            Ok(true)
        }
    }

    struct SentRequest {
        url: url::Url,
        authorization: String,
        body: Vec<u8>,
    }

    struct MockTransport {
        response: Mutex<Option<Result<StatusCode, String>>>,
        request: Mutex<Option<SentRequest>>,
        request_count: Mutex<usize>,
    }

    impl MockTransport {
        fn new(response: Result<StatusCode, String>) -> Self {
            Self {
                response: Mutex::new(Some(response)),
                request: Mutex::new(None),
                request_count: Mutex::new(0),
            }
        }

        fn take_request(&self) -> Option<SentRequest> {
            self.request.lock().expect("transport request lock").take()
        }

        fn request_count(&self) -> usize {
            *self
                .request_count
                .lock()
                .expect("transport request count lock")
        }
    }

    #[async_trait::async_trait]
    impl DeliveryTransport for MockTransport {
        async fn post(
            &self,
            url: &url::Url,
            authorization: &str,
            body: Vec<u8>,
        ) -> Result<StatusCode, String> {
            *self
                .request_count
                .lock()
                .expect("transport request count lock") += 1;
            *self.request.lock().expect("transport request lock") = Some(SentRequest {
                url: url.clone(),
                authorization: authorization.to_owned(),
                body,
            });
            self.response
                .lock()
                .expect("transport response lock")
                .take()
                .expect("delivery should make one request")
        }
    }

    fn delivery_fixture(
        attempt: i32,
    ) -> (
        HashMap<String, url::Url>,
        Keys,
        buzz_db::operator_listener::ClaimedDelivery,
    ) {
        let listener = Keys::generate();
        let listener_pubkey = listener.public_key().to_bytes().to_vec();
        let route = url::Url::parse("https://listener.example/mentions").expect("test URL");
        let routes = HashMap::from([(hex::encode(&listener_pubkey), route)]);
        (
            routes,
            Keys::generate(),
            buzz_db::operator_listener::ClaimedDelivery {
                id: Uuid::new_v4(),
                claim_id: Uuid::new_v4(),
                listener_pubkey,
                target_pubkey: vec![0x11; 32],
                community: CommunityId::from_uuid(Uuid::new_v4()),
                community_host: "community.example".to_owned(),
                event_id: vec![0x22; 32],
                event_kind: 9,
                event_created_at: DateTime::from_timestamp(1_700_000_000, 0)
                    .expect("test timestamp"),
                attempt,
            },
        )
    }

    /// Drives the production delivery loop against an in-memory outbox.
    struct LoopHarness {
        committed: mpsc::Sender<Uuid>,
        outbox: Arc<Mutex<Vec<buzz_db::operator_listener::ClaimedDelivery>>>,
        stalled: Arc<Mutex<Vec<Uuid>>>,
        fail_committed: Arc<std::sync::atomic::AtomicBool>,
        claims: mpsc::UnboundedReceiver<Claim>,
        delivered: mpsc::UnboundedReceiver<Uuid>,
        task: tokio::task::JoinHandle<()>,
    }

    impl LoopHarness {
        fn start() -> Self {
            let (committed, committed_rx) = mpsc::channel(MAX_IN_FLIGHT);
            let outbox = Arc::new(Mutex::new(
                Vec::<buzz_db::operator_listener::ClaimedDelivery>::new(),
            ));
            let stalled = Arc::new(Mutex::new(Vec::new()));
            let fail_committed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (claims_tx, claims) = mpsc::unbounded_channel();
            let (delivered_tx, delivered) = mpsc::unbounded_channel();
            let task = tokio::spawn({
                let outbox = Arc::clone(&outbox);
                let stalled = Arc::clone(&stalled);
                let fail_committed = Arc::clone(&fail_committed);
                async move {
                    run_delivery_loop(
                        Some(committed_rx),
                        |claim: Claim| {
                            let _ = claims_tx.send(claim.clone());
                            let mut outbox = outbox.lock().expect("outbox lock");
                            std::future::ready(match claim {
                                Claim::Due(limit) => {
                                    let take = limit.min(outbox.len());
                                    Ok(outbox.drain(..take).collect())
                                }
                                Claim::Committed(_)
                                    if fail_committed.load(std::sync::atomic::Ordering::SeqCst) =>
                                {
                                    Err("claim by id failed".to_owned())
                                }
                                Claim::Committed(ids) => {
                                    let (claimed, rest): (Vec<_>, Vec<_>) = outbox
                                        .drain(..)
                                        .partition(|delivery| ids.contains(&delivery.id));
                                    *outbox = rest;
                                    Ok(claimed)
                                }
                            })
                        },
                        |delivery: buzz_db::operator_listener::ClaimedDelivery| {
                            let _ = delivered_tx.send(delivery.id);
                            let stalls =
                                stalled.lock().expect("stalled lock").contains(&delivery.id);
                            async move {
                                if stalls {
                                    std::future::pending::<()>().await;
                                }
                            }
                        },
                    )
                    .await;
                }
            });
            Self {
                committed,
                outbox,
                stalled,
                fail_committed,
                claims,
                delivered,
                task,
            }
        }

        /// Add a pending row without handing it to the loop, like a row
        /// committed on another pod.
        fn enqueue(&self) -> Uuid {
            let (_, _, delivery) = delivery_fixture(1);
            let id = delivery.id;
            self.outbox.lock().expect("outbox lock").push(delivery);
            id
        }

        /// Add a pending row and hand it to the loop, like a row committed
        /// on this pod.
        fn commit(&self) -> Uuid {
            let id = self.enqueue();
            self.committed.try_send(id).expect("hand-off has room");
            id
        }

        /// Mark a row's delivery as never finishing, like a hung endpoint.
        fn stall(&self, id: Uuid) -> Uuid {
            self.stalled.lock().expect("stalled lock").push(id);
            id
        }

        /// Let the loop back off to its slowest idle poll, then return just
        /// after a poll, when the loop has the full ceiling left to sleep.
        async fn settle_idle(&mut self) {
            tokio::time::sleep(IDLE_POLL_CEILING * 4).await;
            while self.claims.try_recv().is_ok() {}
            let claim = tokio::time::timeout(IDLE_POLL_CEILING * 2, self.claims.recv())
                .await
                .expect("idle loop keeps polling")
                .expect("idle claim");
            assert!(matches!(claim, Claim::Due(_)));
        }

        fn drain_claims(&mut self) -> Vec<Claim> {
            std::iter::from_fn(|| self.claims.try_recv().ok()).collect()
        }
    }

    impl Drop for LoopHarness {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn committed_row_delivers_without_waiting_for_poll() {
        let mut harness = LoopHarness::start();
        harness.settle_idle().await;

        let id = harness.commit();

        let delivered = tokio::time::timeout(IDLE_POLL_FLOOR, harness.delivered.recv())
            .await
            .expect("committed row delivers before any idle poll fires");
        assert_eq!(delivered, Some(id));
        assert_eq!(
            harness.drain_claims().first(),
            Some(&Claim::Committed(vec![id]))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn poll_delivers_row_that_was_never_handed_off() {
        let mut harness = LoopHarness::start();
        harness.settle_idle().await;

        let id = harness.enqueue();

        let delivered = tokio::time::timeout(IDLE_POLL_CEILING * 2, harness.delivered.recv())
            .await
            .expect("poll delivers the row");
        assert_eq!(delivered, Some(id));
        assert!(harness
            .drain_claims()
            .iter()
            .all(|claim| matches!(claim, Claim::Due(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn poll_delivers_row_whose_committed_claim_failed() {
        let mut harness = LoopHarness::start();
        harness.settle_idle().await;
        harness
            .fail_committed
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let id = harness.commit();

        let delivered = tokio::time::timeout(IDLE_POLL_CEILING * 2, harness.delivered.recv())
            .await
            .expect("poll delivers the row");
        assert_eq!(delivered, Some(id));
        let claims = harness.drain_claims();
        assert_eq!(claims.first(), Some(&Claim::Committed(vec![id])));
        assert!(matches!(claims.last(), Some(Claim::Due(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn steady_hand_offs_do_not_postpone_the_poll() {
        let mut harness = LoopHarness::start();
        harness.settle_idle().await;

        let polled = harness.enqueue();
        // Hand-offs arrive faster than the idle poll interval and their
        // deliveries never finish, so no completion triggers a poll either.
        let mut delivered = Vec::new();
        for _ in 0..MAX_IN_FLIGHT - 1 {
            harness.stall(harness.commit());
            tokio::time::sleep(IDLE_POLL_CEILING / 5).await;
            while let Ok(id) = harness.delivered.try_recv() {
                delivered.push(id);
            }
        }

        assert!(
            delivered.contains(&polled),
            "the poll must still run while hand-offs keep arriving"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_delivery_does_not_hold_up_later_rows() {
        let mut harness = LoopHarness::start();
        let stalled = harness.stall(harness.commit());
        assert_eq!(harness.delivered.recv().await, Some(stalled));
        harness.settle_idle().await;

        let next = harness.commit();

        let delivered = tokio::time::timeout(IDLE_POLL_FLOOR, harness.delivered.recv())
            .await
            .expect("later row delivers while the first is still stalled");
        assert_eq!(delivered, Some(next));
    }

    #[tokio::test(start_paused = true)]
    async fn in_flight_deliveries_are_bounded() {
        let mut harness = LoopHarness::start();
        for _ in 0..MAX_IN_FLIGHT * 2 {
            harness.stall(harness.enqueue());
        }
        tokio::time::sleep(IDLE_POLL_CEILING * 4).await;
        let waiting = harness.commit();
        tokio::time::sleep(IDLE_POLL_CEILING * 4).await;

        let mut delivered = 0;
        while harness.delivered.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(delivered, MAX_IN_FLIGHT);
        assert_eq!(
            harness.outbox.lock().expect("outbox lock").len(),
            MAX_IN_FLIGHT + 1
        );
        for claim in harness.drain_claims() {
            match claim {
                Claim::Due(limit) => assert!(limit > 0, "loop must not poll with no free slot"),
                Claim::Committed(ids) => panic!("loop claimed {ids:?} with no free slot"),
            }
        }
        assert!(harness
            .outbox
            .lock()
            .expect("outbox lock")
            .iter()
            .any(|delivery| delivery.id == waiting));
    }

    #[tokio::test]
    async fn unroutable_delivery_releases_claim_without_http_request() {
        let (_, relay_keypair, delivery) = delivery_fixture(1);
        let store = MockStore::default();
        let transport = MockTransport::new(Ok(StatusCode::NO_CONTENT));
        let before = Utc::now() + TimeDelta::seconds(CLAIM_SECS);

        deliver_one(
            &HashMap::new(),
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        let after = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
        assert!(transport.take_request().is_none());
        let calls = store.calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            StoreCall::Release(id, claim_id, next) => {
                assert_eq!(*id, delivery.id);
                assert_eq!(*claim_id, delivery.claim_id);
                assert!((before..=after).contains(next));
            }
            call => panic!("expected claim release, got {call:?}"),
        }
    }

    #[tokio::test]
    async fn successful_delivery_posts_payload_and_completes_claim() {
        let (routes, relay_keypair, delivery) = delivery_fixture(1);
        let loaded = stored_event(Kind::Custom(9));
        let expected = loaded.event.clone();
        let store = MockStore::loading(Ok(Some(loaded)));
        let transport = MockTransport::new(Ok(StatusCode::NO_CONTENT));

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        let request = transport.take_request().expect("delivery request");
        assert_eq!(request.url, routes[&hex::encode(&delivery.listener_pubkey)]);
        assert!(request.authorization.starts_with("Nostr "));
        let body: serde_json::Value = serde_json::from_slice(&request.body).expect("JSON body");
        assert_eq!(body["v"], 2);
        assert_eq!(body["pubkey"], hex::encode(&delivery.target_pubkey));
        assert_eq!(body["community_host"], delivery.community_host);
        assert_eq!(body["event_id"], hex::encode(&delivery.event_id));
        assert_eq!(body["event_kind"], delivery.event_kind);
        assert_eq!(
            body["event_created_at"],
            delivery.event_created_at.timestamp()
        );
        let event: nostr::Event =
            serde_json::from_value(body["event"].clone()).expect("signed event in body");
        assert_eq!(event, expected);
        event
            .verify()
            .expect("delivered event keeps a valid signature");
        assert_eq!(
            store.calls(),
            [StoreCall::Complete(delivery.id, delivery.claim_id)]
        );
    }

    #[tokio::test]
    async fn unreadable_event_retries_then_fails_without_http_request() {
        // Author-only kinds pass the database check but not the reader check.
        for loaded in [None, Some(stored_event(Kind::Custom(30300)))] {
            for (attempt, expect_retry) in [
                (1, true),
                (buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS, false),
            ] {
                let (routes, relay_keypair, delivery) = delivery_fixture(attempt);
                let store = MockStore::loading(Ok(loaded.clone()));
                let transport = MockTransport::new(Ok(StatusCode::NO_CONTENT));

                deliver_one(
                    &routes,
                    &relay_keypair,
                    &store,
                    &transport,
                    delivery.clone(),
                )
                .await;

                assert_eq!(transport.request_count(), 0);
                match store.calls().as_slice() {
                    [StoreCall::Retry(id, claim_id, _)] if expect_retry => {
                        assert_eq!((*id, *claim_id), (delivery.id, delivery.claim_id));
                    }
                    [StoreCall::Fail(id, claim_id)] if !expect_retry => {
                        assert_eq!((*id, *claim_id), (delivery.id, delivery.claim_id));
                    }
                    calls => panic!("attempt {attempt}: unexpected store calls {calls:?}"),
                }
            }
        }
    }

    #[tokio::test]
    async fn event_load_error_retries_without_http_request() {
        let (routes, relay_keypair, delivery) = delivery_fixture(1);
        let store = MockStore::loading(Err("database unavailable".to_owned()));
        let transport = MockTransport::new(Ok(StatusCode::NO_CONTENT));

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        assert_eq!(transport.request_count(), 0);
        assert!(matches!(
            store.calls().as_slice(),
            [StoreCall::Retry(id, claim_id, _)] if *id == delivery.id && *claim_id == delivery.claim_id
        ));
    }

    #[tokio::test]
    async fn rate_limit_and_server_errors_retry_with_exponential_delay() {
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let (routes, relay_keypair, delivery) = delivery_fixture(3);
            let store = MockStore::default();
            let transport = MockTransport::new(Ok(status));
            let before = Utc::now() + TimeDelta::seconds(4);

            deliver_one(
                &routes,
                &relay_keypair,
                &store,
                &transport,
                delivery.clone(),
            )
            .await;

            let after = Utc::now() + TimeDelta::seconds(4);
            assert_eq!(transport.request_count(), 1, "HTTP {status}");
            assert!(transport.take_request().is_some());
            let calls = store.calls();
            assert_eq!(calls.len(), 1, "HTTP {status}");
            match &calls[0] {
                StoreCall::Retry(id, claim_id, next) => {
                    assert_eq!(*id, delivery.id);
                    assert_eq!(*claim_id, delivery.claim_id);
                    assert!((before..=after).contains(next));
                }
                call => panic!("expected HTTP {status} retry, got {call:?}"),
            }
        }
    }

    #[tokio::test]
    async fn deterministic_client_error_fails_without_retrying() {
        let (routes, relay_keypair, delivery) = delivery_fixture(1);
        let store = MockStore::default();
        let transport = MockTransport::new(Ok(StatusCode::BAD_REQUEST));

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        assert_eq!(transport.request_count(), 1);
        assert!(transport.take_request().is_some());
        assert_eq!(
            store.calls(),
            [StoreCall::Fail(delivery.id, delivery.claim_id)]
        );
    }

    async fn read_http_request(stream: &mut TcpStream) -> (String, Vec<u8>, String) {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .await
            .expect("read request line");

        let mut content_length = 0;
        let mut authorization = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read header");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().expect("valid content length");
                } else if name.eq_ignore_ascii_case("authorization") {
                    authorization = value.trim().to_owned();
                }
            }
        }

        let mut body = vec![0; content_length];
        reader
            .read_exact(&mut body)
            .await
            .expect("read request body");
        (request_line, body, authorization)
    }

    #[tokio::test]
    async fn successful_http_delivery_is_signed_and_completes_claim() {
        use nostr::nips::nip98::{verify_auth_header, HttpMethod};

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("HTTP listener");
        let endpoint = url::Url::parse(&format!(
            "http://{}/mentions",
            listener.local_addr().expect("listener address")
        ))
        .expect("HTTP endpoint");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept delivery");
            let request = read_http_request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .expect("write success response");
            request
        });
        let (_, relay_keypair, delivery) = delivery_fixture(1);
        let routes = HashMap::from([(hex::encode(&delivery.listener_pubkey), endpoint.clone())]);
        let store = MockStore::default();
        let http = build_delivery_http_client(Duration::from_secs(2)).expect("delivery client");
        let transport = ReqwestDeliveryTransport { http: &http };

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        let (request_line, body, authorization) =
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("HTTP server completed")
                .expect("HTTP server task");
        assert!(request_line.starts_with("POST /mentions HTTP/1.1"));
        let signer = verify_auth_header(
            &authorization,
            &endpoint,
            HttpMethod::POST,
            nostr::Timestamp::now(),
            Some(&body),
        )
        .expect("valid signature binding the HTTP endpoint and body");
        assert_eq!(signer, relay_keypair.public_key());
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("notification JSON");
        assert_eq!(payload["pubkey"], hex::encode(&delivery.target_pubkey));
        assert_eq!(payload["event_id"], hex::encode(&delivery.event_id));
        assert_eq!(
            store.calls(),
            [StoreCall::Complete(delivery.id, delivery.claim_id)]
        );
    }

    #[tokio::test]
    async fn redirect_response_fails_delivery_without_retrying() {
        let source = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("source listener");
        let source_addr = source.local_addr().expect("source address");
        let target = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("target listener");
        let target_addr = target.local_addr().expect("target address");
        let target_url = format!("http://{target_addr}/redirected");
        let target_task = tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(1), target.accept()).await {
                Ok(Ok((mut stream, _))) => {
                    let request = read_http_request(&mut stream).await;
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .expect("write target success response");
                    Some(request)
                }
                Ok(Err(error)) => panic!("accept target request: {error}"),
                Err(_) => None,
            }
        });
        let (_, relay_keypair, delivery) = delivery_fixture(1);
        let source_url =
            url::Url::parse(&format!("http://{source_addr}/mentions")).expect("source URL");
        let routes = HashMap::from([(hex::encode(&delivery.listener_pubkey), source_url)]);
        let store = MockStore::default();
        let http = build_delivery_http_client(Duration::from_secs(2)).expect("delivery client");
        let transport = ReqwestDeliveryTransport { http: &http };

        let source_task = tokio::spawn(async move {
            let (mut stream, _) = source.accept().await.expect("accept source request");
            let request = read_http_request(&mut stream).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write redirect response");
            let second_request = tokio::time::timeout(Duration::from_millis(250), source.accept())
                .await
                .is_ok();
            (request, second_request)
        });

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        let (request, second_source_request) =
            tokio::time::timeout(Duration::from_secs(2), source_task)
                .await
                .expect("source server completed")
                .expect("source server task");
        assert!(request.0.starts_with("POST /mentions HTTP/1.1"));
        assert!(!request.1.is_empty());
        assert!(
            !second_source_request,
            "source received more than one request"
        );
        let target_request = tokio::time::timeout(Duration::from_secs(2), target_task)
            .await
            .expect("target server completed")
            .expect("target server task");
        assert_eq!(
            store.calls(),
            [StoreCall::Fail(delivery.id, delivery.claim_id)]
        );
        assert!(
            target_request.is_none(),
            "redirect target unexpectedly received a request"
        );
    }

    #[tokio::test]
    async fn exhausted_delivery_attempts_fail_permanently() {
        let (routes, relay_keypair, delivery) =
            delivery_fixture(buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS);
        let store = MockStore::default();
        let transport = MockTransport::new(Err("connection reset".to_owned()));

        deliver_one(
            &routes,
            &relay_keypair,
            &store,
            &transport,
            delivery.clone(),
        )
        .await;

        assert!(transport.take_request().is_some());
        assert_eq!(
            store.calls(),
            [StoreCall::Fail(delivery.id, delivery.claim_id)]
        );
    }
}
