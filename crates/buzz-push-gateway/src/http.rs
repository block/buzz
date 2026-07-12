//! Stateful installation, delegation, delivery, and health APIs.
use crate::{
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    app_attest::AppAttestVerifier,
    authority::{
        AuthorityError, AuthorityStore, Challenge, Delegation, DeliveryDisposition, NewInstallation,
    },
    grant::GrantKeyring,
    model::*,
    token::TokenKeyring,
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
    pub app_attest: Arc<AppAttestVerifier>,
    pub authority: Arc<dyn AuthorityStore>,
    pub token_keyring: Arc<TokenKeyring>,
    pub transport: Arc<dyn PushTransport>,
    pub delivery_url: url::Url,
    pub max_grant_lifetime_seconds: i64,
    pub max_installation_lifetime_seconds: i64,
    pub endpoint_quota_window_seconds: i64,
    pub endpoint_quota_max_deliveries: i64,
    pub enabled_profiles: HashSet<AppProfile>,
    pub now: fn() -> i64,
    pub accepting: Arc<AtomicBool>,
}
fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}
fn valid_endpoint(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_ENDPOINT_HEX_BYTES * 2
        && v.len().is_multiple_of(2)
        && v.bytes()
            .all(|b| b.is_ascii_hexdigit() && (!b.is_ascii_alphabetic() || b.is_ascii_lowercase()))
}
fn valid_relay_pubkey(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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

fn decode_challenge(value: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .ok()?;
    bytes.try_into().ok()
}
fn authority_error(e: AuthorityError) -> Response {
    match e {
        AuthorityError::Rejected => error(StatusCode::NOT_FOUND, "not_authorized"),
        AuthorityError::Unavailable => {
            error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}
fn endpoint_bytes(endpoint: &str) -> Option<Vec<u8>> {
    valid_endpoint(endpoint)
        .then(|| hex::decode(endpoint).ok())
        .flatten()
}
fn endpoint_fingerprint(profile: AppProfile, token: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"buzz-apns-endpoint-v1\0");
    h.update(profile.as_str().as_bytes());
    h.update([0]);
    h.update(token);
    h.finalize().into()
}
fn transcript<T: serde::Serialize>(domain: &str, value: &T) -> Option<String> {
    let body = serde_json::to_string(value).ok()?;
    Some(format!("{domain}\n{body}"))
}

async fn challenge(State(s): State<AppState>, body: Bytes) -> Response {
    let _r: InstallationChallengeRequest =
        match crate::strict_json::from_slice::<InstallationChallengeRequest>(&body) {
            Ok(r) if r.v == WIRE_VERSION => r,
            _ => return error(StatusCode::BAD_REQUEST, "invalid_request"),
        };
    let now = (s.now)();
    let expires_at = match now.checked_add(300) {
        Some(v) => v,
        None => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let mut value = [0u8; 32];
    if getrandom::fill(&mut value).is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    }
    let c = Challenge {
        id: uuid::Uuid::new_v4(),
        value,
        expires_at,
    };
    if let Err(e) = s.authority.put_challenge(c.clone()).await {
        return authority_error(e);
    }
    (
        StatusCode::OK,
        Json(InstallationChallengeResponse {
            challenge_id: c.id,
            challenge: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c.value),
            expires_at,
        }),
    )
        .into_response()
}

#[derive(serde::Serialize)]
struct EnrollTranscript<'a> {
    v: u8,
    audience: &'static str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    key_id: &'a str,
    app_profile: AppProfile,
    endpoint: &'a str,
    endpoint_epoch: i64,
    expires_at: i64,
}
async fn enroll(State(s): State<AppState>, body: Bytes) -> Response {
    let r: InstallationEnrollRequest = match crate::strict_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let now = (s.now)();
    let token = match endpoint_bytes(&r.endpoint) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION
        || r.endpoint_epoch != 1
        || r.expires_at <= now
        || r.expires_at > now.saturating_add(s.max_installation_lifetime_seconds)
        || !s.enabled_profiles.contains(&r.app_profile)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let challenge = match decode_challenge(&r.challenge) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let t = EnrollTranscript {
        v: r.v,
        audience: "https://push.buzz.xyz/v1/installations",
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        key_id: &r.key_id,
        app_profile: r.app_profile,
        endpoint: &r.endpoint,
        endpoint_epoch: r.endpoint_epoch,
        expires_at: r.expires_at,
    };
    let signed = match transcript("buzz.push.enroll.v1", &t) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let verified =
        match s
            .app_attest
            .verify_attestation(&r.attestation, &r.key_id, signed.as_bytes())
        {
            Ok(v) => v,
            Err(_) => return error(StatusCode::UNAUTHORIZED, "invalid_attestation"),
        };
    if let Err(e) = s
        .authority
        .consume_challenge(r.challenge_id, challenge, now)
        .await
    {
        return authority_error(e);
    }
    let ciphertext = match s.token_keyring.seal(&token) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let id = uuid::Uuid::new_v4();
    let n = NewInstallation {
        id,
        app_attest_key_id: verified.key_id,
        app_attest_public_key: verified.public_key,
        assertion_counter: 0,
        profile: r.app_profile,
        token_ciphertext: ciphertext,
        token_fingerprint: endpoint_fingerprint(r.app_profile, &token),
        endpoint_epoch: 1,
        expires_at: r.expires_at,
    };
    if let Err(e) = s.authority.create_installation(n).await {
        return authority_error(e);
    }
    (
        StatusCode::CREATED,
        Json(InstallationEnrollResponse {
            installation_handle: id,
            endpoint_epoch: 1,
            expires_at: r.expires_at,
        }),
    )
        .into_response()
}

async fn verify_installation_assertion<T: serde::Serialize>(
    s: &AppState,
    installation_id: uuid::Uuid,
    challenge_id: uuid::Uuid,
    challenge_text: &str,
    assertion: &str,
    domain: &str,
    signed: &T,
) -> Result<(), Response> {
    let now = (s.now)();
    let challenge = decode_challenge(challenge_text)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let installation = s
        .authority
        .installation(installation_id, now)
        .await
        .map_err(authority_error)?;
    let transcript = transcript(domain, signed)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let verified = s
        .app_attest
        .verify_assertion(
            assertion,
            transcript.as_bytes(),
            &installation.app_attest_public_key,
            installation.assertion_counter,
            challenge_text,
            challenge_text,
        )
        .map_err(|_| error(StatusCode::UNAUTHORIZED, "invalid_attestation"))?;
    s.authority
        .consume_challenge(challenge_id, challenge, now)
        .await
        .map_err(authority_error)?;
    s.authority
        .advance_assertion_counter(
            installation_id,
            installation.assertion_counter,
            verified.counter,
        )
        .await
        .map_err(authority_error)
}

#[derive(serde::Serialize)]
struct DelegateTranscript<'a> {
    v: u8,
    audience: &'static str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    installation_handle: uuid::Uuid,
    endpoint_epoch: i64,
    generation: i64,
    relay_pubkey: &'a str,
    not_before: i64,
    expires_at: i64,
}
async fn delegate(State(s): State<AppState>, body: Bytes) -> Response {
    let r: DelegationRequest = match crate::strict_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let now = (s.now)();
    if r.v != WIRE_VERSION
        || !valid_relay_pubkey(&r.relay_pubkey)
        || r.endpoint_epoch < 1
        || r.generation < 1
        || r.not_before > now + 300
        || r.expires_at <= r.not_before
        || r.expires_at > now + s.max_grant_lifetime_seconds
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let t = DelegateTranscript {
        v: r.v,
        audience: "https://push.buzz.xyz/v1/delegations",
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        generation: r.generation,
        relay_pubkey: &r.relay_pubkey,
        not_before: r.not_before,
        expires_at: r.expires_at,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        r.installation_handle,
        r.challenge_id,
        &r.challenge,
        &r.assertion,
        "buzz.push.delegate.v1",
        &t,
    )
    .await
    {
        return e;
    }
    let d = Delegation {
        id: uuid::Uuid::new_v4(),
        installation_id: r.installation_handle,
        relay_pubkey: r.relay_pubkey.clone(),
        endpoint_epoch: r.endpoint_epoch,
        generation: r.generation,
        not_before: r.not_before,
        expires_at: r.expires_at,
        revoked: false,
    };
    if let Err(e) = s.authority.upsert_delegation(d.clone()).await {
        return authority_error(e);
    }
    let g = EndpointGrant {
        v: WIRE_VERSION,
        delegation_id: d.id,
        relay_pubkey: d.relay_pubkey,
        app_profile: match s.authority.installation(d.installation_id, now).await {
            Ok(i) => i.profile,
            Err(e) => return authority_error(e),
        },
        endpoint_epoch: d.endpoint_epoch,
        generation: d.generation,
        expires_at: d.expires_at,
    };
    match s.grant_keyring.issue(&g) {
        Ok(delivery_capability) => (
            StatusCode::CREATED,
            Json(DelegationResponse {
                delivery_capability,
            }),
        )
            .into_response(),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    }
}

#[derive(serde::Serialize)]
struct RotateTranscript<'a> {
    v: u8,
    audience: &'static str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    installation_handle: uuid::Uuid,
    endpoint_epoch: i64,
    new_endpoint_epoch: i64,
    endpoint: &'a str,
}
async fn rotate_endpoint(State(s): State<AppState>, body: Bytes) -> Response {
    let r: RotateEndpointRequest = match crate::strict_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let token = match endpoint_bytes(&r.endpoint) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION
        || r.endpoint_epoch < 1
        || r.new_endpoint_epoch != r.endpoint_epoch.saturating_add(1)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let installation = match s
        .authority
        .installation(r.installation_handle, (s.now)())
        .await
    {
        Ok(i) => i,
        Err(e) => return authority_error(e),
    };
    let t = RotateTranscript {
        v: r.v,
        audience: "https://push.buzz.xyz/v1/installations/endpoint",
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        new_endpoint_epoch: r.new_endpoint_epoch,
        endpoint: &r.endpoint,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        r.installation_handle,
        r.challenge_id,
        &r.challenge,
        &r.assertion,
        "buzz.push.rotate-endpoint.v1",
        &t,
    )
    .await
    {
        return e;
    }
    let ciphertext = match s.token_keyring.seal(&token) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    match s
        .authority
        .rotate_endpoint(
            r.installation_handle,
            r.endpoint_epoch,
            r.new_endpoint_epoch,
            ciphertext,
            endpoint_fingerprint(installation.profile, &token),
        )
        .await
    {
        Ok(()) => (StatusCode::OK, Json(MutationResponse { status: "rotated" })).into_response(),
        Err(e) => authority_error(e),
    }
}
#[derive(serde::Serialize)]
struct RevokeDelegationTranscript<'a> {
    v: u8,
    audience: &'static str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    installation_handle: uuid::Uuid,
    relay_pubkey: &'a str,
    generation: i64,
}
async fn revoke_delegation(State(s): State<AppState>, body: Bytes) -> Response {
    let r: RevokeDelegationRequest = match crate::strict_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION || !valid_relay_pubkey(&r.relay_pubkey) || r.generation < 1 {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let t = RevokeDelegationTranscript {
        v: r.v,
        audience: "https://push.buzz.xyz/v1/delegations/revoke",
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        relay_pubkey: &r.relay_pubkey,
        generation: r.generation,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        r.installation_handle,
        r.challenge_id,
        &r.challenge,
        &r.assertion,
        "buzz.push.revoke-delegation.v1",
        &t,
    )
    .await
    {
        return e;
    }
    match s
        .authority
        .revoke_delegation(r.installation_handle, &r.relay_pubkey, r.generation)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(MutationResponse { status: "revoked" })).into_response(),
        Err(e) => authority_error(e),
    }
}
#[derive(serde::Serialize)]
struct RevokeInstallationTranscript<'a> {
    v: u8,
    audience: &'static str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    installation_handle: uuid::Uuid,
    endpoint_epoch: i64,
    new_endpoint_epoch: i64,
}
async fn revoke_installation(State(s): State<AppState>, body: Bytes) -> Response {
    let r: RevokeInstallationRequest = match crate::strict_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION
        || r.endpoint_epoch < 1
        || r.new_endpoint_epoch != r.endpoint_epoch.saturating_add(1)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let t = RevokeInstallationTranscript {
        v: r.v,
        audience: "https://push.buzz.xyz/v1/installations/revoke",
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        new_endpoint_epoch: r.new_endpoint_epoch,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        r.installation_handle,
        r.challenge_id,
        &r.challenge,
        &r.assertion,
        "buzz.push.revoke-installation.v1",
        &t,
    )
    .await
    {
        return e;
    }
    match s
        .authority
        .revoke_installation(
            r.installation_handle,
            r.endpoint_epoch,
            r.new_endpoint_epoch,
        )
        .await
    {
        Ok(()) => (StatusCode::OK, Json(MutationResponse { status: "revoked" })).into_response(),
        Err(e) => authority_error(e),
    }
}

async fn deliver(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let r: DeliveryRequest = match crate::strict_json::from_slice(&body) {
        Ok(x) => x,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let auth = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(x) => x,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
    };
    let event_id = match auth_event_id(auth) {
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
        || !valid_relay_pubkey(&grant.relay_pubkey)
        || grant.relay_pubkey != relay
        || grant.endpoint_epoch < 1
        || grant.generation < 1
        || grant.expires_at < now
        || r.expires_at < now
        || r.expires_at > grant.expires_at
    {
        return error(StatusCode::NOT_FOUND, "invalid_grant");
    }
    let permit = match s
        .authority
        .authorize_delivery(
            grant.delegation_id,
            &relay,
            grant.endpoint_epoch,
            grant.generation,
            &event_id,
            r.request_id,
            r.expires_at,
            s.endpoint_quota_window_seconds,
            s.endpoint_quota_max_deliveries,
            now,
        )
        .await
    {
        Ok(permit) => {
            crate::metrics::record_admission(crate::metrics::Admission::Admitted);
            permit
        }
        Err(AuthorityError::Rejected) => {
            crate::metrics::record_admission(crate::metrics::Admission::Rejected);
            crate::metrics::record_delivery_error("invalid_grant");
            return error(StatusCode::NOT_FOUND, "invalid_grant");
        }
        Err(AuthorityError::Unavailable) => {
            crate::metrics::record_admission(crate::metrics::Admission::Unavailable);
            crate::metrics::record_delivery_error("temporarily_unavailable");
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    if permit.authority.profile != grant.app_profile {
        crate::metrics::record_delivery_error("profile_mismatch");
        let _ = s
            .authority
            .finish_delivery(permit, DeliveryDisposition::Terminal)
            .await;
        return error(StatusCode::NOT_FOUND, "invalid_grant");
    }
    let profile = permit.authority.profile;
    let endpoint = match s.token_keyring.open(&permit.authority.token_ciphertext) {
        Ok(token) => hex::encode(token),
        Err(_) => {
            crate::metrics::record_delivery_error("token_custody");
            let _ = s
                .authority
                .finish_delivery(permit, DeliveryDisposition::Retryable)
                .await;
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    let attempt = DeliveryAttempt {
        request_id: r.request_id,
        expires_at: r.expires_at,
    };
    let transport = Arc::clone(&s.transport);
    let authority_store = Arc::clone(&s.authority);
    // Admission already committed, so cancellation cannot undo either replay
    // fence. The detached task completes disposition bookkeeping.
    let delivery = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut outcome = transport.send(attempt, profile, &endpoint).await;
        if outcome == DeliveryOutcome::RefreshCredential {
            crate::metrics::record_credential_refresh();
            transport.refresh_credential();
            outcome = transport.send(attempt, profile, &endpoint).await;
        }
        crate::metrics::record_apns_delivery(outcome, started.elapsed().as_secs_f64());
        let disposition = match outcome {
            DeliveryOutcome::Retry { .. }
            | DeliveryOutcome::ConfigurationFault
            | DeliveryOutcome::RefreshCredential => DeliveryDisposition::Retryable,
            DeliveryOutcome::Accepted
            | DeliveryOutcome::InvalidEndpoint { .. }
            | DeliveryOutcome::PermanentRequestFault => DeliveryDisposition::Terminal,
        };
        authority_store
            .finish_delivery(permit, disposition)
            .await
            .map(|()| outcome)
    });
    let outcome = match delivery.await {
        Ok(Ok(outcome)) => outcome,
        _ => {
            crate::metrics::record_delivery_error("finish_failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    match outcome {
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
        DeliveryOutcome::ConfigurationFault | DeliveryOutcome::RefreshCredential => {
            error(StatusCode::SERVICE_UNAVAILABLE, "configuration_fault")
        }
        DeliveryOutcome::PermanentRequestFault => error(StatusCode::BAD_REQUEST, "invalid_request"),
    }
}
async fn live() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"alive"}))
}
async fn ready(State(s): State<AppState>) -> Response {
    if !s.accepting.load(Ordering::Relaxed) {
        crate::metrics::record_readiness_failure(crate::metrics::ReadinessFailure::NotAccepting);
        return error(StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    }
    if s.authority.ready().await.is_err() {
        crate::metrics::record_readiness_failure(crate::metrics::ReadinessFailure::Authority);
        return error(StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    }
    Json(serde_json::json!({"status":"ready"})).into_response()
}
pub fn router(state: AppState) -> (Router, Router) {
    router_with_metrics(state, None)
}

/// Build the public and private routers. When `metrics_handle` is provided, the
/// private health router additionally serves `GET /metrics` in Prometheus text
/// format. Metrics live only on the private router, never on the public port.
pub fn router_with_metrics(
    state: AppState,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
) -> (Router, Router) {
    let public = Router::new()
        .route("/v1/installations/challenges", post(challenge))
        .route("/v1/installations", post(enroll))
        .route("/v1/delegations", post(delegate))
        .route("/v1/delegations/revoke", post(revoke_delegation))
        .route("/v1/installations/endpoint", post(rotate_endpoint))
        .route("/v1/installations/revoke", post(revoke_installation))
        .route("/v1/deliveries/apns", post(deliver))
        .with_state(state.clone())
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(ConcurrencyLimitLayer::new(256))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(20),
        ));
    let mut health = Router::new()
        .route("/_liveness", get(live))
        .route("/_readiness", get(ready))
        .with_state(state);
    if let Some(handle) = metrics_handle {
        health = health.route(
            "/metrics",
            get(move || {
                let handle = handle.clone();
                async move {
                    handle.run_upkeep();
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        handle.render(),
                    )
                }
            }),
        );
    }
    (public, health)
}
