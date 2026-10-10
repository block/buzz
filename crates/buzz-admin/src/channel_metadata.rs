//! Operator signing for application-owned canonical metadata repair.

use anyhow::{Context, Result};
use buzz_core::{kind::KIND_NIP29_GROUP_METADATA, TenantContext};
use buzz_db::Db;
use nostr::{EventBuilder, Keys, Kind};
use uuid::Uuid;

pub(crate) async fn repair(
    db: &Db,
    tenant: &TenantContext,
    channel: Uuid,
    keys: &Keys,
) -> Result<bool> {
    let mut write = db
        .begin_channel_metadata_write(tenant.community(), channel, keys.public_key())
        .await?;
    if !write.needs_snapshot().await? {
        write.rollback().await?;
        return Ok(false);
    }
    let tags = write.snapshot_tags().await?;
    let timestamp = write.snapshot_timestamp()?;
    let keys = keys.clone();
    let event = tokio::task::spawn_blocking(move || {
        EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_METADATA as u16), "")
            .tags(tags)
            .allow_self_tagging()
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
    })
    .await??;
    let max_bytes = match std::env::var("BUZZ_MAX_FRAME_BYTES") {
        Ok(value) => value
            .parse::<usize>()
            .context("invalid BUZZ_MAX_FRAME_BYTES")?,
        Err(std::env::VarError::NotPresent) => 512 * 1024,
        Err(error) => return Err(error.into()),
    };
    write.store_snapshot(&event, max_bytes).await?;
    write.commit().await?;
    Ok(true)
}
