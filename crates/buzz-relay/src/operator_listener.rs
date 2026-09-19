//! Deployment-global operator-listener mention matching and webhook delivery.

use std::{sync::Arc, time::Duration};

use chrono::{TimeDelta, Utc};
use futures_util::future::join_all;
use serde::Serialize;
use tracing::{error, warn};

use crate::{nip98::nip98_header, state::AppState};

use reqwest::StatusCode;

const CLAIM_SECS: i64 = 30;
const MATCH_BATCH_LIMIT: i64 = 64;
const DELIVERY_BATCH_LIMIT: i64 = 10;
const IDLE_POLL_FLOOR: Duration = Duration::from_millis(250);
const IDLE_POLL_CEILING: Duration = Duration::from_secs(2);
const REAP_INTERVAL: Duration = Duration::from_secs(60);

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

/// Continuously expand mention jobs into the durable webhook outbox.
pub async fn run_matcher(state: Arc<AppState>) {
    let mut idle_delay = IDLE_POLL_FLOOR;
    loop {
        match run_matcher_once(&state).await {
            WorkerIteration::Worked => idle_delay = IDLE_POLL_FLOOR,
            WorkerIteration::Idle => {
                tokio::time::sleep(idle_delay).await;
                idle_delay = (idle_delay * 2).min(IDLE_POLL_CEILING);
            }
            WorkerIteration::Failed => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn run_matcher_once(state: &AppState) -> WorkerIteration {
    let lease_until = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
    match state
        .db
        .claim_operator_listener_match_jobs(MATCH_BATCH_LIMIT, lease_until)
        .await
    {
        Ok(Some(claim_id)) => {
            if let Err(error) = state.db.match_operator_listener_jobs(claim_id).await {
                warn!(%claim_id, %error, "operator-listener matching failed");
                let _ = state
                    .db
                    .retry_operator_listener_match_jobs(
                        claim_id,
                        Utc::now() + TimeDelta::seconds(2),
                    )
                    .await;
            }
            WorkerIteration::Worked
        }
        Ok(None) => WorkerIteration::Idle,
        Err(error) => {
            error!(%error, "operator-listener matcher claim failed");
            WorkerIteration::Failed
        }
    }
}

/// Reap stale operator-listener rows even when listener workers are disabled.
pub async fn run_reaper(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(REAP_INTERVAL).await;
        if let Err(error) = state.db.reap_operator_listener_match_jobs().await {
            warn!(%error, "operator-listener matcher reap failed");
        }
        if let Err(error) = state.db.reap_operator_listener_deliveries().await {
            warn!(%error, "operator-listener delivery reap failed");
        }
    }
}

/// Continuously deliver operator-listener webhook rows with bounded concurrency and retries.
pub async fn run_delivery_worker(state: Arc<AppState>) {
    let http = match reqwest::Client::builder()
        .timeout(state.config.operator_listener_timeout)
        .build()
    {
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
    join_all(
        claimed
            .into_iter()
            .map(|delivery| deliver_one(state, transport, delivery)),
    )
    .await;
    WorkerIteration::Worked
}

async fn deliver_one<T: DeliveryTransport>(
    state: &AppState,
    transport: &T,
    delivery: buzz_db::operator_listener::ClaimedDelivery,
) {
    let listener_hex = hex::encode(&delivery.listener_pubkey);
    let Some(url) = state
        .config
        .operator_listener_delivery_urls
        .get(&listener_hex)
    else {
        warn!(
            delivery=%delivery.id,
            listener=%listener_hex,
            "operator-listener delivery has no route on this pod; releasing claim"
        );
        if let Err(error) = state
            .db
            .release_operator_listener_delivery(
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
                state,
                &delivery,
                &format!("notification encoding failed: {error}"),
            )
            .await;
            return;
        }
    };
    let auth = match nip98_header(&state.relay_keypair, url.as_str(), &body) {
        Ok(auth) => auth,
        Err(error) => {
            fail_permanently(
                state,
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
            let _ = state
                .db
                .complete_operator_listener_delivery(delivery.id, delivery.claim_id)
                .await;
            metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "accepted")
                .increment(1);
        }
        Ok(status) => retry_or_fail(state, &delivery, format!("HTTP {status}")).await,
        Err(error) => retry_or_fail(state, &delivery, error.to_string()).await,
    }
}

async fn fail_permanently(
    state: &AppState,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: &str,
) {
    error!(delivery=%delivery.id, %reason, "operator-listener delivery failed permanently");
    if let Err(error) = state
        .db
        .fail_operator_listener_delivery(delivery.id, delivery.claim_id)
        .await
    {
        error!(delivery=%delivery.id, %error, "failed to delete terminal operator-listener delivery");
    }
    metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "failed")
        .increment(1);
}

async fn retry_or_fail(
    state: &AppState,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: String,
) {
    if delivery.attempt >= buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS {
        fail_permanently(state, delivery, &format!("retries exhausted: {reason}")).await;
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
    if let Err(error) = state
        .db
        .retry_operator_listener_delivery(
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
    use std::sync::{Arc, Mutex};

    use buzz_core::CommunityId;
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::{PgPool, Row};
    use uuid::Uuid;

    use super::*;

    #[derive(Clone)]
    struct RecordingTransport {
        status: reqwest::StatusCode,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
    }

    #[derive(Clone)]
    struct RecordedRequest {
        url: String,
        authorization: String,
        body: Vec<u8>,
    }

    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static MIGRATIONS: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    async fn test_lock() -> tokio::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().await
    }

    #[async_trait::async_trait]
    impl DeliveryTransport for RecordingTransport {
        async fn post(
            &self,
            url: &url::Url,
            authorization: &str,
            body: Vec<u8>,
        ) -> Result<reqwest::StatusCode, String> {
            self.requests
                .lock()
                .expect("request lock")
                .push(RecordedRequest {
                    url: url.to_string(),
                    authorization: authorization.to_owned(),
                    body,
                });
            Ok(self.status)
        }
    }

    async fn setup() -> (PgPool, Arc<AppState>) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("TEST_DATABASE_URL"))
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("Postgres test URL");
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to Postgres");
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            let migration_pool = pool.clone();
            MIGRATIONS
                .get_or_init(|| async move {
                    buzz_db::migration::run_migrations(&migration_pool)
                        .await
                        .expect("run migrations");
                })
                .await;
        }
        sqlx::query("DELETE FROM operator_listener_outbox")
            .execute(&pool)
            .await
            .expect("clear operator-listener outbox");
        sqlx::query("DELETE FROM operator_listener_match_queue")
            .execute(&pool)
            .await
            .expect("clear operator-listener match queue");
        let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        (pool, state)
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!(
                "operator-listener-relay-test-{}.example",
                id.simple()
            ))
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    async fn insert_delivery(pool: &PgPool, listener: &Keys, target: &Keys) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO operator_listener_outbox \
             (id, listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(listener.public_key().to_bytes().as_slice())
        .bind(target.public_key().to_bytes().as_slice())
        .bind(make_community(pool).await)
        .bind([7_u8; 32].as_slice())
        .bind(9_i32)
        .bind(Utc::now())
        .execute(pool)
        .await
        .expect("insert delivery");
        id
    }

    fn add_route(state: &mut Arc<AppState>, listener: &Keys) {
        let state = Arc::get_mut(state).expect("test state is uniquely owned");
        Arc::get_mut(&mut state.config)
            .expect("test state config is uniquely owned")
            .operator_listener_delivery_urls
            .insert(
                listener.public_key().to_hex(),
                url::Url::parse("https://listener.example/mentions").expect("URL"),
            );
    }

    mod postgres_tests {
        use super::*;

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn matcher_iteration_expands_triggered_mention() {
            let _test_lock = test_lock().await;
            let (pool, state) = setup().await;
            let community = make_community(&pool).await;
            let listener = Keys::generate();
            let target = Keys::generate();
            state
                .db
                .register_operator_listener_pubkeys(
                    listener.public_key().as_bytes(),
                    &[target.public_key().to_bytes().to_vec()],
                )
                .await
                .expect("register target");

            let event = EventBuilder::new(Kind::Custom(9), "mention")
                .tag(Tag::parse(["p", target.public_key().to_hex().as_str()]).expect("p tag"))
                .sign_with_keys(&Keys::generate())
                .expect("sign event");
            let community = CommunityId::from_uuid(community);
            let (_, inserted) = buzz_db::event::insert_event(&pool, community, &event, None)
                .await
                .expect("insert event");
            assert!(inserted);
            buzz_db::insert_mentions(&pool, community, &event, None)
                .await
                .expect("index mention");

            assert_eq!(run_matcher_once(&state).await, WorkerIteration::Worked);
            let deliveries = state
                .db
                .claim_operator_listener_deliveries(10, Utc::now() + TimeDelta::seconds(30))
                .await
                .expect("claim delivery");
            assert_eq!(deliveries.len(), 1);
            assert_eq!(
                deliveries[0].listener_pubkey,
                listener.public_key().to_bytes()
            );
            assert_eq!(deliveries[0].target_pubkey, target.public_key().to_bytes());
            assert_eq!(deliveries[0].event_id, event.id.as_bytes());
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn delivery_worker_posts_signed_notification_and_completes() {
            let _test_lock = test_lock().await;
            let (pool, mut state) = setup().await;
            let listener = Keys::generate();
            let target = Keys::generate();
            add_route(&mut state, &listener);
            let delivery_id = insert_delivery(&pool, &listener, &target).await;
            let requests = Arc::new(Mutex::new(Vec::new()));
            let transport = RecordingTransport {
                status: StatusCode::NO_CONTENT,
                requests: Arc::clone(&requests),
            };

            assert_eq!(
                run_delivery_once_with_transport(&state, &transport).await,
                WorkerIteration::Worked
            );
            let request = requests
                .lock()
                .expect("request lock")
                .pop()
                .expect("request");
            let body: serde_json::Value = serde_json::from_slice(&request.body).expect("JSON body");
            assert_eq!(request.url, "https://listener.example/mentions");
            assert_eq!(body["pubkey"], target.public_key().to_hex());
            assert_eq!(body["event_kind"], 9);

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                "authorization",
                axum::http::HeaderValue::from_str(&request.authorization).expect("auth header"),
            );
            let verified = crate::api::bridge::verify_bridge_auth_with_options(
                &headers,
                "POST",
                &request.url,
                Some(&request.body),
                true,
                true,
            )
            .expect("valid relay NIP-98 signature");
            assert_eq!(verified.pubkey, state.relay_keypair.public_key());
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM operator_listener_outbox WHERE id = $1",
                )
                .bind(delivery_id)
                .fetch_one(&pool)
                .await
                .expect("count delivery"),
                0
            );
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn delivery_worker_persists_retry_after_failure() {
            let _test_lock = test_lock().await;
            let (pool, mut state) = setup().await;
            let listener = Keys::generate();
            let target = Keys::generate();
            add_route(&mut state, &listener);
            let delivery_id = insert_delivery(&pool, &listener, &target).await;
            let transport = RecordingTransport {
                status: StatusCode::BAD_GATEWAY,
                requests: Arc::new(Mutex::new(Vec::new())),
            };

            assert_eq!(
                run_delivery_once_with_transport(&state, &transport).await,
                WorkerIteration::Worked
            );
            let row = sqlx::query(
                "SELECT state, attempts, next_attempt_at > now() AS delayed \
                 FROM operator_listener_outbox WHERE id = $1",
            )
            .bind(delivery_id)
            .fetch_one(&pool)
            .await
            .expect("read retry");
            assert_eq!(row.get::<String, _>("state"), "pending");
            assert_eq!(row.get::<i32, _>("attempts"), 1);
            assert!(row.get::<bool, _>("delayed"));
        }
    }
}
