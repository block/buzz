//! Relay signing and post-commit dispatch for the shared metadata write boundary.

use std::sync::Arc;

use buzz_core::{kind::KIND_NIP29_GROUP_METADATA, TenantContext};
use buzz_db::channel_metadata::ChannelMetadataWrite;
use nostr::{EventBuilder, Kind};
use uuid::Uuid;

use crate::state::AppState;

/// Sign the locked current projection if it is missing or stale.
///
/// This stages storage only. The transaction owner commits before dispatching.
pub(crate) async fn store_snapshot_if_needed(
    state: &Arc<AppState>,
    write: &mut ChannelMetadataWrite,
) -> anyhow::Result<()> {
    if !write.needs_snapshot().await? {
        return Ok(());
    }
    let tags = write.snapshot_tags().await?;
    let timestamp = write.snapshot_timestamp()?;
    let keys = state.relay_keypair.clone();
    let event = tokio::task::spawn_blocking(move || {
        EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_METADATA as u16), "")
            .tags(tags)
            .allow_self_tagging()
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
    })
    .await??;
    write
        .store_snapshot(&event, state.config.max_frame_bytes)
        .await?;
    Ok(())
}

/// Reconcile complete metadata under its transaction lock, then dispatch a new head.
/// Returns whether a replacement was committed. Deleted channels are never repaired.
pub(crate) async fn publish_metadata(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
) -> anyhow::Result<bool> {
    let mut write = state
        .db
        .begin_channel_metadata_write(
            tenant.community(),
            channel_id,
            state.relay_keypair.public_key(),
        )
        .await?;
    store_snapshot_if_needed(state, &mut write).await?;
    let published = write.commit().await?;
    if let Some(stored) = published {
        super::event::dispatch_persistent_event(
            tenant,
            state,
            &stored,
            KIND_NIP29_GROUP_METADATA,
            &state.relay_keypair.public_key().to_hex(),
            None,
        )
        .await;
        Ok(true)
    } else {
        Ok(false)
    }
}
