//! Restore-independent monotonic version snapshot for protected authority.

use std::collections::BTreeMap;

use buzz_core::CommunityId;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{Db, Result};

/// Hashed per-resource high-water marks safe for an external checkpoint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthorizationVersionVector {
    /// Issuer-qualified binding selector fingerprint to maximum version.
    pub bindings: BTreeMap<String, u64>,
    /// Git repository selector fingerprint to publication version.
    pub git_publications: BTreeMap<String, u64>,
    /// Media-object selector fingerprint to publication version.
    pub media_publications: BTreeMap<String, u64>,
    /// Protected object surface to cutover generation.
    pub object_authority: BTreeMap<String, u64>,
    /// Durable invalidation generation.
    pub invalidation_generation: u64,
    /// Complete PostgreSQL authority epoch, including tombstones and membership.
    pub authority_epoch: u64,
    /// Durable current-only client-status revision floor.
    pub status_revision: u64,
}

impl AuthorizationVersionVector {
    /// True when every recorded floor exists at an equal or higher version.
    pub fn dominates(&self, floor: &Self) -> bool {
        dominates_map(&self.bindings, &floor.bindings)
            && dominates_map(&self.git_publications, &floor.git_publications)
            && dominates_map(&self.media_publications, &floor.media_publications)
            && dominates_map(&self.object_authority, &floor.object_authority)
            && self.invalidation_generation >= floor.invalidation_generation
            && self.authority_epoch >= floor.authority_epoch
            && self.status_revision >= floor.status_revision
    }
}

fn dominates_map(current: &BTreeMap<String, u64>, floor: &BTreeMap<String, u64>) -> bool {
    floor
        .iter()
        .all(|(selector, version)| current.get(selector).is_some_and(|value| value >= version))
}

fn selector_fingerprint(namespace: &[u8], parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"buzz-authorization-version-selector-v1");
    digest.update((namespace.len() as u64).to_be_bytes());
    digest.update(namespace);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    hex::encode(digest.finalize())
}

impl Db {
    /// Exact domains that have crossed the one-way Enforce activation boundary.
    pub async fn activated_authorization_domains(&self) -> Result<Vec<CommunityId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT community_id FROM authorization_invalidation_domains ORDER BY community_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(CommunityId::from_uuid).collect())
    }

    /// Idempotently activate one Enforce invalidation domain and commit the
    /// exact receipt in the same transaction.
    ///
    /// Production construction witnesses this mutation independently before
    /// making the protected transport reachable. Observational modes never
    /// call this API.
    pub async fn activate_authorization_domain(
        &self,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<()> {
        const OPERATION_KIND: &str = "authorization_domain_activate_v1";
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(operation_id.to_string())
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query(
            "SELECT operation_kind, request_fingerprint FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2 FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let kind: String = row.try_get("operation_kind")?;
            let fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
            if kind != OPERATION_KIND || fingerprint.as_slice() != request_fingerprint {
                return Err(crate::DbError::InvalidData(
                    "protected-domain activation retry conflicts with its durable receipt"
                        .to_owned(),
                ));
            }
            tx.commit().await?;
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id) VALUES ($1) \
             ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, operation_kind, request_fingerprint, \
              result_version, result_payload, lease_expires_at) \
             VALUES ($1,$2,$3,$4,1,$5,clock_timestamp()+interval '100 years')",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .bind(OPERATION_KIND)
        .bind(request_fingerprint.as_slice())
        .bind([1_u8].as_slice())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether a transaction-owned protected operation committed.
    pub async fn has_authorization_operation_receipt(
        &self,
        community_id: CommunityId,
        operation_id: uuid::Uuid,
    ) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2)",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    /// Return the exact request fingerprint for a committed protected operation.
    pub async fn authorization_operation_receipt_fingerprint(
        &self,
        community_id: CommunityId,
        operation_id: uuid::Uuid,
    ) -> Result<Option<[u8; 32]>> {
        let value: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT request_fingerprint FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    crate::DbError::InvalidData(
                        "authorization receipt fingerprint must be 32 bytes".to_owned(),
                    )
                })
            })
            .transpose()
    }

    /// Read the complete protected-authority high-water vector from the writer.
    pub async fn authorization_version_vector(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationVersionVector> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let vector = authorization_version_vector_tx(&mut tx, community_id).await?;
        tx.commit().await?;
        Ok(vector)
    }

    /// Read an exact operation receipt and the complete authority vector from
    /// one writer-consistent snapshot. Pending restore recovery must not join
    /// a receipt from one database state to a vector from another.
    pub async fn authorization_receipt_and_version_vector(
        &self,
        community_id: CommunityId,
        operation_id: Uuid,
    ) -> Result<(Option<[u8; 32]>, AuthorizationVersionVector)> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let fingerprint: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT request_fingerprint FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?;
        let fingerprint = fingerprint
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    crate::DbError::InvalidData(
                        "authorization receipt fingerprint must be 32 bytes".to_owned(),
                    )
                })
            })
            .transpose()?;
        let vector = authorization_version_vector_tx(&mut tx, community_id).await?;
        tx.commit().await?;
        Ok((fingerprint, vector))
    }
}

async fn authorization_version_vector_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
) -> Result<AuthorizationVersionVector> {
    let mut vector = AuthorizationVersionVector::default();
    for row in sqlx::query(
        "SELECT issuer, uid AS subject, max(binding_version) AS version \
             FROM identity_bindings WHERE community_id=$1 \
             GROUP BY issuer, uid",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?
    {
        let issuer: String = row.try_get("issuer")?;
        let subject: String = row.try_get("subject")?;
        let version: i64 = row.try_get("version")?;
        if let Ok(version) = u64::try_from(version) {
            vector.bindings.insert(
                selector_fingerprint(b"binding", &[issuer.as_bytes(), subject.as_bytes()]),
                version,
            );
        }
    }
    for row in sqlx::query(
        "SELECT repo_id, publication_version FROM git_repo_publications \
             WHERE community_id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?
    {
        let repo_id: String = row.try_get("repo_id")?;
        let version: i64 = row.try_get("publication_version")?;
        if let Ok(version) = u64::try_from(version) {
            vector
                .git_publications
                .insert(selector_fingerprint(b"git", &[repo_id.as_bytes()]), version);
        }
    }
    for row in sqlx::query(
        "SELECT sha256, publication_version FROM media_publications \
             WHERE community_id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?
    {
        let digest: String = row.try_get("sha256")?;
        let version: i64 = row.try_get("publication_version")?;
        if let Ok(version) = u64::try_from(version) {
            vector.media_publications.insert(
                selector_fingerprint(b"media", &[digest.as_bytes()]),
                version,
            );
        }
    }
    for row in sqlx::query(
        "SELECT surface, generation FROM protected_object_authority \
             WHERE community_id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?
    {
        let surface: String = row.try_get("surface")?;
        let generation: i64 = row.try_get("generation")?;
        if let Ok(generation) = u64::try_from(generation) {
            vector.object_authority.insert(surface, generation);
        }
    }
    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT generation FROM authorization_invalidation_domains WHERE community_id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    vector.invalidation_generation = generation
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default();
    let epoch: Option<(i64, i64)> = sqlx::query_as(
        "SELECT authority_epoch, status_revision \
             FROM authorization_authority_epochs WHERE community_id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((authority_epoch, status_revision)) = epoch {
        vector.authority_epoch = u64::try_from(authority_epoch).unwrap_or_default();
        vector.status_revision = u64::try_from(status_revision).unwrap_or_default();
    }
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_lower_component_never_dominates() {
        let mut floor = AuthorizationVersionVector::default();
        floor.bindings.insert("a".into(), 2);
        let mut current = floor.clone();
        assert!(current.dominates(&floor));
        current.bindings.insert("a".into(), 1);
        assert!(!current.dominates(&floor));
        current.bindings.clear();
        assert!(!current.dominates(&floor));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn activation_precedes_first_mutation_and_makes_lifecycle_visible() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrations");
        let db = Db::from_pool(pool);
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("activation-{}.example", community.as_uuid()))
            .execute(&db.pool)
            .await
            .expect("community");
        let operation_id = Uuid::new_v4();
        let fingerprint = [0x71; 32];

        db.activate_authorization_domain(community, operation_id, fingerprint)
            .await
            .expect("domain activation");
        db.activate_authorization_domain(community, operation_id, fingerprint)
            .await
            .expect("activation retry");
        let before = db
            .authorization_invalidation_snapshot(community)
            .await
            .expect("initial snapshot");

        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(community.as_uuid())
            .bind("11".repeat(32))
            .execute(&db.pool)
            .await
            .expect("first protected lifecycle mutation");
        let after = db
            .authorization_invalidation_snapshot(community)
            .await
            .expect("advanced snapshot");

        assert_eq!(after.generation, before.generation + 1);
        assert_eq!(
            db.authorization_operation_receipt_fingerprint(community, operation_id)
                .await
                .expect("activation receipt"),
            Some(fingerprint)
        );
        let (atomic_receipt, atomic_vector) = db
            .authorization_receipt_and_version_vector(community, operation_id)
            .await
            .expect("receipt and vector snapshot");
        assert_eq!(atomic_receipt, Some(fingerprint));
        assert_eq!(
            atomic_vector,
            db.authorization_version_vector(community)
                .await
                .expect("stable vector after snapshot")
        );
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn git_policy_replacement_advances_restore_authority() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrations");
        let db = Db::from_pool(pool);
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("git-policy-{}.example", community.as_uuid()))
            .execute(&db.pool)
            .await
            .expect("community");
        sqlx::query("INSERT INTO authorization_invalidation_domains (community_id) VALUES ($1)")
            .bind(community.as_uuid())
            .execute(&db.pool)
            .await
            .expect("activate protected domain");
        let before = db
            .authorization_version_vector(community)
            .await
            .expect("pre-policy vector");
        let event_id = Sha256::digest(Uuid::new_v4().as_bytes()).to_vec();
        let owner = [0x42_u8; 32];
        sqlx::query(
            "INSERT INTO events \
             (community_id,id,pubkey,created_at,kind,tags,content,sig,d_tag) \
             VALUES ($1,$2,$3,clock_timestamp(),30617,$4,'',$5,'repo')",
        )
        .bind(community.as_uuid())
        .bind(&event_id)
        .bind(owner.as_slice())
        .bind(serde_json::json!([["d", "repo"], ["protected", "true"]]))
        .bind(vec![0_u8; 64])
        .execute(&db.pool)
        .await
        .expect("protected Git policy");
        let inserted = db
            .authorization_version_vector(community)
            .await
            .expect("inserted policy vector");
        assert!(inserted.authority_epoch > before.authority_epoch);
        assert!(inserted.dominates(&before));
        assert!(!before.dominates(&inserted));

        sqlx::query(
            "UPDATE events SET deleted_at=clock_timestamp() \
             WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(&event_id)
        .execute(&db.pool)
        .await
        .expect("retire protected Git policy");
        let retired = db
            .authorization_version_vector(community)
            .await
            .expect("retired policy vector");
        assert!(retired.authority_epoch > inserted.authority_epoch);
        assert!(!inserted.dominates(&retired));
    }
}
