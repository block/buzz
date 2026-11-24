//! Operator archive of a confirmed community-local identity set.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use sha2::{Digest, Sha256};

use crate::archived_identities::archive_in_tx;
use crate::{Db, DbError};

/// Largest identity set an Operator may archive in one request, including the target.
pub const MAX_ARCHIVE_IDENTITIES: usize = 200;

/// A key included in the confirmed archive and whether this call archived it.
#[derive(Debug)]
pub struct ArchiveChange {
    /// Lowercase hex key.
    pub pubkey: String,
    /// Whether this call inserted the archive row.
    pub changed: bool,
}

/// Why an Operator archive was refused.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveInventoryError {
    /// The recorded ownership changed since the preview.
    #[error("identity ownership changed since it was confirmed")]
    Changed,
    /// The confirmed set would exceed the size limit.
    #[error("identity inventory exceeds {MAX_ARCHIVE_IDENTITIES} keys")]
    TooLarge,
    /// Database failure.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<sqlx::Error> for ArchiveInventoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(error.into())
    }
}

/// Hash the sorted lowercase hex keys that the Operator is asked to confirm.
pub(crate) fn inventory_version<'a>(keys: impl IntoIterator<Item = &'a str>) -> String {
    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort_unstable();
    let mut hash = Sha256::new();
    hash.update(b"identity-inventory-v2\n");
    for key in keys {
        hash.update(key.as_bytes());
        hash.update(b"\n");
    }
    hex::encode(hash.finalize())
}

impl Db {
    /// Revalidate the target and its direct agents and archive them together.
    ///
    /// Take the invite-admission lock used by the ownership writers before
    /// reading the confirmed set. The target is first in the returned list.
    #[datastore_span(name = "archive_operator_identities", system = "postgresql")]
    pub async fn archive_operator_identities(
        &self,
        community: CommunityId,
        target: &[u8; 32],
        actor: &str,
        confirmed_version: &str,
        reason: Option<&str>,
        replaced_by: Option<&str>,
    ) -> Result<Vec<ArchiveChange>, ArchiveInventoryError> {
        let mut tx = crate::begin_community_write_transaction(
            self.pool(),
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query("SELECT set_config('lock_timeout', '5s', true), set_config('statement_timeout', '15s', true)")
            .execute(tx.conn())
            .await?;
        crate::relay_invite::lock_admission(tx.conn(), community).await?;

        let agents: Vec<String> = sqlx::query_scalar(
            "SELECT encode(pubkey, 'hex') FROM users \
             WHERE community_id = $1 AND agent_owner_pubkey = $2 AND pubkey <> $2 \
             ORDER BY pubkey LIMIT $3",
        )
        .bind(community.as_uuid())
        .bind(target.as_slice())
        .bind(MAX_ARCHIVE_IDENTITIES as i64)
        .fetch_all(tx.conn())
        .await?;
        if agents.len() >= MAX_ARCHIVE_IDENTITIES {
            return Err(ArchiveInventoryError::TooLarge);
        }

        let root = hex::encode(target);
        let keys = std::iter::once(&root).chain(&agents);
        if inventory_version(keys.clone().map(String::as_str)) != confirmed_version {
            return Err(ArchiveInventoryError::Changed);
        }

        let mut identities = Vec::with_capacity(1 + agents.len());
        for key in keys {
            let replacement = if key == &root { replaced_by } else { None };
            let changed =
                archive_in_tx(&mut tx, key, "relay", actor, reason, replacement, "").await?;
            identities.push(ArchiveChange {
                pubkey: key.clone(),
                changed,
            });
        }
        tx.commit().await?;
        Ok(identities)
    }
}
