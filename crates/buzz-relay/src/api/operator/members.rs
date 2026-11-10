//! Member search, the ownership preview and identity retirement on the
//! deployment-operator API, for an operator that reaches the relay at
//! `RELAY_OPERATOR_API_ORIGIN` instead of the admin host.
//!
//! Authority is the `RELAY_OPERATOR_PUBKEYS` allowlist. The NIP-98 `u` tag
//! must equal the configured origin plus the received path and raw query, so
//! the signature binds `communityHost`; mutations also bind the exact body
//! bytes, DELETE included. After that check each route runs the same core as
//! its `/api/admin/v1` counterpart and answers with that route's success body
//! and lifecycle error envelope. Auth refusals keep this surface's own
//! envelope, with a `code` of `unauthorized` (401) or `not_operator` (403).
//! See `docs/operator-identity-retirement.md`.

use axum::{
    body::Bytes,
    extract::Path,
    http::{Method, Uri},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use buzz_db::identity_retirement::RetirementOperation;
use tower_http::limit::RequestBodyLimitLayer;

use super::*;
use crate::api::admin::{self, CommunityQuery, MemberSearchQuery};

/// Same cap as the admin API: mutation bodies are a few hundred bytes.
const BODY_LIMIT: usize = 4096;

/// The operator member routes, with their own body limit.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/operator/communities/members/search", get(search_members))
        .route(
            "/operator/communities/members/{pubkey}/ownership",
            get(identity_inventory),
        )
        .route(
            "/operator/communities/members/{pubkey}/archive",
            post(archive).delete(unarchive),
        )
        .route(
            "/operator/communities/members/{pubkey}/execution-hold",
            delete(release),
        )
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .with_state(state)
}

/// Authorize an allowlisted operator for this exact request and return its
/// pubkey, or the refusal with a stable `code`.
async fn authorize(
    state: &Arc<AppState>,
    slot: &AccessSlot,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: Option<&[u8]>,
) -> Result<[u8; 32], (StatusCode, Json<Value>)> {
    authorize_operator_request(
        state,
        slot,
        headers,
        method.as_str(),
        uri.path(),
        uri.query(),
        body,
    )
    .await
    .map(|pubkey| pubkey.to_bytes())
    .map_err(|(status, Json(mut error))| {
        let code = match status {
            StatusCode::UNAUTHORIZED => Some("unauthorized"),
            StatusCode::FORBIDDEN => Some("not_operator"),
            _ => None,
        };
        if let Some(code) = code {
            error["code"] = code.into();
        }
        (status, Json(error))
    })
}

/// `GET /operator/communities/members/search?communityHost=&q=&limit=`
async fn search_members(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<MemberSearchQuery>,
) -> Response {
    if let Err(refusal) = authorize(&state, &slot, &headers, &method, &uri, None).await {
        return refusal.into_response();
    }
    admin::search_members_in(&state, &query)
        .await
        .map(Json)
        .into_response()
}

/// `GET /operator/communities/members/{pubkey}/ownership?communityHost=`
async fn identity_inventory(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(pubkey): Path<String>,
    Query(query): Query<CommunityQuery>,
) -> Response {
    if let Err(refusal) = authorize(&state, &slot, &headers, &method, &uri, None).await {
        return refusal.into_response();
    }
    admin::identity_inventory_in(&state, &pubkey, &query.community_host)
        .await
        .map(Json)
        .into_response()
}

/// The three retirement mutations: authorize over the exact body, then the
/// shared lifecycle core.
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument, plus the operation
async fn retire(
    state: Arc<AppState>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    operation: RetirementOperation,
    target: String,
    query: CommunityQuery,
    body: Bytes,
) -> Response {
    let actor = match authorize(&state, &slot, &headers, &method, &uri, Some(&body)).await {
        Ok(actor) => actor,
        Err(refusal) => return refusal.into_response(),
    };
    admin::execute_identity_retirement(
        &state,
        actor,
        operation,
        target,
        &query.community_host,
        &body,
    )
    .await
    .into_response()
}

/// `POST /operator/communities/members/{pubkey}/archive?communityHost=`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
async fn archive(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(query): Query<CommunityQuery>,
    body: Bytes,
) -> Response {
    let operation = RetirementOperation::Archive;
    retire(
        state, slot, method, uri, headers, operation, target, query, body,
    )
    .await
}

/// `DELETE /operator/communities/members/{pubkey}/archive?communityHost=`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
async fn unarchive(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(query): Query<CommunityQuery>,
    body: Bytes,
) -> Response {
    let operation = RetirementOperation::Unarchive;
    retire(
        state, slot, method, uri, headers, operation, target, query, body,
    )
    .await
}

/// `DELETE /operator/communities/members/{pubkey}/execution-hold?communityHost=`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
async fn release(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(query): Query<CommunityQuery>,
    body: Bytes,
) -> Response {
    let operation = RetirementOperation::Release;
    retire(
        state, slot, method, uri, headers, operation, target, query, body,
    )
    .await
}
