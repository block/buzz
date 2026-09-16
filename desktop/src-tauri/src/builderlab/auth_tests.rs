//! Real production owner + auth transport, served exclusively on loopback.
use super::*;
use axum::{routing::post, Json};
use chrono::Utc;
use nostr::{EventBuilder, JsonUtil, Keys};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

struct MockApi {
    base: Url,
    state: Arc<ApiState>,
    server: tokio::task::JoinHandle<()>,
}
struct ApiState {
    keys: Keys,
    expiry: Mutex<String>,
    oa_fault: Mutex<Option<&'static str>>,
    decrypt_fail_ciphertext: Mutex<Option<String>>,
    agent_publications: Mutex<Vec<(nostr::Event, String)>>,
    relay_events: Mutex<Vec<nostr::Event>>,
    identity: Mutex<Option<String>>,
    fail: Mutex<Option<(String, u16)>>,
    hold: Mutex<Option<String>>,
    arrived: Semaphore,
    release: Semaphore,
    requests: Mutex<Vec<(String, Option<String>, serde_json::Value)>>,
}
impl Drop for MockApi {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl MockApi {
    async fn new() -> Self {
        let state = Arc::new(ApiState {
            keys: Keys::generate(),
            oa_fault: Mutex::new(None),
            decrypt_fail_ciphertext: Mutex::new(None),
            agent_publications: Mutex::new(vec![]),
            relay_events: Mutex::new(vec![]),
            expiry: Mutex::new((Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
            identity: Mutex::new(None),
            fail: Mutex::new(None),
            hold: Mutex::new(None),
            arrived: Semaphore::new(0),
            release: Semaphore::new(0),
            requests: Mutex::new(vec![]),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!(
            "http://{}/prefix/goose",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let router = Router::new()
            .route("/prefix/goose/{*path}", post(handle).get(handle))
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            base,
            state,
            server,
        }
    }
    async fn login(
        &self,
        owner: &BuilderlabSession,
        code: &str,
    ) -> Result<BuilderlabAuthInfo, String> {
        auth::finish_login(owner.begin()?, &self.base, code.into(), true).await
    }
    fn hold(&self, path: &str) {
        *self.state.hold.lock().unwrap() = Some(path.into());
    }
    async fn arrived(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.state.arrived.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
    fn release(&self) {
        self.state.release.add_permits(1);
    }
}
async fn handle(
    Path(path): Path<String>,
    AxumState(state): AxumState<Arc<ApiState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body: serde_json::Value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    let credential = headers
        .get(BB_SESSION_CREDENTIAL_HEADER)
        .map(|h| h.to_str().unwrap().to_owned());
    state
        .requests
        .lock()
        .unwrap()
        .push((path.clone(), credential, body.clone()));
    let hold = state.hold.lock().unwrap().take();
    if hold.as_deref() == Some(&path) {
        state.arrived.add_permits(1);
        state.release.acquire().await.unwrap().forget();
    } else if hold.is_some() {
        *state.hold.lock().unwrap() = hold;
    }
    if let Some((target, status)) = state.fail.lock().unwrap().clone() {
        if target == path {
            return (StatusCode::from_u16(status).unwrap(), "DO_NOT_LOG_SECRET").into_response();
        }
    }
    if path == "v1/buzz/identity/decrypt"
        && state
            .decrypt_fail_ciphertext
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|ciphertext| body["ciphertext"].as_str() == Some(ciphertext))
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let expiry = state.expiry.lock().unwrap().clone();
    let value = match path.as_str() {
        "v1/auth/login/exchange" => {
            serde_json::json!({"session_credential":format!("credential-{}", body["code"].as_str().unwrap()), "expires_at":expiry})
        }
        "v1/auth/me" => {
            serde_json::json!({"expires_at":expiry, "email":"person@example.test", "name":"Person"})
        }
        "v1/buzz/identity" => {
            serde_json::json!({"pubkey":state.identity.lock().unwrap().clone().unwrap_or_else(|| state.keys.public_key().to_hex())})
        }
        "v1/buzz/identity/sign" => {
            let event = EventBuilder::new(
                nostr::Kind::from(body["kind"].as_u64().unwrap() as u16),
                body["content"].as_str().unwrap(),
            )
            .tags(serde_json::from_value::<nostr::Tags>(body["tags"].clone()).unwrap())
            .custom_created_at(nostr::Timestamp::from(body["created_at"].as_u64().unwrap()))
            .sign_with_keys(&state.keys)
            .unwrap();
            serde_json::from_str(&event.as_json()).unwrap()
        }
        "v1/buzz/identity/encrypt" => {
            let peer = nostr::PublicKey::from_hex(body["peer_pubkey"].as_str().unwrap()).unwrap();
            serde_json::json!({"ciphertext":nostr::nips::nip44::encrypt(state.keys.secret_key(), &peer, body["plaintext"].as_str().unwrap(), nostr::nips::nip44::Version::V2).unwrap()})
        }
        "v1/buzz/identity/decrypt" => {
            let peer = nostr::PublicKey::from_hex(body["peer_pubkey"].as_str().unwrap()).unwrap();
            serde_json::json!({"plaintext":nostr::nips::nip44::decrypt(state.keys.secret_key(), &peer, body["ciphertext"].as_str().unwrap()).unwrap()})
        }
        "v1/buzz/identity/authorize-agent" => {
            let fault = *state.oa_fault.lock().unwrap();
            let agent = if fault == Some("agent") {
                Keys::generate().public_key()
            } else {
                nostr::PublicKey::from_hex(body["agent_pubkey"].as_str().unwrap()).unwrap()
            };
            let other = Keys::generate();
            let keys = if fault == Some("owner") {
                &other
            } else {
                &state.keys
            };
            let conditions = if fault == Some("conditions") {
                "kind=1"
            } else {
                body["conditions"].as_str().unwrap()
            };
            let tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(keys, &agent, conditions).unwrap();
            let mut tag: serde_json::Value = serde_json::from_str(&tag).unwrap();
            if fault == Some("signature") {
                tag[3] = serde_json::json!("0".repeat(128));
            }
            serde_json::json!({"auth_tag":tag})
        }
        "query" => serde_json::to_value(state.relay_events.lock().unwrap().clone()).unwrap(),
        "events" => {
            if let Some(proof) = headers.get("x-auth-tag") {
                let event: nostr::Event = serde_json::from_value(body.clone()).unwrap();
                event.verify().unwrap();
                let proof = proof.to_str().unwrap().to_string();
                buzz_sdk_pkg::nip_oa::verify_auth_tag(&proof, &event.pubkey).unwrap();
                state
                    .agent_publications
                    .lock()
                    .unwrap()
                    .push((event, proof));
            }
            serde_json::json!({"event_id":body["id"], "accepted":true, "message":""})
        }
        _ => panic!("unexpected endpoint {path}"),
    };
    Json(value).into_response()
}

#[tokio::test]
async fn remote_login_installs_shared_keyless_owner_and_exact_credential_signer() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    let owner = state.native_auth.clone();
    api.login(&owner, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    assert_eq!(signer.public_key(), api.state.keys.public_key());
    let event = signer
        .sign_event(EventBuilder::text_note("native foreground"))
        .await
        .unwrap();
    event.verify().unwrap();
    assert!(state.local_identity_keys().is_err());
    assert_eq!(
        state.identity_storage(),
        crate::app_state::IdentityStorage::Absent
    );
    let status = serde_json::to_value(state.native_identity_status().unwrap()).unwrap();
    assert_eq!(status["authState"], "authenticated");
    assert_eq!(status["workspaceActive"], false);
    assert!(!status.to_string().contains("credential"));
    let requests = api.state.requests.lock().unwrap();
    assert_eq!(
        requests.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        [
            "v1/auth/login/exchange",
            "v1/auth/me",
            "v1/buzz/identity",
            "v1/buzz/identity/sign"
        ]
    );
    assert!(requests[1..]
        .iter()
        .all(|r| r.1.as_deref() == Some("credential-A")));
    assert_eq!(requests[2].2, serde_json::json!({}));
}

#[tokio::test]
async fn malformed_curve_hex_and_http_failures_never_install_or_fallback() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    for key in [
        "".into(),
        "a".repeat(63),
        "A".repeat(64),
        "0".repeat(64),
        "f".repeat(64),
        "g".repeat(64),
    ] {
        *api.state.identity.lock().unwrap() = Some(key);
        assert!(api.login(&owner, "bad").await.is_err());
        assert!(owner.active_signer().is_err());
    }
    *api.state.identity.lock().unwrap() = None;
    for path in ["v1/auth/login/exchange", "v1/auth/me", "v1/buzz/identity"] {
        for status in [302, 401, 403, 429, 503] {
            *api.state.fail.lock().unwrap() = Some((path.into(), status));
            let error = api.login(&owner, "bad").await.unwrap_err();
            assert!(!error.contains("DO_NOT_LOG_SECRET"));
            assert!(error.contains(&status.to_string()));
            assert_eq!(owner.status().unwrap().0, "signed-out");
        }
    }
}

#[tokio::test]
async fn logout_during_each_auth_await_and_old_login_drop_cannot_clear_replacement() {
    for path in ["v1/auth/login/exchange", "v1/auth/me", "v1/buzz/identity"] {
        let api = MockApi::new().await;
        let owner = BuilderlabSession::default();
        api.hold(path);
        let a = owner.begin().unwrap();
        let base = api.base.clone();
        let task =
            tokio::spawn(async move { auth::finish_login(a, &base, "A".into(), true).await });
        api.arrived().await;
        owner.clear().unwrap();
        api.login(&owner, "B").await.unwrap();
        let generation = owner.active_signer().unwrap().generation();
        api.release();
        assert!(task.await.unwrap().is_err());
        assert_eq!(owner.active_signer().unwrap().generation(), generation);
        assert_eq!(
            owner.snapshot().unwrap().unwrap().credential,
            "credential-B"
        );
    }
}

#[tokio::test]
async fn stale_me_success_or_denial_cannot_return_or_clear_new_session() {
    for deny in [false, true] {
        let api = MockApi::new().await;
        let owner = BuilderlabSession::default();
        api.login(&owner, "A").await.unwrap();
        api.hold("v1/auth/me");
        let clone = owner.clone();
        let base = api.base.clone();
        let task = tokio::spawn(async move { auth::check_auth(&clone, &base).await });
        api.arrived().await;
        api.login(&owner, "B").await.unwrap();
        if deny {
            *api.state.fail.lock().unwrap() = Some(("v1/auth/me".into(), 401));
        }
        api.release();
        assert!(task.await.unwrap().is_err());
        assert_eq!(
            owner.snapshot().unwrap().unwrap().credential,
            "credential-B"
        );
    }
}

#[tokio::test]
async fn same_key_reauth_and_account_swap_revoke_held_signers_and_relay_leases() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    api.login(&owner, "A").await.unwrap();
    let a = owner.active_signer().unwrap();
    let client = crate::native_relay_client::NativeRelayClient::default();
    let lease_a = client
        .session_with_signer("ws://127.0.0.1:9".into(), a.clone())
        .await;
    api.login(&owner, "B").await.unwrap();
    let b = owner.active_signer().unwrap();
    assert_eq!(a.public_key(), b.public_key());
    assert_ne!(a.generation(), b.generation());
    assert!(a
        .sign_event(EventBuilder::text_note("stale"))
        .await
        .is_err());
    client.clear_invalidated().await;
    let stale_empty = client
        .session_with_signer("ws://127.0.0.1:9".into(), a.clone())
        .await;
    assert!(stale_empty
        .fetch_events(serde_json::json!({"kinds":[1]}), Duration::from_secs(1))
        .await
        .is_err());
    let lease_b = client
        .session_with_signer("ws://127.0.0.1:9".into(), b.clone())
        .await;
    assert!(!Arc::ptr_eq(&lease_a.handle(), &lease_b.handle()));
    let stale = client
        .session_with_signer("ws://127.0.0.1:9".into(), a.clone())
        .await;
    assert!(stale
        .fetch_events(serde_json::json!({"kinds":[1]}), Duration::from_secs(1))
        .await
        .is_err());
    let again = client
        .session_with_signer("ws://127.0.0.1:9".into(), b.clone())
        .await;
    assert!(Arc::ptr_eq(&again.handle(), &lease_b.handle()));
    assert!(lease_a
        .fetch_events(serde_json::json!({"kinds":[1]}), Duration::from_secs(1))
        .await
        .is_err());
    let other = MockApi::new().await;
    other.login(&owner, "C").await.unwrap();
    assert_ne!(owner.active_signer().unwrap().public_key(), b.public_key());
    assert!(b.check_valid().is_err());
    owner.clear().unwrap();
}

#[tokio::test]
async fn held_sign_http_and_post_sign_effect_cancel_without_waiting_for_server() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    api.login(&owner, "A").await.unwrap();
    let signer = owner.active_signer().unwrap();
    api.hold("v1/buzz/identity/sign");
    let clone = signer.clone();
    let pending =
        tokio::spawn(async move { clone.sign_event(EventBuilder::text_note("held")).await });
    api.arrived().await;
    owner.clear().unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    api.release();
    assert!(signer.check_valid().is_err());
    api.login(&owner, "B").await.unwrap();
    let signer = owner.active_signer().unwrap();
    let reached = Arc::new(Semaphore::new(0));
    let mark = reached.clone();
    let pending = tokio::spawn(async move {
        signer
            .run(async {
                mark.add_permits(1);
                std::future::pending::<Result<(), String>>().await
            })
            .await
    });
    reached.acquire().await.unwrap().forget();
    owner.clear().unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}

#[tokio::test]
async fn expiry_revokes_access_and_invalid_me_clears_only_current_auth() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    // The real loopback login makes three HTTP requests; a 150ms lifetime
    // expired during login under full-suite CPU contention, before this test
    // could exercise expiry of an installed signer. Allow setup headroom and
    // wait for the actual deadline rather than a fixed post-login sleep.
    let expires = Utc::now() + chrono::Duration::seconds(10);
    *api.state.expiry.lock().unwrap() = expires.to_rfc3339();
    api.login(&owner, "short").await.unwrap();
    let signer = owner.active_signer().unwrap();
    tokio::time::sleep(
        (expires - Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(30),
    )
    .await;
    assert!(signer.check_valid().is_err());
    assert!(owner.active_signer().is_err());
    assert_eq!(owner.status().unwrap().0, "signed-out");
    assert!(api.login(&owner, "expired").await.is_err());
    *api.state.expiry.lock().unwrap() = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    api.login(&owner, "valid").await.unwrap();
    *api.state.fail.lock().unwrap() = Some(("v1/auth/me".into(), 503));
    assert!(auth::check_auth(&owner, &api.base).await.is_err());
    assert!(owner.active_signer().is_ok());
    *api.state.fail.lock().unwrap() = Some(("v1/auth/me".into(), 401));
    assert!(auth::check_auth(&owner, &api.base).await.is_err());
    assert!(owner.active_signer().is_err());
}

#[tokio::test]
async fn actual_browser_callback_nonce_code_cancel_and_opener_failure() {
    let owner = BuilderlabSession::default();
    let api = MockApi::new().await;
    let attempt = owner.begin().unwrap();
    let (tx, rx) = oneshot::channel::<String>();
    let callback = async {
        let url = Url::parse(&rx.await.unwrap()).unwrap();
        assert_eq!(url.path(), "/prefix/goose/v1/auth/login");
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["type"], "cli");
        assert_eq!(query["product"], "buzz");
        let return_to = &query["returnTo"];
        let wrong = format!("{}-wrong?code=bad", return_to);
        assert_eq!(
            reqwest::get(wrong).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        reqwest::get(format!("{return_to}?code=one-use"))
            .await
            .unwrap();
    };
    let (code, ()) = tokio::join!(
        auth::browser_code(&attempt, &api.base, |url| {
            tx.send(url.to_owned()).unwrap();
            Ok(())
        }),
        callback
    );
    assert_eq!(code.unwrap(), "one-use");
    drop(attempt);
    let attempt = owner.begin().unwrap();
    assert!(
        auth::browser_code(&attempt, &api.base, |_| Err("opener failed".into()))
            .await
            .is_err()
    );
    drop(attempt);
    assert_eq!(owner.status().unwrap().0, "signed-out");
    let attempt = owner.begin().unwrap();
    assert!(auth::browser_code(&attempt, &api.base, |_| {
        owner.cancel_login().unwrap();
        Ok(())
    })
    .await
    .is_err());
}

#[path = "auth_foreground_tests.rs"]
mod foreground;

#[path = "auth_safety_gate_tests.rs"]
mod safety_gates;

#[path = "auth_capabilities_tests.rs"]
mod capabilities;

#[path = "auth_owner_authorization_tests.rs"]
mod owner_authorization_tests;

#[path = "auth_self_crypto_tests.rs"]
mod self_crypto;

#[path = "auth_snapshot_envelope_tests.rs"]
mod snapshot_envelope;

#[path = "auth_archive_ingress_tests.rs"]
mod archive_ingress;

#[path = "auth_archive_sync_tests.rs"]
mod archive_sync;

#[path = "auth_join_tests.rs"]
mod join;

#[path = "auth_media_proxy_tests.rs"]
mod media_proxy_tests;

#[path = "auth_operation_tests.rs"]
mod operation_tests;

#[path = "auth_raw_upload_tests.rs"]
mod raw_upload_tests;

#[path = "auth_profile_tests.rs"]
mod profile_tests;
