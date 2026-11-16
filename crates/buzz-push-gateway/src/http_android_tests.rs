//! Android installations through the public router: enrollment with the
//! test-only attestation verifier, Keystore-signed installation calls, and
//! delivery through a fake FCM sender. Transcripts are built with the
//! production types, so a drift in them fails here.
use super::*;
use crate::{
    android_attestation::testing::TestAttestationVerifier,
    authority::MemoryAuthorityStore,
    grant::{GrantKey, GrantKeyring},
    token::{TokenKey, TokenKeyring},
};
use axum::{body::Body, http::Request};
use nostr::{EventBuilder, Keys, Kind, Tag};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use tower::ServiceExt;

const NOW: i64 = 1_750_000_000;
const FCM_TOKEN: &str = "cExample-token:APA91bExample_token-1";
const ROTATED_FCM_TOKEN: &str = "dExample-token:APA91bExample_token-2";
const DELIVERY_URL: &str = "https://push.example/v1/deliveries";

fn fixed_now() -> i64 {
    NOW
}

/// Records every wake the gateway hands to the FCM boundary.
#[derive(Default)]
struct FakeFcm(Mutex<Vec<(String, DeliveryAttempt)>>);

#[async_trait::async_trait]
impl PushTransport for FakeFcm {
    async fn send(&self, attempt: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        self.0.lock().unwrap().push((endpoint.to_owned(), attempt));
        DeliveryOutcome::Accepted
    }
}

struct NeverApns;

#[async_trait::async_trait]
impl PushTransport for NeverApns {
    async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
        panic!("an Android installation never reaches APNs")
    }
}

struct Gateway {
    app: Router,
    fcm: Arc<FakeFcm>,
    grant_keyring: Arc<GrantKeyring>,
}

fn gateway(with_android: bool) -> Gateway {
    let fcm = Arc::new(FakeFcm::default());
    let grant_keyring =
        Arc::new(GrantKeyring::new(vec![GrantKey::new("test", &[1; 32]).unwrap()]).unwrap());
    let (app, _) = router(AppState {
        grant_keyring: grant_keyring.clone(),
        authority: Arc::new(MemoryAuthorityStore::default()),
        token_keyring: Arc::new(
            TokenKeyring::new(vec![TokenKey::new("test", &[2; 32]).unwrap()]).unwrap(),
        ),
        application: Arc::new(ApplicationRuntime {
            app_attest: Arc::new(
                AppAttestVerifier::new(
                    "TEAMID.com.example.buzz".into(),
                    include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
                )
                .unwrap(),
            ),
            transport: Arc::new(NeverApns),
            android: with_android.then(|| AndroidRuntime {
                attestation: Arc::new(TestAttestationVerifier),
                transport: fcm.clone(),
            }),
        }),
        max_grant_lifetime_seconds: 600,
        max_installation_lifetime_seconds: 600,
        endpoint_quota_window_seconds: 60,
        endpoint_quota_max_deliveries: 10,
        now: fixed_now,
        accepting: Arc::new(AtomicBool::new(true)),
    });
    Gateway {
        app,
        fcm,
        grant_keyring,
    }
}

async fn post(app: &Router, path: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// One issued enrollment challenge.
struct Challenge {
    id: uuid::Uuid,
    text: String,
}

async fn challenge(app: &Router) -> Challenge {
    let (status, body) = post(app, "/v1/installations/challenges", &json!({"v": 1})).await;
    assert_eq!(status, StatusCode::OK);
    Challenge {
        id: body["challenge_id"].as_str().unwrap().parse().unwrap(),
        text: body["challenge"].as_str().unwrap().to_owned(),
    }
}

/// The phone: an attested Keystore key.
struct Phone {
    key: SigningKey,
}

impl Phone {
    fn new(seed: u8) -> Self {
        Self {
            key: SigningKey::from_slice(&[seed; 32]).unwrap(),
        }
    }
    fn public_key(&self) -> Vec<u8> {
        self.key
            .verifying_key()
            .to_sec1_point(false)
            .as_ref()
            .to_vec()
    }
    fn key_id(&self) -> String {
        STANDARD.encode(Sha256::digest(self.public_key()))
    }
    /// A signature over `transcript`, as Keystore's SHA256withECDSA makes one.
    fn sign(&self, transcript: &str) -> String {
        let signature: Signature = self.key.sign(transcript.as_bytes());
        STANDARD.encode(signature.to_der().as_bytes())
    }
    /// The test verifier's chain: the key and the attested challenge.
    fn chain(&self, transcript: &str) -> Vec<String> {
        vec![
            STANDARD.encode(self.public_key()),
            STANDARD.encode(Sha256::digest(transcript.as_bytes())),
        ]
    }
}

/// The transcript whose digest the phone uses as the attestation challenge.
/// It takes no key: a Keystore key is generated from this digest.
fn enroll_transcript(ch: &Challenge, endpoint: &str) -> String {
    transcript(
        ANDROID_ENROLL_DOMAIN,
        &AndroidEnrollTranscript {
            v: 1,
            audience: ENROLL_AUDIENCE,
            challenge_id: ch.id,
            challenge: &ch.text,
            endpoint,
            endpoint_epoch: 1,
            expires_at: NOW + 600,
        },
    )
    .unwrap()
}

fn enroll_body(phone: &Phone, ch: &Challenge, endpoint: &str) -> Value {
    json!({
        "v": 1, "challenge_id": ch.id, "challenge": ch.text,
        "android_attestation": phone.chain(&enroll_transcript(ch, endpoint)),
        "endpoint": endpoint, "endpoint_epoch": 1, "expires_at": NOW + 600,
    })
}

async fn enroll(g: &Gateway, phone: &Phone) -> uuid::Uuid {
    let ch = challenge(&g.app).await;
    let (status, body) = post(
        &g.app,
        "/v1/installations",
        &enroll_body(phone, &ch, FCM_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["installation_handle"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// An installation call after enrollment.
#[derive(Clone)]
enum Call {
    Delegate {
        relay: String,
        epoch: i64,
        generation: i64,
    },
    Rotate {
        epoch: i64,
        endpoint: &'static str,
    },
    RevokeDelegation {
        relay: String,
        generation: i64,
    },
    RevokeInstallation {
        epoch: i64,
    },
}

/// The route, the request fields before `assertion`, and the exact transcript
/// the call's assertion signs.
fn prepare(call: &Call, handle: uuid::Uuid, ch: &Challenge) -> (&'static str, Value, String) {
    let common = |extra: Value| {
        let mut body = json!({
            "v": 1, "challenge_id": ch.id, "challenge": ch.text, "installation_handle": handle,
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    match call {
        Call::Delegate {
            relay,
            epoch,
            generation,
        } => (
            "/v1/delegations",
            common(
                json!({"endpoint_epoch": epoch, "generation": generation, "relay_pubkey": relay,
                "not_before": NOW, "expires_at": NOW + 300}),
            ),
            transcript(
                "buzz.push.delegate.v1",
                &DelegateTranscript {
                    v: 1,
                    audience: DELEGATE_AUDIENCE,
                    challenge_id: ch.id,
                    challenge: &ch.text,
                    installation_handle: handle,
                    endpoint_epoch: *epoch,
                    generation: *generation,
                    relay_pubkey: relay,
                    not_before: NOW,
                    expires_at: NOW + 300,
                },
            )
            .unwrap(),
        ),
        Call::Rotate { epoch, endpoint } => (
            "/v1/installations/endpoint",
            common(
                json!({"endpoint_epoch": epoch, "new_endpoint_epoch": epoch + 1, "endpoint": endpoint}),
            ),
            transcript(
                "buzz.push.rotate-endpoint.v1",
                &RotateTranscript {
                    v: 1,
                    audience: ROTATE_ENDPOINT_AUDIENCE,
                    challenge_id: ch.id,
                    challenge: &ch.text,
                    installation_handle: handle,
                    endpoint_epoch: *epoch,
                    new_endpoint_epoch: epoch + 1,
                    endpoint,
                },
            )
            .unwrap(),
        ),
        Call::RevokeDelegation { relay, generation } => (
            "/v1/delegations/revoke",
            common(json!({"relay_pubkey": relay, "generation": generation})),
            transcript(
                "buzz.push.revoke-delegation.v1",
                &RevokeDelegationTranscript {
                    v: 1,
                    audience: REVOKE_DELEGATION_AUDIENCE,
                    challenge_id: ch.id,
                    challenge: &ch.text,
                    installation_handle: handle,
                    relay_pubkey: relay,
                    generation: *generation,
                },
            )
            .unwrap(),
        ),
        Call::RevokeInstallation { epoch } => (
            "/v1/installations/revoke",
            common(json!({"endpoint_epoch": epoch, "new_endpoint_epoch": epoch + 1})),
            transcript(
                "buzz.push.revoke-installation.v1",
                &RevokeInstallationTranscript {
                    v: 1,
                    audience: REVOKE_INSTALLATION_AUDIENCE,
                    challenge_id: ch.id,
                    challenge: &ch.text,
                    installation_handle: handle,
                    endpoint_epoch: *epoch,
                    new_endpoint_epoch: epoch + 1,
                },
            )
            .unwrap(),
        ),
    }
}

/// Send `call` with a fresh challenge, signed by `signer` over the transcript
/// of `signed_call` (the same call unless a test alters it).
async fn send_call(
    g: &Gateway,
    handle: uuid::Uuid,
    call: &Call,
    signed_call: &Call,
    signer: &Phone,
) -> (StatusCode, Value) {
    let ch = challenge(&g.app).await;
    let (path, mut body, _) = prepare(call, handle, &ch);
    let (_, _, signed) = prepare(signed_call, handle, &ch);
    body["assertion"] = signer.sign(&signed).into();
    post(&g.app, path, &body).await
}

async fn call_ok(g: &Gateway, handle: uuid::Uuid, call: &Call, phone: &Phone) -> Value {
    let (status, body) = send_call(g, handle, call, call, phone).await;
    assert!(status.is_success(), "{status} {body}");
    body
}

async fn deliver_wake(g: &Gateway, relay: &Keys, grant: &str) -> StatusCode {
    let body = serde_json::to_vec(&json!({
        "v": 1, "endpoint_grant": grant, "request_id": uuid::Uuid::new_v4(), "expires_at": NOW + 60,
    }))
    .unwrap();
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", DELIVERY_URL]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
        ])
        .sign_with_keys(relay)
        .unwrap();
    let auth = format!(
        "Nostr {}",
        STANDARD.encode(serde_json::to_vec(&event).unwrap())
    );
    g.app
        .clone()
        .oneshot(
            Request::post(DELIVERY_PATH)
                .header("authorization", auth)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

fn sent_tokens(g: &Gateway) -> Vec<String> {
    g.fcm
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(t, _)| t.clone())
        .collect()
}

#[tokio::test]
async fn android_installation_journey() {
    let g = gateway(true);
    let phone = Phone::new(7);
    let relay = Keys::generate();
    let relay_hex = relay.public_key().to_hex();
    let handle = enroll(&g, &phone).await;

    let delegate = |epoch, generation| Call::Delegate {
        relay: relay_hex.clone(),
        epoch,
        generation,
    };
    let grant = call_ok(&g, handle, &delegate(1, 1), &phone).await["endpoint_grant"]
        .as_str()
        .unwrap()
        .to_owned();
    // The grant the relay stores names no platform.
    let opened = g.grant_keyring.open(&grant).unwrap();
    assert!(!serde_json::to_string(&opened)
        .unwrap()
        .to_lowercase()
        .contains("android"));

    assert_eq!(deliver_wake(&g, &relay, &grant).await, StatusCode::OK);
    assert_eq!(sent_tokens(&g), [FCM_TOKEN]);
    // The wake expires with the delivery request.
    assert_eq!(g.fcm.0.lock().unwrap()[0].1.expires_at, NOW + 60);

    // Rotating the endpoint retires the epoch-1 grant; the relay's renewed
    // delegation wakes the new token.
    let rotate = Call::Rotate {
        epoch: 1,
        endpoint: ROTATED_FCM_TOKEN,
    };
    call_ok(&g, handle, &rotate, &phone).await;
    assert_eq!(
        deliver_wake(&g, &relay, &grant).await,
        StatusCode::NOT_FOUND
    );
    let renewed = call_ok(&g, handle, &delegate(2, 2), &phone).await["endpoint_grant"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(deliver_wake(&g, &relay, &renewed).await, StatusCode::OK);
    assert_eq!(sent_tokens(&g), [FCM_TOKEN, ROTATED_FCM_TOKEN]);

    // Revoking the delegation stops wakes; revoking the installation ends it.
    let revoke_delegation = Call::RevokeDelegation {
        relay: relay_hex.clone(),
        generation: 2,
    };
    call_ok(&g, handle, &revoke_delegation, &phone).await;
    assert_eq!(
        deliver_wake(&g, &relay, &renewed).await,
        StatusCode::NOT_FOUND
    );
    call_ok(&g, handle, &Call::RevokeInstallation { epoch: 2 }, &phone).await;
    let (status, _) = send_call(&g, handle, &delegate(2, 3), &delegate(2, 3), &phone).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(sent_tokens(&g), [FCM_TOKEN, ROTATED_FCM_TOKEN]);
}

#[tokio::test]
async fn android_signed_calls_reject_wrong_key_altered_transcript_and_reused_challenge() {
    let g = gateway(true);
    let (phone, other) = (Phone::new(7), Phone::new(9));
    let relay = "aa".repeat(32);
    let handle = enroll(&g, &phone).await;
    let delegate = |generation| Call::Delegate {
        relay: relay.clone(),
        epoch: 1,
        generation,
    };
    // Each call, and a different call of the same kind whose transcript is
    // signed instead.
    let rows = [
        (delegate(1), delegate(2)),
        (
            Call::Rotate {
                epoch: 1,
                endpoint: ROTATED_FCM_TOKEN,
            },
            Call::Rotate {
                epoch: 1,
                endpoint: "eExample-other:token",
            },
        ),
        (
            Call::RevokeDelegation {
                relay: relay.clone(),
                generation: 1,
            },
            Call::RevokeDelegation {
                relay: relay.clone(),
                generation: 2,
            },
        ),
        (
            Call::RevokeInstallation { epoch: 1 },
            Call::RevokeInstallation { epoch: 5 },
        ),
    ];
    for (call, altered) in &rows {
        // Signed by a key other than the one attested at enrollment.
        let (status, _) = send_call(&g, handle, call, call, &other).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "wrong key");
        // The right key, over a transcript that is not the request's.
        let (status, _) = send_call(&g, handle, call, altered, &phone).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "altered transcript");
        // An assertion in the iOS format is not an Android signature.
        let ch = challenge(&g.app).await;
        let (path, mut body, _) = prepare(call, handle, &ch);
        body["assertion"] = STANDARD.encode(b"\xa2\x6fauthenticatorData").into();
        assert_eq!(post(&g.app, path, &body).await.0, StatusCode::UNAUTHORIZED);
    }
    // None of the rejections changed the installation or used up the key.
    let ch = challenge(&g.app).await;
    let (path, mut body, signed) = prepare(&delegate(1), handle, &ch);
    body["assertion"] = phone.sign(&signed).into();
    assert_eq!(post(&g.app, path, &body).await.0, StatusCode::CREATED);
    // Replaying the identical signed request fails: the challenge is single use.
    assert_eq!(post(&g.app, path, &body).await.0, StatusCode::NOT_FOUND);
    // A fresh signature over a new transcript but the consumed challenge fails too.
    let mut resigned = body.clone();
    resigned["generation"] = 2.into();
    let (_, _, signed) = prepare(
        &delegate(2),
        handle,
        &Challenge {
            id: ch.id,
            text: ch.text.clone(),
        },
    );
    resigned["assertion"] = phone.sign(&signed).into();
    assert_eq!(post(&g.app, path, &resigned).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn android_enrollment_derives_platform_from_the_attestation_and_rejects_bad_ones() {
    let g = gateway(true);
    let phone = Phone::new(7);
    let enroll_status = |g: &Gateway, body: Value| {
        let app = g.app.clone();
        async move { post(&app, "/v1/installations", &body).await.0 }
    };
    let ch = challenge(&g.app).await;
    let good = enroll_body(&phone, &ch, FCM_TOKEN);

    // Exactly one attestation must be present.
    let mut both = good.clone();
    both["attestation"] = STANDARD.encode(b"x").into();
    let mut neither = good.clone();
    neither
        .as_object_mut()
        .unwrap()
        .remove("android_attestation");
    for body in [both, neither] {
        assert_eq!(enroll_status(&g, body).await, StatusCode::BAD_REQUEST);
    }
    // A gateway without Android configuration accepts no Android enrollment.
    let plain = gateway(false);
    let plain_ch = challenge(&plain.app).await;
    let status = enroll_status(&plain, enroll_body(&phone, &plain_ch, FCM_TOKEN)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // FCM tokens are opaque ASCII, not hex, and bounded.
    for endpoint in [
        "",
        "has space",
        "bad/char",
        &"a".repeat(MAX_FCM_TOKEN_BYTES + 1),
    ] {
        let mut body = good.clone();
        body["endpoint"] = endpoint.into();
        assert_eq!(
            enroll_status(&g, body).await,
            StatusCode::BAD_REQUEST,
            "{endpoint}"
        );
    }

    // The gateway derives the key id from the attested key, so an Android
    // enrollment carries none.
    let mut with_key_id = good.clone();
    with_key_id["key_id"] = phone.key_id().into();
    assert_eq!(
        enroll_status(&g, with_key_id).await,
        StatusCode::BAD_REQUEST
    );

    // The attestation must bind this exact transcript, prove a P-256 point and
    // stay within the chain bounds.
    let other_transcript = enroll_transcript(&ch, ROTATED_FCM_TOKEN);
    let mut wrong_binding = good.clone();
    wrong_binding["android_attestation"] = phone.chain(&other_transcript).into();
    let mut not_a_point = good.clone();
    not_a_point["android_attestation"] = json!([
        STANDARD.encode([4u8; 65]),
        STANDARD.encode(Sha256::digest(enroll_transcript(&ch, FCM_TOKEN).as_bytes())),
    ]);
    let mut long_chain = good.clone();
    long_chain["android_attestation"] =
        vec![STANDARD.encode(b"c"); MAX_ANDROID_CHAIN_CERTIFICATES + 1].into();
    for body in [wrong_binding, not_a_point, long_chain] {
        assert_eq!(enroll_status(&g, body).await, StatusCode::UNAUTHORIZED);
    }

    // The accepted enrollment, and its exact replay after a lost response,
    // name one installation.
    let first = post(&g.app, "/v1/installations", &good).await;
    let replay = post(&g.app, "/v1/installations", &good).await;
    assert_eq!(
        (first.0, replay.0),
        (StatusCode::CREATED, StatusCode::CREATED)
    );
    assert_eq!(
        first.1["installation_handle"],
        replay.1["installation_handle"]
    );
}
