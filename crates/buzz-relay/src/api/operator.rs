//! Deployment-operator HTTP APIs.
//!
//! These routes are outside the Nostr event data plane. They still use NIP-98
//! request signing and replay protection, but they do not run through event
//! ingest, relay membership, channel scoping, storage, or fan-out.

use std::sync::Arc;

use axum::{
    extract::{Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use buzz_core::{CommunityId, TenantContext};

use crate::handlers::community_provisioning::{
    normalize_candidate_host, validate_pubkey_hex, ProvisionCommunityRequest,
};
use crate::state::AppState;

use super::{api_error, bridge, internal_error};

fn coded_api_error(
    status: StatusCode,
    code: &'static str,
    message: &str,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(serde_json::json!({ "error": message, "code": code })),
    )
}

/// Query parameters for `GET /operator/communities`.
#[derive(Debug, Deserialize)]
pub struct ListCommunitiesQuery {
    owner_pubkey: String,
}

/// Query parameters for `GET /operator/communities/availability`.
#[derive(Debug, Deserialize)]
pub struct CommunityAvailabilityQuery {
    host: String,
}

#[derive(Debug, Deserialize)]
struct TransferCommunityRequest {
    community_id: String,
    new_owner_pubkey: String,
    expected_owner_pubkey: String,
}

#[derive(Debug, Serialize)]
struct TransferCommunityResponse {
    community_id: String,
    new_owner_pubkey: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_owner: Option<String>,
}

const OPERATOR_REPLAY_SCOPE: &str = "operator-management";

#[derive(Debug, Deserialize)]
struct ListenerPubkeysRequest {
    pubkeys: Vec<String>,
}

fn parse_listener_pubkeys(body: &[u8]) -> Result<Vec<Vec<u8>>, (StatusCode, Json<Value>)> {
    let request: ListenerPubkeysRequest = serde_json::from_slice(body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid operator-listener pubkeys JSON: {e}"),
        )
    })?;
    if request.pubkeys.is_empty() || request.pubkeys.len() > 1_000 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "pubkeys must contain between 1 and 1000 entries",
        ));
    }
    request
        .pubkeys
        .into_iter()
        .map(|value| {
            let normalized = validate_pubkey_hex(&value).ok_or_else(|| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "pubkeys must contain 64-char hex public keys",
                )
            })?;
            hex::decode(normalized).map_err(|_| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "pubkeys must contain 64-char hex public keys",
                )
            })
        })
        .collect()
}

/// Shared deployment-global operator auth prelude. The canonical management
/// origin and replay namespace are configuration, never tenant registry state
/// or an inbound proxy `Host` header.
async fn authorize_operator_request(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<nostr::PublicKey, (StatusCode, Json<Value>)> {
    let origin = state
        .config
        .relay_operator_api_origin
        .as_deref()
        .ok_or_else(|| internal_error("operator API origin is not configured"))?;
    let path_with_query = match raw_query {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let url = format!("{origin}{path_with_query}");
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        ..
    } = bridge::verify_nip98_exempt_operator(headers, method, &url, body)?;
    check_operator_replay(state, event_id_bytes).await?;

    let pubkey_hex = pubkey.to_hex();
    if !state
        .config
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &pubkey_hex)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "actor not authorized: not a relay operator",
        ));
    }

    Ok(pubkey)
}

/// Authenticate a deployment-global operator listener using its configured
/// identity and a NIP-98 request signature.
async fn authorize_operator_listener_request(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<nostr::PublicKey, (StatusCode, Json<Value>)> {
    let origin = state
        .config
        .relay_operator_api_origin
        .as_deref()
        .ok_or_else(|| internal_error("operator API origin is not configured"))?;
    let url = format!("{origin}{path}");
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        ..
    } = bridge::verify_nip98_exempt_operator(headers, method, &url, Some(body))?;
    check_operator_replay(state, event_id_bytes).await?;
    if !state
        .config
        .operator_listener_delivery_urls
        .contains_key(&pubkey.to_hex())
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "actor not authorized: not a configured operator listener",
        ));
    }
    Ok(pubkey)
}

/// Register target pubkeys for the authenticated operator listener.
pub async fn register_listener_pubkeys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let listener = authorize_operator_listener_request(
        &state,
        &headers,
        "POST",
        "/operator/listener/pubkeys",
        &body,
    )
    .await?;
    let target_pubkeys = parse_listener_pubkeys(&body)?;
    state
        .db
        .register_operator_listener_pubkeys(listener.as_bytes(), &target_pubkeys)
        .await
        .map_err(|e| internal_error(&format!("register operator-listener pubkeys: {e}")))?;
    Ok(Json(serde_json::json!({})))
}

/// Remove target pubkeys for the authenticated operator listener.
pub async fn remove_listener_pubkeys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let listener = authorize_operator_listener_request(
        &state,
        &headers,
        "DELETE",
        "/operator/listener/pubkeys",
        &body,
    )
    .await?;
    let target_pubkeys = parse_listener_pubkeys(&body)?;
    state
        .db
        .remove_operator_listener_pubkeys(listener.as_bytes(), &target_pubkeys)
        .await
        .map_err(|e| internal_error(&format!("remove operator-listener pubkeys: {e}")))?;
    Ok(Json(serde_json::json!({})))
}

async fn check_operator_replay(
    state: &AppState,
    event_id_bytes: [u8; 32],
) -> Result<(), (StatusCode, Json<Value>)> {
    let event_id = nostr::EventId::from_byte_array(event_id_bytes);
    match state
        .nip98_replay
        .try_mark_in_scope(
            OPERATOR_REPLAY_SCOPE,
            &event_id,
            buzz_auth::DEFAULT_REPLAY_TTL_SECS,
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(api_error(
            StatusCode::UNAUTHORIZED,
            "NIP-98: replay detected",
        )),
        Err(error) => {
            tracing::warn!(
                scope = OPERATOR_REPLAY_SCOPE,
                error = %error,
                "operator NIP-98 replay guard failed; rejecting request fail-closed"
            );
            Err(api_error(
                StatusCode::UNAUTHORIZED,
                "NIP-98: replay check unavailable",
            ))
        }
    }
}

/// Create a community host and atomically bootstrap its initial owner.
///
/// `POST /operator/communities`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// { "host": "acme.communities.buzz.xyz", "initial_owner_pubkey": "<hex>" }
/// ```
///
/// The request is authenticated against `RELAY_OPERATOR_API_ORIGIN` and does
/// not bind the inbound host to a tenant. The operator allowlist is the
/// authority for this deployment-root control-plane surface.
pub async fn provision_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pubkey = authorize_operator_request(
        &state,
        &headers,
        "POST",
        "/operator/communities",
        None,
        Some(&body),
    )
    .await?;

    let request: ProvisionCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid provision-community JSON: {e}"),
        )
    })?;

    match crate::handlers::community_provisioning::provision_community(&state, &pubkey, request)
        .await
    {
        Ok(response) => Ok(Json(serde_json::to_value(response).map_err(|e| {
            tracing::error!("failed to serialize provision-community response: {e}");
            internal_error("operator provision response serialization failed")
        })?)),
        Err(msg) if msg.starts_with("actor not authorized") => {
            Err(api_error(StatusCode::FORBIDDEN, &msg))
        }
        Err(msg) if msg.starts_with("limit_reached:") => {
            Err(coded_api_error(StatusCode::CONFLICT, "limit_reached", &msg))
        }
        Err(msg) if msg == "community already exists" || msg.starts_with("owner_conflict:") => {
            Err(api_error(StatusCode::CONFLICT, &msg))
        }
        Err(msg)
            if msg.starts_with("failed to create community:")
                || msg.starts_with("community provisioned but owner bootstrap failed:") =>
        {
            tracing::error!(error = %msg, "operator community persistence failed");
            Err(internal_error("operator community persistence failed"))
        }
        Err(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
    }
}

/// Owner assertion supplied by the trusted operator client.
#[derive(Debug, Deserialize)]
pub struct ArchiveCommunityRequest {
    host: String,
    owner_pubkey: String,
}

/// Operator-attested owner intent mediated by a trusted deployment operator.
#[derive(Debug, Deserialize)]
pub struct DeleteCommunityRequest {
    host: String,
    community_id: Option<Uuid>,
    owner_pubkey: String,
    request_id: Uuid,
    acknowledgement_version: i32,
}

/// Idempotently archive a community owned by the asserted end-user identity.
pub async fn archive_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    const PATH: &str = "/operator/communities/archive";
    authorize_operator_request(&state, &headers, "POST", PATH, None, Some(&body)).await?;
    let request: ArchiveCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid archive-community JSON: {e}"),
        )
    })?;
    let normalized_host = normalize_candidate_host(&request.host)
        .map_err(|msg| api_error(StatusCode::BAD_REQUEST, &msg))?;
    let deployment_host = buzz_core::tenant::relay_url_authority(&state.config.relay_url);
    if normalized_host == deployment_host {
        return Err(api_error(
            StatusCode::CONFLICT,
            "the deployment community cannot be archived",
        ));
    }
    let owner = validate_pubkey_hex(&request.owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid owner_pubkey: expected 64-char hex pubkey",
        )
    })?;
    let record = state
        .db
        .archive_community_owned_by(&normalized_host, &owner, &deployment_host)
        .await
        .map_err(|e| internal_error(&format!("archive community: {e}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "community not found"))?;
    let tenant = TenantContext::resolved(record.id, &record.host);
    let closed = match state
        .disconnect_community_clusterwide(&tenant, record.archived_at)
        .await
    {
        Ok(closed) => closed,
        Err(error) => {
            tracing::warn!(community = %record.id, host = %record.host, %error, "community archived but disconnect propagation is pending");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "community_id": record.id.to_string(),
                    "host": record.host,
                    "archived_at": record.archived_at,
                    "status": "archived",
                    "propagation": "pending",
                    "error": "connection propagation pending — retry this request",
                })),
            ));
        }
    };
    tracing::info!(community = %record.id, host = %record.host, local_connections_closed = closed, "community archived");
    Ok(Json(serde_json::json!({
        "community_id": record.id.to_string(),
        "host": record.host,
        "archived_at": record.archived_at,
        "status": "archived",
    })))
}

/// Idempotently restore an archived community owned by the asserted end-user identity.
pub async fn unarchive_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    const PATH: &str = "/operator/communities/unarchive";
    authorize_operator_request(&state, &headers, "POST", PATH, None, Some(&body)).await?;
    let request: ArchiveCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid unarchive-community JSON: {e}"),
        )
    })?;
    let normalized_host = normalize_candidate_host(&request.host)
        .map_err(|msg| api_error(StatusCode::BAD_REQUEST, &msg))?;
    let owner = validate_pubkey_hex(&request.owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid owner_pubkey: expected 64-char hex pubkey",
        )
    })?;
    let result = state
        .db
        .unarchive_community_owned_by(&normalized_host, &owner)
        .await
        .map_err(|e| internal_error(&format!("unarchive community: {e}")))?;
    let record = match result {
        buzz_db::UnarchiveCommunityResult::Unarchived(record) => record,
        buzz_db::UnarchiveCommunityResult::DeletionPending => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "deletion_lifecycle_conflict",
                "community deletion is pending",
            ));
        }
        buzz_db::UnarchiveCommunityResult::NotFound => {
            return Err(api_error(StatusCode::NOT_FOUND, "community not found"));
        }
    };
    tracing::info!(community = %record.id, host = %record.host, "community unarchived");
    Ok(Json(serde_json::json!({
        "community_id": record.id.to_string(),
        "host": record.host,
        "archived_at": null,
        "status": "active",
    })))
}

/// Persist operator-attested owner deletion intent without executing deletion work.
///
/// `POST /operator/communities/delete`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// {
///   "host": "archived.communities.example",
///   "community_id": "<optional community UUID>",
///   "owner_pubkey": "<64-char hex>",
///   "request_id": "<stable UUID>",
///   "acknowledgement_version": 1
/// }
/// ```
///
/// The request UUID is the correlation/idempotency key. Acceptance is a fast
/// PostgreSQL-only transaction and returns `202`; inventory, approval,
/// quiescing, object-store access, and executor work remain asynchronous.
///
/// Resubmitting the same UUID with the same host, owner, and acknowledgement
/// version returns `202` with that request's current `status`, at any stage and
/// even after membership purge, and never admits new work. Callers recover an
/// ambiguous submission by resending it; any other owner request under a known
/// UUID (different host, owner, or stored acknowledgement version, or a UUID
/// held by an operator-origin request) is `409 deletion_request_conflict`. An unsupported
/// acknowledgement version is rejected before the UUID lookup with
/// `400 unsupported_acknowledgement_version`, even for a known UUID.
/// If supplied, `community_id` must identify the community bound to `host`;
/// a mismatch returns `409 community_id_mismatch`, including on UUID replay.
/// A malformed UUID returns `400 invalid_request`.
///
/// Owner consent is asserted by the operator, not proven to the relay. The
/// operator authenticates the owner and collects the acknowledgement upstream;
/// this request carries only the operator's NIP-98 signature. Authorization is
/// therefore operator authority plus "the asserted pubkey is the community's
/// sole current owner".
/// `owner_pubkey` and `acknowledgement_version` are recorded as provenance for
/// that upstream ceremony, not verified as cryptographic owner consent.
pub async fn delete_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    const PATH: &str = "/operator/communities/delete";
    let operator =
        authorize_operator_request(&state, &headers, "POST", PATH, None, Some(&body)).await?;
    let request: DeleteCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        coded_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            &format!("invalid delete-community JSON: {e}"),
        )
    })?;
    let normalized_host = normalize_candidate_host(&request.host)
        .map_err(|msg| coded_api_error(StatusCode::BAD_REQUEST, "invalid_request", &msg))?;
    if normalized_host != request.host {
        return Err(coded_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "host must use its exact canonical authority spelling",
        ));
    }
    let deployment_host = buzz_core::tenant::relay_url_authority(&state.config.relay_url);
    if normalized_host == deployment_host {
        return Err(coded_api_error(
            StatusCode::CONFLICT,
            "protected_community",
            "the deployment community cannot be deleted",
        ));
    }
    let owner = validate_pubkey_hex(&request.owner_pubkey).ok_or_else(|| {
        coded_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid owner_pubkey: expected 64-char hex pubkey",
        )
    })?;
    let operator = operator.to_hex();
    let admission = state
        .db
        .deletion_store()
        .admit_owner_request(
            &normalized_host,
            &owner,
            &operator,
            request.acknowledgement_version,
            request.request_id,
            request.community_id,
        )
        .await
        .map_err(|error| internal_error(&format!("admit owner deletion request: {error}")))?;
    let accepted = match admission {
        buzz_db::deletion::OwnerDeletionAdmission::Accepted(request) => request,
        buzz_db::deletion::OwnerDeletionAdmission::NotFoundOrNotOwner => {
            return Err(coded_api_error(
                StatusCode::NOT_FOUND,
                "community_not_found",
                "community not found",
            ));
        }
        buzz_db::deletion::OwnerDeletionAdmission::NotArchived => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "community_not_archived",
                "community must be archived before deletion",
            ));
        }
        buzz_db::deletion::OwnerDeletionAdmission::LifecycleConflict => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "deletion_lifecycle_conflict",
                "community deletion lifecycle is already active",
            ));
        }
        buzz_db::deletion::OwnerDeletionAdmission::CommunityIdMismatch => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "community_id_mismatch",
                "community_id does not match the community resolved for host",
            ));
        }
        buzz_db::deletion::OwnerDeletionAdmission::RequestConflict => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "deletion_request_conflict",
                "deletion request conflicts with existing intent",
            ));
        }
        buzz_db::deletion::OwnerDeletionAdmission::UnsupportedAcknowledgementVersion => {
            return Err(coded_api_error(
                StatusCode::BAD_REQUEST,
                "unsupported_acknowledgement_version",
                "unsupported acknowledgement_version",
            ));
        }
    };
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": accepted.id,
            "community_id": accepted.community_id.to_string(),
            "host": accepted.community_host,
            "acknowledgement_version": accepted.acknowledgement_version,
            "status": accepted.stage.to_string(),
        })),
    ))
}

/// List communities where a pubkey currently holds the `owner` role.
pub async fn list_owned_communities(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<ListCommunitiesQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authorize_operator_request(
        &state,
        &headers,
        "GET",
        "/operator/communities",
        raw_query.as_deref(),
        None,
    )
    .await?;

    let owner_pubkey = validate_pubkey_hex(&query.owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid owner_pubkey: expected 64-char hex pubkey",
        )
    })?;

    let page = state
        .db
        .list_communities_owned_by(&owner_pubkey)
        .await
        .map_err(|e| internal_error(&format!("list owned communities: {e}")))?;

    Ok(Json(serde_json::json!({
        "owner_pubkey": owner_pubkey,
        "communities": page.communities.into_iter().map(|row| serde_json::json!({
            "community_id": row.id.to_string(),
            "host": row.host,
            "created_at": row.created_at,
            "archived_at": row.archived_at,
        })).collect::<Vec<_>>(),
        "quota_used": page.quota_used,
        "quota_limit": page.quota_limit,
        "can_create": page.can_create,
    })))
}

/// Transfer ownership of a community to a new owner pubkey.
///
/// `POST /operator/communities/transfer`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// { "community_id": "<uuid>", "new_owner_pubkey": "<hex>" }
/// ```
///
/// The previous owner is demoted to `member` (not `admin`). The transfer is
/// instant and atomic at the database layer. Publication of the updated
/// NIP-43 membership list is best-effort, matching the provision path.
pub async fn transfer_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _pubkey = authorize_operator_request(
        &state,
        &headers,
        "POST",
        "/operator/communities/transfer",
        None,
        Some(&body),
    )
    .await?;

    let request: TransferCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid transfer-community JSON: {e}"),
        )
    })?;

    let community_uuid = Uuid::parse_str(&request.community_id).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid community_id: {e}"),
        )
    })?;

    let new_owner_pubkey = validate_pubkey_hex(&request.new_owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid new_owner_pubkey: expected 64-char hex pubkey",
        )
    })?;

    let expected_owner_pubkey =
        validate_pubkey_hex(&request.expected_owner_pubkey).ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "invalid expected_owner_pubkey: expected 64-char hex pubkey",
            )
        })?;

    let community = CommunityId::from_uuid(community_uuid);

    let result = state
        .db
        .transfer_ownership(community, &new_owner_pubkey, &expected_owner_pubkey)
        .await
        .map_err(|e| internal_error(&format!("transfer ownership: {e}")))?;

    let (status, previous_owner) = match result {
        buzz_db::relay_members::TransferResult::Transferred { previous_owner } => {
            ("transferred", previous_owner)
        }
        buzz_db::relay_members::TransferResult::AlreadyOwner => ("already_owner", None),
        buzz_db::relay_members::TransferResult::NoOwner => {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                "community has no owner to transfer from",
            ));
        }
        buzz_db::relay_members::TransferResult::OwnerConflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "owner_conflict: the current owner no longer matches expected_owner_pubkey",
            ));
        }
        buzz_db::relay_members::TransferResult::LifecycleConflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "community must be active to transfer ownership",
            ));
        }
        buzz_db::relay_members::TransferResult::DeletionPending => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "deletion_lifecycle_conflict",
                "community deletion is pending",
            ));
        }
        buzz_db::relay_members::TransferResult::LimitReached => {
            return Err(coded_api_error(
                StatusCode::CONFLICT,
                "limit_reached",
                "limit_reached: transferee has reached the community limit",
            ));
        }
    };

    // Best-effort NIP-43 membership snapshot publication — same pattern as
    // provision_community. The DB mutation is already committed; a publication
    // failure must not turn a success into an HTTP error.
    if state.config.require_relay_membership {
        if let Some(host) = state
            .db
            .lookup_community_host(community)
            .await
            .map_err(|e| internal_error(&format!("lookup community host: {e}")))?
        {
            let tenant = TenantContext::resolved(community, host);
            if let Err(error) =
                crate::handlers::side_effects::publish_nip43_membership_list(&tenant, &state).await
            {
                tracing::warn!(
                    community = %community,
                    error = %error,
                    "ownership transferred but NIP-43 membership snapshot publication failed"
                );
            }
        }
    }

    let response = TransferCommunityResponse {
        community_id: request.community_id,
        new_owner_pubkey,
        status,
        previous_owner,
    };

    Ok(Json(serde_json::to_value(response).map_err(|e| {
        tracing::error!("failed to serialize transfer-community response: {e}");
        internal_error("operator transfer response serialization failed")
    })?))
}
/// Check whether a community host is available, returning the relay-canonical
/// normalized authority used by create.
pub async fn community_availability(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<CommunityAvailabilityQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authorize_operator_request(
        &state,
        &headers,
        "GET",
        "/operator/communities/availability",
        raw_query.as_deref(),
        None,
    )
    .await?;

    let normalized_host = normalize_candidate_host(&query.host)
        .map_err(|msg| api_error(StatusCode::BAD_REQUEST, &msg))?;
    let existing = state
        .db
        .lookup_community_by_host_for_management(&normalized_host)
        .await
        .map_err(|e| internal_error(&format!("check community availability: {e}")))?;

    Ok(Json(serde_json::json!({
        "host": query.host,
        "normalized_host": normalized_host,
        "available": existing.is_none(),
        "community_id": existing.map(|record| record.id.to_string()),
    })))
}

#[cfg(test)]
mod postgres_tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

    use buzz_core::{kind::KIND_NIP43_MEMBERSHIP_LIST, CommunityId};
    use buzz_db::event::EventQuery;

    use crate::router::build_router;
    use crate::state::AppState;

    struct AlwaysFreshReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    struct SeenOnceReplayGuard(Mutex<HashSet<[u8; 32]>>);

    impl buzz_auth::Nip98ReplayGuard for SeenOnceReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            let inserted = self
                .0
                .lock()
                .expect("replay set")
                .insert(*event_id.as_bytes());
            Box::pin(async move { Ok(inserted) })
        }
    }
    const INGRESS_HOST: &str = "operator-ingress.example";

    fn nip98_auth_header(keys: &Keys, url: &str, method: &str, body: Option<&[u8]>) -> String {
        let mut tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
        ];
        if let Some(body) = body {
            let hash: [u8; 32] = Sha256::digest(body).into();
            let hash_hex = hex::encode(hash);
            tags.push(Tag::parse(["payload", hash_hex.as_str()]).expect("payload tag"));
        }
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
    }

    fn nip98_auth_header_without_payload(keys: &Keys, url: &str, method: &str) -> String {
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
    }

    async fn operator_test_state(operator_keys: &[Keys]) -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
        config.database_url = crate::test_support::database_url();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_url = "wss://tenant.example".to_string();
        config.relay_operator_api_origin = Some(format!("http://{INGRESS_HOST}"));
        config.relay_operator_pubkeys = operator_keys
            .iter()
            .map(|keys| keys.public_key().to_hex())
            .collect();
        config.require_relay_membership = true;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());

        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    async fn read_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response JSON")
    }

    async fn signed_operator_request(
        state: Arc<AppState>,
        keys: &Keys,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> axum::response::Response {
        let url = format!("http://{INGRESS_HOST}{path}");
        let auth = nip98_auth_header(keys, &url, method, body.as_deref().map(str::as_bytes));
        operator_request_with_auth(state, method, path, body, auth).await
    }

    async fn operator_request_with_auth(
        state: Arc<AppState>,
        method: &str,
        path: &str,
        body: Option<String>,
        auth: String,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, INGRESS_HOST)
            .header(header::AUTHORIZATION, auth);
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        build_router(state)
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, Body::from))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    /// Send a delete-community request with a caller-controlled `Authorization`
    /// header so signature-binding failures can be exercised directly.
    async fn raw_owner_delete(
        state: Arc<AppState>,
        auth: Option<String>,
        extra_header: Option<(&str, String)>,
        body: String,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/operator/communities/delete")
            .header(header::HOST, INGRESS_HOST)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        if let Some((name, value)) = extra_header {
            request = request.header(name, value);
        }
        build_router(state)
            .oneshot(request.body(Body::from(body)).expect("request"))
            .await
            .expect("response")
    }

    /// The endpoint must not have persisted intent under `request_id`.
    async fn assert_no_persisted_request(state: &AppState, request_id: Uuid, case: &str) {
        let found = state.db.deletion_store().get(request_id).await;
        assert!(
            matches!(found, Err(buzz_db::DbError::NotFound(_))),
            "{case}: rejected request must not persist deletion intent"
        );
    }

    async fn provision_community(
        state: Arc<AppState>,
        operator: &Keys,
        host: &str,
        owner: &Keys,
    ) -> axum::response::Response {
        let body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": owner.public_key().to_hex(),
            "create_only": true,
        })
        .to_string();
        signed_operator_request(state, operator, "POST", "/operator/communities", Some(body)).await
    }

    async fn archive_for_owner_deletion(state: &AppState, host: &str, owner: &Keys) {
        state
            .db
            .archive_community_owned_by(
                host,
                &owner.public_key().to_hex(),
                &buzz_core::tenant::relay_url_authority(&state.config.relay_url),
            )
            .await
            .expect("archive community")
            .expect("owned community");
    }

    fn owner_delete_body(host: &str, owner: &Keys, request_id: Uuid) -> String {
        serde_json::json!({
            "host": host,
            "owner_pubkey": owner.public_key().to_hex(),
            "request_id": request_id,
            "acknowledgement_version": 1,
        })
        .to_string()
    }

    fn is_member_tag(tag: &Tag, pubkey: &str, role: &str) -> bool {
        let values = tag.as_slice();
        values.first().is_some_and(|value| value == "member")
            && values.get(1).is_some_and(|value| value == pubkey)
            && values.get(2).is_some_and(|value| value == role)
    }

    async fn assert_snapshot_roles(
        state: &AppState,
        community: CommunityId,
        expected: &[(&str, &str)],
    ) {
        let snapshot = state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![KIND_NIP43_MEMBERSHIP_LIST as i32]),
                global_only: true,
                limit: Some(1),
                ..EventQuery::for_community(community)
            })
            .await
            .expect("query membership snapshot")
            .into_iter()
            .next()
            .expect("membership snapshot published");
        for &(pubkey, role) in expected {
            assert!(
                snapshot
                    .event
                    .tags
                    .iter()
                    .any(|tag| is_member_tag(tag, pubkey, role)),
                "missing {role} snapshot tag for {pubkey}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn non_allowlisted_operator_key_gets_403() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let Some(state) = operator_test_state(&[operator]).await else {
            return;
        };
        let body = format!(
            r#"{{"host":"community-{}.example"}}"#,
            Uuid::new_v4().simple()
        );
        let url = format!("http://{INGRESS_HOST}/operator/communities");
        let auth = nip98_auth_header(&outsider, &url, "POST", Some(body.as_bytes()));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_endpoint_accepts_archived_owner_intent_without_running_inventory() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;
        let request_id = Uuid::new_v4();
        let response = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, request_id)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let json = read_json(response).await;
        assert_eq!(json["request_id"], request_id.to_string());
        assert_eq!(json["acknowledgement_version"], 1);
        assert_eq!(json["status"], "submitted");
        let request = state
            .db
            .deletion_store()
            .get(request_id)
            .await
            .expect("persisted deletion request");
        assert_eq!(request.stage, buzz_db::deletion::DeletionStage::Submitted);
        assert!(request.inventory_manifest.is_none());
        assert!(request.inventory_digest.is_none());
        assert_eq!(
            request.mediating_operator_pubkey.as_deref(),
            Some(operator.public_key().to_hex().as_str())
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_community_id_confirms_host_on_admission_and_replay() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let created = provision_community(Arc::clone(&state), &operator, &host, &owner).await;
        assert_eq!(created.status(), StatusCode::OK);
        let community_id: Uuid = read_json(created).await["community_id"]
            .as_str()
            .expect("community id")
            .parse()
            .expect("valid community id");
        archive_for_owner_deletion(&state, &host, &owner).await;
        let pool = state.db.pool();
        let lifecycle_before: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community_id)
        .fetch_one(pool)
        .await
        .expect("archived lifecycle");
        assert!(lifecycle_before.0.is_some());
        let owner_before = state
            .db
            .get_relay_member(
                CommunityId::from_uuid(community_id),
                &owner.public_key().to_hex(),
            )
            .await
            .expect("read owner")
            .expect("owner exists")
            .role;
        let request_id = Uuid::new_v4();
        let body = |community_id: Value| {
            serde_json::json!({
                "host": host,
                "community_id": community_id,
                "owner_pubkey": owner.public_key().to_hex(),
                "request_id": request_id,
                "acknowledgement_version": 1,
                "ignored_extension": "forward-compatible",
            })
            .to_string()
        };
        // A non-owner must see the same 404 as an unknown host, even with a wrong id.
        let stranger_mismatch = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": host,
                    "community_id": Uuid::new_v4(),
                    "owner_pubkey": Keys::generate().public_key().to_hex(),
                    "request_id": request_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(stranger_mismatch.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            read_json(stranger_mismatch).await["code"],
            "community_not_found"
        );
        assert_no_persisted_request(&state, request_id, "non-owner mismatched id").await;
        let mismatch = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body(serde_json::json!(Uuid::new_v4()))),
        )
        .await;
        assert_eq!(mismatch.status(), StatusCode::CONFLICT);
        assert_eq!(read_json(mismatch).await["code"], "community_id_mismatch");
        assert_no_persisted_request(&state, request_id, "mismatched community id").await;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_deletion_requests WHERE community_id = $1",
        )
        .bind(community_id)
        .fetch_one(pool)
        .await
        .expect("count rejected requests");
        assert_eq!(count, 0);
        let lifecycle_after: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community_id)
        .fetch_one(pool)
        .await
        .expect("unchanged lifecycle");
        assert_eq!(lifecycle_after, lifecycle_before);
        assert_eq!(
            state
                .db
                .get_relay_member(
                    CommunityId::from_uuid(community_id),
                    &owner.public_key().to_hex()
                )
                .await
                .expect("read unchanged owner")
                .expect("owner still exists")
                .role,
            owner_before
        );

        let malformed = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body(serde_json::json!("not-a-uuid"))),
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(malformed).await["code"], "invalid_request");
        assert_no_persisted_request(&state, request_id, "malformed community id").await;

        let matched = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body(serde_json::json!(community_id))),
        )
        .await;
        assert_eq!(matched.status(), StatusCode::ACCEPTED);
        assert_eq!(
            read_json(matched).await["community_id"],
            community_id.to_string()
        );
        let admitted = state
            .db
            .deletion_store()
            .get(request_id)
            .await
            .expect("admitted request");
        let replay = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body(serde_json::json!(Uuid::new_v4()))),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CONFLICT);
        assert_eq!(read_json(replay).await["code"], "community_id_mismatch");
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("unchanged request"),
            admitted
        );
        let matched_replay = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body(serde_json::json!(community_id))),
        )
        .await;
        assert_eq!(matched_replay.status(), StatusCode::ACCEPTED);
        let matched_replay = read_json(matched_replay).await;
        assert_eq!(matched_replay["request_id"], request_id.to_string());
        assert_eq!(matched_replay["community_id"], community_id.to_string());
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("replayed request"),
            admitted
        );

        let other_host = format!("community-{}.example", Uuid::new_v4().simple());
        let other_created =
            provision_community(Arc::clone(&state), &operator, &other_host, &owner).await;
        assert_eq!(other_created.status(), StatusCode::OK);
        let other_id: Uuid = read_json(other_created).await["community_id"]
            .as_str()
            .expect("other community id")
            .parse()
            .expect("valid other community id");
        archive_for_owner_deletion(&state, &other_host, &owner).await;
        // A known request id reused for a different host is a request conflict,
        // even when `community_id` correctly names that other host.
        let collision = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": other_host,
                    "community_id": other_id,
                    "owner_pubkey": owner.public_key().to_hex(),
                    "request_id": request_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(collision.status(), StatusCode::CONFLICT);
        assert_eq!(
            read_json(collision).await["code"],
            "deletion_request_conflict"
        );
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("request unchanged by collision"),
            admitted
        );
        let other_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_deletion_requests WHERE community_id = $1",
        )
        .bind(other_id)
        .fetch_one(pool)
        .await
        .expect("count other-host requests");
        assert_eq!(other_count, 0);
        let absent_id = Uuid::new_v4();
        let absent = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&other_host, &owner, absent_id)),
        )
        .await;
        assert_eq!(absent.status(), StatusCode::ACCEPTED);
        assert_eq!(read_json(absent).await["request_id"], absent_id.to_string());
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(absent_id)
                .await
                .expect("legacy request")
                .stage,
            buzz_db::deletion::DeletionStage::Submitted
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_admission_requires_exact_canonical_host_without_mutation() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;

        for (label, repaired) in [
            ("whitespace", format!(" {host}")),
            ("case", host.to_uppercase()),
            ("url", format!("https://{host}")),
        ] {
            let request_id = Uuid::new_v4();
            let response = signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(owner_delete_body(&repaired, &owner, request_id)),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
            assert_eq!(read_json(response).await["code"], "invalid_request");
            assert_no_persisted_request(&state, request_id, label).await;
        }

        let request_id = Uuid::new_v4();
        let accepted = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, request_id)),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        assert_eq!(read_json(accepted).await["host"], host);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_resubmission_reports_current_status_without_new_intent() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;
        let request_id = Uuid::new_v4();
        let body = owner_delete_body(&host, &owner, request_id);
        assert_eq!(
            signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(body.clone()),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        let before = state
            .db
            .deletion_store()
            .list(1_000)
            .await
            .expect("list requests before replays")
            .into_iter()
            .filter(|request| request.id == request_id)
            .count();

        let replay = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body.clone()),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::ACCEPTED);
        let replay = read_json(replay).await;
        assert_eq!(replay["request_id"], request_id.to_string());
        assert_eq!(replay["host"], host);
        assert_eq!(replay["acknowledgement_version"], 1);
        assert_eq!(replay["status"], "submitted");

        // Recovery must survive membership purge: the replay converges on the
        // stored tuple before any owner or archive check runs.
        sqlx::query("DELETE FROM relay_members WHERE pubkey = $1 AND role = 'owner'")
            .bind(owner.public_key().to_hex())
            .execute(state.db.pool())
            .await
            .expect("simulate membership purge");
        let purged_replay = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body.clone()),
        )
        .await;
        assert_eq!(purged_replay.status(), StatusCode::ACCEPTED);
        assert_eq!(read_json(purged_replay).await["status"], "submitted");

        for mismatch in [
            serde_json::json!({
                "host": format!("wrong-{host}"),
                "owner_pubkey": owner.public_key().to_hex(),
                "request_id": request_id,
                "acknowledgement_version": 1,
            }),
            serde_json::json!({
                "host": host,
                "owner_pubkey": Keys::generate().public_key().to_hex(),
                "request_id": request_id,
                "acknowledgement_version": 1,
            }),
        ] {
            let response = signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(mismatch.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
            assert_eq!(
                read_json(response).await["code"],
                "deletion_request_conflict"
            );
        }

        // Version validation precedes the UUID lookup, so a changed version is
        // rejected as unsupported rather than as a request conflict.
        let changed_version = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": host,
                    "owner_pubkey": owner.public_key().to_hex(),
                    "request_id": request_id,
                    "acknowledgement_version": 2,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(changed_version.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            read_json(changed_version).await["code"],
            "unsupported_acknowledgement_version"
        );

        let outsider_response = signed_operator_request(
            Arc::clone(&state),
            &outsider,
            "POST",
            "/operator/communities/delete",
            Some(body.clone()),
        )
        .await;
        assert_eq!(outsider_response.status(), StatusCode::FORBIDDEN);

        state
            .db
            .deletion_store()
            .abort(request_id, &operator.public_key().to_hex(), "test abort")
            .await
            .expect("abort request");
        let aborted = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(body),
        )
        .await;
        assert_eq!(aborted.status(), StatusCode::ACCEPTED);
        assert_eq!(read_json(aborted).await["status"], "aborted");

        let after = state
            .db
            .deletion_store()
            .list(1_000)
            .await
            .expect("list requests after replays")
            .into_iter()
            .filter(|request| request.id == request_id)
            .count();
        assert_eq!(before, after, "replays must not add request rows");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_endpoint_rejects_protected_host_and_malformed_or_mismatched_target() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let protected_host = buzz_core::tenant::relay_url_authority(&state.config.relay_url);
        let protected = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&protected_host, &owner, Uuid::new_v4())),
        )
        .await;
        assert_eq!(protected.status(), StatusCode::CONFLICT);
        assert_eq!(read_json(protected).await["code"], "protected_community");

        // Each malformed case keeps every unrelated field valid so it reaches
        // the guard under test instead of tripping an earlier one, and none of
        // them may leave durable intent behind.
        let valid_owner = owner.public_key().to_hex();
        let reachable_host = format!("community-{}.example", Uuid::new_v4().simple());

        let bad_host_id = Uuid::new_v4();
        let bad_host = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": "https://not-an-authority.example/path",
                    "owner_pubkey": valid_owner,
                    "request_id": bad_host_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(bad_host.status(), StatusCode::BAD_REQUEST);
        assert_eq!(read_json(bad_host).await["code"], "invalid_request");
        assert_no_persisted_request(&state, bad_host_id, "unnormalizable host").await;

        let bad_pubkey_id = Uuid::new_v4();
        let bad_pubkey = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": reachable_host,
                    "owner_pubkey": "not-a-pubkey",
                    "request_id": bad_pubkey_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(bad_pubkey.status(), StatusCode::BAD_REQUEST);
        let bad_pubkey_error = read_json(bad_pubkey).await;
        assert!(
            bad_pubkey_error
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("owner_pubkey"),
            "invalid owner_pubkey must reach the pubkey guard: {bad_pubkey_error:?}"
        );
        assert_no_persisted_request(&state, bad_pubkey_id, "invalid owner_pubkey").await;

        let bad_version_id = Uuid::new_v4();
        let bad_version = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": reachable_host,
                    "owner_pubkey": valid_owner,
                    "request_id": bad_version_id,
                    "acknowledgement_version": 2,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(bad_version.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            read_json(bad_version).await["code"],
            "unsupported_acknowledgement_version"
        );
        assert_no_persisted_request(&state, bad_version_id, "unsupported acknowledgement").await;

        let unknown_host_id = Uuid::new_v4();
        let unknown_host = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": reachable_host,
                    "owner_pubkey": valid_owner,
                    "request_id": unknown_host_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(unknown_host.status(), StatusCode::NOT_FOUND);
        assert_eq!(read_json(unknown_host).await["code"], "community_not_found");
        assert_no_persisted_request(&state, unknown_host_id, "unprovisioned host").await;

        let bad_uuid = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(
                serde_json::json!({
                    "host": reachable_host,
                    "owner_pubkey": valid_owner,
                    "request_id": "not-a-uuid",
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(bad_uuid.status(), StatusCode::BAD_REQUEST);

        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;
        let request_id = Uuid::new_v4();
        let first = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, request_id)),
        )
        .await;
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let other_host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &other_host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &other_host, &owner).await;
        let mismatched = signed_operator_request(
            state,
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&other_host, &owner, request_id)),
        )
        .await;
        assert_eq!(mismatched.status(), StatusCode::CONFLICT);
    }

    /// Every signature-binding failure mode on the owner-delete endpoint.
    ///
    /// The endpoint mediates an irreversible request, so a caller that cannot
    /// prove operator authority over this exact method, URL, and body must be
    /// refused without leaving durable intent behind.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_endpoint_requires_operator_bound_nip98_signature() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;

        let delete_url = format!("http://{INGRESS_HOST}/operator/communities/delete");

        // 1. No Authorization header at all.
        let unsigned_id = Uuid::new_v4();
        let unsigned = raw_owner_delete(
            Arc::clone(&state),
            None,
            None,
            owner_delete_body(&host, &owner, unsigned_id),
        )
        .await;
        assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, unsigned_id, "unsigned").await;

        // 2. X-Pubkey only: the dev fallback is disabled for operator endpoints.
        let x_pubkey_id = Uuid::new_v4();
        let x_pubkey_only = raw_owner_delete(
            Arc::clone(&state),
            None,
            Some(("x-pubkey", operator.public_key().to_hex())),
            owner_delete_body(&host, &owner, x_pubkey_id),
        )
        .await;
        assert_eq!(x_pubkey_only.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, x_pubkey_id, "X-Pubkey only").await;

        // 3. Valid NIP-98 from a key that is not an allowlisted operator.
        let outsider_id = Uuid::new_v4();
        let outsider_body = owner_delete_body(&host, &owner, outsider_id);
        let outsider_response = raw_owner_delete(
            Arc::clone(&state),
            Some(nip98_auth_header(
                &outsider,
                &delete_url,
                "POST",
                Some(outsider_body.as_bytes()),
            )),
            None,
            outsider_body,
        )
        .await;
        assert_eq!(outsider_response.status(), StatusCode::FORBIDDEN);
        assert_no_persisted_request(&state, outsider_id, "non-operator signer").await;

        // 4. Operator signature that omits the payload tag.
        let no_payload_id = Uuid::new_v4();
        let no_payload = raw_owner_delete(
            Arc::clone(&state),
            Some(nip98_auth_header_without_payload(
                &operator,
                &delete_url,
                "POST",
            )),
            None,
            owner_delete_body(&host, &owner, no_payload_id),
        )
        .await;
        assert_eq!(no_payload.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, no_payload_id, "missing payload tag").await;

        // 5. Payload tag bound to a different body than the one sent.
        let signed_id = Uuid::new_v4();
        let tampered_id = Uuid::new_v4();
        let signed_body = owner_delete_body(&host, &owner, signed_id);
        let tampered = raw_owner_delete(
            Arc::clone(&state),
            Some(nip98_auth_header(
                &operator,
                &delete_url,
                "POST",
                Some(signed_body.as_bytes()),
            )),
            None,
            owner_delete_body(&host, &owner, tampered_id),
        )
        .await;
        assert_eq!(tampered.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, tampered_id, "tampered payload").await;
        assert_no_persisted_request(&state, signed_id, "tampered payload (signed id)").await;

        // 6. Signature bound to a different operator URL.
        let wrong_url_id = Uuid::new_v4();
        let wrong_url_body = owner_delete_body(&host, &owner, wrong_url_id);
        let wrong_url = raw_owner_delete(
            Arc::clone(&state),
            Some(nip98_auth_header(
                &operator,
                &format!("http://{INGRESS_HOST}/operator/communities"),
                "POST",
                Some(wrong_url_body.as_bytes()),
            )),
            None,
            wrong_url_body,
        )
        .await;
        assert_eq!(wrong_url.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, wrong_url_id, "wrong signed URL").await;

        // 7. Signature bound to a different method.
        let wrong_method_id = Uuid::new_v4();
        let wrong_method_body = owner_delete_body(&host, &owner, wrong_method_id);
        let wrong_method = raw_owner_delete(
            Arc::clone(&state),
            Some(nip98_auth_header(
                &operator,
                &delete_url,
                "GET",
                Some(wrong_method_body.as_bytes()),
            )),
            None,
            wrong_method_body,
        )
        .await;
        assert_eq!(wrong_method.status(), StatusCode::UNAUTHORIZED);
        assert_no_persisted_request(&state, wrong_method_id, "wrong signed method").await;

        // Falsifiability: the same shape, correctly bound, is accepted.
        let accepted_id = Uuid::new_v4();
        let accepted = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, accepted_id)),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    }

    /// The request UUID is the correlation identity, so a replay that changes
    /// any bound field is a conflict rather than a second interpretation.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_delete_replay_rejects_any_changed_field_without_mutating_intent() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let other_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &host, &owner).await;

        let other_host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &other_host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        archive_for_owner_deletion(&state, &other_host, &owner).await;

        let request_id = Uuid::new_v4();
        assert_eq!(
            signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(owner_delete_body(&host, &owner, request_id)),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        let admitted = state
            .db
            .deletion_store()
            .get(request_id)
            .await
            .expect("admitted request");

        // Exactly one field differs per case; the rest replay verbatim.
        let owner_hex = owner.public_key().to_hex();
        let cases: Vec<(&str, StatusCode, String)> = vec![
            (
                "changed owner_pubkey",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "host": host,
                    "owner_pubkey": other_owner.public_key().to_hex(),
                    "request_id": request_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
            (
                "changed host",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "host": other_host,
                    "owner_pubkey": owner_hex,
                    "request_id": request_id,
                    "acknowledgement_version": 1,
                })
                .to_string(),
            ),
            (
                "changed acknowledgement_version",
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "host": host,
                    "owner_pubkey": owner_hex,
                    "request_id": request_id,
                    "acknowledgement_version": 2,
                })
                .to_string(),
            ),
        ];
        for (case, expected, body) in cases {
            let response = signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(body),
            )
            .await;
            assert_eq!(response.status(), expected, "{case}");
            let current = state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("request still readable");
            assert_eq!(current.community_id, admitted.community_id, "{case}");
            assert_eq!(current.community_host, admitted.community_host, "{case}");
            assert_eq!(current.owner_pubkey, admitted.owner_pubkey, "{case}");
            assert_eq!(
                current.acknowledgement_version, admitted.acknowledgement_version,
                "{case}"
            );
            assert_eq!(current.stage, admitted.stage, "{case}");
        }

        // An identical replay still converges on the same request.
        let converged = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/delete",
            Some(owner_delete_body(&host, &owner, request_id)),
        )
        .await;
        assert_eq!(converged.status(), StatusCode::ACCEPTED);
        assert_eq!(
            read_json(converged).await["request_id"],
            request_id.to_string()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn post_operator_body_requires_payload_tag() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = format!(
            r#"{{"host":"community-{}.example"}}"#,
            Uuid::new_v4().simple()
        );
        let url = format!("http://{INGRESS_HOST}/operator/communities");
        let auth = nip98_auth_header_without_payload(&operator, &url, "POST");

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = read_json(response).await;
        assert!(
            json.get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("missing payload tag"),
            "unexpected response: {json:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn listener_pubkey_routes_register_and_remove_only_for_authenticated_listener() {
        let listener = Keys::generate();
        let other_listener = Keys::generate();
        let outsider = Keys::generate();
        let Some(mut state) = operator_test_state(&[]).await else {
            return;
        };
        Arc::get_mut(&mut state)
            .expect("test state has a single owner")
            .nip98_replay = Arc::new(SeenOnceReplayGuard(Mutex::new(HashSet::new())));
        let config = Arc::make_mut(
            &mut Arc::get_mut(&mut state)
                .expect("test state has a single owner")
                .config,
        );
        config.operator_listener_delivery_urls.insert(
            listener.public_key().to_hex(),
            url::Url::parse("http://listener.example/deliver").expect("listener URL"),
        );
        config.operator_listener_delivery_urls.insert(
            other_listener.public_key().to_hex(),
            url::Url::parse("http://other-listener.example/deliver").expect("listener URL"),
        );

        let targets = [Keys::generate(), Keys::generate()];
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let target_hex = targets
            .iter()
            .map(|target| target.public_key().to_hex())
            .collect::<Vec<_>>();
        let register_body = serde_json::json!({"pubkeys": target_hex}).to_string();
        let response = signed_operator_request(
            Arc::clone(&state),
            &listener,
            "POST",
            "/operator/listener/pubkeys",
            Some(register_body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let listener_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_pubkeys WHERE listener_pubkey = $1",
        )
        .bind(listener.public_key().as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count registered listener targets");
        let other_listener_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM operator_listener_pubkeys WHERE listener_pubkey = $1",
        )
        .bind(other_listener.public_key().as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count other listener targets");
        assert_eq!(listener_count, 2);
        assert_eq!(other_listener_count, 0);

        let body = serde_json::json!({"pubkeys": [target_hex[0]]}).to_string();
        let missing_payload_auth = nip98_auth_header_without_payload(
            &listener,
            &format!("http://{INGRESS_HOST}/operator/listener/pubkeys"),
            "DELETE",
        );
        let missing_payload = operator_request_with_auth(
            Arc::clone(&state),
            "DELETE",
            "/operator/listener/pubkeys",
            Some(body),
            missing_payload_auth,
        )
        .await;
        assert_eq!(missing_payload.status(), StatusCode::UNAUTHORIZED);

        let unauthorized = signed_operator_request(
            Arc::clone(&state),
            &outsider,
            "POST",
            "/operator/listener/pubkeys",
            Some(serde_json::json!({"pubkeys": [target_hex[0]]}).to_string()),
        )
        .await;
        assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);

        let malformed = signed_operator_request(
            Arc::clone(&state),
            &listener,
            "POST",
            "/operator/listener/pubkeys",
            Some(r#"{"pubkeys":["not-a-pubkey"]}"#.to_string()),
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        let replay_body = serde_json::json!({"pubkeys": [target_hex[1]]}).to_string();
        let replay_auth = nip98_auth_header(
            &listener,
            &format!("http://{INGRESS_HOST}/operator/listener/pubkeys"),
            "DELETE",
            Some(replay_body.as_bytes()),
        );
        let first_use = operator_request_with_auth(
            Arc::clone(&state),
            "DELETE",
            "/operator/listener/pubkeys",
            Some(replay_body.clone()),
            replay_auth.clone(),
        )
        .await;
        assert_eq!(first_use.status(), StatusCode::OK);
        let replay = operator_request_with_auth(
            Arc::clone(&state),
            "DELETE",
            "/operator/listener/pubkeys",
            Some(replay_body),
            replay_auth,
        )
        .await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
        let replay_error = read_json(replay).await;
        assert!(replay_error["error"]
            .as_str()
            .unwrap_or_default()
            .contains("replay"));

        let remove_body = serde_json::json!({"pubkeys": [target_hex[0]]}).to_string();
        let removed = signed_operator_request(
            Arc::clone(&state),
            &listener,
            "DELETE",
            "/operator/listener/pubkeys",
            Some(remove_body),
        )
        .await;
        assert_eq!(removed.status(), StatusCode::OK);
        let remaining: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT target_pubkey FROM operator_listener_pubkeys WHERE listener_pubkey = $1",
        )
        .bind(listener.public_key().as_bytes().as_slice())
        .fetch_all(&pool)
        .await
        .expect("read remaining listener targets");
        assert!(remaining.is_empty());

        sqlx::query("DELETE FROM operator_listener_pubkeys WHERE listener_pubkey = $1 OR listener_pubkey = $2")
            .bind(listener.public_key().as_bytes().as_slice())
            .bind(other_listener.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("clean up listener registrations");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unmapped_management_host_can_check_availability() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let query = format!("host={host}");
        let url = format!("http://{INGRESS_HOST}/operator/communities/availability?{query}");
        let auth = nip98_auth_header(&operator, &url, "GET", None);

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/operator/communities/availability?{query}"))
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(json.get("available").and_then(Value::as_bool), Some(true));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unmapped_management_host_can_list_owned_communities() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let owner_hex = owner.public_key().to_hex();
        let query = format!("owner_pubkey={owner_hex}");
        let url = format!("http://{INGRESS_HOST}/operator/communities?{query}");
        let auth = nip98_auth_header(&operator, &url, "GET", None);

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/operator/communities?{query}"))
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(
            json.get("owner_pubkey").and_then(Value::as_str),
            Some(owner_hex.as_str())
        );
    }

    /// A relay that predates the archive transition timestamp publishes a bare
    /// `DisconnectCommunity`. A current relay must still apply it under the
    /// lifecycle fence: never against a restored community, and with the
    /// archive close reason against an archived one.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_disconnect_command_is_fenced_by_community_lifecycle() {
        use crate::state::{CommunityConnectionControl, CommunityDisconnectReason};
        use tokio_util::sync::CancellationToken;

        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("active admission lookup")
            .expect("active community")
            .id;
        let owner_hex = owner.public_key().to_hex();

        // Active (for example, unarchived before the command arrived): keep sockets.
        let active_cancel = CancellationToken::new();
        let active_control = CommunityConnectionControl::new(active_cancel.clone());
        let active_reason = active_control.disconnect_reason();
        let active_guard =
            state
                .community_connections
                .register(Uuid::new_v4(), community, active_control);
        assert_eq!(
            state
                .apply_community_disconnect(community, None)
                .await
                .expect("fenced legacy disconnect"),
            None
        );
        assert!(
            !active_cancel.is_cancelled(),
            "a legacy disconnect must not close an active community"
        );
        assert_eq!(*active_reason.borrow(), None);

        // Archived: close with the archive reason, not `community deleted`.
        state
            .db
            .archive_community_owned_by(&host, &owner_hex, "protected.example")
            .await
            .expect("archive community")
            .expect("owned community");
        assert_eq!(
            state
                .apply_community_disconnect(community, None)
                .await
                .expect("fenced legacy disconnect"),
            Some(1)
        );
        assert!(active_cancel.is_cancelled());
        assert_eq!(
            *active_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityArchived)
        );

        // Fenced deletion: this is the state `buzz-deletion` publishes the bare
        // command from, so it must close with the deletion reason even though
        // the row is also archived.
        drop(active_guard);
        let deleted_cancel = CancellationToken::new();
        let deleted_control = CommunityConnectionControl::new(deleted_cancel.clone());
        let deleted_reason = deleted_control.disconnect_reason();
        let _deleted_guard =
            state
                .community_connections
                .register(Uuid::new_v4(), community, deleted_control);
        let mut tx = state
            .db
            .pool()
            .begin()
            .await
            .expect("begin deletion fixture");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
             set_config('buzz.deletion_fence_generation', '0', true)",
        )
        .bind(community.to_string())
        .execute(&mut *tx)
        .await
        .expect("enter deletion executor scope");
        sqlx::query("UPDATE communities SET deletion_state = 'fenced' WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("fence community deletion");
        tx.commit().await.expect("commit deletion fixture");
        assert_eq!(
            state
                .apply_community_disconnect(community, None)
                .await
                .expect("fenced legacy disconnect"),
            Some(1)
        );
        assert!(deleted_cancel.is_cancelled());
        assert_eq!(
            *deleted_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
    }

    /// Lifecycle revalidation runs for every live community on every pod each
    /// tick. Active communities must be checked without the row lock, so a
    /// conflicting lock on an active row must not stall the tick, while an
    /// archived community is still closed under the fence.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revalidation_skips_the_row_lock_for_active_communities() {
        use crate::state::{CommunityConnectionControl, CommunityDisconnectReason};
        use tokio_util::sync::CancellationToken;

        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("active admission lookup")
            .expect("active community")
            .id;
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let reason = control.disconnect_reason();
        let guard = state
            .community_connections
            .register(Uuid::new_v4(), community, control);

        let mut row_lock = state.db.pool().begin().await.expect("begin row lock");
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
            .bind(community.as_uuid())
            .execute(&mut *row_lock)
            .await
            .expect("hold the community row lock");
        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                state.revalidate_live_communities()
            )
            .await
            .expect("revalidation must not wait on an active community's row lock"),
            0
        );
        row_lock.rollback().await.expect("release row lock");
        assert!(!cancel.is_cancelled());

        state
            .db
            .archive_community_owned_by(&host, &owner.public_key().to_hex(), "protected.example")
            .await
            .expect("archive community")
            .expect("owned community");
        assert_eq!(state.revalidate_live_communities().await, 1);
        assert!(cancel.is_cancelled());
        assert_eq!(
            *reason.borrow(),
            Some(CommunityDisconnectReason::CommunityArchived)
        );

        drop(guard);

        // A community fenced for deletion but never archived must not pass the
        // unlocked active pre-check; it closes with the deletion reason.
        let deleted_host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &deleted_host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let deleted_community = state
            .db
            .lookup_community_by_host(&deleted_host)
            .await
            .expect("active admission lookup")
            .expect("active community")
            .id;
        let deleted_cancel = CancellationToken::new();
        let deleted_control = CommunityConnectionControl::new(deleted_cancel.clone());
        let deleted_reason = deleted_control.disconnect_reason();
        let _deleted_guard = state.community_connections.register(
            Uuid::new_v4(),
            deleted_community,
            deleted_control,
        );
        let mut tx = state
            .db
            .pool()
            .begin()
            .await
            .expect("begin deletion fixture");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
             set_config('buzz.deletion_fence_generation', '0', true)",
        )
        .bind(deleted_community.to_string())
        .execute(&mut *tx)
        .await
        .expect("enter deletion executor scope");
        sqlx::query("UPDATE communities SET deletion_state = 'fenced' WHERE id = $1")
            .bind(deleted_community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("fence community deletion");
        tx.commit().await.expect("commit deletion fixture");
        assert_eq!(state.revalidate_live_communities().await, 1);
        assert!(deleted_cancel.is_cancelled());
        assert_eq!(
            *deleted_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
    }

    /// The connection-control consumer serves ban disconnects and archive
    /// disconnects from one receiver. An archive disconnect waits on the
    /// community row lock, so a pubkey disconnect queued behind it must still
    /// close its socket promptly, and the archive disconnect must still land
    /// once the lock is released.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn conn_control_consumer_does_not_queue_pubkey_disconnects_behind_a_locked_archive() {
        use crate::state::{CommunityConnectionControl, CommunityDisconnectReason};
        use buzz_pubsub::conn_control::{ConnControl, ScopedConnControl};
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("active admission lookup")
            .expect("active community")
            .id;
        let community_control = CommunityConnectionControl::new(CancellationToken::new());
        let community_reason = community_control.disconnect_reason();
        let _community_guard = state.community_connections.register(
            Uuid::new_v4(),
            community,
            community_control.clone(),
        );
        let banned = [7u8; 32];
        let banned_control = CommunityConnectionControl::new(CancellationToken::new());
        banned_control.bind_pubkey(banned);
        let banned_reason = banned_control.disconnect_reason();
        let _banned_guard =
            state
                .community_connections
                .register(Uuid::new_v4(), community, banned_control.clone());

        let archived = state
            .db
            .archive_community_owned_by(&host, &owner.public_key().to_hex(), "protected.example")
            .await
            .expect("archive community")
            .expect("owned community");
        let mut row_lock = state.db.pool().begin().await.expect("begin row lock");
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE")
            .bind(community.as_uuid())
            .execute(&mut *row_lock)
            .await
            .expect("hold the community row lock");

        let (tx, rx) = tokio::sync::broadcast::channel(8);
        let consumer = tokio::spawn(Arc::clone(&state).run_conn_control_consumer(rx));
        tx.send(ScopedConnControl {
            community_id: community,
            command: ConnControl::DisconnectCommunity {
                archived_at: Some(archived.archived_at),
            },
        })
        .expect("send community disconnect");
        tx.send(ScopedConnControl {
            community_id: community,
            command: ConnControl::DisconnectPubkey {
                pubkey: banned.to_vec(),
                event_id: "ban".to_string(),
                reason: "blocked: you are banned from this community".to_string(),
                unowned_only: false,
            },
        })
        .expect("send pubkey disconnect");

        tokio::time::timeout(
            Duration::from_secs(1),
            banned_control.cancellation_token().cancelled(),
        )
        .await
        .expect("a pubkey disconnect must not wait behind a locked community disconnect");
        assert_eq!(
            *banned_reason.borrow(),
            Some(CommunityDisconnectReason::AccessRevoked)
        );
        assert!(
            !community_control.cancellation_token().is_cancelled(),
            "the community disconnect must wait for the row lock"
        );

        row_lock.rollback().await.expect("release row lock");
        tokio::time::timeout(
            Duration::from_secs(5),
            community_control.cancellation_token().cancelled(),
        )
        .await
        .expect("the community disconnect must land once the row lock is released");
        assert_eq!(
            *community_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityArchived)
        );

        drop(tx);
        tokio::time::timeout(Duration::from_secs(5), consumer)
            .await
            .expect("the consumer must stop when the broadcast closes")
            .expect("consumer task");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unarchive_restores_admission_and_is_idempotent_without_changing_ownership() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let outsider = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let owner_hex = owner.public_key().to_hex();
        let archived = state
            .db
            .archive_community_owned_by(&host, &owner_hex, "protected.example")
            .await
            .expect("archive community")
            .expect("owned community");
        assert!(state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("archived admission lookup")
            .is_none());

        let request = |host: &str, owner_pubkey: String| {
            serde_json::json!({
                "host": host,
                "owner_pubkey": owner_pubkey,
            })
            .to_string()
        };
        let wrong_owner = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/unarchive",
            Some(request(&host, outsider.public_key().to_hex())),
        )
        .await;
        assert_eq!(wrong_owner.status(), StatusCode::NOT_FOUND);
        assert_eq!(read_json(wrong_owner).await["error"], "community not found");
        let unknown = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/unarchive",
            Some(request("missing.example", owner_hex.clone())),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(read_json(unknown).await["error"], "community not found");

        for attempt in 0..2 {
            let response = signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/unarchive",
                Some(request(&host, owner_hex.clone())),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "attempt {attempt}");
            let json = read_json(response).await;
            assert_eq!(json["community_id"], archived.id.to_string());
            assert_eq!(json["host"], host);
            assert!(json["archived_at"].is_null());
            assert_eq!(json["status"], "active");
        }

        let active = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("restored admission lookup")
            .expect("active community");
        assert_eq!(active.id, archived.id);
        let owner_member = state
            .db
            .get_relay_member(active.id, &owner_hex)
            .await
            .expect("owner lookup")
            .expect("owner remains");
        assert_eq!(owner_member.role, "owner");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unarchive_pending_deletion_returns_stable_conflict_without_mutation() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        archive_for_owner_deletion(&state, &host, &owner).await;
        let request_id = Uuid::new_v4();
        assert_eq!(
            signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(owner_delete_body(&host, &owner, request_id)),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect lifecycle assertion pool");
        let before_lifecycle: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community.id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read lifecycle before unarchive conflict");
        let before_request = state
            .db
            .deletion_store()
            .get(request_id)
            .await
            .expect("read deletion request before unarchive conflict");
        let owner_hex = owner.public_key().to_hex();
        let before_owner_role = state
            .db
            .get_relay_member(community.id, &owner_hex)
            .await
            .expect("read owner before unarchive conflict")
            .expect("owner exists before unarchive conflict")
            .role;

        let body = serde_json::json!({
            "host": host,
            "owner_pubkey": owner_hex,
        })
        .to_string();
        let response = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/unarchive",
            Some(body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = read_json(response).await;
        assert_eq!(json["code"], "deletion_lifecycle_conflict");
        assert_eq!(json["error"], "community deletion is pending");

        let after_lifecycle: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community.id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read lifecycle after unarchive conflict");
        assert_eq!(after_lifecycle, before_lifecycle);
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("read deletion request after unarchive conflict"),
            before_request
        );
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &owner.public_key().to_hex())
                .await
                .expect("read owner after unarchive conflict")
                .expect("owner exists after unarchive conflict")
                .role,
            before_owner_role
        );
        assert!(state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("active lookup after unarchive conflict")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn archive_publish_failure_is_retryable_and_preserves_timestamp() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let owner_hex = owner.public_key().to_hex();
        let create_body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": owner_hex,
            "create_only": true,
        })
        .to_string();
        let create_url = format!("http://{INGRESS_HOST}/operator/communities");
        let create_auth =
            nip98_auth_header(&operator, &create_url, "POST", Some(create_body.as_bytes()));
        let create_response = build_router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, create_auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .expect("create request"),
            )
            .await
            .expect("create response");
        assert_eq!(create_response.status(), StatusCode::OK);

        let archive_body = serde_json::json!({
            "host": host,
            "owner_pubkey": owner.public_key().to_hex(),
        })
        .to_string();
        let archive_url = format!("http://{INGRESS_HOST}/operator/communities/archive");
        let archive_once = |state: Arc<AppState>| {
            let auth = nip98_auth_header(
                &operator,
                &archive_url,
                "POST",
                Some(archive_body.as_bytes()),
            );
            let body = archive_body.clone();
            async move {
                build_router(state)
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/operator/communities/archive")
                            .header(header::HOST, INGRESS_HOST)
                            .header(header::AUTHORIZATION, auth)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body))
                            .expect("archive request"),
                    )
                    .await
                    .expect("archive response")
            }
        };

        let first = archive_once(Arc::clone(&state)).await;
        assert_eq!(first.status(), StatusCode::SERVICE_UNAVAILABLE);
        let first_json = read_json(first).await;
        assert_eq!(first_json["status"], "archived");
        assert_eq!(first_json["propagation"], "pending");
        let first_archived_at = first_json["archived_at"].clone();
        assert!(!first_archived_at.is_null());
        assert!(state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("active lookup")
            .is_none());

        assert_eq!(
            state
                .community_disconnect_publish_attempts
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let second = archive_once(Arc::clone(&state)).await;
        assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
        let second_json = read_json(second).await;
        assert_eq!(second_json["archived_at"], first_archived_at);
        assert_eq!(
            state
                .community_disconnect_publish_attempts
                .load(std::sync::atomic::Ordering::Relaxed),
            2,
            "idempotent archive retry must republish the disconnect"
        );

        let owned = state
            .db
            .list_communities_owned_by(&owner.public_key().to_hex())
            .await
            .expect("owned communities");
        let row = owned
            .communities
            .iter()
            .find(|row| row.host == host)
            .expect("archived row");
        assert_eq!(
            serde_json::to_value(row.archived_at).expect("timestamp JSON"),
            first_archived_at
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn happy_path_create_returns_created_and_bootstraps_owner() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let response = provision_community(state.clone(), &operator, &host, &owner).await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(json.get("status").and_then(Value::as_str), Some("created"));
        assert_eq!(
            json.get("host").and_then(Value::as_str),
            Some(host.as_str())
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        let owner_hex = owner.public_key().to_hex();
        let member = state
            .db
            .get_relay_member(community.id, &owner_hex)
            .await
            .expect("lookup owner role")
            .expect("owner member exists");
        assert_eq!(member.role, "owner");

        assert_snapshot_roles(&state, community.id, &[(&owner_hex, "owner")]).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn archived_legacy_owner_convergence_returns_conflict_without_membership_change() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let replacement = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(state.clone(), &operator, &host, &owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        archive_for_owner_deletion(&state, &host, &owner).await;

        let body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": replacement.public_key().to_hex(),
            "create_only": false,
        })
        .to_string();
        let response = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities",
            Some(body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            read_json(response).await["error"],
            "owner_conflict: community must be active to rotate ownership"
        );
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &owner.public_key().to_hex())
                .await
                .expect("get owner")
                .expect("owner exists")
                .role,
            "owner"
        );
        assert!(state
            .db
            .get_relay_member(community.id, &replacement.public_key().to_hex())
            .await
            .expect("get replacement")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fresh_host_at_owner_limit_returns_limit_reached_conflict() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };

        for _ in 0..buzz_db::relay_members::MAX_COMMUNITIES_PER_OWNER {
            let host = format!("community-{}.example", Uuid::new_v4().simple());
            assert_eq!(
                provision_community(state.clone(), &operator, &host, &owner)
                    .await
                    .status(),
                StatusCode::OK
            );
        }

        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let response = provision_community(state.clone(), &operator, &host, &owner).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = read_json(response).await;
        assert_eq!(json["code"], "limit_reached");
        assert!(json["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("limit_reached:")));
        assert!(state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("look up rejected fresh host")
            .is_none());
    }

    /// Happy path: POST /operator/communities/transfer swaps ownership, demotes
    /// the old owner to `member`, and publishes a NIP-43 snapshot reflecting the
    /// new roles.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn happy_path_transfer_swaps_owner_and_demotes_old_to_member() {
        let operator = Keys::generate();
        let initial_owner = Keys::generate();
        let new_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };

        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let create_response =
            provision_community(state.clone(), &operator, &host, &initial_owner).await;
        assert_eq!(create_response.status(), StatusCode::OK);

        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        let community_id = community.id.to_string();
        let initial_owner_hex = initial_owner.public_key().to_hex();
        let new_owner_hex = new_owner.public_key().to_hex();

        let transfer_body = serde_json::json!({
            "community_id": community_id,
            "new_owner_pubkey": new_owner_hex,
            "expected_owner_pubkey": initial_owner_hex,
        })
        .to_string();
        let response = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/transfer",
            Some(transfer_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(
            json.get("status").and_then(Value::as_str),
            Some("transferred")
        );
        assert_eq!(
            json.get("new_owner_pubkey").and_then(Value::as_str),
            Some(new_owner_hex.as_str())
        );
        assert_eq!(
            json.get("previous_owner").and_then(Value::as_str),
            Some(initial_owner_hex.as_str())
        );

        // New owner is owner.
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &new_owner_hex)
                .await
                .expect("get new owner")
                .expect("new owner exists")
                .role,
            "owner"
        );
        // Old owner is member (not admin).
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &initial_owner_hex)
                .await
                .expect("get old owner")
                .expect("old owner exists")
                .role,
            "member"
        );

        assert_snapshot_roles(
            &state,
            community.id,
            &[(&new_owner_hex, "owner"), (&initial_owner_hex, "member")],
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_to_owner_at_limit_returns_coded_limit_reached_without_mutation() {
        let operator = Keys::generate();
        let initial_owner = Keys::generate();
        let transferee = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };

        for _ in 0..buzz_db::relay_members::MAX_COMMUNITIES_PER_OWNER {
            let host = format!("community-{}.example", Uuid::new_v4().simple());
            assert_eq!(
                provision_community(state.clone(), &operator, &host, &transferee)
                    .await
                    .status(),
                StatusCode::OK
            );
        }
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(state.clone(), &operator, &host, &initial_owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        let initial_owner_hex = initial_owner.public_key().to_hex();
        let transferee_hex = transferee.public_key().to_hex();

        let transfer_body = serde_json::json!({
            "community_id": community.id.to_string(),
            "new_owner_pubkey": transferee_hex,
            "expected_owner_pubkey": initial_owner_hex,
        })
        .to_string();
        let response = signed_operator_request(
            state.clone(),
            &operator,
            "POST",
            "/operator/communities/transfer",
            Some(transfer_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = read_json(response).await;
        assert_eq!(json["code"], "limit_reached");
        assert!(json["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("limit_reached:")));
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &initial_owner_hex)
                .await
                .expect("get initial owner")
                .expect("initial owner exists")
                .role,
            "owner"
        );
        assert!(state
            .db
            .get_relay_member(community.id, &transferee_hex)
            .await
            .expect("get transferee")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_pending_deletion_returns_stable_conflict_without_mutation() {
        let operator = Keys::generate();
        let initial_owner = Keys::generate();
        let new_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            provision_community(Arc::clone(&state), &operator, &host, &initial_owner)
                .await
                .status(),
            StatusCode::OK
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        archive_for_owner_deletion(&state, &host, &initial_owner).await;
        let request_id = Uuid::new_v4();
        assert_eq!(
            signed_operator_request(
                Arc::clone(&state),
                &operator,
                "POST",
                "/operator/communities/delete",
                Some(owner_delete_body(&host, &initial_owner, request_id)),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect lifecycle assertion pool");
        let before_lifecycle: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community.id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read lifecycle before transfer conflict");
        let before_request = state
            .db
            .deletion_store()
            .get(request_id)
            .await
            .expect("read deletion request before transfer conflict");
        let initial_owner_hex = initial_owner.public_key().to_hex();
        let new_owner_hex = new_owner.public_key().to_hex();
        let before_owner_role = state
            .db
            .get_relay_member(community.id, &initial_owner_hex)
            .await
            .expect("read owner before transfer conflict")
            .expect("owner exists before transfer conflict")
            .role;
        assert!(state
            .db
            .get_relay_member(community.id, &new_owner_hex)
            .await
            .expect("read transferee before transfer conflict")
            .is_none());

        let body = serde_json::json!({
            "community_id": community.id.to_string(),
            "new_owner_pubkey": new_owner_hex,
            "expected_owner_pubkey": initial_owner_hex,
        })
        .to_string();
        let response = signed_operator_request(
            Arc::clone(&state),
            &operator,
            "POST",
            "/operator/communities/transfer",
            Some(body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = read_json(response).await;
        assert_eq!(json["code"], "deletion_lifecycle_conflict");
        assert_eq!(json["error"], "community deletion is pending");

        let after_lifecycle: (
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT archived_at, deletion_state, deleted_at FROM communities WHERE id = $1",
        )
        .bind(community.id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read lifecycle after transfer conflict");
        assert_eq!(after_lifecycle, before_lifecycle);
        assert_eq!(
            state
                .db
                .deletion_store()
                .get(request_id)
                .await
                .expect("read deletion request after transfer conflict"),
            before_request
        );
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &initial_owner.public_key().to_hex())
                .await
                .expect("read owner after transfer conflict")
                .expect("owner exists after transfer conflict")
                .role,
            before_owner_role
        );
        assert!(state
            .db
            .get_relay_member(community.id, &new_owner.public_key().to_hex())
            .await
            .expect("read transferee after transfer conflict")
            .is_none());
    }

    /// Transfer with an invalid community_id returns 400.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_with_invalid_community_id_returns_400() {
        let operator = Keys::generate();
        let new_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = serde_json::json!({
            "community_id": "not-a-uuid",
            "new_owner_pubkey": new_owner.public_key().to_hex(),
            "expected_owner_pubkey": new_owner.public_key().to_hex(),
        })
        .to_string();
        let response = signed_operator_request(
            state,
            &operator,
            "POST",
            "/operator/communities/transfer",
            Some(body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Transfer with an invalid new_owner_pubkey returns 400.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_with_invalid_pubkey_returns_400() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = serde_json::json!({
            "community_id": Uuid::new_v4().to_string(),
            "new_owner_pubkey": "not-a-pubkey",
            "expected_owner_pubkey": "not-a-pubkey",
        })
        .to_string();
        let response = signed_operator_request(
            state,
            &operator,
            "POST",
            "/operator/communities/transfer",
            Some(body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Regression for the RELAY_OPERATOR_API_ORIGIN decoupling: with the
    /// operator allowlist set but no origin configured (the shape an
    /// admin-console-only operator boots in), the provisioning endpoints must
    /// fail closed with a clean 500 — never a panic, and never a silent
    /// success. This exercises the request-time guard that replaced the boot
    /// hard-error. It uses a lazy pool and needs no Postgres, because the
    /// origin check in `authorize_operator_request` runs before any DB access.
    #[tokio::test]
    async fn provisioning_fails_closed_when_origin_unset_but_pubkeys_set() {
        let operator = Keys::generate();

        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_operator_pubkeys = vec![operator.public_key().to_hex()];
        config.relay_operator_api_origin = None;

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        let state = Arc::new(state);

        let response =
            provision_community(state, &operator, "acme.example", &Keys::generate()).await;

        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "provisioning must reject fail-closed when the operator API origin is unset"
        );
        let body = read_json(response).await;
        assert_eq!(body["error"], "internal server error");
    }
}
