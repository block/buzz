//! Parent ownership and bounded progress for the real account stage.
use super::*;
use crate::entity_deletion::{EntityLease, EntityStage};
use crate::store::entity_deletion::{lock_owned, validate_approved_inventory};
use crate::user_deletion::UserDeletionInventory;

pub(super) async fn guard(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    lease: &EntityLease,
    stage_id: Uuid,
    options: &AccountBatchOptions,
) -> Result<()> {
    let request = lock_owned(tx, lease).await?;
    validate_approved_inventory(tx, &request).await?;
    if request.status != "running"
        || normalize_user_deletion_pubkey(&request.target)? != options.pubkey
    {
        return Err(invalid("account stage target or request status changed"));
    }
    let inputs =
        UserDeletionInventory::read(&request)?.inputs(&request.target, Some(options.community))?;
    let row: Option<EntityStage> = sqlx::query_as(
        "SELECT id, request_id, target_community_id, stage, status, is_current,
                started_at, finished_at, metadata_json, last_error, updated_at
         FROM entity_deletion_stage_history
         WHERE id=$1 AND request_id=$2 FOR UPDATE",
    )
    .bind(stage_id)
    .bind(lease.request_id)
    .fetch_optional(&mut **tx)
    .await?;
    let row = row.ok_or_else(|| invalid("account stage does not belong to request"))?;
    if !row.is_current
        || row.status != "running"
        || row.stage != "purge_account"
        || row.target_community_id != Some(options.community)
        || row.metadata_json["inputs"] != inputs
    {
        return Err(invalid(
            "account stage is not current, running, and in approved scope",
        ));
    }
    Ok(())
}

pub(super) async fn progress(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    stage_id: Uuid,
    op: &Operation,
    items: usize,
) -> Result<()> {
    // The parent and stage rows are already locked by guard. Keep only bounded
    // counters and the last operation; retry selects remaining source rows.
    let saved: Option<Value> = sqlx::query_scalar(
        "SELECT metadata_json->'account_batches' FROM entity_deletion_stage_history WHERE id=$1",
    )
    .bind(stage_id)
    .fetch_one(&mut **tx)
    .await?;
    let (old_items, old_batches) = match saved {
        None => (0_u64, 0_u64),
        Some(saved) if saved["version"] == 1 && saved["account_erasure_complete"] == false => (
            saved["items"]
                .as_u64()
                .ok_or_else(|| invalid("invalid account progress count"))?,
            saved["batches"]
                .as_u64()
                .ok_or_else(|| invalid("invalid account progress count"))?,
        ),
        Some(_) => return Err(invalid("unsupported account progress")),
    };
    let items = old_items
        .checked_add(items as u64)
        .ok_or_else(|| invalid("account progress overflow"))?;
    let batches = old_batches
        .checked_add(1)
        .ok_or_else(|| invalid("account progress overflow"))?;
    sqlx::query(
        "UPDATE entity_deletion_stage_history
         SET metadata_json=jsonb_set(metadata_json,'{account_batches}',$2),
             updated_at=clock_timestamp() WHERE id=$1",
    )
    .bind(stage_id)
    .bind(json!({"version":1,"items":items,"batches":batches,
        "last_table":op.table,"last_action":op.action,"account_erasure_complete":false}))
    .execute(&mut **tx)
    .await?;
    Ok(())
}
