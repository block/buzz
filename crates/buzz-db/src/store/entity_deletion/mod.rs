//! Shared deletion-workflow persistence. User policy lives in buzz-deletion.
use crate::{Db, DbError, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Writer-database access to deletion workflow requests.
#[derive(Clone)]
pub struct EntityDeletionStore {
    pool: PgPool,
}

impl Db {
    /// Use the writer pool for operator-global request intake and inspection.
    pub fn entity_deletion_store(&self) -> EntityDeletionStore {
        EntityDeletionStore {
            pool: self.pool().clone(),
        }
    }
}

mod approvals;
mod leases;
pub(crate) mod requests;
mod stages;
pub use approvals::EntityApproval;
pub(super) use leases::{lock_owned, validate_approved_inventory};
pub use leases::{EntityExecution, EntityLease};
pub use requests::{inventory_digest, EntityInspection, EntityRequest};
use requests::{validate_operator, validated_inventory_digest};
pub use stages::{EntityStage, NewEntityStage, NextStage, StageTransition};

#[cfg(test)]
#[path = "../entity_deletion_postgres_tests.rs"]
mod postgres_tests;
