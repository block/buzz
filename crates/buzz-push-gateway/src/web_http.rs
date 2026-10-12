//! Browser/PWA Web Push enrollment API (`buzz-web` profile).
//!
//! Ownership is proven with NIP-98 (the member's own signer signs each
//! request); the gateway never sees a private key. A web installation reuses
//! the installation/delegation/grant authority of the APNs path, so relay
//! delivery, replay fences, quotas, revocation and reaping are shared.
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use nostr::{
    nips::nip98::{verify_auth_header, HttpMethod},
    Timestamp,
};
use sha2::{Digest, Sha256};

use crate::{
    authority::{AuthorityError, Delegation, NewInstallation},
    http::AppState,
    model::*,
    webpush::{self, Vapid, WebPushTransport},
};

/// Web profile runtime: VAPID identity, transport and the public origin the
/// NIP-98 `u` tag must name.
pub struct WebRuntime {
    pub vapid_public_key: String,
    pub transport: Arc<dyn WebPushTransport>,
    /// Exact external base URL of this gateway, e.g. `https://push.hulabuzz.huladesk.com`.
    pub public_base_url: String,
    /// Browser origin allowed by CORS, e.g. `https://hulabuzz.huladesk.com`.
    pub allowed_origin: String,
}

impl WebRuntime {
    pub fn new(
        vapid: &Vapid,
        transport: Arc<dyn WebPushTransport>,
        public_base_url: String,
        allowed_origin: String,
    ) -> Self {
        Self::new_with_public(
            vapid.public_key_b64url().to_owned(),
            transport,
            public_base_url,
            allowed_origin,
        )
    }
    pub fn new_with_public(
        vapid_public_key: String,
        transport: Arc<dyn WebPushTransport>,
        public_base_url: String,
        allowed_origin: String,
    ) -> Self {
        Self {
            vapid_public_key,
            transport,
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            allowed_origin,
        }
    }
}

fn err(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}

fn with_cors(web: &WebRuntime, mut r: Response) -> Response {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&web.allowed_origin) {
        h.insert("access-control-allow-origin", v);
    }
    h.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("authorization, content-type"),
    );
    h.insert("access-control-max-age", HeaderValue::from_static("600"));
    h.insert("vary", HeaderValue::from_static("origin"));
    r
}

/// Installation identity for a member pubkey + endpoint (unique per device).
fn web_key_id(pubkey: &str, endpoint: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"buzz-web-installation-v1\0");
    h.update(pubkey.as_bytes());
    h.update([0]);
    h.update(endpoint.as_bytes());
    h.finalize().to_vec()
}

/// Stored "public key" for a web installation: SEC1 compressed-style prefix + x-only member pubkey.
fn owner_bytes(pubkey_hex: &str) -> Option<Vec<u8>> {
    let raw = hex::decode(pubkey_hex).ok()?;
    (raw.len() == 32).then(|| [vec![0x02], raw].concat())
}

fn fingerprint(sub_bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"buzz-web-endpoint-v1\0");
    h.update(AppProfile::BuzzWeb.as_str().as_bytes());
    h.update([0]);
    h.update(sub_bytes);
    h.finalize().into()
}

fn nip98_member(web: &WebRuntime, headers: &HeaderMap, path: &str, body: &[u8]) -> Option<String> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    if header.len() > 16 * 1024 {
        return None;
    }
    let url = url::Url::parse(&format!("{}{path}", web.public_base_url)).ok()?;
    // Library checks kind 27235, exact u, method, ±60s timestamp, payload hash and signature.
    verify_auth_header(header, &url, HttpMethod::POST, Timestamp::now(), Some(body))
        .ok()
        .map(|pk| pk.to_hex())
}

fn authority_err(e: AuthorityError) -> Response {
    match e {
        AuthorityError::Rejected => err(StatusCode::NOT_FOUND, "not_authorized"),
        AuthorityError::Conflict => err(StatusCode::CONFLICT, "installation_conflict"),
        AuthorityError::RateLimited => err(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        AuthorityError::Unavailable => {
            err(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}

pub async fn web_options(State(s): State<AppState>) -> Response {
    match &s.web {
        Some(web) => with_cors(web, StatusCode::NO_CONTENT.into_response()),
        None => err(StatusCode::NOT_FOUND, "not_found"),
    }
}

pub async fn vapid_public_key(State(s): State<AppState>) -> Response {
    match &s.web {
        Some(web) => with_cors(
            web,
            Json(serde_json::json!({ "key": web.vapid_public_key })).into_response(),
        ),
        None => err(StatusCode::NOT_FOUND, "not_found"),
    }
}

pub async fn web_enroll(
    State(s): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(web) = s.web.clone() else {
        return err(StatusCode::NOT_FOUND, "not_found");
    };
    with_cors(
        &web,
        enroll_inner(&s, &web, uri.path(), &headers, &body).await,
    )
}

async fn enroll_inner(
    s: &AppState,
    web: &WebRuntime,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let Some(member) = nip98_member(web, headers, path, body) else {
        return err(StatusCode::UNAUTHORIZED, "invalid_auth");
    };
    let r: WebInstallationRequest = match crate::strict_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return err(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let now = (s.now)();
    if r.v != WIRE_VERSION
        || r.expires_at <= now
        || r.expires_at > now.saturating_add(s.max_installation_lifetime_seconds)
    {
        return err(StatusCode::BAD_REQUEST, "invalid_request");
    }
    if webpush::validate_subscription(&r.subscription).is_err() {
        return err(StatusCode::BAD_REQUEST, "unsupported_subscription");
    }
    let sub_bytes = webpush::subscription_bytes(&r.subscription);
    if sub_bytes.len() > webpush::MAX_SUBSCRIPTION_JSON_BYTES {
        return err(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let Some(owner) = owner_bytes(&member) else {
        return err(StatusCode::UNAUTHORIZED, "invalid_auth");
    };
    let key_id = web_key_id(&member, &r.subscription.endpoint);
    let fp = fingerprint(&sub_bytes);
    match s
        .authority
        .matching_installation(&key_id, AppProfile::BuzzWeb, fp, 1, r.expires_at, now)
        .await
    {
        Ok(Some(existing)) if existing.app_attest_public_key == owner => {
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
        Ok(Some(_)) => return err(StatusCode::CONFLICT, "installation_conflict"),
        Ok(None) => {}
        Err(e) => return authority_err(e),
    }
    let Ok(ciphertext) = s.token_keyring.seal(&sub_bytes) else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
    };
    let id = uuid::Uuid::new_v4();
    let n = NewInstallation {
        id,
        app_attest_key_id: key_id,
        app_attest_public_key: owner,
        assertion_counter: 0,
        profile: AppProfile::BuzzWeb,
        token_ciphertext: ciphertext,
        token_fingerprint: fp,
        endpoint_epoch: 1,
        expires_at: r.expires_at,
    };
    if let Err(e) = s.authority.create_installation(n, now).await {
        return authority_err(e);
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

async fn owned_web_installation(
    s: &AppState,
    member: &str,
    id: uuid::Uuid,
) -> Result<crate::authority::Installation, Response> {
    let i = s
        .authority
        .installation(id, (s.now)())
        .await
        .map_err(authority_err)?;
    if i.profile != AppProfile::BuzzWeb
        || Some(&i.app_attest_public_key) != owner_bytes(member).as_ref()
    {
        return Err(err(StatusCode::NOT_FOUND, "not_authorized"));
    }
    Ok(i)
}

pub async fn web_delegate(
    State(s): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(web) = s.web.clone() else {
        return err(StatusCode::NOT_FOUND, "not_found");
    };
    let resp = async {
        let Some(member) = nip98_member(&web, &headers, uri.path(), &body) else {
            return err(StatusCode::UNAUTHORIZED, "invalid_auth");
        };
        let r: WebDelegationRequest = match crate::strict_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return err(StatusCode::BAD_REQUEST, "invalid_request"),
        };
        let now = (s.now)();
        let relay_ok = r.relay_pubkey.len() == 64
            && r.relay_pubkey
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if r.v != WIRE_VERSION
            || !relay_ok
            || r.generation < 1
            || r.not_before > now + 300
            || r.expires_at <= r.not_before
            || r.expires_at > now + s.max_grant_lifetime_seconds
        {
            return err(StatusCode::BAD_REQUEST, "invalid_request");
        }
        let i = match owned_web_installation(&s, &member, r.installation_handle).await {
            Ok(i) => i,
            Err(r) => return r,
        };
        let d = Delegation {
            id: uuid::Uuid::new_v4(),
            installation_id: i.id,
            relay_pubkey: r.relay_pubkey.clone(),
            endpoint_epoch: i.endpoint_epoch,
            generation: r.generation,
            not_before: r.not_before,
            expires_at: r.expires_at.min(i.expires_at),
            revoked: false,
        };
        if let Err(e) = s.authority.upsert_delegation(d.clone()).await {
            return authority_err(e);
        }
        let g = EndpointGrant {
            v: WIRE_VERSION,
            delegation_id: d.id,
            relay_pubkey: d.relay_pubkey,
            app_profile: AppProfile::BuzzWeb,
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
            Err(_) => err(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
        }
    }
    .await;
    with_cors(&web, resp)
}

pub async fn web_revoke(
    State(s): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(web) = s.web.clone() else {
        return err(StatusCode::NOT_FOUND, "not_found");
    };
    let resp = async {
        let Some(member) = nip98_member(&web, &headers, uri.path(), &body) else {
            return err(StatusCode::UNAUTHORIZED, "invalid_auth");
        };
        let r: WebRevokeRequest = match crate::strict_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return err(StatusCode::BAD_REQUEST, "invalid_request"),
        };
        if r.v != WIRE_VERSION {
            return err(StatusCode::BAD_REQUEST, "invalid_request");
        }
        let i = match owned_web_installation(&s, &member, r.installation_handle).await {
            Ok(i) => i,
            Err(r) => return r,
        };
        match s
            .authority
            .revoke_installation(i.id, i.endpoint_epoch, i.endpoint_epoch + 1)
            .await
        {
            Ok(()) => Json(MutationResponse { status: "revoked" }).into_response(),
            Err(e) => authority_err(e),
        }
    }
    .await;
    with_cors(&web, resp)
}
