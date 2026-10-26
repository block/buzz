//! User-specific submission and inventory preparation over the shared deletion tables.
use crate::Db;
use sqlx::PgPool;
mod inventory;
mod requests;
pub use inventory::UserDeletionInventory;

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
