//! NIP-CL is a transaction, never generic event storage followed by side effects.

use std::sync::Arc;

use buzz_auth::Scope;
use buzz_core::{channel::ChannelVisibility, channel_labels::LabelCommand, TenantContext};
use buzz_db::channel_metadata::{ChannelCreate, ChannelMetadataWrite, CommandApplication};
use nostr::Event;
use tracing::warn;

use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;

/// Classify commands only after generic signature/auth/scope/write admission.
/// Returns `None` for ordinary metadata and the legacy no-h creation path.
pub(crate) async fn handle_if_covered(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Option<Result<IngestResult, IngestError>> {
    let command = match LabelCommand::parse(event) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => {
            return Some(Ok(response(
                event,
                false,
                format!("invalid: nip-cl-rejected {error}"),
            )))
        }
    };
    if !state.config.nip_cl_enabled {
        // Until fleet activation, existing unlabeled creation retains its old
        // contract. Never accept label operations and silently drop their state.
        if matches!(&command, LabelCommand::Create { labels, .. } if labels.is_empty()) {
            return None;
        }
        return Some(Ok(response(
            event,
            false,
            "restricted: nip-cl-rejected channel labels are disabled".into(),
        )));
    }
    Some(Ok(apply(tenant, state, event, auth, command).await))
}

async fn apply(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
    command: LabelCommand,
) -> IngestResult {
    let channel = command.channel();
    if !auth.scopes().contains(&Scope::ChannelsWrite)
        || auth
            .channel_ids()
            .is_some_and(|ids| !ids.contains(&channel))
    {
        return response(
            event,
            false,
            "restricted: nip-cl-rejected insufficient scope".into(),
        );
    }
    let create = if matches!(command, LabelCommand::Create { .. }) {
        match creation_fields(event, state) {
            Ok(fields) => Some(fields),
            Err(detail) => {
                return response(event, false, format!("invalid: nip-cl-rejected {detail}"))
            }
        }
    } else {
        None
    };
    let mut write = match state
        .db
        .begin_channel_metadata_write(
            tenant.community(),
            channel,
            state.relay_keypair.public_key(),
        )
        .await
    {
        Ok(write) => write,
        Err(error) => {
            warn!(%error, "channel label transaction admission failed");
            return response(
                event,
                false,
                "error: nip-cl-rejected admission failed".into(),
            );
        }
    };
    let decision = match write.apply_command(event, create.as_ref()).await {
        Ok(decision) => decision,
        Err(error) => {
            warn!(%error, "channel label application failed");
            let prefix = if matches!(error, buzz_db::DbError::InvalidData(_)) {
                "invalid: nip-cl-rejected invalid command or resulting state"
            } else {
                "error: nip-cl-rejected application failed"
            };
            return rollback_response(event, write, prefix).await;
        }
    };
    if decision != CommandApplication::Applied {
        let message = match decision {
            CommandApplication::Committed => "duplicate: nip-cl-committed",
            CommandApplication::Unknown => "error: nip-cl-unknown application evidence unavailable",
            CommandApplication::Conflict => {
                "duplicate: nip-cl-rejected channel UUID is unavailable"
            }
            CommandApplication::Restricted => "restricted: nip-cl-rejected current access denied",
            CommandApplication::Expired => {
                "invalid: nip-cl-rejected timestamp outside admission window"
            }
            CommandApplication::Applied => unreachable!(),
        };
        if let Err(error) = write.rollback().await {
            warn!(%error, "channel label read-only transaction rollback failed");
        }
        return response(
            event,
            decision == CommandApplication::Committed,
            message.into(),
        );
    }
    if let Err(error) = super::channel_metadata::store_snapshot_if_needed(state, &mut write).await {
        warn!(%error, "channel label snapshot failed");
        return rollback_response(event, write, "error: nip-cl-rejected snapshot failed").await;
    }
    let published = match write.commit().await {
        Ok(published) => published,
        Err(error) => {
            warn!(%error, "channel label commit outcome uncertain");
            return response(
                event,
                false,
                "error: nip-cl-unknown commit outcome uncertain".into(),
            );
        }
    };
    // Do not send the command through generic post-storage handlers: that would
    // repeat creation or turn metadata tags into conversation activity.
    if let Some(stored) = published {
        super::event::dispatch_persistent_event(
            tenant,
            state,
            &stored,
            buzz_core::kind::KIND_NIP29_GROUP_METADATA,
            &state.relay_keypair.public_key().to_hex(),
            None,
        )
        .await;
    }
    if let Some(create) = create {
        state.invalidate_membership(tenant, channel, event.pubkey.as_bytes());
        if create.visibility == ChannelVisibility::Open {
            state.invalidate_all_accessible_channels(tenant);
        }
        metrics::counter!("buzz_channels_created_total", "community" => tenant.host().to_owned(),
            "type" => create.channel_type.to_string())
        .increment(1);
        // These auxiliary snapshots/notifications are not part of NIP-CL's
        // atomic acknowledgement. Failure cannot undo the committed command.
        if let Err(error) =
            super::side_effects::emit_group_discovery_events(tenant, state, channel).await
        {
            warn!(%error, "post-create discovery failed");
        }
        if let Err(error) = super::side_effects::emit_membership_notification(
            tenant,
            state,
            channel,
            event.pubkey.as_bytes(),
            event.pubkey.as_bytes(),
            buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION,
        )
        .await
        {
            warn!(%error, "post-create membership notification failed");
        }
    }
    response(event, true, String::new())
}

async fn rollback_response(
    event: &Event,
    write: ChannelMetadataWrite,
    rejected: &str,
) -> IngestResult {
    match write.rollback().await {
        Ok(()) => response(event, false, rejected.into()),
        Err(error) => {
            warn!(%error, "channel label rollback could not be confirmed");
            response(
                event,
                false,
                "error: nip-cl-unknown rollback could not be confirmed".into(),
            )
        }
    }
}

fn response(event: &Event, accepted: bool, message: String) -> IngestResult {
    IngestResult {
        event_id: event.id.to_hex(),
        accepted,
        message,
    }
}

fn creation_fields(event: &Event, state: &AppState) -> Result<ChannelCreate, &'static str> {
    let value = |name: &str| {
        event.tags.iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts[0] == name)
                .then(|| parts.get(1).map(String::as_str))
                .flatten()
        })
    };
    let name = value("name").ok_or("channel name is required")?;
    let name = buzz_core::channel::canonical_channel_name(name);
    if name.trim().is_empty() || name.chars().count() > 255 {
        return Err("invalid channel name");
    }
    Ok(ChannelCreate {
        name: name.to_owned(),
        channel_type: value("channel_type")
            .unwrap_or("stream")
            .parse()
            .map_err(|_| "invalid channel type")?,
        visibility: value("visibility")
            .unwrap_or("open")
            .parse()
            .map_err(|_| "invalid visibility")?,
        description: value("about").map(str::to_owned),
        ttl_seconds: super::resolve_ttl(event, state.config.ephemeral_ttl_override),
    })
}
