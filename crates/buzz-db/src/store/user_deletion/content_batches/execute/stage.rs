//! Request ownership and atomic progress for the workflow adapter.
use super::*;
use crate::entity_deletion::EntityLease;
use sqlx::{Postgres, Transaction};

pub(super) struct Context<'a> {
    pub lease: &'a EntityLease,
    pub id: Uuid,
    pub options: &'a ContentBatchOptions,
}

impl Context<'_> {
    fn binding(&self) -> Result<Value> {
        Ok(json!({
            "version":1,
            "community":self.options.community,
            "target":normalize_user_deletion_pubkey(&self.options.pubkey)?,
            "relay_pubkey":self.options.relay_pubkey.as_deref().map(normalize_user_deletion_pubkey).transpose()?
        }))
    }

    pub async fn bind(&self, tx: &mut Transaction<'_, Postgres>) -> Result<()> {
        let metadata = self.validate_stage(tx).await?;
        let binding = self.binding()?;
        if let Some(saved) = metadata.get("content_binding") {
            if saved != &binding {
                return Err(invalid("content stage semantic options changed"));
            }
        } else {
            if metadata.get("content_batches").is_some() {
                return Err(invalid("content progress has no bound semantic options"));
            }
            sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{content_binding}',$2),updated_at=clock_timestamp() WHERE id=$1")
                .bind(self.id).bind(binding).execute(&mut **tx).await?;
        }
        Ok(())
    }

    pub async fn validate(&self, tx: &mut Transaction<'_, Postgres>) -> Result<()> {
        if self.validate_stage(tx).await?["content_binding"] != self.binding()? {
            return Err(invalid("content stage semantic options changed"));
        }
        Ok(())
    }

    async fn validate_stage(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Value> {
        let request = crate::store::entity_deletion::lock_owned(tx, self.lease).await?;
        crate::store::entity_deletion::validate_approved_inventory(tx, &request).await?;
        if request.target != normalize_user_deletion_pubkey(&self.options.pubkey)? {
            return Err(invalid("content target does not match approved request"));
        }
        let inventory = crate::user_deletion::UserDeletionInventory::read(&request)?;
        let expected = inventory.inputs(&request.target, Some(self.options.community))?;
        let metadata: Option<Value> = sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$2 AND stage='purge_content' AND target_community_id=$3 AND is_current AND status='running' FOR UPDATE")
            .bind(self.id).bind(self.lease.request_id).bind(self.options.community)
            .fetch_optional(&mut **tx).await?;
        let metadata = metadata.ok_or_else(|| invalid("content stage is no longer running"))?;
        if metadata["inputs"] != expected {
            return Err(invalid(
                "content stage is not running with the approved inputs",
            ));
        }
        Ok(metadata)
    }

    pub async fn record_batch(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: usize,
    ) -> Result<()> {
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{content_batches}',jsonb_build_object('items',COALESCE((metadata_json->'content_batches'->>'items')::bigint,0)+$2,'batches',COALESCE((metadata_json->'content_batches'->>'batches')::bigint,0)+1,'account_erasure_complete',false)),updated_at=clock_timestamp() WHERE id=$1")
            .bind(self.id).bind(i64::try_from(count).map_err(|_| invalid("batch count overflow"))?)
            .execute(&mut **tx).await?;
        Ok(())
    }
}
