//! Common transaction admission for standalone user and agent cleanup.
use super::{normalize_user_deletion_pubkey, UserDeletionInventory, UserDeletionStore};
use crate::entity_deletion::{lock_owned, validate_approved_inventory, EntityLease, EntityStage};
use crate::{AdmittedTx, DbError, Result};
use buzz_core::CommunityId;
use uuid::Uuid;

pub(super) fn safety(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}

pub(super) async fn begin(
    store: &UserDeletionStore,
    lease: &EntityLease,
    stage_id: Uuid,
    community: CommunityId,
    relay_pubkey: &[u8; 32],
    stage_name: &str,
) -> Result<(AdmittedTx, EntityStage, UserDeletionInventory, String)> {
    let mut tx = store.pool.begin().await?;
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
        || stage.stage != stage_name
        || stage.status != "running"
        || !stage.is_current
        || stage.target_community_id != Some(*community.as_uuid())
        || stage.metadata_json["inputs"] != expected_inputs
    {
        return Err(safety(&format!(
            "expected the current approved {stage_name} stage"
        )));
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
    let tx = AdmittedTx::admit(
        tx,
        &crate::deletion::DeletionStore::new(store.pool.clone()),
        community,
    )
    .await?;
    Ok((tx, stage, inventory, subject_hex))
}

pub(super) async fn verify_live(tx: &mut AdmittedTx, lease: &EntityLease) -> Result<()> {
    let unexpired: bool = sqlx::query_scalar(
        "SELECT lease_expires_at > clock_timestamp() FROM entity_deletion_requests WHERE id=$1",
    )
    .bind(lease.request_id)
    .fetch_one(tx.conn())
    .await?;
    if !unexpired {
        return Err(DbError::AccessDenied(
            "deletion lease expired during cleanup batch".into(),
        ));
    }
    Ok(())
}
