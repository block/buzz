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

/// The Operator's confirmed archive request.
pub struct OperatorArchiveRequest<'a> {
    /// Community whose archive state is changing.
    pub community: CommunityId,
    /// Selected key, first in the confirmed identity set.
    pub target: &'a [u8; 32],
    /// Lowercase hex key of the authenticated Operator.
    pub actor: &'a str,
    /// Version of the identity set from the ownership preview.
    pub confirmed_version: &'a str,
    /// Optional NIP-IA reason.
    pub reason: Option<&'a str>,
    /// Optional replacement key for the target.
    pub replaced_by: Option<&'a str>,
    /// Lowercase hex keys from the deployment's Operator allowlist.
    pub configured_operators: &'a [String],
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
    /// The target or one of its direct agents is relay staff.
    #[error("identity set includes relay staff")]
    TargetIsStaff,
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
        request: OperatorArchiveRequest<'_>,
    ) -> Result<Vec<ArchiveChange>, ArchiveInventoryError> {
        let OperatorArchiveRequest {
            community,
            target,
            actor,
            confirmed_version,
            reason,
            replaced_by,
            configured_operators,
        } = request;
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

        let agents: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT pubkey FROM users \
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

        let pubkeys: Vec<_> = std::iter::once(target.to_vec()).chain(agents).collect();
        let keys: Vec<_> = pubkeys.iter().map(hex::encode).collect();
        if inventory_version(keys.iter().map(String::as_str)) != confirmed_version {
            return Err(ArchiveInventoryError::Changed);
        }

        // Check the exact set under the same ownership lock used for the write.
        // The Operator API requires a configured key, so the admin-host owner
        // fallback can never be effective here.
        if keys.iter().any(|key| configured_operators.contains(key))
            || sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM relay_operators WHERE pubkey = ANY($1))",
            )
            .bind(&pubkeys)
            .fetch_one(tx.conn())
            .await?
        {
            return Err(ArchiveInventoryError::TargetIsStaff);
        }

        let mut identities = Vec::with_capacity(keys.len());
        for (index, key) in keys.into_iter().enumerate() {
            let replacement = if index == 0 { replaced_by } else { None };
            let changed =
                archive_in_tx(&mut tx, &key, "relay", actor, reason, replacement, "").await?;
            identities.push(ArchiveChange {
                pubkey: key,
                changed,
            });
        }
        tx.commit().await?;
        Ok(identities)
    }
}
