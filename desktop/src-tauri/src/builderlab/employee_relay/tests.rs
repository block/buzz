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
    assert!(send_authorized_request(request, evidence)
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
