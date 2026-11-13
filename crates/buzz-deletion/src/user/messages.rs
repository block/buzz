//! Standalone message handler. Deliberately absent from dispatch and routing.

use anyhow::{bail, Context, Result};
use buzz_core::CommunityId;
use buzz_db::{
    entity_deletion::{EntityLease, EntityStage},
    user_deletion::messages::MessageBatch,
    Db,
};

/// Run one atomic thread batch and record failure or final stage success.
///
/// The caller owns heartbeat/release and invokes this again while `exhausted`
/// is false. Restart selects remaining database rows; no message cursor or
/// message content is written to stage metadata. This handler never chooses
/// another stage or community. See `purge_message_batch` for drain prerequisites
/// and the intentionally blocked attachment/workflow cases.
pub async fn handle(
    db: &Db,
    lease: &EntityLease,
    stage: &EntityStage,
    relay_pubkey: &[u8; 32],
    batch_size: u16,
) -> Result<MessageBatch> {
    if stage.request_id != lease.request_id || stage.stage != "purge_messages" {
        bail!("expected this request's purge_messages stage");
    }
    let community = CommunityId::from_uuid(
        stage
            .target_community_id
            .context("missing message community")?,
    );
    let store = db.entity_deletion_store();
    let stage = store.start_stage(lease, stage.id).await?;
    match db
        .user_deletion_store()
        .purge_message_batch(lease, stage.id, community, relay_pubkey, batch_size)
        .await
    {
        Ok(batch) => {
            if batch.exhausted {
                store
                    .save_stage_success(
                        lease,
                        stage.id,
                        serde_json::json!({"message_rows_exhausted": true}),
                    )
                    .await?;
            }
            Ok(batch)
        }
        Err(error) => {
            if let Err(save_error) = store
                .fail_stage(lease, stage.id, &error.to_string(), crate::RETRY_DELAY)
                .await
            {
                return Err(anyhow::Error::new(error)
                    .context(format!("also could not save retry: {save_error}")));
            }
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod postgres_tests;
