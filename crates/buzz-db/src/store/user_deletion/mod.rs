//! User-specific submission and inventory preparation over the shared deletion tables.
use crate::Db;
use sqlx::PgPool;
/// Bounded cleanup of one human's community account records.
pub mod account_batches;
/// Ordered user and owned-agent content cleanup in read-only and delete modes.
pub mod content_batches;
mod discovery;
mod inventory;
mod media;
/// Standalone, offline profile cleanup with read-only preview.
pub mod profile_batches;
mod requests;
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
