//! Deployment-global operator-listener mention matching and notification delivery.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use futures_util::future::join_all;
use serde::Serialize;
use tracing::{error, warn};
use uuid::Uuid;

use crate::{nip98::nip98_header, state::AppState};

use reqwest::StatusCode;

const CLAIM_SECS: i64 = 30;
const DELIVERY_BATCH_LIMIT: i64 = 10;
const IDLE_POLL_FLOOR: Duration = Duration::from_millis(250);
const IDLE_POLL_CEILING: Duration = Duration::from_secs(2);
const OUTBOX_REAP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const REGISTRATION_REAP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Notification sent to the configured operator-listener endpoint.
#[derive(Debug, Serialize)]
struct MentionNotification {
    v: u8,
    pubkey: String,
    community_host: String,
    event_id: String,
    event_kind: i32,
    event_created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerIteration {
    Worked,
    Idle,
    Failed,
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
    let mut idle_delay = IDLE_POLL_FLOOR;
    loop {
        match run_delivery_once(&state, &http).await {
            WorkerIteration::Worked => idle_delay = IDLE_POLL_FLOOR,
            WorkerIteration::Idle => {
                tokio::time::sleep(idle_delay).await;
                idle_delay = (idle_delay * 2).min(IDLE_POLL_CEILING);
            }
            WorkerIteration::Failed => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

fn build_delivery_http_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

async fn run_delivery_once(state: &AppState, http: &reqwest::Client) -> WorkerIteration {
    let transport = ReqwestDeliveryTransport { http };
    run_delivery_once_with_transport(state, &transport).await
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
    async fn release(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String>;
    async fn complete(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String>;
    async fn retry(&self, id: Uuid, claim_id: Uuid, next: DateTime<Utc>) -> Result<bool, String>;
    async fn fail(&self, id: Uuid, claim_id: Uuid) -> Result<bool, String>;
}

struct DbDeliveryStore<'a> {
    db: &'a buzz_db::Db,
}

#[async_trait::async_trait]
impl DeliveryStore for DbDeliveryStore<'_> {
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

async fn run_delivery_once_with_transport<T: DeliveryTransport>(
    state: &AppState,
    transport: &T,
) -> WorkerIteration {
    let claimed = match state
        .db
        .claim_operator_listener_deliveries(
            DELIVERY_BATCH_LIMIT,
            Utc::now() + TimeDelta::seconds(CLAIM_SECS),
        )
        .await
    {
        Ok(claimed) => claimed,
        Err(error) => {
            error!(%error, "operator-listener delivery claim failed");
            return WorkerIteration::Failed;
        }
    };
    if claimed.is_empty() {
        return WorkerIteration::Idle;
    }
    let store = DbDeliveryStore { db: &state.db };
    join_all(claimed.into_iter().map(|delivery| {
        deliver_one(
            &state.config.operator_listener_delivery_urls,
            &state.relay_keypair,
            &store,
            transport,
            delivery,
        )
    }))
    .await;
    WorkerIteration::Worked
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
    let body = match serde_json::to_vec(&MentionNotification {
        v: 1,
        pubkey: hex::encode(&delivery.target_pubkey),
        community_host: delivery.community_host.clone(),
        event_id: hex::encode(&delivery.event_id),
        event_kind: delivery.event_kind,
        event_created_at: delivery.event_created_at.timestamp(),
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
    use nostr::Keys;

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

    #[derive(Default)]
    struct MockStore {
        calls: Mutex<Vec<StoreCall>>,
    }

    impl MockStore {
        fn calls(&self) -> Vec<StoreCall> {
            self.calls.lock().expect("store calls lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl DeliveryStore for MockStore {
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
        let store = MockStore::default();
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
        assert_eq!(body["v"], 1);
        assert_eq!(body["pubkey"], hex::encode(&delivery.target_pubkey));
        assert_eq!(body["community_host"], delivery.community_host);
        assert_eq!(body["event_id"], hex::encode(&delivery.event_id));
        assert_eq!(body["event_kind"], delivery.event_kind);
        assert_eq!(
            body["event_created_at"],
            delivery.event_created_at.timestamp()
        );
        assert_eq!(
            store.calls(),
            [StoreCall::Complete(delivery.id, delivery.claim_id)]
        );
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

    async fn read_http_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .await
            .expect("read request line");

        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read header");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().expect("valid content length");
                }
            }
        }

        let mut body = vec![0; content_length];
        reader
            .read_exact(&mut body)
            .await
            .expect("read request body");
        (request_line, body)
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
