use super::*;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct Stub {
    captured: Mutex<Vec<(HeaderMap, Vec<u8>)>>,
    response: Mutex<serde_json::Value>,
    entered: Notify,
    release: Notify,
    hold: bool,
    redirect: bool,
    status: Mutex<StatusCode>,
}
async fn issue(
    State(stub): State<Arc<Stub>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    stub.captured.lock().unwrap().push((headers, body.to_vec()));
    stub.entered.notify_one();
    if stub.hold {
        stub.release.notified().await;
    }
    if stub.redirect {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [("Location", "/must-not-receive-credential")],
        )
            .into_response();
    }
    (
        *stub.status.lock().unwrap(),
        axum::Json(stub.response.lock().unwrap().clone()),
    )
        .into_response()
}
struct Fixture {
    state: Arc<crate::app_state::AppState>,
    session: Arc<BuilderlabSession>,
    stub: Arc<Stub>,
    task: tokio::task::JoinHandle<()>,
    relay: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(hold: bool, redirect: bool) -> Self {
        let state = Arc::new(crate::app_state::build_app_state());
        let keys = state.signing_keys().unwrap();
        let relay = "wss://employee.example".to_owned();
        *state.relay_url_override.lock().unwrap() = Some(relay.clone());
        let stub = Arc::new(Stub {
            hold,
            redirect,
            ..Stub::default()
        });
        *stub.response.lock().unwrap() = serde_json::json!({"assertion":"test.assertion.signature","nostr_pubkey":keys.public_key().to_hex(),"expires_at":chrono::Utc::now().timestamp()+120});
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base: &'static str = Box::leak(
            format!("http://{}/api/goose", listener.local_addr().unwrap()).into_boxed_str(),
        );
        let app = Router::new()
            .route("/api/goose/v1/buzz/identity/assertions", post(issue))
            .route("/must-not-receive-credential", post(issue))
            .route("/query", post(issue))
            .with_state(stub.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let session = Arc::new(BuilderlabSession::default());
        session
            .replace(StoredSession {
                credential: "private-session-fixture".to_owned(),
                api_base: base,
                employee: Some(EmployeeBinding {
                    pubkey: keys.public_key().to_hex(),
                    expires_at: chrono::Utc::now().timestamp() + 240,
                    relay: relay.clone(),
                }),
                assertion: None,
                refusal: None,
                cancel: CancellationToken::new(),
            })
            .unwrap();
        Self {
            state,
            session,
            stub,
            task,
            relay,
        }
    }
    async fn evidence(&self) -> Result<EmployeeEvidence, String> {
        self.session
            .evidence(
                &self.state,
                &self.state.signing_keys().unwrap(),
                &self.relay,
            )
            .await
    }
}

#[tokio::test]
async fn employee_assertion_uses_exact_own_key_proof_and_caches_only_scoped_native_evidence() {
    let fixture = Fixture::new(false, false).await;
    assert!(
        fixture.session.employee_expiry(&fixture.state).is_none(),
        "a CLI receipt alone is not connected Buzz access"
    );
    let evidence = fixture.evidence().await.unwrap();
    assert!(fixture.session.employee_expiry(&fixture.state).is_some());
    assert!(evidence.header.is_sensitive());
    assert_eq!(evidence.header, "Bearer test.assertion.signature");
    fixture.evidence().await.unwrap();
    let captured = fixture.stub.captured.lock().unwrap();
    assert_eq!(
        captured.len(),
        1,
        "the same employee/key/relay reuses its short-lived assertion"
    );
    let (headers, body) = &captured[0];
    assert_eq!(
        headers[BB_SESSION_CREDENTIAL_HEADER],
        "private-session-fixture"
    );
    let proof = headers["Authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let proof = base64::engine::general_purpose::STANDARD
        .decode(proof)
        .unwrap();
    let event: nostr::Event = serde_json::from_slice(&proof).unwrap();
    event.verify().unwrap();
    assert_eq!(
        event.pubkey,
        fixture.state.signing_keys().unwrap().public_key()
    );
    let event: serde_json::Value = serde_json::from_slice(&proof).unwrap();
    let tags = event["tags"].as_array().unwrap();
    let base = fixture
        .session
        .request_context(&fixture.state)
        .unwrap()
        .unwrap()
        .1;
    assert!(tags.contains(&serde_json::json!([
        "u",
        format!("{base}/v1/buzz/identity/assertions")
    ])));
    assert!(tags.contains(&serde_json::json!(["method", "POST"])));
    assert!(tags.contains(&serde_json::json!([
        "payload",
        hex::encode(Sha256::digest(body))
    ])));
    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(body["relay_url"], fixture.relay);
    assert_eq!(body["nostr_pubkey"], event["pubkey"]);
}

#[tokio::test]
async fn employee_assertion_refuses_wrong_key_or_expiry_without_caching() {
    for mutation in ["key", "expired", "beyond-session", "malformed"] {
        let fixture = Fixture::new(false, false).await;
        {
            let mut response = fixture.stub.response.lock().unwrap();
            match mutation {
                "key" => response["nostr_pubkey"] = "00".repeat(32).into(),
                "expired" => response["expires_at"] = (chrono::Utc::now().timestamp() - 1).into(),
                "beyond-session" => {
                    response["expires_at"] = (chrono::Utc::now().timestamp() + 280).into()
                }
                _ => response["assertion"] = "not a JWT".into(),
            }
        }
        assert!(fixture.evidence().await.is_err(), "{mutation}");
        assert!(fixture
            .session
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .assertion
            .is_none());
    }
}

#[tokio::test]
async fn sign_out_cancels_an_inflight_assertion_and_never_restores_its_cache() {
    let fixture = Fixture::new(true, false).await;
    let request = fixture.evidence();
    tokio::pin!(request);
    tokio::select! { _ = fixture.stub.entered.notified()=>{}, result = &mut request => panic!("premature result: {}",result.is_ok()) }
    fixture.session.clear().unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .unwrap()
        .is_err());
    assert!(fixture.session.0.lock().unwrap().is_none());
}

#[tokio::test]
async fn community_switch_during_issuance_refuses_the_old_response() {
    let fixture = Fixture::new(true, false).await;
    let request = fixture.evidence();
    tokio::pin!(request);
    tokio::select! { _ = fixture.stub.entered.notified()=>{}, result = &mut request => panic!("premature result: {}",result.is_ok()) }
    fixture
        .state
        .workspace_apply_generation
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    fixture.stub.release.notify_one();
    assert!(request.await.is_err());
    assert!(fixture
        .session
        .0
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .assertion
        .is_none());
}

#[tokio::test]
async fn assertion_redirect_is_refused_without_forwarding_the_session() {
    let fixture = Fixture::new(false, true).await;
    assert!(fixture.evidence().await.is_err());
    assert_eq!(fixture.stub.captured.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn employee_authorization_denial_explains_recovery_without_echoing_the_response() {
    let fixture = Fixture::new(false, false).await;
    *fixture.stub.status.lock().unwrap() = StatusCode::FORBIDDEN;
    *fixture.stub.response.lock().unwrap() =
        serde_json::json!({"error":"binding_mismatch","diagnostic":"private-session-fixture"});
    let error = fixture.evidence().await.err().unwrap();
    assert!(error.contains("another Buzz key"));
    assert!(!error.contains("private-session-fixture"));
    assert!(fixture.session.employee_expiry(&fixture.state).is_none());
    assert!(fixture.session.employee_can_retry(&fixture.state));
    assert!(fixture.evidence().await.is_err());
    assert_eq!(
        fixture.stub.captured.lock().unwrap().len(),
        1,
        "refused access must not automatically contact the issuer again"
    );
    *fixture.stub.status.lock().unwrap() = StatusCode::OK;
    let keys = fixture.state.signing_keys().unwrap();
    *fixture.stub.response.lock().unwrap() = serde_json::json!({"assertion":"test.assertion.signature","nostr_pubkey":keys.public_key().to_hex(),"expires_at":chrono::Utc::now().timestamp()+120});
    let evidence = fixture
        .session
        .evidence_with_refresh(&fixture.state, &keys, &fixture.relay, true)
        .await
        .unwrap();
    assert!(!evidence.cancel.is_cancelled());
    assert!(!fixture.session.employee_can_retry(&fixture.state));
    assert_eq!(fixture.stub.captured.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn committed_identity_or_community_change_cancels_previous_employee_evidence() {
    for change_key in [true, false] {
        let fixture = Fixture::new(false, false).await;
        let evidence = fixture.evidence().await.unwrap();
        if change_key {
            *fixture.state.keys.lock().unwrap() = nostr::Keys::generate();
        } else {
            *fixture.state.relay_url_override.lock().unwrap() =
                Some("wss://other.example".to_owned());
        }
        assert!(fixture.session.request_context(&fixture.state).is_err());
        assert!(evidence.cancel.is_cancelled());
        assert!(fixture.session.0.lock().unwrap().is_none());
    }
}

#[tokio::test]
async fn relay_http_preserves_own_nip98_proof_without_forwarding_the_bb_session() {
    let fixture = Fixture::new(false, false).await;
    let evidence = fixture.evidence().await.unwrap();
    let base = fixture
        .session
        .request_context(&fixture.state)
        .unwrap()
        .unwrap()
        .1;
    let origin = Url::parse(base).unwrap().origin().ascii_serialization();
    let url = format!("{origin}/query");
    let body = br#"[{"kinds":[9]}]"#.to_vec();
    let keys = fixture.state.signing_keys().unwrap();
    let proof =
        crate::relay::build_nip98_auth_header_for_keys(&keys, &reqwest::Method::POST, &url, &body)
            .unwrap();
    let request = fixture
        .state
        .media_fetch_client
        .post(&url)
        .header("Authorization", &proof)
        .body(body.clone());
    let (client, request) = request.build_split();
    assert!(send_authorized_request(client, request.unwrap(), evidence)
        .await
        .unwrap()
        .status()
        .is_success());
    let captured = fixture.stub.captured.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let (headers, actual_body) = &captured[1];
    assert_eq!(
        headers.get_all("Nostr-Federated-Identity").iter().count(),
        1
    );
    assert_eq!(
        headers["Nostr-Federated-Identity"],
        "Bearer test.assertion.signature"
    );
    assert_eq!(headers["Authorization"], proof);
    assert!(!headers.contains_key(BB_SESSION_CREDENTIAL_HEADER));
    assert_eq!(actual_body, &body);
}

#[tokio::test]
async fn employee_manual_refresh_waits_for_old_issuance_then_requires_a_fresh_receipt() {
    let fixture = Fixture::new(true, false).await;
    let old = fixture.evidence();
    tokio::pin!(old);
    tokio::select! { _ = fixture.stub.entered.notified() => {}, _ = &mut old => panic!("must wait") }
    let keys = fixture.state.signing_keys().unwrap();
    let refresh =
        fixture
            .session
            .evidence_with_refresh(&fixture.state, &keys, &fixture.relay, true);
    tokio::pin!(refresh);
    fixture.stub.release.notify_one();
    old.await.unwrap();
    tokio::select! { _ = fixture.stub.entered.notified() => {}, _ = &mut refresh => panic!("manual refresh reused a pre-retry receipt") }
    assert_eq!(fixture.stub.captured.lock().unwrap().len(), 2);
    fixture.stub.release.notify_one();
    refresh.await.unwrap();
}

#[tokio::test]
async fn employee_http_response_cannot_reflect_assertion_through_errors_or_success_json() {
    for success in [true, false] {
        let fixture = Fixture::new(false, false).await;
        let evidence = fixture.evidence().await.unwrap();
        *fixture.stub.response.lock().unwrap() = serde_json::json!({"message":"test.assertion.signature","nested":{"value":"test.assertion.signature"}});
        *fixture.stub.status.lock().unwrap() = if success {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        let base = fixture
            .session
            .request_context(&fixture.state)
            .unwrap()
            .unwrap()
            .1;
        let url = format!(
            "{}/query",
            Url::parse(base).unwrap().origin().ascii_serialization()
        );
        let (client, request) = fixture.state.media_fetch_client.post(&url).build_split();
        let response = send_authorized_request(client, request.unwrap(), evidence)
            .await
            .unwrap();
        let error = if success {
            crate::relay::parse_json_response::<serde_json::Value>(response)
                .await
                .unwrap_err()
        } else {
            crate::relay::relay_error_message(response).await
        };
        assert!(!error.contains("test.assertion.signature"));
    }
}

#[tokio::test]
async fn employee_response_body_is_canceled_on_logout_or_assertion_expiry() {
    for expired in [true, false] {
        let cancel = CancellationToken::new();
        let lifetime = if expired {
            chrono::Utc::now().timestamp() - 1
        } else {
            chrono::Utc::now().timestamp() + 60
        };
        let scope = ProtectedAssertion(
            "test.assertion.signature".to_owned(),
            cancel.clone(),
            lifetime,
        );
        if !expired {
            cancel.cancel();
        }
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            guard_employee_response(Some(scope), std::future::pending::<()>()),
        )
        .await
        .unwrap();
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn ordinary_media_remains_available_while_identity_is_in_recovery() {
    let fixture = Fixture::new(false, false).await;
    fixture
        .state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    let base = fixture.session.0.lock().unwrap().as_ref().unwrap().api_base;
    let url = format!(
        "{}/query",
        Url::parse(base).unwrap().origin().ascii_serialization()
    );
    let response = send_employee_media_request(
        &fixture.state,
        &url,
        fixture.state.media_fetch_client.post(&url),
    )
    .await
    .unwrap();
    assert!(response.status().is_success());
    assert!(fixture.stub.captured.lock().unwrap()[0]
        .0
        .get("Nostr-Federated-Identity")
        .is_none());
}

#[tokio::test]
async fn employee_transfer_proofs_keep_blossom_kind_and_streaming_body() {
    use nostr::JsonUtil;
    for streaming in [true, false] {
        let keys = nostr::Keys::generate();
        let proof = crate::commands::media::sign_blossom_upload_auth(
            &keys,
            &"a".repeat(64),
            300,
            "https://employee.example",
        )
        .unwrap();
        let auth = format!(
            "Nostr {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_json())
        );
        let body = if streaming {
            reqwest::Body::wrap_stream(futures_util::stream::iter([Ok::<_, std::io::Error>(
                bytes::Bytes::from_static(b"media"),
            )]))
        } else {
            reqwest::Body::from("media")
        };
        let mut request = reqwest::Client::new()
            .put("https://employee.example/upload")
            .header("Authorization", &auth)
            .body(body)
            .build()
            .unwrap();
        refresh_nip98_request(&mut request, &keys).unwrap();
        assert_eq!(request.headers()["Authorization"], auth);
        assert_eq!(request.body().unwrap().as_bytes().is_none(), streaming);
    }
}

#[tokio::test]
async fn ended_employee_sessions_still_require_a_new_browser_login() {
    let fixture = Fixture::new(false, false).await;
    *fixture.stub.status.lock().unwrap() = StatusCode::UNAUTHORIZED;
    *fixture.stub.response.lock().unwrap() = serde_json::json!({"error":"session_expired"});
    assert!(fixture.evidence().await.is_err());
    assert!(fixture.session.0.lock().unwrap().is_none());
    assert!(!fixture.session.employee_can_retry(&fixture.state));
}

#[test]
fn queued_employee_request_refreshes_only_the_expired_nip98_proof() {
    use nostr::JsonUtil;
    let keys = nostr::Keys::generate();
    let url = "https://employee.example/query";
    let body = b"[]";
    let old = nostr::EventBuilder::new(nostr::Kind::Custom(27235), "")
        .tags([
            nostr::Tag::parse(["u", url]).unwrap(),
            nostr::Tag::parse(["method", "POST"]).unwrap(),
            nostr::Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap(),
        ])
        .custom_created_at(nostr::Timestamp::from_secs(1))
        .sign_with_keys(&keys)
        .unwrap();
    let mut request = reqwest::Client::new()
        .post(url)
        .header(
            "Authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD.encode(old.as_json())
            ),
        )
        .body(body.to_vec())
        .build()
        .unwrap();
    refresh_nip98_request(&mut request, &keys).unwrap();
    let encoded = request.headers()["Authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let event: nostr::Event = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap();
    event.verify().unwrap();
    assert_eq!(event.kind, nostr::Kind::Custom(27235));
    assert_eq!(event.pubkey, keys.public_key());
    assert!(event.created_at.as_secs() > 1);
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().is_some_and(|value| value == "nonce")));
    for tag in old.tags.iter() {
        assert!(event.tags.iter().any(|current| current == tag));
    }
    assert_eq!(request.body().unwrap().as_bytes().unwrap(), body);
}

#[tokio::test]
async fn transient_assertion_refresh_failure_can_retry_without_another_login() {
    let fixture = Fixture::new(false, false).await;
    fixture.evidence().await.unwrap();
    assert!(!fixture.session.employee_can_retry(&fixture.state));
    fixture.session.invalidate_employee_assertion().unwrap();
    *fixture.stub.status.lock().unwrap() = StatusCode::SERVICE_UNAVAILABLE;
    assert!(fixture.evidence().await.is_err());
    assert!(fixture.session.employee_expiry(&fixture.state).is_none());
    assert!(fixture.session.employee_can_retry(&fixture.state));
    assert!(fixture
        .session
        .request_context(&fixture.state)
        .unwrap()
        .is_some());
    *fixture.stub.status.lock().unwrap() = StatusCode::OK;
    let keys = fixture.state.signing_keys().unwrap();
    fixture
        .session
        .evidence_with_refresh(&fixture.state, &keys, &fixture.relay, true)
        .await
        .unwrap();
    assert!(fixture.session.employee_expiry(&fixture.state).is_some());
    assert!(!fixture.session.employee_can_retry(&fixture.state));
}

#[tokio::test]
async fn staging_namespace_merchants_cannot_reuse_another_community_session_or_assertion() {
    let first = "wss://viva3.communities.square.builderlabstaging.xyz";
    let second = "wss://merchant-0123456789abcdef0123456789abcdef0123456789abcdef.communities.square.builderlabstaging.xyz";
    for (current, next) in [(first, second), (second, first)] {
        let mut fixture = Fixture::new(false, false).await;
        fixture.relay = current.to_owned();
        *fixture.state.relay_url_override.lock().unwrap() = Some(current.to_owned());
        fixture
            .session
            .0
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .employee
            .as_mut()
            .unwrap()
            .relay = current.to_owned();
        let evidence = fixture.evidence().await.unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&fixture.stub.captured.lock().unwrap()[0].1).unwrap();
        assert_eq!(body["relay_url"], current);
        fixture.relay = next.to_owned();
        *fixture.state.relay_url_override.lock().unwrap() = Some(next.to_owned());
        assert!(fixture.evidence().await.is_err());
        assert!(evidence.cancel.is_cancelled());
        assert!(fixture.session.0.lock().unwrap().is_none());
        assert_eq!(
            fixture.stub.captured.lock().unwrap().len(),
            1,
            "old credential must never request authority for the next merchant"
        );
    }
}
