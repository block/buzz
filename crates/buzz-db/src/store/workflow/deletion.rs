//! Keep the executable workflow and its client-visible definition in sync.

use buzz_core::{kind::KIND_WORKFLOW_DEF, CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{Db, DbError, Result};

/// Committed changes made by a workflow-coordinate deletion.
#[derive(Debug, Default)]
pub struct WorkflowDeletionOutcome {
    /// Whether either the executable workflow or a visible definition was removed.
    pub changed: bool,
    /// Removed executable workflow's channel, for trigger-cache invalidation.
    pub channel_id: Option<Uuid>,
}

impl Db {
    /// Delete an authorized workflow coordinate and its definition atomically.
    ///
    /// Shares the replacement lock with definition saves. A deletion older than
    /// the live definition is a no-op. Missing projections are tolerated so a
    /// retry can remove definitions left behind by older relay versions.
    /// Reports committed changes independently of the optional workflow channel.
    /// No tombstone is retained: clients may intentionally publish backdated definitions.
    #[datastore_span(name = "delete_workflow_by_coordinate", system = "postgresql")]
    pub async fn delete_workflow_by_coordinate(
        &self,
        community_id: CommunityId,
        owner_pubkey: &[u8],
        d_tag: &str,
        deletion_created_at_secs: i64,
    ) -> Result<WorkflowDeletionOutcome> {
        let mut tx = self.begin_event_write_transaction().await?;
        let outcome = delete_workflow_in_transaction(
            &mut tx,
            community_id,
            owner_pubkey,
            d_tag,
            deletion_created_at_secs,
        )
        .await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Commit the public deletion request and both workflow projections together.
    ///
    /// The returned boolean grants dispatch ownership: either this transaction
    /// inserted the request, or it repaired an older relay's incomplete deletion.
    /// Concurrent identical requests cannot split those two outcomes across commits.
    /// Authorization of the coordinate is the caller's responsibility.
    pub async fn insert_workflow_deletion(
        &self,
        community_id: CommunityId,
        event: &Event,
        owner_pubkey: &[u8],
        d_tag: &str,
    ) -> Result<(StoredEvent, bool, Option<Uuid>)> {
        let mut tx = self.begin_event_write_transaction().await?;
        let (stored, inserted) =
            crate::event::insert_event_in_transaction(&mut tx, community_id, event, None).await?;
        if inserted {
            // Unlike best-effort indexing for ordinary events, deletion fails
            // closed: its public request and discoverability commit together.
            crate::runtime::insert_mentions_in_transaction(&mut tx, community_id, event, None)
                .await?;
        }
        let outcome = delete_workflow_in_transaction(
            &mut tx,
            community_id,
            owner_pubkey,
            d_tag,
            event.created_at.as_secs() as i64,
        )
        .await?;
        tx.commit().await?;
        Ok((stored, inserted || outcome.changed, outcome.channel_id))
    }
}

async fn delete_workflow_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    owner_pubkey: &[u8],
    d_tag: &str,
    deletion_created_at_secs: i64,
) -> Result<WorkflowDeletionOutcome> {
    let cutoff = DateTime::from_timestamp(deletion_created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(deletion_created_at_secs))?;
    let lock_key = crate::store::replaceable::event_replacement_lock_key(
        community_id,
        KIND_WORKFLOW_DEF as i32,
        owner_pubkey,
        Some(d_tag.as_bytes()),
    );
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::Replacement,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **tx),
    )
    .await?;

    let head: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT created_at FROM events WHERE community_id = $1 AND kind = $2 \
             AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
             ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(owner_pubkey)
    .bind(d_tag)
    .fetch_optional(&mut **tx)
    .await?;
    if head.is_some_and(|created_at| created_at > cutoff) {
        return Ok(WorkflowDeletionOutcome::default());
    }

    // UUID coordinates are canonical; retain the legacy name-based path.
    // The owner predicate remains in the mutation, not just a prior check.
    let workflow_id = Uuid::parse_str(d_tag).ok();
    let row = sqlx::query(
        "DELETE FROM workflows WHERE community_id = $1 AND owner_pubkey = $2 \
             AND id = COALESCE($3::uuid, (SELECT id FROM workflows \
             WHERE community_id = $1 AND owner_pubkey = $2 AND name = $4 LIMIT 1)) \
             RETURNING channel_id",
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .bind(workflow_id)
    .bind(d_tag)
    .fetch_optional(&mut **tx)
    .await?;
    let definitions = sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 \
             AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL AND created_at <= $5",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(owner_pubkey)
    .bind(d_tag)
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let changed = row.is_some() || definitions.rows_affected() > 0;
    let channel_id = row
        .map(|row| row.try_get("channel_id"))
        .transpose()?
        .flatten();
    Ok(WorkflowDeletionOutcome {
        changed,
        channel_id,
    })
}

#[cfg(test)]
#[path = "deletion_tests.rs"]
mod postgres_tests;
