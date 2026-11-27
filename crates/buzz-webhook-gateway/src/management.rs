//! Private management API called by relays over NIP-98 signed HTTP.
//!
//! ```text
//! relay ──PUT /v1/integrations/{id}──▶ authenticate (NIP-98, relay allowlist)
//!        Authorization: Nostr …              │
//!                                            ▼
//!                               Store::apply(operation_id, revision, mutation)
//!                                 ├─ Applied  ──▶ 200 integration view
//!                                 ├─ Replayed ──▶ 200 stored response
//!                                 ├─ Conflict ──▶ 409 operation_conflict
//!                                 ├─ Stale    ──▶ 409 { current }
//!                                 └─ NotFound ──▶ 404
//! ```
//!
//! Every mutation carries a caller-chosen `operation_id` and the integration
//! `revision` it produces, so a relay can retry blindly: a replay returns the
//! original response and an out-of-date relay learns the current revision.
mod cel_validator;
mod subscriptions;

use cel_validator::refuse_config;

use crate::http::{error, AppState};
use crate::secrets::MAX_SECRET_BYTES;
use crate::store::{
    ApplyOutcome, DeliveryCursor, DeliveryMode, DeliveryPage, DeliveryState, GroupKeyAction,
    IntegrationConfig, IntegrationView, Mutation, Operation, Provider, StoreError,
};
use axum::{
    body::Bytes,
    extract::{rejection::QueryRejection, OriginalUri, Path, Query, State},
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
const DEFAULT_DELIVERY_PAGE: usize = 50;
const MAX_DELIVERY_PAGE: usize = 200;

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
        .route("/v1/integrations/{id}/deliveries", get(list_deliveries))
        .route("/v1/operations/{operation_id}", get(get_operation))
        .merge(subscriptions::routes())
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
    #[serde(default)]
    delivery_mode: DeliveryMode,
    #[serde(default)]
    channel_expr: Option<String>,
    #[serde(default)]
    message_expr: Option<String>,
    #[serde(default)]
    message_data_expr: Option<String>,
    #[serde(default)]
    group_key_expr: Option<String>,
    #[serde(default)]
    group_key_action: GroupKeyAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutSecretBody {
    operation_id: Uuid,
    revision: i64,
    secret: String,
    grace_seconds: u64,
}

/// Serialises an integration view with the `delivery_url` the provider must
/// call. Added here rather than stored so replays of older operations carry it.
fn integration_response(state: &AppState, view: IntegrationView) -> Response {
    match integration_json(state, view) {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(e) => store_error(e),
    }
}

fn integration_json(
    state: &AppState,
    view: IntegrationView,
) -> Result<serde_json::Value, StoreError> {
    let delivery_url = view.delivery_url(state.public_scheme);
    let mut body = match serde_json::to_value(view) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => return Err(StoreError::Invalid("integration view is not serializable")),
    };
    body.insert("delivery_url".to_owned(), delivery_url.into());
    Ok(serde_json::Value::Object(body))
}

fn stored_integration_json(
    state: &AppState,
    response: serde_json::Value,
) -> Result<serde_json::Value, StoreError> {
    let view = serde_json::from_value::<IntegrationView>(response)
        .map_err(|_| StoreError::Invalid("stored operation response"))?;
    integration_json(state, view)
}

async fn apply(state: &AppState, operation: Operation) -> Response {
    if operation.revision <= 0 {
        return invalid();
    }
    match state.store.apply(operation, (state.now)()).await {
        Ok(ApplyOutcome::Applied(applied) | ApplyOutcome::Replayed(applied)) => {
            match stored_integration_json(state, applied.response) {
                Ok(body) => (StatusCode::OK, Json(body)).into_response(),
                Err(e) => store_error(e),
            }
        }
        Ok(ApplyOutcome::Conflict) => error(StatusCode::CONFLICT, "operation_conflict"),
        Ok(ApplyOutcome::StaleRevision { current }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "stale_revision", "current": current })),
        )
            .into_response(),
        Ok(ApplyOutcome::NotFound) => error(StatusCode::NOT_FOUND, "not_found"),
        Ok(ApplyOutcome::CommunityChanged) => {
            error(StatusCode::CONFLICT, "community_host_immutable")
        }
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
    let config = IntegrationConfig {
        delivery_mode: request.delivery_mode,
        channel_expr: request.channel_expr,
        message_expr: request.message_expr,
        message_data_expr: request.message_data_expr,
        group_key_expr: request.group_key_expr,
        group_key_action: request.group_key_action,
    };
    if let Some(refusal) = refuse_config(&config).await {
        return refusal;
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
                config,
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
    match state.store.integration_with_secrets(id, now).await {
        Ok(Some((integration, secrets))) => {
            integration_response(&state, IntegrationView::new(&integration, &secrets, now))
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => store_error(e),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DeliveriesQuery {
    since: Option<chrono::DateTime<chrono::Utc>>,
    cursor: Option<String>,
    state: Option<DeliveryState>,
    limit: Option<usize>,
}

/// Delivery timeline for the admin UI, proxied by the relay. Payloads are
/// never returned. The signed URL binds the path only, so query parameters
/// are read from the request; they select a page, not a security boundary.
/// `next` is a keyset cursor over `(received_at, delivery_id)`, so rows that
/// share a timestamp across a page boundary are neither skipped nor repeated.
async fn list_deliveries(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    query: Result<Query<DeliveriesQuery>, QueryRejection>,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, None) {
        return reject.into_response();
    }
    let Ok(Query(query)) = query else {
        return invalid();
    };
    let limit = query.limit.unwrap_or(DEFAULT_DELIVERY_PAGE);
    if limit == 0 || limit > MAX_DELIVERY_PAGE {
        return invalid();
    }
    match state.store.integration(id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => return store_error(e),
    }
    let before = match query.cursor.as_deref().map(DeliveryCursor::parse) {
        None => None,
        Some(Some(cursor)) => Some(cursor),
        Some(None) => return invalid(),
    };
    let page = DeliveryPage {
        since: query.since,
        before,
        state: query.state,
        limit,
    };
    match state.store.list_deliveries(id, page).await {
        Ok(deliveries) => {
            let next = (deliveries.len() == limit)
                .then(|| deliveries.last())
                .flatten()
                .map(|last| {
                    DeliveryCursor {
                        received_at: last.received_at,
                        delivery_id: last.delivery_id.clone(),
                    }
                    .encode()
                });
            Json(serde_json::json!({ "deliveries": deliveries, "next": next })).into_response()
        }
        Err(e) => store_error(e),
    }
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
        Ok(Some(applied)) => match stored_integration_json(&state, applied.response) {
            Ok(response) => Json(serde_json::json!({
                "operation_id": applied.operation_id,
                "integration_id": applied.integration_id,
                "revision": applied.revision,
                "applied_at": applied.applied_at,
                "response": response,
            }))
            .into_response(),
            Err(e) => store_error(e),
        },
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => store_error(e),
    }
}

#[cfg(test)]
mod tests;
