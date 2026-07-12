//! Stateless delivery API and health routers.
use crate::{
    apns::{DeliveryOutcome, PushTransport},
    grant::GrantKeyring,
    model::*,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{
    nips::nip98::{verify_auth_header, HttpMethod},
    Event, JsonUtil, Timestamp,
};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

#[derive(Clone)]
pub struct AppState {
    pub grant_keyring: Arc<GrantKeyring>,
    pub transport: Arc<dyn PushTransport>,
    pub delivery_url: url::Url,
    pub issuance_url: url::Url,
    pub max_grant_lifetime_seconds: i64,
    pub enabled_profiles: HashSet<AppProfile>,
    pub authorized_relays: HashSet<String>,
    pub now: fn() -> i64,
    pub accepting: Arc<AtomicBool>,
}
fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}
fn valid_hex(v: &str, n: usize) -> bool {
    v.len() == n * 2
        && v.bytes()
            .all(|b| b.is_ascii_hexdigit() && (!b.is_ascii_alphabetic() || b.is_ascii_lowercase()))
}
fn auth_event_id(header: &str) -> Option<String> {
    let (prefix, encoded) = header.split_once(' ')?;
    if prefix != "Nostr" {
        return None;
    }
    Event::from_json(STANDARD.decode(encoded).ok()?)
        .ok()
        .map(|e| e.id.to_hex())
}
fn valid_wake(r: &DeliveryRequest) -> bool {
    r.v == WIRE_VERSION
        && r.wake.v == WIRE_VERSION
        && r.wake
            .grant
            .as_ref()
            .is_none_or(|g| g.len() <= MAX_GRANT_BYTES)
        && match r.class {
            DeliveryClass::Silent => r.wake.fallback.is_none(),
            _ => r.wake.fallback.as_deref() == Some(FALLBACK_TEXT),
        }
}
async fn issue_grant(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let request: GrantIssueRequest = match crate::strict_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let auth = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        Some(auth) => auth,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    let relay = match verify_auth_header(
        auth,
        &s.issuance_url,
        HttpMethod::POST,
        Timestamp::now(),
        Some(&body),
    ) {
        Ok(pubkey) => pubkey.to_hex(),
        Err(_) => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    if !s.authorized_relays.contains(&relay) {
        return error(StatusCode::UNAUTHORIZED, "invalid_auth");
    }
    let now = (s.now)();
    let max_expires_at = match now.checked_add(s.max_grant_lifetime_seconds) {
        Some(max_expires_at) => max_expires_at,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if request.v != WIRE_VERSION
        || !valid_hex(&request.endpoint, 32)
        || request.generation < 1
        || request.expires_at <= now
        || request.expires_at > max_expires_at
        || !s.enabled_profiles.contains(&request.app_profile)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let grant = EndpointGrant {
        v: request.v,
        endpoint: request.endpoint,
        relay_pubkey: relay,
        app_profile: request.app_profile,
        max_class: request.max_class,
        generation: request.generation,
        expires_at: request.expires_at,
    };
    match s.grant_keyring.issue(&grant) {
        Ok(endpoint_grant) => {
            (StatusCode::OK, Json(GrantIssueResponse { endpoint_grant })).into_response()
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "issuance_failed"),
    }
}

async fn deliver(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let r: DeliveryRequest = match crate::strict_json::from_slice(&body) {
        Ok(x) => x,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if !valid_wake(&r) {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let auth = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(x) => x,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    let _event = match auth_event_id(auth) {
        Some(x) => x,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    let relay = match verify_auth_header(
        auth,
        &s.delivery_url,
        HttpMethod::POST,
        Timestamp::now(),
        Some(&body),
    ) {
        Ok(x) => x.to_hex(),
        Err(_) => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    let grant = match s.grant_keyring.open(&r.endpoint_grant) {
        Ok(x) => x,
        Err(_) => return error(StatusCode::NOT_FOUND, "invalid_grant"),
    };
    let now = (s.now)();
    if grant.v != WIRE_VERSION
        || !valid_hex(&grant.endpoint, 32)
        || !valid_hex(&grant.relay_pubkey, 32)
        || grant.relay_pubkey != relay
        || grant.generation < 1
        || grant.expires_at < now
        || r.expires_at < now
        || r.expires_at > grant.expires_at
        || r.class > grant.max_class
    {
        return error(StatusCode::NOT_FOUND, "invalid_grant");
    }
    match s
        .transport
        .send(&r, grant.app_profile, &grant.endpoint)
        .await
    {
        DeliveryOutcome::Accepted => {
            (StatusCode::OK, Json(DeliveryResponse::Accepted)).into_response()
        }
        DeliveryOutcome::InvalidEndpoint { unregistered_at } => (
            StatusCode::GONE,
            Json(DeliveryResponse::InvalidEndpoint {
                generation: grant.generation,
                invalid_at: unregistered_at,
            }),
        )
            .into_response(),
        DeliveryOutcome::Retry {
            retry_after_seconds,
        } => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(DeliveryResponse::Retry {
                retry_after_seconds,
            }),
        )
            .into_response(),
        DeliveryOutcome::RefreshCredential => {
            s.transport.refresh_credential();
            match s
                .transport
                .send(&r, grant.app_profile, &grant.endpoint)
                .await
            {
                DeliveryOutcome::Accepted => {
                    (StatusCode::OK, Json(DeliveryResponse::Accepted)).into_response()
                }
                DeliveryOutcome::InvalidEndpoint { unregistered_at } => (
                    StatusCode::GONE,
                    Json(DeliveryResponse::InvalidEndpoint {
                        generation: grant.generation,
                        invalid_at: unregistered_at,
                    }),
                )
                    .into_response(),
                DeliveryOutcome::Retry {
                    retry_after_seconds,
                } => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(DeliveryResponse::Retry {
                        retry_after_seconds,
                    }),
                )
                    .into_response(),
                _ => error(StatusCode::SERVICE_UNAVAILABLE, "configuration_fault"),
            }
        }
        DeliveryOutcome::ConfigurationFault => {
            error(StatusCode::SERVICE_UNAVAILABLE, "configuration_fault")
        }
        DeliveryOutcome::PermanentRequestFault => error(StatusCode::BAD_REQUEST, "invalid_request"),
    }
}
async fn live() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"alive"}))
}
async fn ready(State(s): State<AppState>) -> Response {
    if s.accepting.load(Ordering::Relaxed) {
        Json(serde_json::json!({"status":"ready"})).into_response()
    } else {
        error(StatusCode::SERVICE_UNAVAILABLE, "not_ready")
    }
}
pub fn router(state: AppState) -> (Router, Router) {
    let public = Router::new()
        .route("/v1/grants/apns", post(issue_grant))
        .route("/v1/deliveries/apns", post(deliver))
        .with_state(state.clone())
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(ConcurrencyLimitLayer::new(256))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(20),
        ));
    let health = Router::new()
        .route("/_liveness", get(live))
        .route("/_readiness", get(ready))
        .with_state(state);
    (public, health)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, http::Request};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    struct AcceptTransport;

    #[async_trait]
    impl PushTransport for AcceptTransport {
        async fn send(
            &self,
            _request: &DeliveryRequest,
            _profile: AppProfile,
            _endpoint: &str,
        ) -> DeliveryOutcome {
            DeliveryOutcome::Accepted
        }
    }

    fn state(keys: &Keys) -> AppState {
        AppState {
            grant_keyring: Arc::new(
                GrantKeyring::new(vec![
                    crate::grant::GrantKey::new("current", &[7; 32]).unwrap()
                ])
                .unwrap(),
            ),
            transport: Arc::new(AcceptTransport),
            delivery_url: "https://push.example/v1/deliveries/apns".parse().unwrap(),
            issuance_url: "https://push.example/v1/grants/apns".parse().unwrap(),
            max_grant_lifetime_seconds: 3600,
            enabled_profiles: HashSet::from([AppProfile::BuzzIosProduction]),
            authorized_relays: HashSet::from([keys.public_key().to_hex()]),
            now: || 1_700_000_000,
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }

    fn auth(keys: &Keys, url: &url::Url, body: &[u8]) -> String {
        let tags = vec![
            Tag::parse(["u", url.as_str()]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        format!(
            "Nostr {}",
            STANDARD.encode(serde_json::to_vec(&event).unwrap())
        )
    }

    async fn post(app: Router, path: &str, body: Vec<u8>, auth: Option<String>) -> Response {
        let mut request = Request::post(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        if let Some(auth) = auth {
            request
                .headers_mut()
                .insert(axum::http::header::AUTHORIZATION, auth.parse().unwrap());
        }
        app.oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn issuance_is_strict_bounded_and_uses_the_delivery_keyring() {
        let keys = Keys::generate();
        let state = state(&keys);
        let (app, _) = router(state.clone());
        let expires_at = (state.now)() + 60;
        let issue_body = serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "endpoint": "00".repeat(32),
            "app_profile": "buzz-ios-production",
            "max_class": "default",
            "generation": 2,
            "expires_at": expires_at,
        }))
        .unwrap();
        let issue_response = post(
            app.clone(),
            "/v1/grants/apns",
            issue_body.clone(),
            Some(auth(&keys, &state.issuance_url, &issue_body)),
        )
        .await;
        assert_eq!(issue_response.status(), StatusCode::OK);
        let body = to_bytes(issue_response.into_body(), MAX_REQUEST_BYTES)
            .await
            .unwrap();
        let issued: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let endpoint_grant = issued["endpoint_grant"].as_str().unwrap();
        let grant = state.grant_keyring.open(endpoint_grant).unwrap();
        assert_eq!(grant.relay_pubkey, keys.public_key().to_hex());

        let delivery_body = serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "endpoint_grant": endpoint_grant,
            "request_id": uuid::Uuid::new_v4(),
            "class": "default",
            "expires_at": expires_at,
            "wake": {"v": 1, "fallback": FALLBACK_TEXT},
        }))
        .unwrap();
        let delivery_response = post(
            app,
            "/v1/deliveries/apns",
            delivery_body.clone(),
            Some(auth(&keys, &state.delivery_url, &delivery_body)),
        )
        .await;
        assert_eq!(delivery_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn issuance_rejects_authority_override_wrong_url_and_invalid_claims() {
        let keys = Keys::generate();
        let state = state(&keys);
        let (app, _) = router(state.clone());
        let now = (state.now)();
        let cases = [
            serde_json::json!({
                "v": 1, "endpoint": "00".repeat(32),
                "relay_pubkey": "11".repeat(32),
                "app_profile": "buzz-ios-production", "max_class": "default",
                "generation": 1, "expires_at": now + 60,
            }),
            serde_json::json!({
                "v": 1, "endpoint": "00".repeat(32),
                "app_profile": "buzz-ios-sandbox", "max_class": "default",
                "generation": 1, "expires_at": now + 60,
            }),
            serde_json::json!({
                "v": 1, "endpoint": "00".repeat(32),
                "app_profile": "buzz-ios-production", "max_class": "default",
                "generation": 1, "expires_at": now,
            }),
            serde_json::json!({
                "v": 1, "endpoint": "00".repeat(32),
                "app_profile": "buzz-ios-production", "max_class": "default",
                "generation": 1, "expires_at": now + 3601,
            }),
        ];
        for value in cases {
            let body = serde_json::to_vec(&value).unwrap();
            let response = post(
                app.clone(),
                "/v1/grants/apns",
                body.clone(),
                Some(auth(&keys, &state.issuance_url, &body)),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let body = serde_json::to_vec(&serde_json::json!({
            "v": 1, "endpoint": "00".repeat(32),
            "app_profile": "buzz-ios-production", "max_class": "default",
            "generation": 1, "expires_at": now + 60,
        }))
        .unwrap();
        let response = post(
            app,
            "/v1/grants/apns",
            body.clone(),
            Some(auth(&keys, &state.delivery_url, &body)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    fn valid_issue_body(expires_at: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1, "endpoint": "00".repeat(32),
            "app_profile": "buzz-ios-production", "max_class": "default",
            "generation": 1, "expires_at": expires_at,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn issuance_rejects_unauthorized_signer_wrong_body_and_invalid_json() {
        let keys = Keys::generate();
        let unauthorized = Keys::generate();
        let state = state(&keys);
        let (app, _) = router(state.clone());
        let body = valid_issue_body((state.now)() + 60);

        let response = post(
            app.clone(),
            "/v1/grants/apns",
            body.clone(),
            Some(auth(&unauthorized, &state.issuance_url, &body)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let other_body = valid_issue_body((state.now)() + 61);
        let response = post(
            app.clone(),
            "/v1/grants/apns",
            body.clone(),
            Some(auth(&keys, &state.issuance_url, &other_body)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        for invalid in [
            b"{".as_slice(),
            br#"{"v":1,"v":1,"endpoint":"00","app_profile":"buzz-ios-production","max_class":"default","generation":1,"expires_at":1700000060}"#,
        ] {
            let response = post(
                app.clone(),
                "/v1/grants/apns",
                invalid.to_vec(),
                Some(auth(&keys, &state.issuance_url, invalid)),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn issuance_accepts_exact_ceiling_and_rejects_overflowing_ceiling() {
        let keys = Keys::generate();
        let configured_state = state(&keys);
        let (app, _) = router(configured_state.clone());
        let body = valid_issue_body(
            (configured_state.now)() + configured_state.max_grant_lifetime_seconds,
        );
        let response = post(
            app,
            "/v1/grants/apns",
            body.clone(),
            Some(auth(&keys, &configured_state.issuance_url, &body)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut overflow_state = state(&keys);
        overflow_state.now = || i64::MAX;
        let (app, _) = router(overflow_state.clone());
        let body = valid_issue_body(i64::MAX);
        let response = post(
            app,
            "/v1/grants/apns",
            body.clone(),
            Some(auth(&keys, &overflow_state.issuance_url, &body)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
