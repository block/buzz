//! Private management API called by relays over NIP-98 signed HTTP.
//!
//! ```text
//! relay ──PUT /v1/integrations/{id}──▶ authenticate (NIP-98, relay allowlist)
//!        Authorization: Nostr …              │
//!                                            ▼
//!                               Store::apply(operation_id, revision, mutation)
//!                                 ├─ Applied  ──▶ 200 integration view
//!                                 ├─ Replayed ──▶ 200 stored response
//!                                 ├─ Stale    ──▶ 409 { current }
//!                                 └─ NotFound ──▶ 404
//! ```
//!
//! Every mutation carries a caller-chosen `operation_id` and the integration
//! `revision` it produces, so a relay can retry blindly: a replay returns the
//! original response and an out-of-date relay learns the current revision.
use crate::http::{error, AppState};
use crate::secrets::MAX_SECRET_BYTES;
use crate::store::{ApplyOutcome, IntegrationView, Mutation, Operation, Provider, StoreError};
use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{header::AUTHORIZATION, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post, put},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Duration;
use nostr::{
    nips::nip98::{verify_auth_header, HttpMethod},
    Event, JsonUtil, PublicKey, TagKind, TagStandard, Timestamp,
};
use serde::Deserialize;
use uuid::Uuid;

/// Upper bound on a management request body; integration descriptors and
/// provider secrets are small, so anything larger is a malformed client.
pub const MAX_MANAGEMENT_BODY_BYTES: usize = 64 * 1024;
const MAX_COMMUNITY_HOST_BYTES: usize = 253;
const MAX_GRACE_SECONDS: u64 = 30 * 24 * 3600;

/// Management routes, mounted on the private router only.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/integrations/{id}",
            put(put_integration).get(get_integration),
        )
        .route("/v1/integrations/{id}/secret", put(put_secret))
        .route("/v1/integrations/{id}/disable", post(disable))
        .route("/v1/integrations/{id}/enable", post(enable))
        .route("/v1/operations/{operation_id}", get(get_operation))
}

/// Verify a NIP-98 header against the request method, path, and body, and
/// require the signer to be a configured relay. The signed URL's origin is not
/// bound: the gateway sits behind a proxy and relays address it by service
/// name, so method, path, and body hash are the authenticated facts.
fn relay_auth(
    header: &str,
    method: HttpMethod,
    path: &str,
    body: Option<&[u8]>,
    relays: &[PublicKey],
) -> Option<PublicKey> {
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
        || signed_url.path() != path
        || signed_url.query().is_some()
        || signed_url.fragment().is_some()
        || !signed_url.username().is_empty()
        || signed_url.password().is_some()
    {
        return None;
    }
    let signer = verify_auth_header(header, signed_url, method, Timestamp::now(), body).ok()?;
    relays.contains(&signer).then_some(signer)
}

fn http_method(method: &Method) -> Option<HttpMethod> {
    match *method {
        Method::GET => Some(HttpMethod::GET),
        Method::POST => Some(HttpMethod::POST),
        Method::PUT => Some(HttpMethod::PUT),
        Method::PATCH => Some(HttpMethod::PATCH),
        _ => None,
    }
}

fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    method: &Method,
    uri: &OriginalUri,
    body: Option<&[u8]>,
) -> Result<PublicKey, Reject> {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(Reject::Unauthorized)?;
    let method = http_method(method).ok_or(Reject::Unauthorized)?;
    relay_auth(header, method, uri.path(), body, &state.relay_pubkeys).ok_or(Reject::Unauthorized)
}

#[derive(Debug, Clone, Copy)]
enum Reject {
    Unauthorized,
    Invalid,
}

impl IntoResponse for Reject {
    fn into_response(self) -> Response {
        match self {
            Reject::Unauthorized => error(StatusCode::UNAUTHORIZED, "unauthorized"),
            Reject::Invalid => invalid(),
        }
    }
}

fn store_error(e: StoreError) -> Response {
    tracing::warn!(error = %e, "management store call failed");
    error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
}

fn invalid() -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request")
}

fn parse_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Reject> {
    serde_json::from_slice(body).map_err(|_| Reject::Invalid)
}

fn valid_community_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_COMMUNITY_HOST_BYTES
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && !host.contains("..")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationEnvelope {
    operation_id: Uuid,
    revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutIntegrationBody {
    operation_id: Uuid,
    revision: i64,
    community_host: String,
    provider: Provider,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutSecretBody {
    operation_id: Uuid,
    revision: i64,
    secret: String,
    grace_seconds: u64,
}

async fn apply(state: &AppState, operation: Operation) -> Response {
    if operation.revision <= 0 {
        return invalid();
    }
    let integration_id = operation.integration_id;
    match state.store.apply(operation, (state.now)()).await {
        Ok(ApplyOutcome::Applied(applied)) => {
            (StatusCode::OK, Json(applied.response)).into_response()
        }
        Ok(ApplyOutcome::Replayed(applied)) if applied.integration_id == integration_id => {
            (StatusCode::OK, Json(applied.response)).into_response()
        }
        Ok(ApplyOutcome::Replayed(_)) => error(StatusCode::CONFLICT, "operation_conflict"),
        Ok(ApplyOutcome::StaleRevision { current }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "stale_revision", "current": current })),
        )
            .into_response(),
        Ok(ApplyOutcome::NotFound) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => store_error(e),
    }
}

async fn put_integration(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, Some(&body)) {
        return reject.into_response();
    }
    let request: PutIntegrationBody = match parse_body(&body) {
        Ok(request) => request,
        Err(reject) => return reject.into_response(),
    };
    if !valid_community_host(&request.community_host) {
        return invalid();
    }
    apply(
        &state,
        Operation {
            operation_id: request.operation_id,
            integration_id: id,
            revision: request.revision,
            mutation: Mutation::PutIntegration {
                community_host: request.community_host,
                provider: request.provider,
            },
        },
    )
    .await
}

async fn put_secret(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, Some(&body)) {
        return reject.into_response();
    }
    let request: PutSecretBody = match parse_body(&body) {
        Ok(request) => request,
        Err(reject) => return reject.into_response(),
    };
    if request.secret.is_empty()
        || request.secret.len() > MAX_SECRET_BYTES
        || request.grace_seconds > MAX_GRACE_SECONDS
    {
        return invalid();
    }
    let ciphertext = match state.secrets.seal(request.secret.as_bytes()) {
        Ok(ciphertext) => ciphertext,
        Err(_) => return invalid(),
    };
    apply(
        &state,
        Operation {
            operation_id: request.operation_id,
            integration_id: id,
            revision: request.revision,
            mutation: Mutation::PutSecret {
                key_id: state.secrets.current_id().to_owned(),
                ciphertext,
                grace: Duration::seconds(request.grace_seconds as i64),
            },
        },
    )
    .await
}

async fn toggle(
    state: AppState,
    id: Uuid,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
    mutation: Mutation,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, Some(&body)) {
        return reject.into_response();
    }
    let request: OperationEnvelope = match parse_body(&body) {
        Ok(request) => request,
        Err(reject) => return reject.into_response(),
    };
    apply(
        &state,
        Operation {
            operation_id: request.operation_id,
            integration_id: id,
            revision: request.revision,
            mutation,
        },
    )
    .await
}

async fn disable(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    toggle(state, id, method, uri, headers, body, Mutation::Disable).await
}

async fn enable(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    toggle(state, id, method, uri, headers, body, Mutation::Enable).await
}

async fn get_integration(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, None) {
        return reject.into_response();
    }
    let now = (state.now)();
    let integration = match state.store.integration(id).await {
        Ok(Some(integration)) => integration,
        Ok(None) => return error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => return store_error(e),
    };
    let secrets = match state.store.verifying_secrets(id, now).await {
        Ok(secrets) => secrets,
        Err(e) => return store_error(e),
    };
    Json(IntegrationView::new(&integration, &secrets, now)).into_response()
}

async fn get_operation(
    State(state): State<AppState>,
    Path(operation_id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, None) {
        return reject.into_response();
    }
    match state.store.applied_operation(operation_id).await {
        Ok(Some(applied)) => Json(serde_json::json!({
            "operation_id": applied.operation_id,
            "integration_id": applied.integration_id,
            "revision": applied.revision,
            "applied_at": applied.applied_at,
            "response": applied.response,
        }))
        .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => store_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{SecretKey, SecretKeyring};
    use crate::store::{MemoryStore, Store};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use std::sync::{atomic::AtomicBool, Arc};
    use tower::ServiceExt;

    struct Fixture {
        private: Router,
        relay: Keys,
        store: Arc<MemoryStore>,
        secrets: Arc<SecretKeyring>,
    }

    fn fixture() -> Fixture {
        let relay = Keys::generate();
        let store = Arc::new(MemoryStore::default());
        let secrets =
            Arc::new(SecretKeyring::new(vec![SecretKey::new("test", &[7; 32]).unwrap()]).unwrap());
        let state = AppState {
            store: store.clone(),
            secrets: secrets.clone(),
            relay_pubkeys: Arc::new(vec![relay.public_key()]),
            now: chrono::Utc::now,
            accepting: Arc::new(AtomicBool::new(true)),
        };
        let (_, private) = crate::http::router(state);
        Fixture {
            private,
            relay,
            store,
            secrets,
        }
    }

    fn signed_header(keys: &Keys, method: &str, url: &str, body: Option<&[u8]>) -> String {
        let mut tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ];
        if let Some(body) = body {
            tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap());
        }
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        format!("Nostr {}", STANDARD.encode(event.as_json()))
    }

    async fn call(
        fx: &Fixture,
        keys: &Keys,
        method: &str,
        path: &str,
        signed_path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
        let auth = signed_header(
            keys,
            method,
            &format!("https://ingress.example.org{signed_path}"),
            bytes.as_deref(),
        );
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", auth);
        if bytes.is_some() {
            request = request.header("content-type", "application/json");
        }
        let request = request.body(Body::from(bytes.unwrap_or_default())).unwrap();
        let response = fx.private.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, json)
    }

    fn put_body(operation_id: Uuid, revision: i64) -> serde_json::Value {
        serde_json::json!({
            "operation_id": operation_id,
            "revision": revision,
            "community_host": "acme.communities.example.org",
            "provider": "github",
        })
    }

    #[tokio::test]
    async fn relay_creates_reads_and_idempotently_retries_an_integration() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let path = format!("/v1/integrations/{id}");
        let op = Uuid::new_v4();

        let (status, body) = call(&fx, &fx.relay, "PUT", &path, &path, Some(put_body(op, 1))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["revision"], 1);
        assert_eq!(body["provider"], "github");
        assert_eq!(body["enabled"], true);
        assert_eq!(body["has_secret"], false);

        let (status, replay) =
            call(&fx, &fx.relay, "PUT", &path, &path, Some(put_body(op, 1))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replay, body);

        let (status, stale) = call(
            &fx,
            &fx.relay,
            "PUT",
            &path,
            &path,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            stale,
            serde_json::json!({ "error": "stale_revision", "current": 1 })
        );

        let (status, read) = call(&fx, &fx.relay, "GET", &path, &path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(read["community_host"], "acme.communities.example.org");

        let op_path = format!("/v1/operations/{op}");
        let (status, recorded) = call(&fx, &fx.relay, "GET", &op_path, &op_path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(recorded["integration_id"], serde_json::json!(id));
        assert_eq!(recorded["response"], body);

        let other = format!("/v1/integrations/{}", Uuid::new_v4());
        let (status, conflict) =
            call(&fx, &fx.relay, "PUT", &other, &other, Some(put_body(op, 1))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
        assert_eq!(conflict["error"], "operation_conflict");
    }

    #[tokio::test]
    async fn secrets_are_sealed_with_the_keyring_and_rotated_with_grace() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let path = format!("/v1/integrations/{id}");
        call(
            &fx,
            &fx.relay,
            "PUT",
            &path,
            &path,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;

        let secret_path = format!("{path}/secret");
        let first = serde_json::json!({
            "operation_id": Uuid::new_v4(),
            "revision": 2,
            "secret": "whsec_first",
            "grace_seconds": 0,
        });
        let (status, body) = call(
            &fx,
            &fx.relay,
            "PUT",
            &secret_path,
            &secret_path,
            Some(first),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["has_secret"], true);
        assert_eq!(body["rotating_until"], serde_json::Value::Null);

        let second = serde_json::json!({
            "operation_id": Uuid::new_v4(),
            "revision": 3,
            "secret": "whsec_second",
            "grace_seconds": 600,
        });
        let (status, body) = call(
            &fx,
            &fx.relay,
            "PUT",
            &secret_path,
            &secret_path,
            Some(second),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["rotating_until"].is_string());

        let secrets = fx
            .store
            .verifying_secrets(id, chrono::Utc::now())
            .await
            .unwrap();
        let plaintexts: Vec<Vec<u8>> = secrets
            .iter()
            .map(|s| fx.secrets.open(&s.ciphertext).unwrap())
            .collect();
        assert_eq!(
            plaintexts,
            vec![b"whsec_second".to_vec(), b"whsec_first".to_vec()]
        );
        assert!(secrets.iter().all(|s| s.key_id == "test"));
        assert!(secrets
            .iter()
            .all(|s| !s.ciphertext.windows(5).any(|w| w == b"whsec")));

        let (status, body) = call(&fx, &fx.relay, "GET", &path, &path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 3);
        assert_eq!(body["has_secret"], true);
    }

    #[tokio::test]
    async fn disable_and_enable_flip_the_flag_under_revision_control() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let path = format!("/v1/integrations/{id}");
        call(
            &fx,
            &fx.relay,
            "PUT",
            &path,
            &path,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;
        let envelope = |revision: i64| serde_json::json!({ "operation_id": Uuid::new_v4(), "revision": revision });

        let disable = format!("{path}/disable");
        let (status, body) = call(
            &fx,
            &fx.relay,
            "POST",
            &disable,
            &disable,
            Some(envelope(2)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["enabled"], false);

        let enable = format!("{path}/enable");
        let (status, body) =
            call(&fx, &fx.relay, "POST", &enable, &enable, Some(envelope(2))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let (status, body) =
            call(&fx, &fx.relay, "POST", &enable, &enable, Some(envelope(3))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["enabled"], true);

        let missing = format!("/v1/integrations/{}/disable", Uuid::new_v4());
        let (status, _) = call(
            &fx,
            &fx.relay,
            "POST",
            &missing,
            &missing,
            Some(envelope(1)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unauthorized_callers_and_mismatched_signatures_are_rejected() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let path = format!("/v1/integrations/{id}");
        let body = put_body(Uuid::new_v4(), 1);

        let stranger = Keys::generate();
        let (status, _) = call(&fx, &stranger, "PUT", &path, &path, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let other = format!("/v1/integrations/{}", Uuid::new_v4());
        let (status, _) = call(&fx, &fx.relay, "PUT", &path, &other, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let signed_for_other_body = signed_header(
            &fx.relay,
            "PUT",
            &format!("https://ingress.example.org{path}"),
            Some(b"{}"),
        );
        let request = Request::builder()
            .method("PUT")
            .uri(&path)
            .header("authorization", signed_for_other_body)
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = fx.private.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let request = Request::builder()
            .method("GET")
            .uri(&path)
            .body(Body::empty())
            .unwrap();
        let response = fx.private.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        assert!(fx.store.integration(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_requests_are_rejected_before_reaching_the_store() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let path = format!("/v1/integrations/{id}");
        let mut unknown_field = put_body(Uuid::new_v4(), 1);
        unknown_field["extra"] = serde_json::json!(true);
        let mut bad_host = put_body(Uuid::new_v4(), 1);
        bad_host["community_host"] = serde_json::json!("Acme.Example.org");
        let mut bad_provider = put_body(Uuid::new_v4(), 1);
        bad_provider["provider"] = serde_json::json!("carrier_pigeon");
        let zero_revision = put_body(Uuid::new_v4(), 0);
        for body in [unknown_field, bad_host, bad_provider, zero_revision] {
            let (status, _) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }

        call(
            &fx,
            &fx.relay,
            "PUT",
            &path,
            &path,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;
        let secret_path = format!("{path}/secret");
        let too_long = serde_json::json!({
            "operation_id": Uuid::new_v4(),
            "revision": 2,
            "secret": "x".repeat(MAX_SECRET_BYTES + 1),
            "grace_seconds": 0,
        });
        let (status, _) = call(
            &fx,
            &fx.relay,
            "PUT",
            &secret_path,
            &secret_path,
            Some(too_long),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let oversized = vec![b' '; MAX_MANAGEMENT_BODY_BYTES + 1];
        let request = Request::builder()
            .method("PUT")
            .uri(&path)
            .header(
                "authorization",
                signed_header(
                    &fx.relay,
                    "PUT",
                    &format!("https://ingress.example.org{path}"),
                    Some(&oversized),
                ),
            )
            .body(Body::from(oversized))
            .unwrap();
        let response = fx.private.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        assert!(fx.store.integration(id).await.unwrap().unwrap().revision == 1);
    }

    #[test]
    fn community_hosts_are_lowercase_dns_names() {
        for ok in [
            "a.example.org",
            "acme-1.communities.example.org",
            "localhost",
        ] {
            assert!(valid_community_host(ok), "{ok}");
        }
        for bad in [
            "",
            "Acme.example.org",
            "-a.example.org",
            "a..b",
            "a.example.org/",
            "a b",
        ] {
            assert!(!valid_community_host(bad), "{bad}");
        }
        assert!(!valid_community_host(
            &"a".repeat(MAX_COMMUNITY_HOST_BYTES + 1)
        ));
    }
}
