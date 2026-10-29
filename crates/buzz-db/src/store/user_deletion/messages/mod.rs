//! Standalone, bounded message removal. No workflow router calls this module.
//!
//! Each call owns one transaction and selects remaining rows, so retries need no
//! message cursor. A community deletion lock excludes admitted database writers
//! for the short batch; it does not replace prior user gating or external drain.

use super::{normalize_user_deletion_pubkey, UserDeletionInventory, UserDeletionStore};
use crate::entity_deletion::{lock_owned, validate_approved_inventory, EntityLease, EntityStage};
use crate::{AdmittedTx, DbError, Result};
use buzz_core::CommunityId;
use serde::Serialize;
use uuid::Uuid;

mod cleanup;

/// Maximum authored events removed by one transaction. Dependents have a separate cap.
pub const MAX_BATCH_SIZE: u16 = 100;
const MAX_DEPENDENTS: usize = 2000;
const SELECT: &str = include_str!("select.sql");

/// Result of one committed batch, not completion of the user's overall deletion.
#[derive(Debug, Default, Serialize)]
pub struct MessageBatch {
    /// Thread processed; absent when the community has no remaining message work.
    pub root_id: Option<String>,
    /// Authored and dependent event rows removed in this transaction.
    pub events_deleted: u64,
    /// Materialized reactions removed, including already removed reactions.
    pub reactions_deleted: u64,
    /// Mention-index entries belonging to deleted events.
    pub mentions_deleted: u64,
    /// Queue and delivery-history entries for deleted events.
    pub deliveries_deleted: u64,
    /// Terminal workflow executions removed with their approvals.
    pub workflow_runs_deleted: u64,
    /// No supported message events, orphan reactions or identifiable workflow copies remain.
    pub exhausted: bool,
}

#[derive(Clone, sqlx::FromRow)]
struct Candidate {
    id: Vec<u8>,
    channel_id: Option<Uuid>,
    root_id: Vec<u8>,
    has_media: bool,
}

fn safety(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}

impl UserDeletionStore {
    /// Remove one batch in one thread under the current `purge_messages` stage.
    ///
    /// Requires the exact approved inventory, a current lease, a global user
    /// fence from this request, and drained external community writes. The
    /// caller must already have stopped user writes and workflow execution;
    /// fence storage alone does not yet enforce serving paths. The relay signer
    /// must come from trusted relay configuration, never user-supplied tags.
    ///
    /// Attachment/link-bearing events and nonterminal workflow runs fail closed
    /// before any deletion. Media, audit/moderation, personal state and other
    /// users' signed messages are outside this operation. No stage advancement,
    /// lease release, network call or main-workflow integration occurs here.
    pub async fn purge_message_batch(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        community: CommunityId,
        relay_pubkey: &[u8; 32],
        batch_size: u16,
    ) -> Result<MessageBatch> {
        if !(1..=MAX_BATCH_SIZE).contains(&batch_size) {
            return Err(safety("message batch size must be 1..=100"));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '10s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout = '2s'")
            .execute(&mut *tx)
            .await?;
        // Existing lock order: community, request, identities. A shared lock
        // followed by an upgrade would deadlock two deletion workers.
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await?;
        crate::deletion::guard_community_write(&mut tx, community).await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        let inventory = UserDeletionInventory::read(&request)?;
        let expected_inputs = inventory.inputs(&request.target, Some(*community.as_uuid()))?;
        let stage: EntityStage = sqlx::query_as(
            "SELECT id, request_id, target_community_id, stage, status, is_current,
             started_at, finished_at, metadata_json, last_error, updated_at
             FROM entity_deletion_stage_history WHERE id = $1 FOR UPDATE",
        )
        .bind(stage_id)
        .fetch_one(&mut *tx)
        .await?;
        if stage.request_id != lease.request_id
            || stage.stage != "purge_messages"
            || stage.status != "running"
            || !stage.is_current
            || stage.target_community_id != Some(*community.as_uuid())
            || stage.metadata_json["inputs"] != expected_inputs
        {
            return Err(safety("expected the current approved purge_messages stage"));
        }
        let subject_hex = normalize_user_deletion_pubkey(&request.target)?;
        let subject = hex::decode(&subject_hex).map_err(|_| safety("invalid subject"))?;
        if subject == relay_pubkey {
            return Err(safety("cannot purge the relay signing identity"));
        }
        let identity = crate::user_deletion_fences::FenceIdentity::user(
            nostr::PublicKey::from_slice(&subject).map_err(|_| safety("invalid subject"))?,
        );
        let fence = crate::user_deletion_fences::find(&mut tx, identity)
            .await?
            .ok_or_else(|| safety("user must be fenced before message deletion"))?;
        if fence.request_id != lease.request_id {
            return Err(safety("user fence belongs to another deletion request"));
        }
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM community_serving_write_leases
             WHERE community_id = $1 AND lease_until > clock_timestamp())",
        )
        .bind(community.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        if active {
            return Err(safety(
                "external community writes must drain before message deletion",
            ));
        }
        let mut tx = AdmittedTx::admit(
            tx,
            &crate::deletion::DeletionStore::new(self.pool.clone()),
            community,
        )
        .await?;
        let seed = select(&mut tx, community, &subject, relay_pubkey, None, 1).await?;
        let mut result = MessageBatch::default();
        if let Some(seed) = seed.first() {
            result.root_id = Some(hex::encode(&seed.root_id));
            let candidates = select(
                &mut tx,
                community,
                &subject,
                relay_pubkey,
                Some(seed),
                batch_size,
            )
            .await?;
            cleanup::remove(&mut tx, community, &candidates, &mut result).await?;
        } else {
            // A historic reaction can outlive its source event. Drain these in
            // bounded batches too; deleting only signed events would miss them.
            result.reactions_deleted =
                cleanup::remove_orphan_reactions(&mut tx, community, &subject, batch_size).await?;
            result.workflow_runs_deleted =
                cleanup::remove_orphan_runs(&mut tx, community, &subject_hex, batch_size).await?;
            result.exhausted = result.reactions_deleted == 0 && result.workflow_runs_deleted == 0;
        }
        // Expiry during a long query must roll back even though the parent row
        // lock prevented a replacement worker from claiming in the meantime.
        // Ownership cannot change while the request row is locked. Only the
        // clock can invalidate the lease after admission.
        let unexpired: bool = sqlx::query_scalar(
            "SELECT lease_expires_at > clock_timestamp() FROM entity_deletion_requests WHERE id=$1",
        )
        .bind(lease.request_id)
        .fetch_one(tx.conn())
        .await?;
        if !unexpired {
            return Err(DbError::AccessDenied(
                "deletion lease expired during message batch".into(),
            ));
        }
        tx.commit().await?;
        Ok(result)
    }
}

async fn select(
    tx: &mut AdmittedTx,
    community: CommunityId,
    subject: &[u8],
    relay: &[u8; 32],
    thread: Option<&Candidate>,
    limit: u16,
) -> Result<Vec<Candidate>> {
    Ok(sqlx::query_as(SELECT)
        .bind(community.as_uuid())
        .bind(subject)
        .bind(relay.as_slice())
        .bind(thread.map(|t| &t.root_id))
        .bind(i64::from(limit))
        .bind(thread.and_then(|t| t.channel_id))
        .fetch_all(tx.conn())
        .await?)
}
