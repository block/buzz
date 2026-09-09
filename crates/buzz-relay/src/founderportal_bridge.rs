//! Trusted FounderPortal-to-Buzz V1 collaboration bridge executor.
//!
//! This is an internal service seam, not a browser endpoint. FounderPortal
//! authenticates and authorizes business membership before enqueueing tenant
//! and actor coordinates. Buzz resolves every community and pubkey through its
//! own bridge mappings.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use buzz_db::founderportal_bridge_sync::ClaimedSyncOperation;

use crate::state::AppState;

/// Execute one leased durable sync operation.
///
/// Returns a stable non-sensitive classification. Callers persist it as
/// retryable or terminal according to the prefix.
pub async fn execute(
    state: &Arc<AppState>,
    operation: &ClaimedSyncOperation,
) -> Result<(), String> {
    if !state.config.require_relay_membership {
        return Err("terminal:relay_membership_must_be_required".to_string());
    }
    match operation.operation_type.as_str() {
        "ensure_community" => {
            buzz_db::founderportal_bridge_operations::ensure_community(
                state.db.writer_pool(),
                &operation.tenant_id,
            )
            .await
            .map(|_| ())
            .map_err(|_| "retryable:ensure_community_failed".to_string())
        }
        "ensure_member" => {
            let actor_type = operation.actor_type.as_deref().ok_or("terminal:missing_actor")?;
            let actor_id = operation.actor_id.as_deref().ok_or("terminal:missing_actor")?;
            buzz_db::founderportal_bridge_operations::ensure_member(
                state.db.writer_pool(), &operation.tenant_id, actor_type, actor_id,
            )
            .await
            .map(|_| ())
            .map_err(|_| "retryable:ensure_member_failed".to_string())
        }
        "revoke_member" => revoke_member(state, operation).await,
        "archive_community" => archive_community(state, operation).await,
        _ => Err("terminal:unsupported_operation".to_string()),
    }
}

async fn revoke_member(state: &Arc<AppState>, operation: &ClaimedSyncOperation) -> Result<(), String> {
    let actor_type = operation.actor_type.as_deref().ok_or("terminal:missing_actor")?;
    let actor_id = operation.actor_id.as_deref().ok_or("terminal:missing_actor")?;
    // 1. Durable relay_members deletion.
    let (community_id, pubkey, _) = buzz_db::founderportal_bridge_operations::revoke_member_durable(
        state.db.writer_pool(), &operation.tenant_id, actor_type, actor_id,
    ).await.map_err(|_| "retryable:durable_removal_failed".to_string())?;
    let tenant = TenantContext::resolved(community_id, "founderportal-bridge.internal");
    // 2. Invalidate authorization caches.
    state.invalidate_all_accessible_channels(&tenant);
    let bytes = hex::decode(&pubkey).map_err(|_| "terminal:invalid_mapped_pubkey".to_string())?;
    // 3 + 4. Local disconnect followed by awaited cluster publication.
    state.disconnect_revoked_pubkey_clusterwide(&tenant, &bytes, &operation.operation_key)
        .await.map_err(|_| "retryable:disconnect_publication_failed".to_string())?;
    // 5. Confirm durable absence (the reconnect fence).
    if !buzz_db::founderportal_bridge_operations::confirm_member_absent(state.db.writer_pool(), community_id, &pubkey)
        .await.map_err(|_| "retryable:absence_confirmation_failed".to_string())? {
        return Err("retryable:member_still_present".to_string());
    }
    Ok(())
}

async fn archive_community(state: &Arc<AppState>, operation: &ClaimedSyncOperation) -> Result<(), String> {
    let (community_id, _) = buzz_db::founderportal_bridge_operations::archive_community(
        state.db.writer_pool(), &operation.tenant_id,
    ).await.map_err(|_| "retryable:archive_failed".to_string())?;
    let tenant = TenantContext::resolved(community_id, "founderportal-bridge.internal");
    state.disconnect_community_clusterwide(&tenant).await
        .map_err(|_| "retryable:archive_disconnect_publication_failed".to_string())?;
    Ok(())
}


