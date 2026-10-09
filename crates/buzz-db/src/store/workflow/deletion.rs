//! Keep the executable workflow and its client-visible definition in sync.

use buzz_core::{kind::KIND_WORKFLOW_DEF, CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use nostr::Event;
use uuid::Uuid;

use crate::AdmittedTx;
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
        let mut tx = self.begin_event_write_transaction(community_id).await?;
        let outcome =
            delete_workflow_in_transaction(&mut tx, owner_pubkey, d_tag, deletion_created_at_secs)
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
        let mut tx = self.begin_event_write_transaction(community_id).await?;
        let (stored, inserted) =
            crate::event::insert_event_in_transaction(&mut tx, event, None).await?;
        if inserted {
            // Unlike best-effort indexing for ordinary events, deletion fails
            // closed: its public request and discoverability commit together.
            crate::runtime::insert_mentions_in_transaction(&mut tx, event, None).await?;
        }
        let outcome = delete_workflow_in_transaction(
            &mut tx,
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
    tx: &mut AdmittedTx,
    owner_pubkey: &[u8],
    d_tag: &str,
    deletion_created_at_secs: i64,
) -> Result<WorkflowDeletionOutcome> {
    let community_id = tx.community();
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
            .execute(tx.conn()),
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
    .fetch_optional(tx.conn())
    .await?;
    if head.is_some_and(|created_at| created_at > cutoff) {
        return Ok(WorkflowDeletionOutcome::default());
    }

    // UUID coordinates are canonical; retain the legacy name-based path.
    // The owner predicate remains in the mutation, not just a prior check.
    let workflow_id = Uuid::parse_str(d_tag).ok();
    // Lock the workflow first: a concurrent run or fire insert must lock it for
    // its foreign key, so the child deletes below see every committed child.
    let target: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM workflows WHERE community_id = $1 AND owner_pubkey = $2 \
             AND id = COALESCE($3::uuid, (SELECT id FROM workflows \
             WHERE community_id = $1 AND owner_pubkey = $2 AND name = $4 LIMIT 1)) \
             FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .bind(workflow_id)
    .bind(d_tag)
    .fetch_optional(tx.conn())
    .await?;
    let row = match target {
        Some(id) => Some(delete_workflow_with_children(tx, id).await?),
        None => None,
    };
    let definitions = sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 \
             AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL AND created_at <= $5",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_WORKFLOW_DEF as i32)
    .bind(owner_pubkey)
    .bind(d_tag)
    .bind(cutoff)
    .execute(tx.conn())
    .await?;
    let changed = row.is_some() || definitions.rows_affected() > 0;
    let channel_id = row.flatten();
    Ok(WorkflowDeletionOutcome {
        changed,
        channel_id,
    })
}

/// Delete a locked workflow and the rows that hang off it, children first, so
/// no foreign-key action is needed: approvals (of the workflow or any of its
/// runs), scheduled fires, then runs. Returns the workflow's `channel_id`.
async fn delete_workflow_with_children(tx: &mut AdmittedTx, id: Uuid) -> Result<Option<Uuid>> {
    let community = *tx.community().as_uuid();
    sqlx::query(
        "DELETE FROM workflow_approvals WHERE community_id = $1 AND (workflow_id = $2 \
             OR run_id IN (SELECT id FROM workflow_runs \
             WHERE community_id = $1 AND workflow_id = $2))",
    )
    .bind(community)
    .bind(id)
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "DELETE FROM scheduled_workflow_fires WHERE community_id = $1 AND workflow_id = $2",
    )
    .bind(community)
    .bind(id)
    .execute(tx.conn())
    .await?;
    sqlx::query("DELETE FROM workflow_runs WHERE community_id = $1 AND workflow_id = $2")
        .bind(community)
        .bind(id)
        .execute(tx.conn())
        .await?;
    Ok(sqlx::query_scalar(
        "DELETE FROM workflows WHERE community_id = $1 AND id = $2 RETURNING channel_id",
    )
    .bind(community)
    .bind(id)
    .fetch_one(tx.conn())
    .await?)
}

#[cfg(test)]
#[path = "deletion_tests.rs"]
mod postgres_tests;
