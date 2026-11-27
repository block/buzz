//! Stateful installation, delegation, delivery, and health APIs.
use crate::{
    android_attestation::{self, AndroidAttestationError, AndroidAttestationVerifier},
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    app_attest::AppAttestVerifier,
    authority::{
        AuthorityError, AuthorityStore, Challenge, Delegation, DeliveryDisposition, Installation,
        NewInstallation, Platform,
    },
    config::{
        DELEGATE_AUDIENCE, ENROLL_AUDIENCE, REVOKE_DELEGATION_AUDIENCE,
        REVOKE_INSTALLATION_AUDIENCE, ROTATE_ENDPOINT_AUDIENCE,
    },
    grant::GrantKeyring,
    model::*,
    signed_call_limiter::SignedCallLimiter,
    token::TokenKeyring,
};
use axum::{
    body::Bytes,
    extract::{MatchedPath, OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{
    nips::nip98::{verify_auth_header, HttpMethod},
    Event, JsonUtil, TagKind, TagStandard, Timestamp,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

/// Verification and delivery services for this gateway's configured application.
#[derive(Clone)]
pub struct ApplicationRuntime {
    pub app_attest: Arc<AppAttestVerifier>,
    pub transport: Arc<dyn PushTransport>,
    /// Present only when the gateway is configured for Android.
    pub android: Option<AndroidRuntime>,
}

/// Android attestation and FCM delivery for this gateway's configured app.
#[derive(Clone)]
pub struct AndroidRuntime {
    pub attestation: Arc<dyn AndroidAttestationVerifier>,
    pub transport: Arc<dyn PushTransport>,
}

#[derive(Clone)]
pub struct AppState {
    pub grant_keyring: Arc<GrantKeyring>,
    pub authority: Arc<dyn AuthorityStore>,
    pub token_keyring: Arc<TokenKeyring>,
    /// Server-owned application identity and APNs transport.
    /// App Attest verifies the configured app ID.
    pub application: Arc<ApplicationRuntime>,
    pub max_grant_lifetime_seconds: i64,
    pub max_installation_lifetime_seconds: i64,
    pub endpoint_quota_window_seconds: i64,
    pub endpoint_quota_max_deliveries: i64,
    /// Limits signed installation calls per attested key.
    pub signed_calls: Arc<SignedCallLimiter>,
    pub now: fn() -> i64,
    pub accepting: Arc<AtomicBool>,
}
fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}
fn valid_apns_endpoint(v: &str) -> bool {
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
/// Platform-neutral delivery route. Nothing about the device platform reaches
/// the relay, so deliveries do not name one.
pub(crate) const DELIVERY_PATH: &str = "/v1/deliveries";
/// The original APNs-named delivery route, kept as an alias of
/// [`DELIVERY_PATH`] until relays have moved to the platform-neutral route.
pub(crate) const APNS_DELIVERY_ALIAS_PATH: &str = "/v1/deliveries/apns";

// Keep preliminary decoding bounded to the pinned nostr verifier's limits.
//
// `route_path` is the delivery route the request arrived on. The signed URL
// path must equal it, so a request signed for one route is rejected on the other.
fn delivery_auth(header: &str, body: &[u8], route_path: &str) -> Option<(String, String)> {
    let (prefix, encoded) = header.split_once(' ')?;
    const MAX_EVENT_BYTES: usize = 64 * 1024;
    const MAX_ENCODED_BYTES: usize = MAX_EVENT_BYTES.div_ceil(3) * 4;
    if prefix != "Nostr" || encoded.len() > MAX_ENCODED_BYTES {
        return None;
    }
    let decoded = STANDARD.decode(encoded).ok()?;
    if decoded.len() > MAX_EVENT_BYTES {
        return None;
    }
    let event = Event::from_json(decoded).ok()?;
    let signed_url = match event.tags.find_standardized(TagKind::u())? {
        TagStandard::AbsoluteURL(url) => url,
        _ => return None,
    };
    if !matches!(signed_url.scheme(), "http" | "https")
        || signed_url.path() != route_path
        || signed_url.query().is_some()
        || signed_url.fragment().is_some()
        || !signed_url.username().is_empty()
        || signed_url.password().is_some()
    {
        return None;
    }
    // This delivery profile binds method and path, not request origin. Passing
    // the validated signed URL retains the library's kind, method, timestamp,
    // body hash, event ID and signature checks without reconstructing a URL.
    let relay = verify_auth_header(
        header,
        signed_url,
        HttpMethod::POST,
        Timestamp::now(),
        Some(body),
    )
    .ok()?;
    Some((event.id.to_hex(), relay.to_hex()))
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
        AuthorityError::Conflict => installation_conflict(),
        AuthorityError::RateLimited => error(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        AuthorityError::Unavailable | AuthorityError::InFlight => {
            error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}
fn installation_conflict() -> Response {
    error(StatusCode::CONFLICT, "installation_conflict")
}
/// FCM registration tokens are opaque ASCII, not hex.
fn valid_fcm_token(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_FCM_TOKEN_BYTES
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}
/// The token bytes sealed into custody, or `None` when `endpoint` is not a
/// valid push endpoint for `platform`.
fn endpoint_bytes(platform: Platform, endpoint: &str) -> Option<Vec<u8>> {
    match platform {
        Platform::Ios => valid_apns_endpoint(endpoint)
            .then(|| hex::decode(endpoint).ok())
            .flatten(),
        Platform::Android => valid_fcm_token(endpoint).then(|| endpoint.as_bytes().to_vec()),
    }
}
/// The endpoint string handed to the platform's transport.
fn transport_endpoint(platform: Platform, token: Vec<u8>) -> Option<String> {
    match platform {
        Platform::Ios => Some(hex::encode(token)),
        Platform::Android => String::from_utf8(token).ok(),
    }
}
fn endpoint_fingerprint(platform: Platform, token: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    // Separate namespaces keep a token from one platform from ever owning
    // another platform's endpoint.
    h.update(match platform {
        Platform::Ios => &b"buzz-apns-endpoint-v2\0"[..],
        Platform::Android => &b"buzz-fcm-endpoint-v1\0"[..],
    });
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
        created_at: now,
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
    audience: &'a str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
    key_id: &'a str,

    endpoint: &'a str,
    endpoint_epoch: i64,
    expires_at: i64,
}

/// Domain of the Android enrollment transcript, which the attestation
/// challenge binds.
const ANDROID_ENROLL_DOMAIN: &str = "buzz.push.enroll.android.v1";
/// How long a relay should wait before retrying a request whose earlier
/// attempt is still in flight. Long enough for a send that is minting or
/// waiting on the push service to finish, short enough to keep wakes prompt.
/// The relay backs off from this value; its schedule lives in the relay.
const IN_FLIGHT_RETRY_AFTER_SECONDS: i64 = 5;

/// The Android enrollment transcript: the iOS one without `key_id`, since the
/// key is generated after (and from) this transcript's digest.
#[derive(serde::Serialize)]
struct AndroidEnrollTranscript<'a> {
    v: u8,
    audience: &'a str,
    challenge_id: uuid::Uuid,
    challenge: &'a str,
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
    // The platform is derived from which attestation is present.
    let platform = match (&r.attestation, &r.android_attestation) {
        (Some(_), None) => Platform::Ios,
        (None, Some(_)) if s.application.android.is_some() => Platform::Android,
        _ => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let token = match endpoint_bytes(platform, &r.endpoint) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if r.v != WIRE_VERSION
        || r.endpoint_epoch != 1
        || r.expires_at <= now
        || r.expires_at > now.saturating_add(s.max_installation_lifetime_seconds)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let challenge = match decode_challenge(&r.challenge) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    // An Android key is generated with the attestation challenge as an input,
    // so its transcript holds only values known before the key exists, and
    // the gateway derives the key id from the attested key.
    let signed = match (platform, r.key_id.as_deref()) {
        (Platform::Ios, Some(key_id)) => transcript(
            "buzz.push.enroll.v1",
            &EnrollTranscript {
                v: r.v,
                audience: ENROLL_AUDIENCE,
                challenge_id: r.challenge_id,
                challenge: &r.challenge,
                key_id,

                endpoint: &r.endpoint,
                endpoint_epoch: r.endpoint_epoch,
                expires_at: r.expires_at,
            },
        ),
        (Platform::Android, None) => transcript(
            ANDROID_ENROLL_DOMAIN,
            &AndroidEnrollTranscript {
                v: r.v,
                audience: ENROLL_AUDIENCE,
                challenge_id: r.challenge_id,
                challenge: &r.challenge,
                endpoint: &r.endpoint,
                endpoint_epoch: r.endpoint_epoch,
                expires_at: r.expires_at,
            },
        ),
        _ => None,
    };
    let signed = match signed {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let verified = match verify_enrollment_attestation(&s, &r, signed.as_bytes()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let fingerprint = endpoint_fingerprint(platform, &token);
    match s
        .authority
        .matching_installation(
            &verified.key_id,
            fingerprint,
            r.endpoint_epoch,
            r.expires_at,
            now,
        )
        .await
    {
        Ok(Some(existing)) if existing.app_attest_public_key == verified.public_key => {
            return (
                StatusCode::CREATED,
                Json(InstallationEnrollResponse {
                    installation_handle: existing.id,
                    endpoint_epoch: existing.endpoint_epoch,
                    expires_at: existing.expires_at,
                }),
            )
                .into_response();
        }
        Ok(Some(_)) => return installation_conflict(),
        Ok(None) => {}
        Err(e) => return authority_error(e),
    }
    let ciphertext = match s.token_keyring.seal(&token) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    };
    let n = NewInstallation {
        id: uuid::Uuid::new_v4(),
        platform,
        app_attest_key_id: verified.key_id,
        app_attest_public_key: verified.public_key.clone(),
        assertion_counter: 0,

        token_ciphertext: ciphertext,
        token_fingerprint: fingerprint,
        endpoint_epoch: 1,
        expires_at: r.expires_at,
    };
    // One atomic step, so a replay racing this request either waits for its
    // installation or finds the challenge unspent, never a spent challenge
    // with nothing behind it.
    match s
        .authority
        .enroll_installation(r.challenge_id, challenge, n, now)
        .await
    {
        Ok(installation) if installation.app_attest_public_key == verified.public_key => (
            StatusCode::CREATED,
            Json(InstallationEnrollResponse {
                installation_handle: installation.id,
                endpoint_epoch: installation.endpoint_epoch,
                expires_at: installation.expires_at,
            }),
        )
            .into_response(),
        Ok(_) => installation_conflict(),
        Err(e) => authority_error(e),
    }
}

/// The attested key an enrollment proved, on either platform.
struct VerifiedKey {
    key_id: Vec<u8>,
    public_key: Vec<u8>,
}

/// Verify the enrollment's one attestation against the exact transcript.
/// A rejection answers 401 and never says why (rejections are logged). When
/// the gateway cannot judge an Android chain yet (no fresh revocation list,
/// as right after a restart), it answers a retryable 503 instead, so the
/// client keeps its key and replays the same request later.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
fn verify_enrollment_attestation(
    s: &AppState,
    r: &InstallationEnrollRequest,
    transcript: &[u8],
) -> Result<VerifiedKey, Response> {
    let rejected = || error(StatusCode::UNAUTHORIZED, "invalid_attestation");
    if let Some(attestation) = &r.attestation {
        let key_id = r.key_id.as_deref().ok_or_else(rejected)?;
        let verified = s
            .application
            .app_attest
            .verify_attestation(attestation, key_id, transcript)
            .map_err(|_| rejected())?;
        return Ok(VerifiedKey {
            key_id: verified.key_id,
            public_key: verified.public_key,
        });
    }
    let android = s.application.android.as_ref().ok_or_else(rejected)?;
    let chain = r
        .android_attestation
        .as_deref()
        .and_then(decode_android_chain)
        .ok_or_else(rejected)?;
    let proven = android
        .attestation
        .verify(&chain, transcript)
        .map_err(|e| match e {
            AndroidAttestationError::Unavailable => {
                error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
            }
            AndroidAttestationError::Invalid => rejected(),
        })?;
    use sha2::{Digest, Sha256};
    // The key must be a P-256 point. Its id is its digest; the attestation
    // challenge binds the key to this transcript.
    (proven.public_key.len() == 65
        && p256::ecdsa::VerifyingKey::from_sec1_bytes(&proven.public_key).is_ok())
    .then(|| VerifiedKey {
        key_id: Sha256::digest(&proven.public_key).to_vec(),
        public_key: proven.public_key,
    })
    .ok_or_else(rejected)
}

/// Decode a bounded Key Attestation chain of base64 DER certificates.
fn decode_android_chain(chain: &[String]) -> Option<Vec<Vec<u8>>> {
    if chain.is_empty() || chain.len() > MAX_ANDROID_CHAIN_CERTIFICATES {
        return None;
    }
    let mut total = 0usize;
    chain
        .iter()
        .map(|certificate| {
            // Bound before decoding so an oversized request cannot allocate.
            if certificate.len() > MAX_ANDROID_ATTESTATION_BYTES.div_ceil(3) * 4 {
                return None;
            }
            let der = STANDARD.decode(certificate).ok()?;
            total += der.len();
            (!der.is_empty() && total <= MAX_ANDROID_ATTESTATION_BYTES).then_some(der)
        })
        .collect()
}

struct AssertionChallenge<'a> {
    id: uuid::Uuid,
    text: &'a str,
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn load_installation(
    s: &AppState,
    installation_id: uuid::Uuid,
    include_revoked: bool,
) -> Result<Installation, Response> {
    let now = (s.now)();
    if include_revoked {
        s.authority
            .installation_for_revocation(installation_id, now)
            .await
    } else {
        s.authority.installation(installation_id, now).await
    }
    .map_err(authority_error)
}

/// Authenticate one installation call against the stored attested key. iOS
/// verifies an App Attest assertion and advances its counter. Android verifies
/// a Keystore ECDSA signature over the same transcript bytes; with no counter,
/// the single-use challenge consumed here is its replay protection.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn verify_installation_assertion<T: serde::Serialize>(
    s: &AppState,
    installation: &Installation,
    challenge: AssertionChallenge<'_>,
    assertion: &str,
    domain: &str,
    signed: &T,
) -> Result<(), Response> {
    let now = (s.now)();
    let challenge_bytes = decode_challenge(challenge.text)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let transcript = transcript(domain, signed)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let counter = match installation.platform {
        Platform::Ios => Some(
            s.application
                .app_attest
                .verify_assertion(
                    assertion,
                    transcript.as_bytes(),
                    &installation.app_attest_public_key,
                    installation.assertion_counter,
                    challenge.text,
                    challenge.text,
                )
                .map_err(|_| error(StatusCode::UNAUTHORIZED, "invalid_attestation"))?
                .counter,
        ),
        Platform::Android => {
            android_attestation::verify_signature(
                &installation.app_attest_public_key,
                transcript.as_bytes(),
                assertion,
            )
            .map_err(|_| error(StatusCode::UNAUTHORIZED, "invalid_attestation"))?;
            None
        }
    };
    // Counted only for authentic calls, so nobody can spend another
    // installation's allowance with forged requests.
    if !s.signed_calls.admit(&installation.app_attest_key_id, now) {
        return Err(error(StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    }
    s.authority
        .consume_challenge(challenge.id, challenge_bytes, now)
        .await
        .map_err(authority_error)?;
    match counter {
        Some(next) => s
            .authority
            .advance_assertion_counter(installation.id, installation.assertion_counter, next)
            .await
            .map_err(authority_error),
        None => Ok(()),
    }
}

#[derive(serde::Serialize)]
struct DelegateTranscript<'a> {
    v: u8,
    audience: &'a str,
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
        audience: DELEGATE_AUDIENCE,
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        generation: r.generation,
        relay_pubkey: &r.relay_pubkey,
        not_before: r.not_before,
        expires_at: r.expires_at,
    };
    let installation = match load_installation(&s, r.installation_handle, false).await {
        Ok(installation) => installation,
        Err(e) => return e,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        &installation,
        AssertionChallenge {
            id: r.challenge_id,
            text: &r.challenge,
        },
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
        endpoint_epoch: d.endpoint_epoch,
        generation: d.generation,
        expires_at: d.expires_at,
    };
    match s.grant_keyring.issue(&g) {
        Ok(endpoint_grant) => (
            StatusCode::CREATED,
            Json(DelegationResponse { endpoint_grant }),
        )
            .into_response(),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
    }
}

#[derive(serde::Serialize)]
struct RotateTranscript<'a> {
    v: u8,
    audience: &'a str,
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
    if r.v != WIRE_VERSION
        || r.endpoint_epoch < 1
        || r.new_endpoint_epoch != r.endpoint_epoch.saturating_add(1)
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let t = RotateTranscript {
        v: r.v,
        audience: ROTATE_ENDPOINT_AUDIENCE,
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        new_endpoint_epoch: r.new_endpoint_epoch,
        endpoint: &r.endpoint,
    };
    let installation = match load_installation(&s, r.installation_handle, false).await {
        Ok(installation) => installation,
        Err(e) => return e,
    };
    // The endpoint's shape depends on the installation's platform.
    let token = match endpoint_bytes(installation.platform, &r.endpoint) {
        Some(v) => v,
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        &installation,
        AssertionChallenge {
            id: r.challenge_id,
            text: &r.challenge,
        },
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
            endpoint_fingerprint(installation.platform, &token),
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
    audience: &'a str,
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
        audience: REVOKE_DELEGATION_AUDIENCE,
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        relay_pubkey: &r.relay_pubkey,
        generation: r.generation,
    };
    let installation = match load_installation(&s, r.installation_handle, false).await {
        Ok(installation) => installation,
        Err(e) => return e,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        &installation,
        AssertionChallenge {
            id: r.challenge_id,
            text: &r.challenge,
        },
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
    audience: &'a str,
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
        audience: REVOKE_INSTALLATION_AUDIENCE,
        challenge_id: r.challenge_id,
        challenge: &r.challenge,
        installation_handle: r.installation_handle,
        endpoint_epoch: r.endpoint_epoch,
        new_endpoint_epoch: r.new_endpoint_epoch,
    };
    let installation = match load_installation(&s, r.installation_handle, true).await {
        Ok(installation) => installation,
        Err(e) => return e,
    };
    if let Err(e) = verify_installation_assertion(
        &s,
        &installation,
        AssertionChallenge {
            id: r.challenge_id,
            text: &r.challenge,
        },
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

async fn deliver(
    State(s): State<AppState>,
    matched_path: MatchedPath,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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
    if uri.query().is_some() {
        return error(StatusCode::UNAUTHORIZED, "invalid_auth");
    }
    let (event_id, relay) = match delivery_auth(auth, &body, matched_path.as_str()) {
        Some(auth) => auth,
        None => return error(StatusCode::UNAUTHORIZED, "invalid_auth"),
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
        Err(AuthorityError::Conflict) => {
            crate::metrics::record_admission(crate::metrics::Admission::Rejected);
            crate::metrics::record_delivery_error("invalid_grant");
            return error(StatusCode::NOT_FOUND, "invalid_grant");
        }
        Err(AuthorityError::RateLimited) => {
            crate::metrics::record_admission(crate::metrics::Admission::Rejected);
            crate::metrics::record_delivery_error("rate_limited");
            return error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
        }
        Err(AuthorityError::Unavailable) => {
            crate::metrics::record_admission(crate::metrics::Admission::Unavailable);
            crate::metrics::record_delivery_error("temporarily_unavailable");
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
        // The relay gave up waiting on an earlier attempt of this request and
        // retried while that attempt is still sending. A 404 here would tell
        // the relay the wake was delivered; ask it to retry instead, after the
        // first attempt has had time to finish.
        Err(AuthorityError::InFlight) => {
            crate::metrics::record_admission(crate::metrics::Admission::InFlight);
            return delivery_outcome_response(
                DeliveryOutcome::Retry {
                    retry_after_seconds: Some(IN_FLIGHT_RETRY_AFTER_SECONDS),
                },
                grant.generation,
            );
        }
    };
    let platform = permit.authority.platform;
    // An Android installation can exist only on a gateway configured for
    // Android. If that configuration was removed, retry: do not invalidate.
    let transport = match platform {
        Platform::Ios => Some(Arc::clone(&s.application.transport)),
        Platform::Android => s
            .application
            .android
            .as_ref()
            .map(|a| Arc::clone(&a.transport)),
    };
    let endpoint = match s
        .token_keyring
        .open(&permit.authority.token_ciphertext)
        .ok()
        .and_then(|token| transport_endpoint(platform, token))
        .zip(transport)
    {
        Some(opened) => opened,
        None => {
            crate::metrics::record_delivery_error("token_custody");
            let _ = s
                .authority
                .finish_delivery(permit, DeliveryDisposition::Retryable)
                .await;
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    let (endpoint, transport) = endpoint;
    let attempt = DeliveryAttempt {
        request_id: r.request_id,
        expires_at: r.expires_at,
    };
    let authority_store = Arc::clone(&s.authority);
    // Admission already committed, so cancellation cannot undo either replay
    // fence. The detached task completes disposition bookkeeping.
    let delivery = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let outcome = transport.send(attempt, &endpoint).await;
        let seconds = started.elapsed().as_secs_f64();
        match platform {
            Platform::Ios => crate::metrics::record_apns_delivery(outcome, seconds),
            Platform::Android => crate::metrics::record_fcm_delivery(outcome, seconds),
        }
        let disposition = match outcome {
            DeliveryOutcome::Retry { .. } | DeliveryOutcome::ConfigurationFault => {
                DeliveryDisposition::Retryable
            }
            DeliveryOutcome::Accepted
            | DeliveryOutcome::InvalidEndpoint { .. }
            | DeliveryOutcome::PermanentRequestFault => DeliveryDisposition::Terminal,
        };
        let finished = authority_store.finish_delivery(permit, disposition).await;
        (outcome, disposition, finished)
    });
    let outcome = match delivery.await {
        Ok((outcome, _, Ok(()))) => outcome,
        // The provider has answered for good, so report that answer. The id
        // stays in flight until its request expires, which only makes a
        // replay retry later; it can never cause a second send.
        Ok((outcome, DeliveryDisposition::Terminal, Err(err))) => {
            crate::metrics::record_delivery_error("finish_failed");
            tracing::warn!(error = %err, "failed to mark a terminal delivery finished");
            outcome
        }
        _ => {
            crate::metrics::record_delivery_error("finish_failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    delivery_outcome_response(outcome, grant.generation)
}

fn delivery_outcome_response(outcome: DeliveryOutcome, generation: i64) -> Response {
    match outcome {
        DeliveryOutcome::Accepted => {
            (StatusCode::OK, Json(DeliveryResponse::Accepted)).into_response()
        }
        DeliveryOutcome::InvalidEndpoint { unregistered_at } => (
            StatusCode::GONE,
            Json(DeliveryResponse::InvalidEndpoint {
                generation,
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
    let enrollment = Router::new()
        .route("/v1/installations", post(enroll))
        .layer(RequestBodyLimitLayer::new(MAX_ENROLL_REQUEST_BYTES));
    let standard_requests = Router::new()
        .route("/v1/installations/challenges", post(challenge))
        .route("/v1/delegations", post(delegate))
        .route("/v1/delegations/revoke", post(revoke_delegation))
        .route("/v1/installations/endpoint", post(rotate_endpoint))
        .route("/v1/installations/revoke", post(revoke_installation))
        .route(DELIVERY_PATH, post(deliver))
        .route(APNS_DELIVERY_ALIAS_PATH, post(deliver))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES));
    let public = Router::new()
        .merge(enrollment)
        .merge(standard_requests)
        .with_state(state.clone())
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

#[cfg(test)]
mod request_limit_tests {
    use super::*;
    use crate::{
        authority::MemoryAuthorityStore,
        grant::{GrantKey, GrantKeyring},
        token::{TokenKey, TokenKeyring},
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    struct NeverTransport;

    #[async_trait::async_trait]
    impl PushTransport for NeverTransport {
        async fn send(&self, _: DeliveryAttempt, _: &str) -> DeliveryOutcome {
            panic!("request-size tests never send to APNs")
        }
    }

    fn fixed_now() -> i64 {
        1_750_000_000
    }

    fn state() -> AppState {
        let app_attest = AppAttestVerifier::new(
            "TEAMID.com.example.buzz".to_owned(),
            include_bytes!("../tests/fixtures/apple-app-attestation-root.pem").to_vec(),
        )
        .expect("pinned Apple root fixture");
        AppState {
            grant_keyring: Arc::new(
                GrantKeyring::new(vec![GrantKey::new("test", &[1; 32]).unwrap()]).unwrap(),
            ),
            authority: Arc::new(MemoryAuthorityStore::default()),
            token_keyring: Arc::new(
                TokenKeyring::new(vec![TokenKey::new("test", &[2; 32]).unwrap()]).unwrap(),
            ),
            application: Arc::new(ApplicationRuntime {
                app_attest: Arc::new(app_attest),
                transport: Arc::new(NeverTransport),
                android: None,
            }),
            max_grant_lifetime_seconds: 86_400,
            max_installation_lifetime_seconds: 86_400,
            endpoint_quota_window_seconds: 60,
            endpoint_quota_max_deliveries: 10,
            signed_calls: Arc::new(crate::signed_call_limiter::SignedCallLimiter::new(
                60, 1_000,
            )),
            now: fixed_now,
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }

    fn maximum_enrollment_body() -> Vec<u8> {
        serde_json::to_vec(&InstallationEnrollRequest {
            v: WIRE_VERSION,
            challenge_id: uuid::Uuid::nil(),
            challenge: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0; 32]),
            key_id: Some(STANDARD.encode([0; 32])),
            attestation: Some(STANDARD.encode(vec![0; MAX_APP_ATTESTATION_BYTES])),
            android_attestation: None,

            endpoint: "ab".repeat(MAX_ENDPOINT_HEX_BYTES),
            endpoint_epoch: 1,
            expires_at: fixed_now() + 60,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn maximum_valid_enrollment_envelope_reaches_the_handler() {
        let body = maximum_enrollment_body();
        assert_eq!(MAX_ENROLL_REQUEST_BYTES, 23_960);
        assert!(body.len() > MAX_REQUEST_BYTES);
        assert!(body.len() <= MAX_ENROLL_REQUEST_BYTES);
        let (public, _) = router(state());
        let response = public
            .oneshot(
                Request::post("/v1/installations")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn enrollment_envelope_stays_bounded() {
        let (public, _) = router(state());
        let response = public
            .oneshot(
                Request::post("/v1/installations")
                    .header("content-type", "application/json")
                    .body(Body::from(vec![b' '; MAX_ENROLL_REQUEST_BYTES + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn live_installation_conflict_has_an_unambiguous_status() {
        assert_eq!(
            authority_error(AuthorityError::Conflict).status(),
            StatusCode::CONFLICT
        );
        assert_eq!(installation_conflict().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn ambiguous_apns_profile_failures_remain_retryable_at_the_relay_boundary() {
        for reason in ["BadDeviceToken", "DeviceTokenNotForTopic"] {
            let outcome = crate::apns::classify(400, Some(reason), None);
            assert_eq!(outcome, DeliveryOutcome::ConfigurationFault);
            assert_eq!(
                delivery_outcome_response(outcome, 7).status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }

        let outcome = crate::apns::classify(410, Some("Unregistered"), Some(42));
        assert_eq!(
            delivery_outcome_response(outcome, 7).status(),
            StatusCode::GONE
        );
    }
}

/// Known-answer vectors for the exact App Attest transcript bytes defined by
/// NIP-PL ("Exact App Attest transcript construction"). The fixture file is
/// shared ground truth with client-side canonical encoders (the Swift NIP-PL
/// iOS client): a client encoder that fails to reproduce these bytes exactly
/// fails every enroll/delegate/rotate/revoke call with `invalid_attestation`.
#[cfg(test)]
mod transcript_vector_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const VECTORS_JSON: &str = include_str!("../tests/vectors/app_attest_transcripts.json");

    // Deterministic fixture inputs mirrored in the vector file's `inputs`.
    const CHALLENGE_ID: uuid::Uuid =
        uuid::Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);
    const INSTALLATION: uuid::Uuid =
        uuid::Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);
    // base64url-no-pad of bytes 0x00..=0x1f.
    const CHALLENGE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    // Standard base64 (padded) of 32 bytes of 0xAA.
    const KEY_ID: &str = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=";
    // 32-byte APNs token, lowercase hex.
    const ENDPOINT: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    const RELAY_PUBKEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn assert_vector(name: &str, actual: &str) {
        let file: serde_json::Value = serde_json::from_str(VECTORS_JSON).unwrap();
        let vector = file["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap_or_else(|| panic!("vector {name} missing from fixture"));
        assert_eq!(
            actual,
            vector["transcript"].as_str().unwrap(),
            "{name} bytes"
        );
        assert_eq!(
            hex::encode(Sha256::digest(actual.as_bytes())),
            vector["sha256"].as_str().unwrap(),
            "{name} sha256"
        );
    }

    #[test]
    fn fixture_encodings_match_their_raw_bytes() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let challenge_bytes: Vec<u8> = (0u8..32).collect();
        assert_eq!(URL_SAFE_NO_PAD.encode(&challenge_bytes), CHALLENGE);
        assert_eq!(STANDARD.encode([0xAAu8; 32]), KEY_ID);
        assert_eq!(hex::decode(ENDPOINT).unwrap().len(), 32);
    }

    #[test]
    fn enroll_transcript_vector() {
        let t = EnrollTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/installations",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            key_id: KEY_ID,

            endpoint: ENDPOINT,
            endpoint_epoch: 1,
            expires_at: 1_752_624_000,
        };
        assert_vector("enroll", &transcript("buzz.push.enroll.v1", &t).unwrap());
    }

    #[test]
    fn android_enroll_transcript_vector() {
        let t = AndroidEnrollTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/installations",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            endpoint: ENDPOINT,
            endpoint_epoch: 1,
            expires_at: 1_752_624_000,
        };
        assert_vector(
            "enroll_android",
            &transcript(ANDROID_ENROLL_DOMAIN, &t).unwrap(),
        );
    }

    #[test]
    fn delegate_transcript_vector() {
        let t = DelegateTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/delegations",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            installation_handle: INSTALLATION,
            endpoint_epoch: 1,
            generation: 1,
            relay_pubkey: RELAY_PUBKEY,
            not_before: 1_752_620_000,
            expires_at: 1_752_624_000,
        };
        assert_vector(
            "delegate",
            &transcript("buzz.push.delegate.v1", &t).unwrap(),
        );
    }

    #[test]
    fn rotate_endpoint_transcript_vector() {
        let t = RotateTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/installations/endpoint",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            installation_handle: INSTALLATION,
            endpoint_epoch: 1,
            new_endpoint_epoch: 2,
            endpoint: ENDPOINT,
        };
        assert_vector(
            "rotate_endpoint",
            &transcript("buzz.push.rotate-endpoint.v1", &t).unwrap(),
        );
    }

    #[test]
    fn revoke_delegation_transcript_vector() {
        let t = RevokeDelegationTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/delegations/revoke",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            installation_handle: INSTALLATION,
            relay_pubkey: RELAY_PUBKEY,
            generation: 2,
        };
        assert_vector(
            "revoke_delegation",
            &transcript("buzz.push.revoke-delegation.v1", &t).unwrap(),
        );
    }

    #[test]
    fn revoke_installation_transcript_vector() {
        let t = RevokeInstallationTranscript {
            v: 1,
            audience: "https://push.buzz.xyz/v1/installations/revoke",
            challenge_id: CHALLENGE_ID,
            challenge: CHALLENGE,
            installation_handle: INSTALLATION,
            endpoint_epoch: 1,
            new_endpoint_epoch: 2,
        };
        assert_vector(
            "revoke_installation",
            &transcript("buzz.push.revoke-installation.v1", &t).unwrap(),
        );
    }
}

#[cfg(test)]
#[path = "http_android_tests.rs"]
mod android_tests;
