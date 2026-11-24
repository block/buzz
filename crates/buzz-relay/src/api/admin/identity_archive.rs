//! Operator archive and unarchive for NIP-IA visibility.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use buzz_db::operator_identity_archive::{ArchiveChange, ArchiveInventoryError};
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum ArchiveOperation {
    Archive,
    Unarchive,
}

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
    let principal = authorize_write(&state, &slot, &headers, &method, &uri, Some(&body))
        .await?
        .operator(&slot)?;
    execute(
        &state,
        principal.pubkey,
        ArchiveOperation::Archive,
        target,
        &q.community_host,
        &body,
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

fn is_contention(e: &buzz_db::DbError) -> bool {
    matches!(
        e,
        buzz_db::DbError::Sqlx(sqlx::Error::Database(db))
            if matches!(db.code().as_deref(), Some("55P03" | "57014"))
    )
}

fn map_db_error(error: buzz_db::DbError) -> ApiError {
    if is_contention(&error) {
        ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "retry_later",
            message: "community is busy; nothing was applied, repeat the same request".into(),
        }
    } else {
        error.into()
    }
}

fn map_inventory_error(error: ArchiveInventoryError) -> ApiError {
    let (status, code) = match error {
        ArchiveInventoryError::Changed => (StatusCode::CONFLICT, "inventory_changed"),
        ArchiveInventoryError::TooLarge => {
            (StatusCode::UNPROCESSABLE_ENTITY, "inventory_too_large")
        }
        ArchiveInventoryError::Db(error) => return map_db_error(error),
    };
    ApiError {
        status,
        code,
        message: error.to_string(),
    }
}

/// Apply one visibility operation for an already-authorized Operator.
/// The caller verified the signature over the exact community query and body.
pub(crate) async fn execute(
    state: &Arc<AppState>,
    actor: [u8; 32],
    operation: ArchiveOperation,
    target: String,
    community_host: &str,
    body: &[u8],
) -> Reply {
    if !state.config.admin_identity_archive {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "identity_archive_disabled",
            message: "Operator identity archive is not enabled on this relay".into(),
        });
    }
    if !is_lower_hex_64(&target) {
        return Err(ApiError::bad_request(
            "invalid_pubkey",
            "pubkey must be 64-character lowercase hex",
        ));
    }
    let actor_hex = hex::encode(actor);
    let tenant = tenant_for_host(state, community_host).await?;
    let (request_id, reason, replaced_by, changes) = match operation {
        ArchiveOperation::Archive => {
            let body: ArchiveBody = serde_json::from_slice(body).map_err(|_| invalid_body())?;
            if let Some(replacement) = &body.replaced_by {
                if !is_lower_hex_64(replacement) || replacement == &target {
                    return Err(ApiError::bad_request(
                        "invalid_replaced_by",
                        "replacedBy must be a different 64-character lowercase hex key",
                    ));
                }
            }
            if !is_lower_hex_64(&body.inventory_version) {
                return Err(invalid_body());
            }
            let reason = validate_reason(body.reason)?;
            let mut key = [0; 32];
            hex::decode_to_slice(&target, &mut key).map_err(|_| ApiError::internal())?;
            refuse_staff_target(state, &key).await?;
            let changes = state
                .db
                .archive_operator_identities(
                    tenant.community(),
                    &key,
                    &actor_hex,
                    &body.inventory_version,
                    reason.as_deref(),
                    body.replaced_by.as_deref(),
                )
                .await
                .map_err(map_inventory_error)?;
            (body.request_id, reason, body.replaced_by, changes)
        }
        ArchiveOperation::Unarchive => {
            let body: UnarchiveBody = serde_json::from_slice(body).map_err(|_| invalid_body())?;
            let reason = validate_reason(body.reason)?;
            let changed = state
                .db
                .unarchive(tenant.community(), &target)
                .await
                .map_err(map_db_error)?;
            (
                body.request_id,
                reason,
                None,
                vec![ArchiveChange {
                    pubkey: target.clone(),
                    changed,
                }],
            )
        }
    };

    for change in changes.iter().filter(|change| change.changed) {
        let result = match operation {
            ArchiveOperation::Archive => {
                let replacement = if change.pubkey == target {
                    replaced_by.as_deref()
                } else {
                    None
                };
                publish_nipia_archived(
                    &tenant,
                    state,
                    &change.pubkey,
                    "relay",
                    &actor_hex,
                    "",
                    "",
                    reason.as_deref(),
                    replacement,
                )
                .await
            }
            ArchiveOperation::Unarchive => {
                publish_nipia_unarchived(
                    &tenant,
                    state,
                    &change.pubkey,
                    "relay",
                    &actor_hex,
                    "",
                    "",
                    reason.as_deref(),
                )
                .await
            }
        };
        if let Err(error) = result {
            tracing::warn!(%request_id, pubkey = %change.pubkey, ?operation, %error, "Operator NIP-IA delta failed");
        }
    }

    // Publish from current DB state even on a no-op. The previous call may
    // have committed its visibility change before failing to publish.
    let (status, visibility) = match publish_nipia_archival_list(&tenant, state).await {
        Ok(()) => (StatusCode::OK, "confirmed"),
        Err(error) => {
            tracing::warn!(%request_id, ?operation, %error, "Operator NIP-IA snapshot failed");
            (StatusCode::ACCEPTED, "pending")
        }
    };
    tracing::info!(
        %request_id, ?operation, community = %tenant.community(), %actor_hex, %target,
        changed = changes.iter().filter(|change| change.changed).count(), visibility,
        "Operator identity visibility request processed"
    );
    let (operation, archived) = match operation {
        ArchiveOperation::Archive => ("archive", true),
        ArchiveOperation::Unarchive => ("unarchive", false),
    };
    let identities: Vec<_> = changes
        .into_iter()
        .map(|change| {
            serde_json::json!({
                "role": if change.pubkey == target { "root" } else { "agent" },
                "pubkey": change.pubkey,
                "archived": archived,
                "changed": change.changed,
            })
        })
        .collect();
    Ok((
        status,
        Json(serde_json::json!({
            "requestId": request_id,
            "operation": operation,
            "communityHost": tenant.host(),
            "pubkey": target,
            "identities": identities,
            "stages": { "visibility": visibility },
        })),
    ))
}

#[cfg(test)]
#[path = "identity_archive_postgres_tests.rs"]
mod postgres_tests;
