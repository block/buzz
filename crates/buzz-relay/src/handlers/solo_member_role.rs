//! Transport-neutral ingest for the Solo member role command.

use std::sync::Arc;

use buzz_core::{kind::event_kind_u32, StoredEvent, TenantContext};
use buzz_db::{channel::MemberRole, channel_members::SoloMemberRoleChange, DbError};
use nostr::{Event, PublicKey, Tag};
use uuid::Uuid;

use crate::state::AppState;

use super::{event::dispatch_persistent_event, ingest::IngestError};

fn required_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, IngestError> {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|key| key == name));
    if let (Some([_, value]), None) = (tags.next().map(Tag::as_slice), tags.next()) {
        return Ok(value);
    }
    Err(IngestError::Rejected(format!(
        "invalid: Solo member role command requires exactly one {name} tag with a value"
    )))
}

pub(super) async fn persist(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(StoredEvent, bool), IngestError> {
    if !event.content.is_empty() {
        return Err(IngestError::Rejected(
            "invalid: Solo member role command content must be empty".into(),
        ));
    }
    let channel_id = required_tag(event, "h")?
        .parse::<Uuid>()
        .map_err(|_| IngestError::Rejected("invalid: h tag must be a channel UUID".into()))?;
    let target = PublicKey::from_hex(required_tag(event, "p")?)
        .map_err(|_| IngestError::Rejected("invalid: p tag must be a hex pubkey".into()))?;
    let parse_role = |tag| {
        required_tag(event, tag)?
            .parse::<MemberRole>()
            .map_err(|_| IngestError::Rejected(format!("invalid: unknown {tag} value")))
    };
    let change = SoloMemberRoleChange {
        target,
        expected_role: parse_role("expected_role")?,
        role: parse_role("role")?,
    };
    let (stored, group_state) = state
        .db
        .change_solo_member_role_with_event(
            tenant.community(),
            channel_id,
            change,
            event,
            &state.relay_keypair,
        )
        .await
        .map_err(|error| match error {
            DbError::AccessDenied(reason) => IngestError::Rejected(format!("invalid: {reason}")),
            DbError::ChannelNotFound(_) => {
                IngestError::Rejected("invalid: channel not found".into())
            }
            error => {
                IngestError::Internal(format!("error: Solo member role write failed: {error}"))
            }
        })?;

    let inserted = !group_state.is_empty();
    if inserted {
        state.invalidate_membership(tenant, channel_id, &target.to_bytes());
        let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();
        for snapshot in group_state {
            dispatch_persistent_event(
                tenant,
                state,
                &snapshot,
                event_kind_u32(&snapshot.event),
                &relay_pubkey_hex,
                None,
            )
            .await;
        }
    }
    Ok((stored, inserted))
}
