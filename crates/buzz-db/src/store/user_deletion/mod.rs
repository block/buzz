//! User-specific submission and inventory preparation over the shared deletion tables.
use crate::Db;
use sqlx::PgPool;
/// Bounded cleanup of one human's community account records.
pub mod account_batches;
mod agent_ownership;
/// Ordered user and owned-agent content cleanup in read-only and delete modes.
pub mod content_batches;
mod discovery;
mod inventory;
mod media;
/// Profile cleanup with read-only preview and bounded delete batches.
pub mod profile_batches;
pub use agent_ownership::UserDeletionTargets;
mod requests;
pub(crate) use discovery::check_community_ownership;
pub use discovery::normalize_user_deletion_pubkey;
pub use inventory::UserDeletionInventory;

/// Select whether a cleanup handler reports or applies its operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Report operations without changing source data.
    ReadOnly,
    /// Apply the reported operations.
    Delete,
}

const MAX_COMMUNITIES: usize = 1000;

/// Database operations that prepare user-deletion requests.
#[derive(Clone)]
pub struct UserDeletionStore {
    pool: PgPool,
    account_snapshot_signer: Option<nostr::Keys>,
}
impl Db {
    /// Access user-specific deletion request preparation using the writer pool.
    pub fn user_deletion_store(&self) -> UserDeletionStore {
        UserDeletionStore {
            pool: self.pool().clone(),
            account_snapshot_signer: None,
        }
    }
}

#[cfg(test)]
#[path = "inventory_postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
mod discovery_postgres_tests;

impl UserDeletionStore {
    /// Configure the deployment's existing relay key for atomic account roster repair.
    /// Never generate an ephemeral key: readers trust the serving relay's identity.
    pub fn with_account_snapshot_signer(mut self, signer: nostr::Keys) -> Self {
        self.account_snapshot_signer = Some(signer);
        self
    }
}

/// Verify the worker still owns this running stage for the approved target.
/// Hold the parent lock through the batch commit.
async fn validate_cleanup_stage(
    tx: &mut sqlx::PgConnection,
    lease: &crate::entity_deletion::EntityLease,
    stage_id: uuid::Uuid,
    stage: &str,
    community: Option<uuid::Uuid>,
    target: &str,
) -> crate::Result<()> {
    use crate::store::entity_deletion::{lock_owned, validate_approved_inventory};
    if let Some(id) = community {
        check_community_ownership(tx, target, id, false).await?;
    }
    let request = lock_owned(tx, lease).await?;
    validate_approved_inventory(tx, &request).await?;
    let inputs =
        UserDeletionInventory::read(&request)?.non_owner_inputs(&request.target, community)?;
    if request.status != "running" || normalize_user_deletion_pubkey(&request.target)? != target {
        return Err(crate::DbError::DeletionSafety(format!(
            "{stage} target or status changed"
        )));
    }
    let matches: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM entity_deletion_stage_history
         WHERE id=$1 AND request_id=$2 AND stage=$5 AND is_current
         AND status='running' AND target_community_id=$3 AND metadata_json->'inputs'=$4)",
    )
    .bind(stage_id)
    .bind(lease.request_id)
    .bind(community)
    .bind(inputs)
    .bind(stage)
    .fetch_one(&mut *tx)
    .await?;
    if !matches {
        return Err(crate::DbError::DeletionSafety(format!(
            "{stage} is not current or outside approved scope"
        )));
    }
    Ok(())
}
