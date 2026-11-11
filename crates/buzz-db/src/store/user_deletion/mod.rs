//! User request preparation and offline account-table cleanup.
use crate::Db;
use sqlx::PgPool;
/// Bounded, offline cleanup of one human's community account records.
pub mod account_batches;
mod discovery;
mod inventory;
/// Read-only, ordered previews of human content cleanup batches.
pub mod preview;
mod requests;
pub use discovery::normalize_user_deletion_pubkey;
pub use inventory::UserDeletionInventory;

const MAX_COMMUNITIES: usize = 1000;

/// User-deletion inventory, request preparation, and offline account operations.
#[derive(Clone)]
pub struct UserDeletionStore {
    pool: PgPool,
}
impl Db {
    /// Access user-specific deletion request preparation using the writer pool.
    pub fn user_deletion_store(&self) -> UserDeletionStore {
        UserDeletionStore {
            pool: self.pool().clone(),
        }
    }
}

#[cfg(test)]
#[path = "inventory_postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
mod discovery_postgres_tests;

impl UserDeletionStore {
    /// Refuse offline execution while a serving worker owns external work.
    /// This is a point-in-time check, not an online traffic fence. Operators
    /// must keep the relay and external workers stopped throughout execution.
    pub async fn verify_offline_work_stopped(&self) -> crate::Result<()> {
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM community_serving_write_leases WHERE lease_until>clock_timestamp())")
            .fetch_one(&self.pool).await?;
        if active {
            return Err(crate::DbError::DeletionSafety(
                "stop external serving work before offline deletion".into(),
            ));
        }
        Ok(())
    }
}
