//! Operator identity retirement: archive an exact key and its verified agents,
//! hold their automation, restore visibility, and release the hold.
//!
//! See `docs/operator-identity-retirement.md`. Order: authorize (Operator) →
//! feature gate → parse → tenant → staff guard (archive) → one transaction in
//! `buzz-db` → publish NIP-IA → mark published. A replay returns the stored
//! result and only republishes the canonical snapshot if it was not yet
//! published, so a retry never re-applies intent or emits a stale delta.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use buzz_db::identity_retirement::{
    RetirementError, RetirementOperation, RetirementOutcome, RetirementPayload, RetirementRecord,
    RetirementRequest,
};
use serde::Deserialize;
use uuid::Uuid;

use super::auth::authorize_write;
use super::direct::refuse_staff_target;
use super::error::ApiError;
use super::{tenant_for_host, CommunityQuery};
use crate::handlers::side_effects::{
    publish_nipia_archival_list, publish_nipia_archived, publish_nipia_unarchived,
};
use crate::state::AppState;

const REASON_MAX: usize = 64;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveBody {
    request_id: Uuid,
    reason: Option<String>,
    replaced_by: Option<String>,
    inventory_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnarchiveBody {
    request_id: Uuid,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseBody {
    request_id: Uuid,
}

type Reply = Result<(StatusCode, Json<serde_json::Value>), ApiError>;

/// `POST /members/{pubkey}/archive`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
pub(super) async fn archive(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Reply {
    run(
        state,
        slot,
        method,
        uri,
        headers,
        RetirementOperation::Archive,
        target,
        q,
        body,
    )
    .await
}

/// `DELETE /members/{pubkey}/archive`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
pub(super) async fn unarchive(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Reply {
    run(
        state,
        slot,
        method,
        uri,
        headers,
        RetirementOperation::Unarchive,
        target,
        q,
        body,
    )
    .await
}

/// `DELETE /members/{pubkey}/execution-hold`
#[allow(clippy::too_many_arguments)] // one Axum extractor per argument
pub(super) async fn release(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Reply {
    run(
        state,
        slot,
        method,
        uri,
        headers,
        RetirementOperation::Release,
        target,
        q,
        body,
    )
    .await
}

fn invalid_body() -> ApiError {
    ApiError::bad_request("invalid_body", "invalid JSON body")
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn validate_reason(reason: Option<String>) -> Result<Option<String>, ApiError> {
    match reason {
        Some(r)
            if r.is_empty()
                || r.len() > REASON_MAX
                || !r
                    .bytes()
                    .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-')) =>
        {
            Err(ApiError::bad_request(
                "invalid_reason",
                "reason must be 1-64 characters of a-z, 0-9 and -",
            ))
        }
        other => Ok(other),
    }
}

fn parse_body(
    operation: RetirementOperation,
    target_hex: &str,
    body: &[u8],
) -> Result<(Uuid, RetirementPayload), ApiError> {
    match operation {
        RetirementOperation::Archive => {
            let b: ArchiveBody = serde_json::from_slice(body).map_err(|_| invalid_body())?;
            if let Some(r) = &b.replaced_by {
                if !is_lower_hex_64(r) || r == target_hex {
                    return Err(ApiError::bad_request(
                        "invalid_replaced_by",
                        "replacedBy must be a different 64-character lowercase hex key",
                    ));
                }
            }
            if !is_lower_hex_64(&b.inventory_version) {
                return Err(invalid_body());
            }
            Ok((
                b.request_id,
                RetirementPayload {
                    reason: validate_reason(b.reason)?,
                    replaced_by: b.replaced_by,
                    inventory_version: Some(b.inventory_version),
                },
            ))
        }
        RetirementOperation::Unarchive => {
            let b: UnarchiveBody = serde_json::from_slice(body).map_err(|_| invalid_body())?;
            Ok((
                b.request_id,
                RetirementPayload {
                    reason: validate_reason(b.reason)?,
                    replaced_by: None,
                    inventory_version: None,
                },
            ))
        }
        RetirementOperation::Release => {
            let b: ReleaseBody = serde_json::from_slice(body).map_err(|_| invalid_body())?;
            Ok((
                b.request_id,
                RetirementPayload {
                    reason: None,
                    replaced_by: None,
                    inventory_version: None,
                },
            ))
        }
    }
}

fn is_contention(e: &buzz_db::DbError) -> bool {
    matches!(
        e,
        buzz_db::DbError::Sqlx(sqlx::Error::Database(db))
            if matches!(db.code().as_deref(), Some("55P03" | "57014"))
    )
}

fn map_error(err: RetirementError) -> ApiError {
    let (status, code) = match err {
        RetirementError::RequestIdConflict => (StatusCode::CONFLICT, "request_id_conflict"),
        RetirementError::InventoryChanged => (StatusCode::CONFLICT, "inventory_changed"),
        RetirementError::InventoryTooLarge => {
            (StatusCode::UNPROCESSABLE_ENTITY, "inventory_too_large")
        }
        RetirementError::MissingInventoryVersion => return invalid_body(),
        // Lock or statement timeout: the transaction rolled back, nothing
        // applied. Repeating the same request is safe.
        RetirementError::Db(e) if is_contention(&e) => {
            return ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "retry_later",
                message: "community is busy; nothing was applied, repeat the same request".into(),
            }
        }
        RetirementError::Db(e) => return e.into(),
    };
    ApiError {
        status,
        code,
        message: err.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    state: Arc<AppState>,
    slot: crate::access::AccessSlot,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    operation: RetirementOperation,
    target_hex: String,
    q: CommunityQuery,
    body: Bytes,
) -> Reply {
    let principal = authorize_write(&state, &slot, &headers, &method, &uri, Some(&body))
        .await?
        .operator(&slot)?;
    if !state.config.admin_identity_retirement {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "identity_retirement_disabled",
            message: "identity retirement is not enabled on this relay".into(),
        });
    }
    if !is_lower_hex_64(&target_hex) {
        return Err(ApiError::bad_request(
            "invalid_pubkey",
            "pubkey must be 64-character lowercase hex",
        ));
    }
    let mut target = [0u8; 32];
    hex::decode_to_slice(&target_hex, &mut target).map_err(|_| ApiError::internal())?;
    let (request_id, payload) = parse_body(operation, &target_hex, &body)?;
    let tenant = tenant_for_host(&state, &q.community_host).await?;
    if operation == RetirementOperation::Archive {
        refuse_staff_target(&state, &target).await?;
    }
    let request = RetirementRequest {
        community: tenant.community(),
        request_id,
        operation,
        target,
        actor: principal.pubkey,
        payload,
    };
    let outcome = state
        .db
        .apply_identity_retirement(&request)
        .await
        .map_err(map_error)?;

    let (record, replayed) = match outcome {
        RetirementOutcome::Replayed(record) => (record, true),
        RetirementOutcome::Applied {
            record,
            archived,
            unarchived,
        } => {
            tracing::info!(
                action_id = %record.action_id,
                operation = ?operation,
                target = %target_hex,
                archived = archived.len(),
                unarchived = unarchived.len(),
                "identity retirement action applied"
            );
            publish_deltas(&tenant, &state, &request, &archived, &unarchived).await;
            (record, false)
        }
    };
    let published = record.published || publish(&tenant, &state, &record).await;
    Ok(respond(&tenant, record, replayed, published))
}

/// Emit one NIP-IA delta per key whose archive state this action changed.
/// Deltas are history; the snapshot published next is the state clients
/// converge on, so a failed delta is logged and not retried (a retry could
/// emit it after a later action reversed it).
async fn publish_deltas(
    tenant: &buzz_core::TenantContext,
    state: &Arc<AppState>,
    request: &RetirementRequest,
    archived: &[String],
    unarchived: &[String],
) {
    let actor = hex::encode(request.actor);
    let reason = request.payload.reason.as_deref();
    let root = hex::encode(request.target);
    for key in archived {
        let replaced_by = (*key == root)
            .then_some(request.payload.replaced_by.as_deref())
            .flatten();
        if let Err(e) = publish_nipia_archived(
            tenant,
            state,
            key,
            "relay",
            &actor,
            "",
            "",
            reason,
            replaced_by,
        )
        .await
        {
            tracing::warn!(target = %key, "NIP-IA archived delta failed: {e:#}");
        }
    }
    for key in unarchived {
        if let Err(e) =
            publish_nipia_unarchived(tenant, state, key, "relay", &actor, "", "", reason).await
        {
            tracing::warn!(target = %key, "NIP-IA unarchived delta failed: {e:#}");
        }
    }
}

/// Publish the canonical snapshot and record it. Release does not change
/// visibility, so it records publication without publishing.
async fn publish(
    tenant: &buzz_core::TenantContext,
    state: &Arc<AppState>,
    record: &RetirementRecord,
) -> bool {
    if record.operation != RetirementOperation::Release {
        if let Err(e) = publish_nipia_archival_list(tenant, state).await {
            tracing::warn!(action_id = %record.action_id, "NIP-IA snapshot failed: {e:#}");
            return false;
        }
    }
    match state
        .db
        .mark_identity_retirement_published(tenant.community(), record.action_id)
        .await
    {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(action_id = %record.action_id, "recording publication failed: {e}");
            false
        }
    }
}

fn respond(
    tenant: &buzz_core::TenantContext,
    record: RetirementRecord,
    replayed: bool,
    published: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    let visibility = match (record.operation, published) {
        (RetirementOperation::Release, _) => "unchanged",
        (_, true) => "confirmed",
        (_, false) => "pending",
    };
    let mut stages = serde_json::to_value(&record.result.stages).unwrap_or_default();
    stages["visibility"] = visibility.into();
    let body = serde_json::json!({
        "actionId": record.action_id,
        "requestId": record.request_id,
        "operation": record.operation,
        "replayed": replayed,
        "communityHost": tenant.host(),
        "pubkey": record.target,
        "identities": record.result.identities,
        "workflows": record.result.workflows,
        "runs": record.result.runs,
        "stages": stages,
    });
    let status = if published {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    (status, Json(body))
}

#[cfg(test)]
#[path = "retirement_postgres_tests.rs"]
mod postgres_tests;
