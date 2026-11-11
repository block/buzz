//! User request preparation and offline account-table cleanup.
use crate::Db;
use sqlx::PgPool;
/// Bounded, offline cleanup of one human's community account records.
pub mod account_batches;
mod discovery;
mod inventory;
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
