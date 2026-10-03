//! Deployment-global operator-listener mention matching and notification delivery.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{TimeDelta, Utc};
use futures_util::future::join_all;
use serde::Serialize;
use tracing::{error, warn};

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
    run_delivery_once_with_db(
        &state.db,
        &state.config.operator_listener_delivery_urls,
        &state.relay_keypair,
        http,
    )
    .await
}

async fn run_delivery_once_with_db(
    db: &buzz_db::Db,
    routes: &HashMap<String, url::Url>,
    relay_keypair: &nostr::Keys,
    http: &reqwest::Client,
) -> WorkerIteration {
    let claimed = match db
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
            .map(|delivery| deliver_one(db, routes, relay_keypair, http, delivery)),
    )
    .await;
    WorkerIteration::Worked
}

async fn deliver_one(
    db: &buzz_db::Db,
    routes: &HashMap<String, url::Url>,
    relay_keypair: &nostr::Keys,
    http: &reqwest::Client,
    delivery: buzz_db::operator_listener::ClaimedDelivery,
) {
    let listener_hex = hex::encode(&delivery.listener_pubkey);
    let Some(url) = routes.get(&listener_hex) else {
        warn!(
            delivery=%delivery.id,
            listener=%listener_hex,
            "operator-listener delivery has no route on this pod; releasing claim"
        );
        if let Err(error) = db
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
                db,
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
            fail_permanently(db, &delivery, &format!("notification auth failed: {error}")).await;
            return;
        }
    };
    let response = http
        .post(url.clone())
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map(|response| response.status());
    match response {
        Ok(status) if status.is_success() => {
            match db
                .complete_operator_listener_delivery(delivery.id, delivery.claim_id)
                .await
            {
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
            retry_or_fail(db, &delivery, format!("HTTP {status}")).await
        }
        Ok(status) => {
            fail_permanently(db, &delivery, &format!("HTTP {status}")).await;
        }
        Err(error) => retry_or_fail(db, &delivery, error.to_string()).await,
    }
}

async fn fail_permanently(
    db: &buzz_db::Db,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: &str,
) {
    error!(delivery=%delivery.id, %reason, "operator-listener delivery failed permanently");
    if let Err(error) = db
        .fail_operator_listener_delivery(delivery.id, delivery.claim_id)
        .await
    {
        error!(delivery=%delivery.id, %error, "failed to delete terminal operator-listener delivery");
    }
    metrics::counter!("buzz_operator_listener_deliveries_total", "outcome" => "failed")
        .increment(1);
}

async fn retry_or_fail(
    db: &buzz_db::Db,
    delivery: &buzz_db::operator_listener::ClaimedDelivery,
    reason: String,
) {
    if delivery.attempt >= buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS {
        fail_permanently(db, delivery, &format!("retries exhausted: {reason}")).await;
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
    if let Err(error) = db
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
    use axum::{
        body::Bytes,
        extract::{OriginalUri, State},
        http::{HeaderMap, StatusCode as AxumStatusCode},
        response::IntoResponse,
        routing::post,
        Router,
    };
    use buzz_core::tenant::CommunityId;
    use chrono::DateTime;
    use nostr::Keys;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn reapers_use_their_expected_intervals() {
        assert_eq!(OUTBOX_REAP_INTERVAL, Duration::from_secs(5 * 60));
        assert_eq!(
            REGISTRATION_REAP_INTERVAL,
            Duration::from_secs(24 * 60 * 60)
        );
    }

    // The worker claims from the deployment-global outbox, so these tests
    // must not claim each other's fixture rows in parallel.
    static DB_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    mod postgres_tests {
        use super::*;

        struct Listener {
            status: AxumStatusCode,
            redirect: Option<String>,
            received: Mutex<Vec<Value>>,
        }

        async fn receive(
            State(listener): State<Arc<Listener>>,
            OriginalUri(uri): OriginalUri,
            headers: HeaderMap,
            body: Bytes,
        ) -> impl IntoResponse {
            let url: url::Url = format!("http://{}{}", headers["host"].to_str().unwrap(), uri)
                .parse()
                .unwrap();
            nostr::nips::nip98::verify_auth_header(
                headers["authorization"].to_str().unwrap(),
                &url,
                nostr::nips::nip98::HttpMethod::POST,
                nostr::Timestamp::now(),
                Some(&body),
            )
            .unwrap();
            assert_eq!(headers["content-type"], "application/json");
            listener
                .received
                .lock()
                .await
                .push(serde_json::from_slice(&body).unwrap());
            let mut response = listener.status.into_response();
            if let Some(location) = &listener.redirect {
                response
                    .headers_mut()
                    .insert(axum::http::header::LOCATION, location.parse().unwrap());
            }
            response
        }

        async fn start_listener(
            status: AxumStatusCode,
            redirect: Option<String>,
        ) -> (url::Url, Arc<Listener>, tokio::task::JoinHandle<()>) {
            let listener = Arc::new(Listener {
                status,
                redirect,
                received: Mutex::new(Vec::new()),
            });
            let socket = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind listener");
            let url = format!("http://{}/mentions", socket.local_addr().unwrap())
                .parse()
                .unwrap();
            let router = Router::new()
                .route("/mentions", post(receive))
                .with_state(listener.clone());
            let server = tokio::spawn(async move {
                axum::serve(socket, router).await.expect("serve listener");
            });
            (url, listener, server)
        }

        struct Fixture {
            pool: sqlx::PgPool,
            db: buzz_db::Db,
            community: CommunityId,
            id: Uuid,
            listener: Keys,
            relay: Keys,
            event_id: Vec<u8>,
            target_pubkey: Vec<u8>,
        }

        impl Fixture {
            async fn new(attempt: i32) -> Self {
                let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                    .await
                    .expect("connect to test DB");
                let db = buzz_db::Db::from_pool(pool.clone());
                let community = CommunityId::from_uuid(Uuid::new_v4());
                let host = format!("operator-delivery-{}.example", community.as_uuid().simple());
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(community.as_uuid())
                    .bind(host)
                    .execute(&pool)
                    .await
                    .expect("insert community");
                let listener = Keys::generate();
                let target_pubkey = Keys::generate().public_key().to_bytes().to_vec();
                let event_id = vec![0x22; 32];
                let id: Uuid = sqlx::query_scalar(
                "INSERT INTO operator_listener_outbox \
                 (listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at, attempts) \
                 VALUES ($1, $2, $3, $4, 9, to_timestamp(1700000000), $5) RETURNING id",
            )
            .bind(listener.public_key().as_bytes().as_slice())
            .bind(&target_pubkey)
            .bind(community.as_uuid())
            .bind(&event_id)
            .bind(attempt - 1)
            .fetch_one(&pool)
            .await
            .expect("insert delivery");
                Self {
                    pool,
                    db,
                    community,
                    id,
                    listener,
                    relay: Keys::generate(),
                    event_id,
                    target_pubkey,
                }
            }

            fn routes(&self, url: url::Url) -> HashMap<String, url::Url> {
                HashMap::from([(self.listener.public_key().to_hex(), url)])
            }

            async fn deliver(&self, routes: &HashMap<String, url::Url>) {
                let http = build_delivery_http_client(Duration::from_secs(2)).unwrap();
                assert_eq!(
                    run_delivery_once_with_db(&self.db, routes, &self.relay, &http).await,
                    WorkerIteration::Worked
                );
            }

            async fn row(
                &self,
            ) -> Option<(
                String,
                i32,
                Option<Uuid>,
                Option<DateTime<Utc>>,
                DateTime<Utc>,
            )> {
                use sqlx::Row;
                sqlx::query(
                    "SELECT state, attempts, claim_id, lease_until, next_attempt_at \
                 FROM operator_listener_outbox WHERE id = $1",
                )
                .bind(self.id)
                .fetch_optional(&self.pool)
                .await
                .expect("read outbox")
                .map(|row| {
                    (
                        row.get("state"),
                        row.get("attempts"),
                        row.get("claim_id"),
                        row.get("lease_until"),
                        row.get("next_attempt_at"),
                    )
                })
            }

            async fn cleanup(self) {
                sqlx::query("DELETE FROM operator_listener_outbox WHERE id = $1")
                    .bind(self.id)
                    .execute(&self.pool)
                    .await
                    .expect("delete delivery");
                sqlx::query("DELETE FROM communities WHERE id = $1")
                    .bind(self.community.as_uuid())
                    .execute(&self.pool)
                    .await
                    .expect("delete community");
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn successful_delivery_posts_signed_payload_and_completes_outbox() {
            let _guard = DB_TEST_LOCK.lock().await;
            let fixture = Fixture::new(1).await;
            let (url, listener, server) = start_listener(AxumStatusCode::NO_CONTENT, None).await;
            fixture.deliver(&fixture.routes(url)).await;
            let received = listener.received.lock().await;
            assert_eq!(received.len(), 1);
            let body = &received[0];
            assert_eq!(body["v"], 1);
            assert_eq!(body["pubkey"], hex::encode(&fixture.target_pubkey));
            assert_eq!(
                body["community_host"],
                format!(
                    "operator-delivery-{}.example",
                    fixture.community.as_uuid().simple()
                )
            );
            assert_eq!(body["event_id"], hex::encode(&fixture.event_id));
            assert_eq!(body["event_kind"], 9);
            assert_eq!(body["event_created_at"], 1_700_000_000);
            drop(received);
            assert!(
                fixture.row().await.is_none(),
                "success must remove the outbox row"
            );
            server.abort();
            fixture.cleanup().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn transient_responses_retry_with_persisted_delay() {
            let _guard = DB_TEST_LOCK.lock().await;
            for status in [
                AxumStatusCode::TOO_MANY_REQUESTS,
                AxumStatusCode::SERVICE_UNAVAILABLE,
            ] {
                let fixture = Fixture::new(3).await;
                let (url, listener, server) = start_listener(status, None).await;
                let before = Utc::now() + TimeDelta::seconds(4);
                fixture.deliver(&fixture.routes(url)).await;
                let after = Utc::now() + TimeDelta::seconds(4);
                assert_eq!(listener.received.lock().await.len(), 1, "HTTP {status}");
                let (state, attempts, claim, lease, next) = fixture.row().await.expect("retry row");
                assert_eq!(state, "pending");
                assert_eq!(attempts, 3);
                assert!(claim.is_none() && lease.is_none());
                assert!((before..=after).contains(&next));
                server.abort();
                fixture.cleanup().await;
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn client_error_fails_and_removes_outbox_row() {
            let _guard = DB_TEST_LOCK.lock().await;
            let fixture = Fixture::new(1).await;
            let (url, listener, server) = start_listener(AxumStatusCode::BAD_REQUEST, None).await;
            fixture.deliver(&fixture.routes(url)).await;
            assert_eq!(listener.received.lock().await.len(), 1);
            assert!(fixture.row().await.is_none());
            server.abort();
            fixture.cleanup().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn redirect_is_terminal_and_target_is_not_contacted() {
            let _guard = DB_TEST_LOCK.lock().await;
            let (target_url, target, target_server) =
                start_listener(AxumStatusCode::OK, None).await;
            let (source_url, source, source_server) =
                start_listener(AxumStatusCode::FOUND, Some(target_url.to_string())).await;
            let fixture = Fixture::new(1).await;
            fixture.deliver(&fixture.routes(source_url)).await;
            assert_eq!(source.received.lock().await.len(), 1);
            assert!(target.received.lock().await.is_empty());
            assert!(fixture.row().await.is_none());
            source_server.abort();
            target_server.abort();
            fixture.cleanup().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn exhausted_attempt_is_terminal() {
            let _guard = DB_TEST_LOCK.lock().await;
            let fixture = Fixture::new(buzz_db::operator_listener::MAX_DELIVERY_ATTEMPTS).await;
            let (url, listener, server) =
                start_listener(AxumStatusCode::SERVICE_UNAVAILABLE, None).await;
            fixture.deliver(&fixture.routes(url)).await;
            assert_eq!(listener.received.lock().await.len(), 1);
            assert!(fixture.row().await.is_none());
            server.abort();
            fixture.cleanup().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn unroutable_delivery_releases_claim_without_http() {
            let _guard = DB_TEST_LOCK.lock().await;
            let fixture = Fixture::new(1).await;
            let before = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
            fixture.deliver(&HashMap::new()).await;
            let after = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
            let (state, attempts, claim, lease, next) = fixture.row().await.expect("released row");
            assert_eq!(state, "pending");
            assert_eq!(attempts, 0);
            assert!(claim.is_none() && lease.is_none());
            assert!((before..=after).contains(&next));
            fixture.cleanup().await;
        }
    }
}
