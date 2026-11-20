//! Transport-neutral ingest for the NIP-PM pin and unpin commands.

use std::sync::Arc;

use buzz_core::{
    kind::{event_kind_u32, KIND_CHANNEL_PINS, KIND_PIN_MESSAGE},
    StoredEvent, TenantContext,
};
use buzz_db::{pins::PinChange, DbError};
use nostr::{Event, Tag};
use uuid::Uuid;

use crate::state::AppState;

use super::{
    event::dispatch_persistent_event,
    ingest::IngestError,
    side_effects::{fan_out_system_message, sign_system_message},
};

fn only_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, IngestError> {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|key| key == name));
    if let (Some([_, value, ..]), None) = (tags.next().map(Tag::as_slice), tags.next()) {
        return Ok(value);
    }
    Err(IngestError::Rejected(format!(
        "invalid: pin command requires exactly one {name} tag with a value"
    )))
}

/// Fan out a channel's rewritten pin list after its transaction committed.
pub(super) async fn dispatch_list(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    list: &StoredEvent,
) {
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();
    dispatch_persistent_event(
        tenant,
        state,
        list,
        KIND_CHANNEL_PINS,
        &relay_pubkey_hex,
        None,
    )
    .await;
}

pub(super) async fn persist(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(StoredEvent, bool), IngestError> {
    if !event.content.is_empty() {
        return Err(IngestError::Rejected(
            "invalid: pin command content must be empty".into(),
        ));
    }
    let channel_id = only_tag(event, "h")?
        .parse::<Uuid>()
        .map_err(|_| IngestError::Rejected("invalid: h tag must be a channel UUID".into()))?;
    let target_hex = only_tag(event, "e")?;
    let target = Some(target_hex)
        .filter(|id| id.len() == 64 && !id.bytes().any(|b| b.is_ascii_uppercase()))
        .and_then(|id| hex::decode(id).ok())
        .ok_or_else(|| {
            IngestError::Rejected("invalid: e tag must be a lowercase hex event ID".into())
        })?;

    let notice = (event_kind_u32(event) == KIND_PIN_MESSAGE)
        .then(|| {
            sign_system_message(
                state,
                channel_id,
                serde_json::json!({
                    "type": "message_pinned",
                    "actor": event.pubkey.to_hex(),
                    "target_event_id": target_hex,
                    // Two pins of one message by one member are two notices.
                    "command_event_id": event.id.to_hex(),
                }),
                chrono::Utc::now(),
            )
        })
        .transpose()
        .map_err(|error| IngestError::Internal(format!("error: {error}")))?;
    let change = match &notice {
        Some(notice) => PinChange::Pin { notice },
        None => PinChange::Unpin,
    };
    let write = state
        .db
        .apply_pin_command(
            tenant.community(),
            channel_id,
            event,
            &target,
            change,
            &state.relay_keypair,
        )
        .await
        .map_err(|error| match error {
            DbError::AccessDenied(reason) => IngestError::Rejected(format!("invalid: {reason}")),
            error => IngestError::Internal(format!("error: pin write failed: {}", error.kind())),
        })?;

    // The command is committed. What follows only updates live subscribers,
    // so a failure is logged; the next read returns the stored list.
    if let Some(list) = &write.list {
        dispatch_list(tenant, state, list).await;
        if let Some(notice) = &notice {
            fan_out_system_message(tenant, state, channel_id, notice).await;
        }
    }
    Ok((write.stored, write.inserted))
}
