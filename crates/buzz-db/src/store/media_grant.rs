//! Provenance shared by live media recording, legacy replacement, and backfill.

use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{AdmittedTx, Result};

/// Resolve the first valid edit target, as the relay's edit validator does.
/// Nested edits and missing/deleted/cross-channel targets grant no file access.
pub(crate) async fn edit_target(
    conn: &mut PgConnection,
    community: CommunityId,
    event: &Event,
    channel: Option<Uuid>,
) -> Result<Option<(Vec<u8>, DateTime<Utc>)>> {
    if event.kind.as_u16() != buzz_core::kind::KIND_STREAM_MESSAGE_EDIT as u16 {
        return Ok(None);
    }
    let Some(channel) = channel else {
        return Ok(None);
    };
    let Some(target) = event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().is_some_and(|name| name == "e"))
            .then(|| {
                parts
                    .get(1)
                    .and_then(|id| nostr::EventId::from_hex(id).ok())
            })
            .flatten()
    }) else {
        return Ok(None);
    };
    Ok(sqlx::query_as(
        "SELECT id, created_at FROM events \
         WHERE community_id = $1 AND id = $2 AND channel_id = $3 \
           AND deleted_at IS NULL AND kind <> 40003 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(target.as_bytes().as_slice())
    .bind(channel)
    .fetch_optional(conn)
    .await?)
}

/// Record only server-received pre-cutoff references, with their live source
/// and (for edits) target. The receipt check is repeated in the INSERT.
pub(crate) async fn record_history(
    conn: &mut PgConnection,
    community: CommunityId,
    events: &[StoredEvent],
) -> Result<u64> {
    let mut hashes = Vec::new();
    let mut channels = Vec::new();
    let mut ids = Vec::new();
    let mut times = Vec::new();
    let mut targets = Vec::new();
    let mut target_times = Vec::new();
    for stored in events {
        let event = &stored.event;
        let blobs = buzz_core::media::shared_blob_hashes(event);
        if blobs.is_empty() {
            continue;
        }
        let target = edit_target(conn, community, event, stored.channel_id).await?;
        if event.kind.as_u16() == 40003 && target.is_none() {
            continue;
        }
        let seconds = event.created_at.as_secs() as i64;
        let created_at = DateTime::from_timestamp(seconds, 0)
            .ok_or(crate::DbError::InvalidTimestamp(seconds))?;
        for hash in blobs {
            hashes.push(hash);
            channels.push(stored.channel_id);
            ids.push(event.id.to_bytes().to_vec());
            times.push(created_at);
            targets.push(target.as_ref().map(|(id, _)| id.clone()));
            target_times.push(target.as_ref().map(|(_, at)| *at));
        }
    }
    if hashes.is_empty() {
        return Ok(0);
    }
    Ok(sqlx::query(
        "INSERT INTO media_shares \
             (community_id, sha256, channel_id, event_id, event_created_at, \
              target_event_id, target_event_created_at) \
         SELECT $1, t.h, t.c, e.id, e.created_at, t.target_id, t.target_at \
         FROM unnest($2::text[], $3::uuid[], $4::bytea[], $5::timestamptz[], \
                     $6::bytea[], $7::timestamptz[]) \
             AS t(h, c, event_id, event_created_at, target_id, target_at) \
         JOIN events e ON e.community_id = $1 AND e.id = t.event_id \
           AND e.created_at = t.event_created_at AND e.deleted_at IS NULL \
         JOIN communities c ON c.id = e.community_id \
           AND e.received_at < c.media_shares_recording_started_at \
         ORDER BY t.h, e.id ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(hashes)
    .bind(channels)
    .bind(ids)
    .bind(times)
    .bind(targets)
    .bind(target_times)
    .execute(conn)
    .await?
    .rows_affected())
}

/// Recover an eligible legacy head inside its replacement transaction, while
/// it still grants access. Only references the replacement keeps survive it.
pub(crate) async fn bootstrap_replacement(
    tx: &mut AdmittedTx,
    incoming: &Event,
    previous_id: &[u8],
) -> Result<()> {
    if buzz_core::media::shared_blob_hashes(incoming).is_empty() {
        return Ok(());
    }
    let community = tx.community();
    let row = sqlx::query(
        "SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, \
                e.received_at, e.channel_id \
         FROM events e JOIN communities c ON c.id = e.community_id \
         WHERE e.community_id = $1 AND e.id = $2 AND e.deleted_at IS NULL \
           AND e.received_at < c.media_shares_recording_started_at \
         ORDER BY e.created_at DESC LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(previous_id)
    .fetch_optional(tx.conn())
    .await?;
    if let Some(row) = row {
        if let Some(stored) = crate::event::row_to_stored_event(row)? {
            record_history(tx.conn(), community, &[stored]).await?;
        }
    }
    Ok(())
}
