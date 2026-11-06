//! Delivery authentication through the public router, with an in-memory authority
//! store and APNs transport. No external service or credentials are used.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use crate::{
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    app_attest::AppAttestVerifier,
    authority::{AuthorityStore, Delegation, MemoryAuthorityStore, NewInstallation},
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

const DIRECT_URL: &str = "http://push.example:8080/v1/deliveries/apns";
const FORWARDED_URL: &str = "https://push.example/v1/deliveries/apns";

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
    fixture_with_transport(None).await
}

async fn fixture_with_transport(
    transport: Option<Arc<dyn PushTransport>>,
) -> (axum::Router, Keys, Vec<u8>, Arc<AtomicUsize>) {
    let keys = Keys::generate();
    let authority = Arc::new(MemoryAuthorityStore::default());
    let token_keyring =
        Arc::new(TokenKeyring::new(vec![TokenKey::new("test", &[2; 32]).unwrap()]).unwrap());
    let grant_keyring =
        Arc::new(GrantKeyring::new(vec![GrantKey::new("test", &[1; 32]).unwrap()]).unwrap());
    let installation_id = Uuid::new_v4();
    authority
        .create_installation(
            NewInstallation {
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
    let sends = Arc::new(AtomicUsize::new(0));
    // Capacity regression holds eight sends; keep its quota above that lane.
    let quota = if transport.is_some() { 40 } else { 10 };
    let (app, _) = router(AppState {
        trusted_proxy_cidrs: Vec::new(),
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
            transport: transport.unwrap_or_else(|| Arc::new(TestTransport(sends.clone()))),
        }),
        max_grant_lifetime_seconds: 600,
        max_installation_lifetime_seconds: 600,
        endpoint_quota_window_seconds: 60,
        endpoint_quota_max_deliveries: quota,
        now,
        accepting: Arc::new(AtomicBool::new(true)),
    });
    let body = serde_json::to_vec(&serde_json::json!({
        "v": 1, "endpoint_grant": grant, "request_id": Uuid::new_v4(), "expires_at": now() + 60,
    }))
    .unwrap();
    (app, keys, body, sends)
}

fn signed_header(keys: &Keys, url: &str, method: &str, body: &[u8]) -> String {
    let hash = hex::encode(Sha256::digest(body));
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
            Tag::parse(["payload", &hash]).unwrap(),
        ])
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
    for signed_url in [
        DIRECT_URL,
        FORWARDED_URL,
        "https://other.example:8443/v1/deliveries/apns",
    ] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, signed_url, "POST", &body);
        let mut request = request(DIRECT_URL, Some("invalid,https"), auth, body);
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
    for (url, method, change_body) in [
        ("https://push.example/v1/other", "POST", false),
        ("https://push.example/v1/deliveries/apns/", "POST", false),
        ("https://push.example/v1/deliveries/%61pns", "POST", false),
        (DIRECT_URL, "GET", false),
        (DIRECT_URL, "POST", true),
    ] {
        let (app, keys, mut body, sends) = fixture().await;
        let auth = signed_header(&keys, url, method, &body);
        if change_body {
            body.push(b' ');
        }
        assert_eq!(
            app.oneshot(request(DIRECT_URL, None, auth, body))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED,
            "{url} {method}"
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn signed_url_modifiers_and_non_http_schemes_are_rejected() {
    for url in [
        "https://push.example/v1/deliveries/apns?mode=one",
        "https://push.example/v1/deliveries/apns?",
        "https://push.example/v1/deliveries/apns#fragment",
        "https://push.example/v1/deliveries/apns#",
        "https://user@push.example/v1/deliveries/apns",
        "ftp://push.example/v1/deliveries/apns",
    ] {
        let (app, keys, body, sends) = fixture().await;
        let auth = signed_header(&keys, url, "POST", &body);
        assert_eq!(
            app.oneshot(request(DIRECT_URL, None, auth, body))
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
    let (app, keys, body, sends) = fixture().await;
    let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
    assert_eq!(
        app.oneshot(request(&format!("{DIRECT_URL}?mode=one"), None, auth, body))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn router_rejects_other_methods_and_paths() {
    for (method, path, expected) in [
        (
            "GET",
            crate::http::DELIVERY_PATH,
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        ("POST", "/v1/other", StatusCode::NOT_FOUND),
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

struct GatedTransport {
    entered: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
}
#[async_trait::async_trait]
impl PushTransport for GatedTransport {
    async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        DeliveryOutcome::Accepted
    }
}

#[tokio::test]
async fn cancelled_deliveries_keep_capacity_and_leave_v1_challenge_and_revoke_lanes_free() {
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let (app, keys, template, _) = fixture_with_transport(Some(Arc::new(GatedTransport {
        entered: entered.clone(),
        release: release.clone(),
    })))
    .await;
    for n in 0..8 {
        let mut value: serde_json::Value = serde_json::from_slice(&template).unwrap();
        value["request_id"] = Uuid::new_v4().to_string().into();
        let body = serde_json::to_vec(&value).unwrap();
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        let mut req = request(DIRECT_URL, None, auth, body);
        // Two concurrent requests per source, four independent sources.
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            format!("192.0.2.{}:1234", n / 2 + 1)
                .parse::<std::net::SocketAddr>()
                .unwrap(),
        ));
        let task = tokio::spawn(app.clone().oneshot(req));
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/deliveries/apns")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "1");
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/installations/challenges")
                .body(Body::from(r#"{"v":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(
            Request::post("/v1/delegations/revoke")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    release.add_permits(8);
}

#[tokio::test]
async fn slow_enrollment_bodies_are_bounded_without_occupying_revocation_capacity() {
    use std::{future::IntoFuture, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn headers(socket: &mut tokio::net::TcpStream) -> String {
        let read = async {
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                assert!(bytes.len() < 8192);
                bytes.push(socket.read_u8().await.unwrap());
            }
            String::from_utf8(bytes).unwrap()
        };
        tokio::time::timeout(Duration::from_secs(25), read)
            .await
            .unwrap()
    }

    let (app, _, _, _) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    let mut stalled = Vec::new();
    for _ in 0..2 {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket.write_all(b"POST /v1/installations HTTP/1.1\r\nHost: push.example\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n").await.unwrap();
        // Hyper sends Continue only when the handler starts reading the body:
        // admission has happened, without a scheduling sleep or polling retry.
        assert!(headers(&mut socket).await.starts_with("HTTP/1.1 100"));
        stalled.push(socket);
    }
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{address}/v1/installations"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "1");
    assert_eq!(
        client
            .post(format!("http://{address}/v1/installations/challenges"))
            .body(r#"{"v":1}"#)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(format!("http://{address}/v1/delegations/revoke"))
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    // Exercise the unchanged production timeout, then prove its permits return.
    for socket in &mut stalled {
        assert!(headers(socket).await.starts_with("HTTP/1.1 408"));
    }
    assert_eq!(
        client
            .post(format!("http://{address}/v1/installations"))
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .post(format!("http://{address}/v1/installations"))
            .body(vec![0; 1024 * 1024])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    server.abort();
}
