//! User-specific submission and inventory preparation over the shared deletion tables.
use crate::Db;
use sqlx::PgPool;
/// Standalone owned-agent database cleanup and durable per-agent progress.
pub mod agents;
mod discovery;
mod execution;
mod inventory;
/// Standalone per-thread message cleanup.
pub mod messages;
mod requests;
pub use discovery::normalize_user_deletion_pubkey;
pub use inventory::UserDeletionInventory;

const MAX_COMMUNITIES: usize = 1000;

/// Database operations that prepare user-deletion requests.
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
