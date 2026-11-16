//! Standalone, bounded message removal. No workflow router calls this module.
//!
//! Each call owns one transaction and selects remaining rows, so retries need no
//! message cursor. A community deletion lock excludes admitted database writers
//! for the short batch; it does not replace prior user gating or external drain.

use super::UserDeletionStore;
use crate::entity_deletion::EntityLease;
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
        let (mut tx, _, _, subject_hex) = super::execution::begin(
            self,
            lease,
            stage_id,
            community,
            relay_pubkey,
            "purge_messages",
        )
        .await?;
        let subject = hex::decode(&subject_hex).map_err(|_| safety("invalid subject"))?;
        let result = purge_for_subject(
            &mut tx,
            community,
            &subject,
            &subject_hex,
            relay_pubkey,
            batch_size,
        )
        .await?;
        super::execution::verify_live(&mut tx, lease).await?;
        tx.commit().await?;
        Ok(result)
    }
}

pub(super) async fn purge_for_subject(
    tx: &mut AdmittedTx,
    community: CommunityId,
    subject: &[u8],
    subject_hex: &str,
    relay_pubkey: &[u8; 32],
    batch_size: u16,
) -> Result<MessageBatch> {
    let seed = select(tx, community, subject, relay_pubkey, None, 1).await?;
    let mut result = MessageBatch::default();
    if let Some(seed) = seed.first() {
        result.root_id = Some(hex::encode(&seed.root_id));
        let candidates =
            select(tx, community, subject, relay_pubkey, Some(seed), batch_size).await?;
        cleanup::remove(tx, community, &candidates, &mut result).await?;
    } else {
        // A historic reaction can outlive its source event. Drain these in
        // bounded batches too; deleting only signed events would miss them.
        result.reactions_deleted =
            cleanup::remove_orphan_reactions(tx, community, subject, batch_size).await?;
        result.workflow_runs_deleted =
            cleanup::remove_orphan_runs(tx, community, subject_hex, batch_size).await?;
        result.exhausted = result.reactions_deleted == 0 && result.workflow_runs_deleted == 0;
    }
    Ok(result)
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

/// Remove a bounded set of authored non-message events, preserving the same
/// media, delivery, workflow-copy and thread-repair guarantees as messages.
pub(super) async fn purge_event_kinds(
    tx: &mut AdmittedTx,
    subject: &[u8],
    kinds: &[i32],
    limit: u16,
) -> Result<bool> {
    let community = tx.community();
    let rows: Vec<Candidate> = sqlx::query_as(
        "WITH seed AS (SELECT channel_id FROM events WHERE community_id=$1 AND pubkey=$2
         AND kind=ANY($3) ORDER BY created_at,id LIMIT 1)
         SELECT e.id,e.channel_id,e.id AS root_id,
         (e.content ~* 'https?://|/media/' OR EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) t
          WHERE t->>0 IN ('imeta','link-preview','url','thumb','image','x','emoji'))) AS has_media
         FROM events e, seed WHERE e.community_id=$1 AND e.pubkey=$2 AND e.kind=ANY($3)
         AND e.channel_id IS NOT DISTINCT FROM seed.channel_id ORDER BY e.created_at,e.id LIMIT $4",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .bind(kinds)
    .bind(i64::from(limit))
    .fetch_all(tx.conn())
    .await?;
    if rows.is_empty() {
        return Ok(true);
    }
    cleanup::remove(tx, community, &rows, &mut MessageBatch::default()).await?;
    Ok(false)
}

pub(super) async fn has_remaining(
    tx: &mut AdmittedTx,
    subject: &[u8],
    subject_hex: &str,
    relay: &[u8; 32],
) -> Result<bool> {
    let community = tx.community();
    if !select(tx, community, subject, relay, None, 1)
        .await?
        .is_empty()
    {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reactions WHERE community_id=$1 AND pubkey=$2
         UNION ALL SELECT 1 FROM workflow_runs WHERE community_id=$1 AND lower(trigger_context->>'author')=$3)",
    ).bind(community.as_uuid()).bind(subject).bind(subject_hex).fetch_one(tx.conn()).await?)
}
