//! Real PostgreSQL with the shipped HTTP/App Attest/NIP-98 admission path.
use super::*;
use crate::{
    apns::{DeliveryAttempt, PushTransport},
    postgres::PostgresAuthorityStore,
    token::TokenKey,
};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicI64;
use tower::ServiceExt;
use uuid::Uuid;
static NOW: AtomicI64 = AtomicI64::new(0);
const ORIGIN: &str = "https://push.buzz.xyz";
const APP: &str = "TEAM.test.app";
fn now() -> i64 {
    NOW.load(Ordering::SeqCst)
}
struct Transport;
#[async_trait::async_trait]
impl PushTransport for Transport {
    async fn send(&self, _: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        assert_eq!(endpoint, "01020304");
        DeliveryOutcome::Accepted
    }
}
fn assertion(signed: &str, counter: u32) -> String {
    let mut auth = Sha256::digest(APP.as_bytes()).to_vec();
    auth.push(0x40);
    auth.extend(counter.to_be_bytes());
    let mut digest = Sha256::new();
    digest.update(&auth);
    digest.update(Sha256::digest(signed.as_bytes()));
    let signature: Signature = SigningKey::from_slice(&[7; 32])
        .unwrap()
        .sign(&digest.finalize());
    let mut encoded = Vec::new();
    ciborium::into_writer(
        &ciborium::Value::Map(vec![
            ("authenticatorData".into(), ciborium::Value::Bytes(auth)),
            (
                "signature".into(),
                ciborium::Value::Bytes(signature.to_der().as_bytes().to_vec()),
            ),
        ]),
        &mut encoded,
    )
    .unwrap();
    STANDARD.encode(encoded)
}
async fn post(
    app: &Router,
    path: &str,
    mut body: serde_json::Value,
    signed: Option<(&str, u32)>,
) -> (StatusCode, serde_json::Value) {
    if let Some((transcript, counter)) = signed {
        body.as_object_mut().unwrap().remove("audience");
        body["assertion"] = assertion(transcript, counter).into();
    }
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}
async fn delegate_request(
    app: &Router,
    id: Uuid,
    relay: &str,
    expected: i64,
    generation: i64,
    counter: u32,
) -> (StatusCode, serde_json::Value) {
    let t = DelegateTranscript {
        v: 2,
        audience: DELEGATE_AUDIENCE,
        gateway_origin: ORIGIN,
        request_expires_at: now() + 300,
        installation_handle: id,
        endpoint_epoch: 1,
        expected_generation: expected,
        generation,
        relay_pubkey: relay,
        not_before: now(),
        expires_at: now() + 600,
    };
    post(
        app,
        "/v1/delegations",
        serde_json::to_value(&t).unwrap(),
        Some((&transcript("buzz.push.delegate.v2", &t).unwrap(), counter)),
    )
    .await
}
async fn revoke(app: &Router, id: Uuid, relay: &str, generation: i64, counter: u32) -> StatusCode {
    let t = RevokeDelegationTranscript {
        v: 2,
        audience: REVOKE_DELEGATION_AUDIENCE,
        gateway_origin: ORIGIN,
        request_expires_at: now() + 300,
        installation_handle: id,
        relay_pubkey: relay,
        generation,
    };
    post(
        app,
        "/v1/delegations/revoke",
        serde_json::to_value(&t).unwrap(),
        Some((
            &transcript("buzz.push.revoke-delegation.v2", &t).unwrap(),
            counter,
        )),
    )
    .await
    .0
}
fn delivery_body(grant: &str, request: Uuid) -> Vec<u8> {
    serde_json::to_vec(&DeliveryRequest {
        v: 1,
        endpoint_grant: grant.into(),
        request_id: request,
        expires_at: now() + 120,
    })
    .unwrap()
}
fn auth(keys: &nostr::Keys, body: &[u8]) -> String {
    let event = nostr::EventBuilder::new(nostr::Kind::HttpAuth, "")
        .tags([
            nostr::Tag::parse(["u", "https://push.buzz.xyz/v1/deliveries/apns"]).unwrap(),
            nostr::Tag::parse(["method", "POST"]).unwrap(),
            nostr::Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_vec(&event).unwrap())
    )
}
async fn delivery(app: &Router, body: Vec<u8>, auth: &str) -> Response {
    app.clone()
        .oneshot(
            Request::post(DELIVERY_PATH)
                .header("Authorization", auth)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn management_binds_parameters_versions_expiry_and_commits_counter_with_permission() {
    NOW.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("isolated local test database");
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("protocol_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let search_path = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .after_connect(move |connection, _| {
            let query = format!("SET search_path TO {search_path}");
            Box::pin(async move {
                sqlx::raw_sql(sqlx::AssertSqlSafe(query))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let err = PostgresAuthorityStore::apply_migrations_and_grants(&pool, "")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("runtime database role"), "{err}");
    let authority = Arc::new(PostgresAuthorityStore::new(pool.clone()));
    let keys = nostr::Keys::generate();
    let relay = keys.public_key().to_hex();
    let id = Uuid::new_v4();
    let tokens = TokenKeyring::new(vec![TokenKey::new("token", &[2; 32]).unwrap()]).unwrap();
    authority
        .create_installation(
            NewInstallation {
                id,
                app_attest_key_id: vec![3; 32],
                app_attest_public_key: SigningKey::from_slice(&[7; 32])
                    .unwrap()
                    .verifying_key()
                    .to_sec1_point(false)
                    .as_ref()
                    .to_vec(),
                assertion_counter: 0,
                token_ciphertext: tokens.seal(&[1, 2, 3, 4]).unwrap(),
                token_fingerprint: endpoint_fingerprint(&[1, 2, 3, 4]),
                endpoint_epoch: 1,
                expires_at: now() + 600,
            },
            now(),
        )
        .await
        .unwrap();
    let state = AppState {
        trusted_proxy_cidrs: Vec::new(),
        gateway_origin: ORIGIN.into(),
        authority: authority.clone(),
        token_keyring: Arc::new(tokens),
        application: Arc::new(ApplicationRuntime {
            app_attest: Arc::new(
                AppAttestVerifier::new(
                    APP.into(),
                    include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
                )
                .unwrap(),
            ),
            transport: Arc::new(Transport),
        }),
        max_grant_lifetime_seconds: 600,
        max_installation_lifetime_seconds: 600,
        endpoint_quota_window_seconds: 60,
        endpoint_quota_max_deliveries: 8,
        now,
        accepting: Arc::new(AtomicBool::new(true)),
    };
    let (app, _) = router(state.clone());
    let (status, issued) = delegate_request(&app, id, &relay, 0, 1, 1).await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    let token = issued["endpoint_grant"].as_str().unwrap();
    let body = delivery_body(token, Uuid::new_v4());
    assert_eq!(
        delivery(&app, body.clone(), &auth(&keys, &body))
            .await
            .status(),
        StatusCode::OK
    );
    let body = delivery_body(token, Uuid::new_v4());
    assert_eq!(
        delivery(&app, body.clone(), &auth(&keys, &body))
            .await
            .status(),
        StatusCode::OK
    );
    let body = delivery_body(token, Uuid::new_v4());
    let signed = auth(&keys, &body);
    let limited = delivery(&app, body.clone(), &signed).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers()["retry-after"], "60");
    NOW.fetch_add(60, Ordering::SeqCst);
    assert_eq!(
        delivery(&app, body, &signed).await.status(),
        StatusCode::OK,
        "quota rejection consumes neither request ID nor auth event ID"
    );
    let (status, conflict) = delegate_request(&app, id, &relay, 0, 2, 2).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["current_generation"], 1);
    assert_eq!(
        authority
            .installation(id, now())
            .await
            .unwrap()
            .assertion_counter,
        1
    );
    let t = DelegateTranscript {
        v: 2,
        audience: DELEGATE_AUDIENCE,
        gateway_origin: ORIGIN,
        request_expires_at: now() + 300,
        installation_handle: id,
        endpoint_epoch: 1,
        expected_generation: 1,
        generation: 2,
        relay_pubkey: &relay,
        not_before: now(),
        expires_at: now() + 600,
    };
    let signed = transcript("buzz.push.delegate.v2", &t).unwrap();
    for (field, value, expected) in [
        (
            "gateway_origin",
            serde_json::json!("https://other.example"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "request_expires_at",
            serde_json::json!(now() - 1),
            StatusCode::BAD_REQUEST,
        ),
        (
            "request_expires_at",
            serde_json::json!(now() + 301),
            StatusCode::BAD_REQUEST,
        ),
        ("v", serde_json::json!(1), StatusCode::BAD_REQUEST),
        ("generation", serde_json::json!(3), StatusCode::UNAUTHORIZED),
        (
            "relay_pubkey",
            serde_json::json!("66".repeat(32)),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "expected_generation",
            serde_json::Value::Null,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut body = serde_json::to_value(&t).unwrap();
        body[field] = value;
        assert_eq!(
            post(&app, "/v1/delegations", body, Some((&signed, 2)))
                .await
                .0,
            expected,
            "{field}"
        );
    }
    // Response loss recovers by signing against the observed version, never by reconstructing a hash.
    let (status, renewed) = delegate_request(&app, id, &relay, 1, 3, 2).await;
    assert_eq!(status, StatusCode::CREATED);
    let renewed = renewed["endpoint_grant"].as_str().unwrap();
    assert_ne!(token, renewed);
    let body = delivery_body(token, Uuid::new_v4());
    assert_eq!(
        delivery(&app, body.clone(), &auth(&keys, &body))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    for _ in 0..20 {
        assert_eq!(
            post(
                &app,
                "/v1/installations/challenges",
                serde_json::json!({"v":1}),
                None
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let limited = post(
        &app,
        "/v1/installations/challenges",
        serde_json::json!({"v":1}),
        None,
    )
    .await;
    assert_eq!(limited.0, StatusCode::TOO_MANY_REQUESTS);
    for counter in [3, 4] {
        assert_eq!(revoke(&app, id, &relay, 3, counter).await, StatusCode::OK);
    }
    assert_eq!(
        delegate_request(&app, id, &relay, 3, 4, 5).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        authority
            .installation(id, now())
            .await
            .unwrap()
            .assertion_counter,
        4
    );
    let body = delivery_body(renewed, Uuid::new_v4());
    assert_eq!(
        delivery(&app, body.clone(), &auth(&keys, &body))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
