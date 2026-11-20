//! Delivery authentication through the public router, with an in-memory authority
//! store and APNs transport. No external service or credentials are used.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use crate::{
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    app_attest::AppAttestVerifier,
    authority::{
        AuthorityError, AuthorityStore, Challenge, Delegation, DeliveryDisposition, DeliveryPermit,
        Installation, MemoryAuthorityStore, NewInstallation, Platform,
    },
    grant::{GrantKey, GrantKeyring},
    http::ApplicationRuntime,
    model::EndpointGrant,
    router,
    token::{TokenKey, TokenKeyring},
    AppState,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const DIRECT_URL: &str = "http://push.example:8080/v1/deliveries";
const FORWARDED_URL: &str = "https://push.example/v1/deliveries";
// The APNs-named alias of the delivery route.
const APNS_DIRECT_URL: &str = "http://push.example:8080/v1/deliveries/apns";
const APNS_FORWARDED_URL: &str = "https://push.example/v1/deliveries/apns";

struct TestTransport(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl PushTransport for TestTransport {
    async fn send(&self, _: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        assert_eq!(endpoint, "01020304");
        self.0.fetch_add(1, Ordering::SeqCst);
        DeliveryOutcome::Accepted
    }
}

fn now() -> i64 {
    1_750_000_000
}

async fn fixture() -> (axum::Router, Keys, Vec<u8>, Arc<AtomicUsize>) {
    let sends = Arc::new(AtomicUsize::new(0));
    let (app, keys, body) = fixture_with(Arc::new(TestTransport(sends.clone()))).await;
    (app, keys, body, sends)
}

async fn fixture_with(transport: Arc<dyn PushTransport>) -> (axum::Router, Keys, Vec<u8>) {
    fixture_with_store(transport, Arc::new(MemoryAuthorityStore::default())).await
}

async fn fixture_with_store(
    transport: Arc<dyn PushTransport>,
    authority: Arc<dyn AuthorityStore>,
) -> (axum::Router, Keys, Vec<u8>) {
    let keys = Keys::generate();
    let token_keyring =
        Arc::new(TokenKeyring::new(vec![TokenKey::new("test", &[2; 32]).unwrap()]).unwrap());
    let grant_keyring =
        Arc::new(GrantKeyring::new(vec![GrantKey::new("test", &[1; 32]).unwrap()]).unwrap());
    let installation_id = Uuid::new_v4();
    authority
        .create_installation(
            NewInstallation {
                platform: Platform::Ios,
                id: installation_id,
                app_attest_key_id: vec![3; 32],
                app_attest_public_key: vec![4; 65],
                assertion_counter: 0,

                token_ciphertext: token_keyring.seal(&[1, 2, 3, 4]).unwrap(),
                token_fingerprint: [5; 32],
                endpoint_epoch: 1,
                expires_at: now() + 600,
            },
            now(),
        )
        .await
        .unwrap();
    let delegation_id = Uuid::new_v4();
    authority
        .upsert_delegation(Delegation {
            id: delegation_id,
            installation_id,
            relay_pubkey: keys.public_key().to_hex(),
            endpoint_epoch: 1,
            generation: 1,
            not_before: now(),
            expires_at: now() + 600,
            revoked: false,
        })
        .await
        .unwrap();
    let grant = grant_keyring
        .issue(&EndpointGrant {
            v: 1,
            delegation_id,
            relay_pubkey: keys.public_key().to_hex(),

            endpoint_epoch: 1,
            generation: 1,
            expires_at: now() + 600,
        })
        .unwrap();
    let (app, _) = router(AppState {
        grant_keyring,
        authority,
        token_keyring,
        application: Arc::new(ApplicationRuntime {
            app_attest: Arc::new(
                AppAttestVerifier::new(
                    "TEAMID.com.example.buzz".into(),
                    include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
                )
                .unwrap(),
            ),
            transport,
            android: None,
        }),
        max_grant_lifetime_seconds: 600,
        max_installation_lifetime_seconds: 600,
        endpoint_quota_window_seconds: 60,
        endpoint_quota_max_deliveries: 10,
        signed_calls: Arc::new(crate::signed_call_limiter::SignedCallLimiter::new(
            60, 1_000,
        )),
        now,
        accepting: Arc::new(AtomicBool::new(true)),
    });
    let body = serde_json::to_vec(&serde_json::json!({
        "v": 1, "endpoint_grant": grant, "request_id": Uuid::new_v4(), "expires_at": now() + 60,
    }))
    .unwrap();
    (app, keys, body)
}

fn signed_header(keys: &Keys, url: &str, method: &str, body: &[u8]) -> String {
    signed_header_at(keys, url, method, body, Timestamp::now())
}

/// A distinct `created_at` gives each retry attempt its own NIP-98 event id,
/// as the relay's per-attempt signing does.
fn signed_header_at(
    keys: &Keys,
    url: &str,
    method: &str,
    body: &[u8],
    created_at: Timestamp,
) -> String {
    let hash = hex::encode(Sha256::digest(body));
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
            Tag::parse(["payload", &hash]).unwrap(),
        ])
        .custom_created_at(created_at)
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_vec(&event).unwrap())
    )
}

fn request(url: &str, proto: Option<&str>, auth: String, body: Vec<u8>) -> Request<Body> {
    let url = url::Url::parse(url).unwrap();
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let mut request = Request::post(path)
        .header("host", authority)
        .header("authorization", auth);
    if let Some(proto) = proto {
        request = request.header("x-forwarded-proto", proto);
    }
    request.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn delivery_binds_path_not_origin_or_forwarding_headers() {
    for (request_url, signed_url) in [
        (DIRECT_URL, DIRECT_URL),
        (DIRECT_URL, FORWARDED_URL),
        (DIRECT_URL, "https://other.example:8443/v1/deliveries"),
        (APNS_DIRECT_URL, APNS_DIRECT_URL),
        (APNS_DIRECT_URL, APNS_FORWARDED_URL),
        (
            APNS_DIRECT_URL,
            "https://other.example:8443/v1/deliveries/apns",
        ),
    ] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, signed_url, "POST", &body);
        let mut request = request(request_url, Some("invalid,https"), auth, body);
        request.headers_mut().remove("host");
        request
            .headers_mut()
            .append("x-forwarded-proto", "http".parse().unwrap());
        request
            .headers_mut()
            .insert("x-forwarded-host", "unrelated.example".parse().unwrap());
        request.headers_mut().insert(
            "forwarded",
            "proto=ftp;host=unrelated.example".parse().unwrap(),
        );
        assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::OK);
        assert_eq!(sends.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn signature_is_bound_to_delivery_path_method_and_body() {
    for (request_url, url, method, change_body) in [
        (DIRECT_URL, "https://push.example/v1/other", "POST", false),
        (
            DIRECT_URL,
            "https://push.example/v1/deliveries/",
            "POST",
            false,
        ),
        (
            DIRECT_URL,
            "https://push.example/v1/%64eliveries",
            "POST",
            false,
        ),
        (DIRECT_URL, DIRECT_URL, "GET", false),
        (DIRECT_URL, DIRECT_URL, "POST", true),
        // A request signed for one route is not accepted on the other.
        (DIRECT_URL, APNS_FORWARDED_URL, "POST", false),
        (APNS_DIRECT_URL, FORWARDED_URL, "POST", false),
        (
            APNS_DIRECT_URL,
            "https://push.example/v1/other",
            "POST",
            false,
        ),
        (
            APNS_DIRECT_URL,
            "https://push.example/v1/deliveries/apns/",
            "POST",
            false,
        ),
        (
            APNS_DIRECT_URL,
            "https://push.example/v1/deliveries/%61pns",
            "POST",
            false,
        ),
        (APNS_DIRECT_URL, APNS_DIRECT_URL, "GET", false),
        (APNS_DIRECT_URL, APNS_DIRECT_URL, "POST", true),
    ] {
        let (app, keys, mut body, sends) = fixture().await;
        let auth = signed_header(&keys, url, method, &body);
        if change_body {
            body.push(b' ');
        }
        assert_eq!(
            app.oneshot(request(request_url, None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED,
            "{request_url} {url} {method}"
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn signed_url_modifiers_and_non_http_schemes_are_rejected() {
    for (request_url, url) in [
        (DIRECT_URL, "https://push.example/v1/deliveries?mode=one"),
        (DIRECT_URL, "https://push.example/v1/deliveries?"),
        (DIRECT_URL, "https://push.example/v1/deliveries#fragment"),
        (DIRECT_URL, "https://push.example/v1/deliveries#"),
        (DIRECT_URL, "https://user@push.example/v1/deliveries"),
        (DIRECT_URL, "ftp://push.example/v1/deliveries"),
        (
            APNS_DIRECT_URL,
            "https://push.example/v1/deliveries/apns?mode=one",
        ),
        (APNS_DIRECT_URL, "https://push.example/v1/deliveries/apns?"),
        (
            APNS_DIRECT_URL,
            "https://push.example/v1/deliveries/apns#fragment",
        ),
        (APNS_DIRECT_URL, "https://push.example/v1/deliveries/apns#"),
        (
            APNS_DIRECT_URL,
            "https://user@push.example/v1/deliveries/apns",
        ),
        (APNS_DIRECT_URL, "ftp://push.example/v1/deliveries/apns"),
    ] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, url, "POST", &body);
        assert_eq!(
            app.oneshot(request(request_url, None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED,
            "{url}"
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn request_queries_are_rejected() {
    for url in [DIRECT_URL, APNS_DIRECT_URL] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, url, "POST", &body);
        assert_eq!(
            app.oneshot(request(&format!("{url}?mode=one"), None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn router_rejects_other_methods_and_paths() {
    for (method, path, expected) in [
        (
            "GET",
            crate::http::DELIVERY_PATH,
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            "GET",
            crate::http::APNS_DELIVERY_ALIAS_PATH,
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        ("POST", "/v1/other", StatusCode::NOT_FOUND),
        ("POST", "/v1/deliveries/", StatusCode::NOT_FOUND),
    ] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", auth)
            .body(Body::from(body))
            .unwrap();
        assert_eq!(app.oneshot(request).await.unwrap().status(), expected);
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn verifier_retains_kind_timestamp_and_payload_requirements() {
    for (kind, timestamp, payload) in [
        (Kind::TextNote, Timestamp::now(), true),
        (
            Kind::HttpAuth,
            Timestamp::from(Timestamp::now().as_secs() - 120),
            true,
        ),
        (
            Kind::HttpAuth,
            Timestamp::from(Timestamp::now().as_secs() + 120),
            true,
        ),
        (Kind::HttpAuth, Timestamp::now(), false),
    ] {
        let (app, keys, body, sends) = fixture().await;
        let hash = hex::encode(Sha256::digest(&body));
        let mut tags = vec![
            Tag::parse(["u", DIRECT_URL]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
        ];
        if payload {
            tags.push(Tag::parse(["payload", &hash]).unwrap());
        }
        let event = EventBuilder::new(kind, "")
            .tags(tags)
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
            .unwrap();
        let auth = format!(
            "Nostr {}",
            STANDARD.encode(serde_json::to_vec(&event).unwrap())
        );
        assert_eq!(
            app.oneshot(request(DIRECT_URL, None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn invalid_event_id_and_signature_are_rejected() {
    for field in ["id", "sig"] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        let mut event: serde_json::Value = serde_json::from_slice(
            &STANDARD
                .decode(auth.strip_prefix("Nostr ").unwrap())
                .unwrap(),
        )
        .unwrap();
        let size = event[field].as_str().unwrap().len();
        event[field] = "0".repeat(size).into();
        let auth = format!(
            "Nostr {}",
            STANDARD.encode(serde_json::to_vec(&event).unwrap())
        );
        assert_eq!(
            app.oneshot(request(DIRECT_URL, None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn exhausted_endpoint_quota_answers_429_rate_limited() {
    let (app, keys, body, sends) = fixture().await;
    let mut delivery: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut deliver = || {
        delivery["request_id"] = serde_json::json!(Uuid::new_v4());
        let body = serde_json::to_vec(&delivery).unwrap();
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        app.clone().oneshot(request(DIRECT_URL, None, auth, body))
    };
    for _ in 0..10 {
        assert_eq!(deliver().await.unwrap().status(), StatusCode::OK);
    }
    let response = deliver().await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], br#"{"error":"rate_limited"}"#);
    assert_eq!(sends.load(Ordering::SeqCst), 10);
}

#[tokio::test]
async fn grant_signer_and_replay_checks_remain_enforced() {
    let (app, keys, body, sends) = fixture().await;
    let wrong_signer = signed_header(&Keys::generate(), DIRECT_URL, "POST", &body);
    assert_eq!(
        app.clone()
            .oneshot(request(DIRECT_URL, None, wrong_signer, body.clone()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
    assert_eq!(
        app.clone()
            .oneshot(request(DIRECT_URL, None, auth.clone(), body.clone()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request(DIRECT_URL, None, auth, body))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(sends.load(Ordering::SeqCst), 1);
}

/// Holds its first send open until the test releases it, then answers each
/// send with the next scripted outcome.
struct GatedTransport {
    sends: AtomicUsize,
    first_send_entered: tokio::sync::Notify,
    release_first_send: tokio::sync::Semaphore,
    outcomes: std::sync::Mutex<std::collections::VecDeque<DeliveryOutcome>>,
}

#[async_trait::async_trait]
impl PushTransport for GatedTransport {
    async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
        if self.sends.fetch_add(1, Ordering::SeqCst) == 0 {
            self.first_send_entered.notify_one();
            self.release_first_send.acquire().await.unwrap().forget();
        }
        self.outcomes.lock().unwrap().pop_front().unwrap()
    }
}

async fn status_and_body(
    app: &axum::Router,
    auth: String,
    body: Vec<u8>,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(request(DIRECT_URL, None, auth, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

// The relay times out a slow gateway call and retries the same request id with
// a fresh NIP-98 event. While the first attempt is still sending, the retry
// must be told to retry later, not answered with the 404 the relay reads as
// "already delivered": the first attempt can still fail retryably.
#[tokio::test]
async fn replay_while_first_attempt_is_in_flight_is_retried_not_dropped() {
    let transport = Arc::new(GatedTransport {
        sends: AtomicUsize::new(0),
        first_send_entered: tokio::sync::Notify::new(),
        release_first_send: tokio::sync::Semaphore::new(0),
        outcomes: std::sync::Mutex::new(
            [
                DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                },
                DeliveryOutcome::Accepted,
            ]
            .into(),
        ),
    });
    let (app, keys, body) = fixture_with(transport.clone()).await;
    let at = |seconds_ago: u64| {
        signed_header_at(
            &keys,
            DIRECT_URL,
            "POST",
            &body,
            Timestamp::from(Timestamp::now().as_secs() - seconds_ago),
        )
    };

    let entered = transport.first_send_entered.notified();
    let first = tokio::spawn({
        let (app, auth, body) = (app.clone(), at(4), body.clone());
        async move { status_and_body(&app, auth, body).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered)
        .await
        .expect("first attempt reaches the transport");

    let replay = status_and_body(&app, at(3), body.clone()).await;
    assert_eq!(
        replay,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"status": "retry", "retry_after_seconds": 5})
        )
    );
    assert_eq!(
        transport.sends.load(Ordering::SeqCst),
        1,
        "the replay sends nothing"
    );

    transport.release_first_send.add_permits(1);
    let (status, _) = first.await.unwrap();
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "first attempt failed retryably"
    );

    let (status, _) = status_and_body(&app, at(2), body.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the released request id is sent again"
    );
    let (status, _) = status_and_body(&app, at(1), body).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a finished request id stays burned"
    );
    assert_eq!(transport.sends.load(Ordering::SeqCst), 2);
}

struct FixedOutcome(DeliveryOutcome);

#[async_trait::async_trait]
impl PushTransport for FixedOutcome {
    async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
        self.0
    }
}

/// The in-memory store, except that settling a delivery always fails, as it
/// does when the database errors or the pool times out after the send.
#[derive(Default)]
struct FailingFinishStore(MemoryAuthorityStore);

#[async_trait::async_trait]
impl AuthorityStore for FailingFinishStore {
    async fn ready(&self) -> Result<(), AuthorityError> {
        self.0.ready().await
    }
    async fn put_challenge(&self, challenge: Challenge) -> Result<(), AuthorityError> {
        self.0.put_challenge(challenge).await
    }
    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError> {
        self.0.consume_challenge(id, value, now).await
    }
    async fn create_installation(
        &self,
        installation: NewInstallation,
        now: i64,
    ) -> Result<(), AuthorityError> {
        self.0.create_installation(installation, now).await
    }
    async fn matching_installation(
        &self,
        app_attest_key_id: &[u8],
        token_fingerprint: [u8; 32],
        endpoint_epoch: i64,
        expires_at: i64,
        now: i64,
    ) -> Result<Option<Installation>, AuthorityError> {
        self.0
            .matching_installation(
                app_attest_key_id,
                token_fingerprint,
                endpoint_epoch,
                expires_at,
                now,
            )
            .await
    }
    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError> {
        self.0.installation(id, now).await
    }
    async fn installation_for_revocation(
        &self,
        id: Uuid,
        now: i64,
    ) -> Result<Installation, AuthorityError> {
        self.0.installation_for_revocation(id, now).await
    }
    async fn advance_assertion_counter(
        &self,
        installation_id: Uuid,
        previous: u32,
        next: u32,
    ) -> Result<(), AuthorityError> {
        self.0
            .advance_assertion_counter(installation_id, previous, next)
            .await
    }
    async fn upsert_delegation(&self, delegation: Delegation) -> Result<(), AuthorityError> {
        self.0.upsert_delegation(delegation).await
    }
    async fn rotate_endpoint(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
        token_ciphertext: Vec<u8>,
        token_fingerprint: [u8; 32],
    ) -> Result<(), AuthorityError> {
        self.0
            .rotate_endpoint(
                installation_id,
                expected_epoch,
                new_epoch,
                token_ciphertext,
                token_fingerprint,
            )
            .await
    }
    async fn revoke_delegation(
        &self,
        installation_id: Uuid,
        relay_pubkey: &str,
        expected_generation: i64,
    ) -> Result<(), AuthorityError> {
        self.0
            .revoke_delegation(installation_id, relay_pubkey, expected_generation)
            .await
    }
    async fn revoke_installation(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
    ) -> Result<(), AuthorityError> {
        self.0
            .revoke_installation(installation_id, expected_epoch, new_epoch)
            .await
    }
    async fn authorize_delivery(
        &self,
        delegation_id: Uuid,
        relay_pubkey: &str,
        endpoint_epoch: i64,
        generation: i64,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        self.0
            .authorize_delivery(
                delegation_id,
                relay_pubkey,
                endpoint_epoch,
                generation,
                event_id,
                request_id,
                request_expires_at,
                quota_window_seconds,
                quota_max_deliveries,
                now,
            )
            .await
    }
    async fn finish_delivery(
        &self,
        _: DeliveryPermit,
        _: DeliveryDisposition,
    ) -> Result<(), AuthorityError> {
        Err(AuthorityError::Unavailable)
    }
    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError> {
        self.0.reap_expired(now).await
    }
}

/// Once the provider has answered for good, a failure to record that is the
/// gateway's problem: the relay still gets the provider's answer. A retryable
/// answer still becomes a gateway retry, as before.
#[tokio::test]
async fn failed_finish_reports_terminal_outcomes_and_retries_retryable_ones() {
    for (outcome, expected) in [
        (DeliveryOutcome::Accepted, StatusCode::OK),
        (
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            },
            StatusCode::GONE,
        ),
        (
            DeliveryOutcome::Retry {
                retry_after_seconds: None,
            },
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ] {
        let (app, keys, body) = fixture_with_store(
            Arc::new(FixedOutcome(outcome)),
            Arc::new(FailingFinishStore::default()),
        )
        .await;
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        let (status, _) = status_and_body(&app, auth, body).await;
        assert_eq!(status, expected, "{outcome:?}");
    }
}
