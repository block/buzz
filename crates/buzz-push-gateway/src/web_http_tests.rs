//! Web Push profile through the public router: NIP-98 enrollment, owner-bound
//! delegation, relay delivery with a content-free hint, dead-subscription cleanup.
use std::sync::{atomic::AtomicBool, Arc, Mutex};

use crate::{
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    app_attest::AppAttestVerifier,
    authority::MemoryAuthorityStore,
    grant::{GrantKey, GrantKeyring},
    http::ProfileRuntime,
    router,
    token::{TokenKey, TokenKeyring},
    web_http::WebRuntime,
    webpush::{WebPushSubscription, WebPushTransport},
    AppState,
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const BASE: &str = "https://push.example";
const SUB_ENDPOINT: &str = "https://web.push.apple.com/QGx0abc";

struct NoApns;
#[async_trait::async_trait]
impl PushTransport for NoApns {
    async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
        panic!("APNs must not be used for buzz-web");
    }
}

#[derive(Default)]
struct FakeWeb {
    sent: Mutex<Vec<(String, serde_json::Value)>>,
    outcome: Mutex<Option<DeliveryOutcome>>,
}
#[async_trait::async_trait]
impl WebPushTransport for FakeWeb {
    async fn send(
        &self,
        sub: &WebPushSubscription,
        payload: &[u8],
        _: i64,
        _: &'static str,
    ) -> DeliveryOutcome {
        self.sent.lock().unwrap().push((
            sub.endpoint.clone(),
            serde_json::from_slice(payload).unwrap(),
        ));
        self.outcome
            .lock()
            .unwrap()
            .unwrap_or(DeliveryOutcome::Accepted)
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn app(web: Arc<FakeWeb>) -> axum::Router {
    let (public, _) = router(AppState {
        grant_keyring: Arc::new(
            GrantKeyring::new(vec![GrantKey::new("g", &[1; 32]).unwrap()]).unwrap(),
        ),
        authority: Arc::new(MemoryAuthorityStore::default()),
        token_keyring: Arc::new(
            TokenKeyring::new(vec![TokenKey::new("t", &[2; 32]).unwrap()]).unwrap(),
        ),
        profile: Arc::new(ProfileRuntime {
            app_attest: Arc::new(
                AppAttestVerifier::new(
                    "TEAMID.xyz.block.buzz.dogfood.mobile".into(),
                    include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
                )
                .unwrap(),
            ),
            transport: Arc::new(NoApns),
        }),
        max_grant_lifetime_seconds: 3600,
        max_installation_lifetime_seconds: 86_400,
        endpoint_quota_window_seconds: 60,
        endpoint_quota_max_deliveries: 10,
        now,
        accepting: Arc::new(AtomicBool::new(true)),
        web: Some(Arc::new(WebRuntime::new_with_public(
            "BPUB".into(),
            web,
            BASE.into(),
            "https://app.example".into(),
        ))),
    });
    public
}

fn auth(keys: &Keys, path: &str, body: &[u8]) -> String {
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", &format!("{BASE}{path}")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_vec(&event).unwrap())
    )
}

async fn post(
    app: &axum::Router,
    path: &str,
    auth: String,
    body: Vec<u8>,
) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("authorization", auth)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), 1 << 16).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn subscription() -> serde_json::Value {
    serde_json::json!({
        "endpoint": SUB_ENDPOINT,
        "keys": {
            "p256dh": "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            "auth": "BTBZMqHH6r4Tts7J_aSIgg"
        }
    })
}

async fn enroll_and_delegate(app: &axum::Router, member: &Keys, relay: &Keys) -> (String, String) {
    let body = serde_json::to_vec(
        &serde_json::json!({"v":1,"subscription":subscription(),"expires_at":now()+3600}),
    )
    .unwrap();
    let (st, v) = post(
        app,
        "/v1/web/installations",
        auth(member, "/v1/web/installations", &body),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let handle = v["installation_handle"].as_str().unwrap().to_owned();
    let body = serde_json::to_vec(&serde_json::json!({
        "v":1,"installation_handle":handle,"generation":1,"relay_pubkey":relay.public_key().to_hex(),
        "not_before":now(),"expires_at":now()+1800
    }))
    .unwrap();
    let (st, v) = post(
        app,
        "/v1/web/delegations",
        auth(member, "/v1/web/delegations", &body),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    (handle, v["endpoint_grant"].as_str().unwrap().to_owned())
}

async fn deliver(
    app: &axum::Router,
    relay: &Keys,
    grant: &str,
    hint: serde_json::Value,
) -> StatusCode {
    let body = serde_json::to_vec(&serde_json::json!({
        "v":1,"endpoint_grant":grant,"request_id":uuid::Uuid::new_v4(),"expires_at":now()+60,"hint":hint
    }))
    .unwrap();
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", "https://push.example/v1/deliveries/apns"]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
        ])
        .sign_with_keys(relay)
        .unwrap();
    let a = format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_vec(&event).unwrap())
    );
    post(app, "/v1/deliveries/apns", a, body).await.0
}

#[tokio::test]
async fn web_enroll_delegate_deliver_end_to_end() {
    let web = Arc::new(FakeWeb::default());
    let app = app(web.clone());
    let (member, relay) = (Keys::generate(), Keys::generate());
    let (_, grant) = enroll_and_delegate(&app, &member, &relay).await;
    let event_id = "ab".repeat(32);
    let st = deliver(&app, &relay, &grant, serde_json::json!({"class":"mention","event_id":event_id,"channel_id":"123e4567-e89b-42d3-a456-426614174000"})).await;
    assert_eq!(st, StatusCode::OK);
    let sent = web.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, SUB_ENDPOINT);
    assert_eq!(sent[0].1["body"], "You were mentioned");
    assert_eq!(sent[0].1["target"]["eventId"], event_id);
    assert_eq!(
        sent[0].1["target"]["channelId"],
        "123e4567-e89b-42d3-a456-426614174000"
    );
}

#[tokio::test]
async fn enroll_requires_valid_nip98_and_allowlisted_endpoint() {
    let app = app(Arc::new(FakeWeb::default()));
    let member = Keys::generate();
    let body = serde_json::to_vec(
        &serde_json::json!({"v":1,"subscription":subscription(),"expires_at":now()+3600}),
    )
    .unwrap();
    // Signed for another path.
    let (st, _) = post(
        &app,
        "/v1/web/installations",
        auth(&member, "/v1/web/delegations", &body),
        body.clone(),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let mut bad = subscription();
    bad["endpoint"] = "https://169.254.169.254/latest/meta-data".into();
    let body =
        serde_json::to_vec(&serde_json::json!({"v":1,"subscription":bad,"expires_at":now()+3600}))
            .unwrap();
    let (st, v) = post(
        &app,
        "/v1/web/installations",
        auth(&member, "/v1/web/installations", &body),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "unsupported_subscription");
}

#[tokio::test]
async fn only_the_owner_can_delegate_or_revoke() {
    let app = app(Arc::new(FakeWeb::default()));
    let (member, relay, other) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (handle, grant) = enroll_and_delegate(&app, &member, &relay).await;
    let body = serde_json::to_vec(&serde_json::json!({
        "v":1,"installation_handle":handle,"generation":2,"relay_pubkey":other.public_key().to_hex(),
        "not_before":now(),"expires_at":now()+600
    }))
    .unwrap();
    let (st, _) = post(
        &app,
        "/v1/web/delegations",
        auth(&other, "/v1/web/delegations", &body),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let body =
        serde_json::to_vec(&serde_json::json!({"v":1,"installation_handle":handle})).unwrap();
    let (st, _) = post(
        &app,
        "/v1/web/installations/revoke",
        auth(&other, "/v1/web/installations/revoke", &body),
        body.clone(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = post(
        &app,
        "/v1/web/installations/revoke",
        auth(&member, "/v1/web/installations/revoke", &body),
        body,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        deliver(&app, &relay, &grant, serde_json::Value::Null).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn dead_subscription_returns_gone_and_retires_installation() {
    let web = Arc::new(FakeWeb::default());
    *web.outcome.lock().unwrap() = Some(DeliveryOutcome::InvalidEndpoint {
        unregistered_at: None,
    });
    let app = app(web.clone());
    let (member, relay) = (Keys::generate(), Keys::generate());
    let (_, grant) = enroll_and_delegate(&app, &member, &relay).await;
    assert_eq!(
        deliver(&app, &relay, &grant, serde_json::Value::Null).await,
        StatusCode::GONE
    );
    *web.outcome.lock().unwrap() = None;
    assert_eq!(
        deliver(&app, &relay, &grant, serde_json::Value::Null).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(web.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn bogus_hint_is_dropped_not_forwarded() {
    let web = Arc::new(FakeWeb::default());
    let app = app(web.clone());
    let (member, relay) = (Keys::generate(), Keys::generate());
    let (_, grant) = enroll_and_delegate(&app, &member, &relay).await;
    let st = deliver(
        &app,
        &relay,
        &grant,
        serde_json::json!({"class":"<script>","event_id":"x","channel_id":null}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let sent = web.sent.lock().unwrap();
    assert_eq!(sent[0].1["body"], "New activity");
    assert!(sent[0].1["target"].is_null());
}
