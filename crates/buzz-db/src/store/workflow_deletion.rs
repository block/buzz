//! Transaction-bound deletion of executable workflows and signed definitions.

use std::collections::BTreeSet;

use buzz_core::{kind::KIND_WORKFLOW_DEF, CommunityId};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

/// Retire a workflow coordinate under the same lock used by definition writes.
///
/// The caller must authorize the coordinate owner and commit the signed deletion
/// in this transaction. A newer live definition preserves the executable row;
/// missing rows are allowed so retries can repair definitions left by old relays.
pub async fn delete_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    owner: &[u8],
    workflow_id: Uuid,
    coordinate: &str,
    deleted_at_secs: i64,
) -> Result<Vec<Uuid>> {
    let cutoff = DateTime::<Utc>::from_timestamp(deleted_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(deleted_at_secs))?;
    let lock_key = super::replaceable::event_replacement_lock_key(
        community,
        KIND_WORKFLOW_DEF as i32,
        owner,
        Some(coordinate.as_bytes()),
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut **tx)
        .await?;

    let newer_definition: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
           AND deleted_at IS NULL AND created_at > $5)",
    )
    .bind(community.as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(owner)
    .bind(coordinate)
    .bind(cutoff)
    .fetch_one(&mut **tx)
    .await?;

    let mut channels = BTreeSet::new();
    if !newer_definition {
        let removed: Option<Option<Uuid>> = sqlx::query_scalar(
            "DELETE FROM workflows WHERE community_id = $1 AND id = $2 AND owner_pubkey = $3 \
             RETURNING channel_id",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(owner)
        .fetch_optional(&mut **tx)
        .await?;
        channels.extend(removed.flatten());
    }

    let definition_channels: Vec<Option<Uuid>> = sqlx::query_scalar(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 \
           AND deleted_at IS NULL AND created_at <= $5 RETURNING channel_id",
    )
    .bind(community.as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(owner)
    .bind(coordinate)
    .bind(cutoff)
    .fetch_all(&mut **tx)
    .await?;
    channels.extend(definition_channels.into_iter().flatten());
    Ok(channels.into_iter().collect())
}
