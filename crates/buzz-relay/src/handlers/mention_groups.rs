//! NIP-MG commands: admission, atomic persistence and committed-state delivery.

use std::sync::Arc;

use buzz_auth::Scope;
use buzz_core::kind::{
    event_kind_u32, RELAY_ADMIN_DELETE_MENTION_GROUP, RELAY_ADMIN_PUT_MENTION_GROUP,
    RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
};
use buzz_core::mention_group::{parse_command, parse_policy_command};
use buzz_core::TenantContext;
use buzz_db::mention_group::{GroupOutcome, GroupWrite};
use nostr::Event;

use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;

/// Whether this kind is a direct community Mention Groups command.
pub(crate) fn is_command(kind: u32) -> bool {
    matches!(
        kind,
        RELAY_ADMIN_PUT_MENTION_GROUP
            | RELAY_ADMIN_DELETE_MENTION_GROUP
            | RELAY_ADMIN_SET_MENTION_GROUP_POLICY
    )
}

/// Accept a verified signature before generic freshness rejection, so a durable
/// receipt can settle an old exact retry without granting another mutation.
pub(crate) async fn accept(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    if event.pubkey != *auth.pubkey() {
        return Err(IngestError::AuthFailed(
            "invalid: event pubkey does not match authenticated identity".into(),
        ));
    }
    if auth.channel_ids().is_some() {
        return Err(IngestError::AuthFailed(
            "restricted: mention group commands require a global token".into(),
        ));
    }
    if !auth.scopes().contains(&Scope::UsersWrite) {
        return Err(IngestError::AuthFailed(
            "restricted: insufficient scope (need users:write)".into(),
        ));
    }
    super::ingest::enforce_write_restriction(state, tenant, event_kind_u32(event), auth.pubkey())
        .await?;

    let now = chrono::Utc::now().timestamp();
    if now.abs_diff(event.created_at.as_secs() as i64) > 120
        && !state
            .db
            .mention_group_command_accepted(tenant.community(), event.id.as_bytes())
            .await
            .map_err(storage_error)?
    {
        return Err(IngestError::Rejected(
            "invalid: mention group command timestamp out of range".into(),
        ));
    }

    let write = if event_kind_u32(event) == RELAY_ADMIN_SET_MENTION_GROUP_POLICY {
        let command = parse_policy_command(event).map_err(invalid)?;
        state
            .db
            .accept_mention_group_policy(
                tenant.community(),
                event,
                &command,
                &state.relay_keypair,
                state.config.max_frame_bytes,
                state.config.require_relay_membership,
            )
            .await
    } else {
        let command = parse_command(event).map_err(invalid)?;
        state
            .db
            .accept_mention_group(
                tenant.community(),
                event,
                &command,
                &state.relay_keypair,
                state.config.max_frame_bytes,
                state.config.require_relay_membership,
            )
            .await
    }
    .map_err(storage_error)?;
    finish(state, tenant, event, write).await
}

fn invalid(reason: String) -> IngestError {
    IngestError::Rejected(format!("invalid: {reason}"))
}

fn storage_error(error: buzz_db::DbError) -> IngestError {
    match error {
        buzz_db::DbError::InvalidData(reason) => invalid(reason),
        buzz_db::DbError::AccessDenied(reason) => {
            IngestError::AuthFailed(format!("restricted: {reason}"))
        }
        error => IngestError::Internal(format!("error: mention group storage: {error}")),
    }
}

async fn finish(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &Event,
    write: GroupWrite,
) -> Result<IngestResult, IngestError> {
    match write.outcome {
        GroupOutcome::Conflict => {
            return Err(IngestError::GroupConflict(
                "conflict: mention group version changed".into(),
            ));
        }
        GroupOutcome::NameTaken => {
            return Err(IngestError::GroupConflict(
                "duplicate: mention group name already exists".into(),
            ));
        }
        GroupOutcome::Forbidden => {
            return Err(IngestError::AuthFailed(
                "restricted: community policy does not permit this mention group command".into(),
            ));
        }
        GroupOutcome::Applied | GroupOutcome::Duplicate => {}
    }
    let mut message = if write.outcome == GroupOutcome::Duplicate {
        "duplicate:".to_owned()
    } else {
        String::new()
    };
    if !write.joins.skipped.is_empty() {
        let summary = serde_json::to_string(&write.joins)
            .map_err(|error| IngestError::Internal(format!("error: join summary: {error}")))?;
        if !message.is_empty() {
            message.push(' ');
        }
        message.push_str("joins: ");
        message.push_str(&summary);
    }
    if let Some(snapshot) = write.snapshot {
        super::side_effects::fan_out_mention_group_state(tenant, state, &snapshot).await;
    }
    for (channel, target) in write.joined_members {
        state.invalidate_membership(tenant, channel, &target);
        if let Err(error) = super::side_effects::emit_system_message(
            tenant,
            state,
            channel,
            serde_json::json!({
                "type": "member_joined",
                "actor": event.pubkey.to_hex(),
                "target": hex::encode(&target),
            }),
            chrono::Utc::now(),
        )
        .await
        {
            tracing::warn!(%channel, %error, "default-channel join notice delivery failed");
        }
        match state.db.channel_type_of(tenant.community(), channel).await {
            Ok(Some(channel_type)) => {
                if let Err(error) = super::side_effects::emit_typed_membership_notification(
                    tenant,
                    state,
                    channel,
                    &channel_type,
                    &target,
                    event.pubkey.as_bytes(),
                    buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION,
                    &[],
                )
                .await
                {
                    tracing::warn!(%channel, %error, "default-channel membership notification failed");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%channel, %error, "default-channel membership type lookup failed");
            }
        }
    }
    // Membership rows already committed. Reuse ordinary channel discovery
    // publication; it also has the existing periodic reconciliation owner.
    for channel in write.joined_channels {
        if let Err(error) =
            super::side_effects::emit_group_discovery_events(tenant, state, channel).await
        {
            tracing::warn!(%channel, %error, "default-channel discovery delivery failed");
        }
    }
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message,
    })
}
