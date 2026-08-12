//! PostgreSQL-backed local binding resolution.
//!
//! All existing-binding and status paths are observation-only. They never
//! enroll, consume lifecycle selectors, append history, or refresh timestamps.

use std::fmt;

use buzz_auth::{
    ActiveLocalBinding, BindingResolutionRequest, CurrentBindingStatusEvidenceRequest,
    LocalAuthorizationPolicy, LocalBindingResolution, LocalBindingResolver,
    LocalBindingResolverCapability,
};
use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use sha2::{Digest, Sha256};
use sqlx::Row;
use thiserror::Error;

use crate::{
    authorization_version::{protected_object_key, AuthorizationProtectedObjectKind},
    Db, DbError,
};

/// Concrete provider-free PostgreSQL local binding resolver.
#[derive(Clone)]
pub struct PostgresLocalBindingResolver {
    db: Db,
}

impl PostgresLocalBindingResolver {
    /// Bind the resolver to the authoritative writer database.
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Borrow the underlying database handle for same-authority composition.
    pub const fn db(&self) -> &Db {
        &self.db
    }
}

impl fmt::Debug for PostgresLocalBindingResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PostgresLocalBindingResolver([REDACTED])")
    }
}

/// Fail-closed local resolver errors.
#[derive(Debug, Error)]
pub enum AuthorizationResolverError {
    /// PostgreSQL failed or returned malformed authoritative state.
    #[error(transparent)]
    Database(#[from] DbError),
    /// No exact current binding exists.
    #[error("current local binding is unavailable")]
    BindingUnavailable,
    /// A frozen cross-lane trusted constructor or mutation seam is not installed.
    #[error("local binding resolver contract is unavailable")]
    ContractUnavailable,
    /// Delegated protected transport has no reviewed positive authority source.
    #[error("delegated protected authorization is unavailable")]
    DelegationAuthorityUnavailable,
    /// The exact local policy or protected authority snapshot is unavailable.
    #[error("local authorization policy is unavailable")]
    PolicyUnavailable,
}

impl PostgresLocalBindingResolver {
    /// Resolve the exact direct-route policy and protected-object fence from
    /// one repeatable-read PostgreSQL snapshot.
    pub async fn protected_publication_policy(
        &self,
        request: &BindingResolutionRequest,
        object_kind: AuthorizationProtectedObjectKind,
        maximum_lease: chrono::Duration,
    ) -> std::result::Result<(LocalAuthorizationPolicy, DateTime<Utc>), AuthorizationResolverError>
    {
        if maximum_lease <= chrono::Duration::zero() {
            return Err(AuthorizationResolverError::PolicyUnavailable);
        }
        let (assertion, proof, capability) = match request {
            BindingResolutionRequest::Direct {
                assertion,
                proof,
                capability,
            } => (assertion, proof, *capability),
            BindingResolutionRequest::Delegated { .. } => {
                return Err(AuthorizationResolverError::DelegationAuthorityUnavailable);
            }
            BindingResolutionRequest::Enrollment { .. } => {
                return Err(AuthorizationResolverError::ContractUnavailable);
            }
        };
        if assertion.authorization_domain() != proof.authorization_domain()
            || !object_kind.admits_read(capability)
        {
            return Err(AuthorizationResolverError::PolicyUnavailable);
        }

        let domain = assertion.authorization_domain();
        let principal = assertion.principal_storage_key();
        let object_key = protected_object_key(domain, object_kind, *proof.target_fingerprint());
        let mut transaction = self.db.pool.begin().await.map_err(DbError::from)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(DbError::from)?;
        let row = sqlx::query(
            "SELECT binding.policy_revision,policy.expires_at AS policy_expires_at, \
                    invalidation.current_generation,clock_timestamp() AS authoritative_now, \
                    epoch.authority_epoch AS epoch_authority_epoch,epoch.fence AS epoch_fence, \
                    protected.authority_epoch AS protected_authority_epoch, \
                    protected.fence AS protected_fence \
             FROM identity_bindings binding \
             JOIN identity_enrollment_policies policy \
               ON policy.community_id=binding.community_id \
              AND policy.policy_revision=binding.policy_revision \
             JOIN authorization_invalidation_domains invalidation \
               ON invalidation.community_id=binding.community_id \
             LEFT JOIN authorization_authority_epochs epoch \
               ON epoch.community_id=binding.community_id AND epoch.object_kind=$5 \
              AND epoch.object_key=$6 \
             LEFT JOIN protected_object_authority protected \
               ON protected.community_id=binding.community_id AND protected.object_kind=$5 \
              AND protected.object_key=$6 \
             WHERE binding.community_id=$1 AND binding.issuer=$2 AND binding.subject=$3 \
               AND binding.event_author_pubkey=$4 AND binding.binding_state=1 \
               AND (binding.expires_at IS NULL OR binding.expires_at > clock_timestamp()) \
               AND policy.effective_at <= clock_timestamp() \
               AND (policy.expires_at IS NULL OR policy.expires_at > clock_timestamp())",
        )
        .bind(domain.as_uuid())
        .bind(principal.issuer())
        .bind(principal.subject())
        .bind(proof.actor_pubkey().to_bytes().as_slice())
        .bind(object_kind as i16)
        .bind(object_key.as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(DbError::from)?;
        transaction.commit().await.map_err(DbError::from)?;
        let row = row.ok_or(AuthorizationResolverError::PolicyUnavailable)?;
        let now: DateTime<Utc> = row.try_get("authoritative_now").map_err(DbError::from)?;
        let policy_expires_at: Option<DateTime<Utc>> =
            row.try_get("policy_expires_at").map_err(DbError::from)?;
        let configured_expires_at = now
            .checked_add_signed(maximum_lease)
            .ok_or(AuthorizationResolverError::PolicyUnavailable)?;
        let expires_at = policy_expires_at
            .map(|expiry| expiry.min(configured_expires_at))
            .unwrap_or(configured_expires_at);
        if expires_at <= now {
            return Err(AuthorizationResolverError::PolicyUnavailable);
        }

        let lease_id = uuid::Uuid::new_v4();
        let epoch = row
            .try_get::<Option<i64>, _>("epoch_authority_epoch")
            .map_err(DbError::from)?
            .map(|value| database_u64(value, "authority epoch"))
            .transpose()?;
        let epoch_fence = row
            .try_get::<Option<Vec<u8>>, _>("epoch_fence")
            .map_err(DbError::from)?
            .map(|value| parsed_fence(value, "authority fence"))
            .transpose()?;
        let protected_epoch = row
            .try_get::<Option<i64>, _>("protected_authority_epoch")
            .map_err(DbError::from)?
            .map(|value| database_u64(value, "protected authority epoch"))
            .transpose()?;
        let protected_fence = row
            .try_get::<Option<Vec<u8>>, _>("protected_fence")
            .map_err(DbError::from)?
            .map(|value| parsed_fence(value, "protected authority fence"))
            .transpose()?;
        let (authority_epoch, fence) = match (epoch, epoch_fence, protected_epoch, protected_fence)
        {
            (None, None, None, None) => (
                1,
                initial_publication_fence(domain, object_kind, object_key, lease_id)?,
            ),
            (Some(epoch), Some(epoch_fence), Some(protected_epoch), Some(protected_fence))
                if epoch == protected_epoch && epoch_fence == protected_fence =>
            {
                (epoch, epoch_fence)
            }
            _ => {
                return Err(AuthorizationResolverError::Database(DbError::InvalidData(
                    "protected publication authority rows disagree".to_owned(),
                )))
            }
        };
        let policy_revision = database_u64(
            row.try_get("policy_revision").map_err(DbError::from)?,
            "policy revision",
        )?;
        let generation = database_u64(
            row.try_get("current_generation").map_err(DbError::from)?,
            "invalidation generation",
        )?;
        let policy = LocalAuthorizationPolicy::from_database(
            domain,
            lease_id,
            policy_revision,
            generation,
            authority_epoch,
            fence,
            capability,
            expires_at,
            None,
            None,
        )
        .ok_or_else(|| {
            AuthorizationResolverError::Database(DbError::InvalidData(
                "protected publication policy row is invalid".to_owned(),
            ))
        })?;
        Ok((policy, now))
    }
}

impl LocalBindingResolver for PostgresLocalBindingResolver {
    type Error = AuthorizationResolverError;

    fn capability(&self) -> LocalBindingResolverCapability {
        LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
    }

    async fn resolve<'a>(
        &'a self,
        request: &'a BindingResolutionRequest,
    ) -> std::result::Result<LocalBindingResolution, Self::Error> {
        match request {
            BindingResolutionRequest::Direct {
                assertion, proof, ..
            } => {
                let key = assertion.principal_storage_key();
                let row = read_active_binding(
                    &self.db,
                    assertion.authorization_domain(),
                    key.issuer(),
                    key.subject(),
                    proof.actor_pubkey(),
                )
                .await?
                .ok_or(AuthorizationResolverError::BindingUnavailable)?;
                let binding = ActiveLocalBinding::from_storage(
                    assertion.authorization_domain(),
                    assertion.principal_for_storage(),
                    row.binding_id,
                    row.binding_version,
                    row.event_author_pubkey,
                    row.expires_at,
                )
                .ok_or_else(|| {
                    AuthorizationResolverError::Database(DbError::InvalidData(
                        "active local binding row is invalid".to_owned(),
                    ))
                })?;
                Ok(LocalBindingResolution::direct(assertion.clone(), binding))
            }
            BindingResolutionRequest::Delegated {
                delegation, proof, ..
            } => {
                if proof.actor_pubkey() != delegation.delegate_pubkey() {
                    return Err(AuthorizationResolverError::BindingUnavailable);
                }
                let row = read_active_binding_by_author(
                    &self.db,
                    delegation.authorization_domain(),
                    delegation.owner_pubkey(),
                )
                .await?
                .ok_or(AuthorizationResolverError::BindingUnavailable)?;
                let binding = ActiveLocalBinding::from_storage_parts(
                    delegation.authorization_domain(),
                    row.issuer,
                    row.subject,
                    row.binding_id,
                    row.binding_version,
                    row.event_author_pubkey,
                    row.expires_at,
                )
                .ok_or_else(|| {
                    AuthorizationResolverError::Database(DbError::InvalidData(
                        "delegated owner binding row is invalid".to_owned(),
                    ))
                })?;
                Ok(LocalBindingResolution::delegated(
                    delegation.clone(),
                    binding,
                ))
            }
            BindingResolutionRequest::Enrollment { .. } => {
                Err(AuthorizationResolverError::ContractUnavailable)
            }
        }
    }

    async fn current_status_evidence<'a>(
        &'a self,
        request: &'a CurrentBindingStatusEvidenceRequest,
    ) -> std::result::Result<CanonicalCurrentBindingEvidence, Self::Error> {
        let _ = request;
        Err(AuthorizationResolverError::ContractUnavailable)
    }

    async fn recheck_current_status_evidence<'a>(
        &'a self,
        evidence: &'a CanonicalCurrentBindingEvidence,
    ) -> std::result::Result<CanonicalCurrentBindingEvidence, Self::Error> {
        let _ = evidence;
        Err(AuthorizationResolverError::ContractUnavailable)
    }
}

#[derive(Clone)]
struct ActiveBindingRow {
    issuer: String,
    subject: String,
    binding_id: uuid::Uuid,
    binding_version: u64,
    event_author_pubkey: PublicKey,
    expires_at: Option<DateTime<Utc>>,
}

async fn read_active_binding(
    db: &Db,
    community_id: CommunityId,
    issuer: &str,
    subject: &str,
    event_author_pubkey: PublicKey,
) -> std::result::Result<Option<ActiveBindingRow>, AuthorizationResolverError> {
    let row = sqlx::query(
        "SELECT issuer,subject,binding_id,binding_version,event_author_pubkey,expires_at \
         FROM identity_bindings \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND event_author_pubkey=$4 \
           AND binding_state=1 AND (expires_at IS NULL OR expires_at > clock_timestamp())",
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(subject)
    .bind(event_author_pubkey.to_bytes().as_slice())
    .fetch_optional(&db.pool)
    .await
    .map_err(DbError::from)?;
    row.map(parse_active_binding).transpose()
}

async fn read_active_binding_by_author(
    db: &Db,
    community_id: CommunityId,
    event_author_pubkey: PublicKey,
) -> std::result::Result<Option<ActiveBindingRow>, AuthorizationResolverError> {
    let row = sqlx::query(
        "SELECT issuer,subject,binding_id,binding_version,event_author_pubkey,expires_at \
         FROM identity_bindings \
         WHERE community_id=$1 AND event_author_pubkey=$2 AND binding_state=1 \
           AND (expires_at IS NULL OR expires_at > clock_timestamp())",
    )
    .bind(community_id.as_uuid())
    .bind(event_author_pubkey.to_bytes().as_slice())
    .fetch_optional(&db.pool)
    .await
    .map_err(DbError::from)?;
    row.map(parse_active_binding).transpose()
}

fn parse_active_binding(
    row: sqlx::postgres::PgRow,
) -> std::result::Result<ActiveBindingRow, AuthorizationResolverError> {
    let binding_version: i64 = row.try_get("binding_version").map_err(DbError::from)?;
    let event_author: Vec<u8> = row.try_get("event_author_pubkey").map_err(DbError::from)?;
    Ok(ActiveBindingRow {
        issuer: row.try_get("issuer").map_err(DbError::from)?,
        subject: row.try_get("subject").map_err(DbError::from)?,
        binding_id: row.try_get("binding_id").map_err(DbError::from)?,
        binding_version: u64::try_from(binding_version)
            .map_err(|_| DbError::InvalidData("active binding version is invalid".to_owned()))?,
        event_author_pubkey: PublicKey::from_slice(&event_author)
            .map_err(|_| DbError::InvalidData("active binding author key is invalid".to_owned()))?,
        expires_at: row.try_get("expires_at").map_err(DbError::from)?,
    })
}

fn bytes32(value: Vec<u8>, name: &str) -> crate::Result<[u8; 32]> {
    value
        .try_into()
        .map_err(|_| DbError::InvalidData(format!("authorization {name} is malformed")))
}

fn database_u64(value: i64, name: &str) -> crate::Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData(format!("authorization {name} is negative")))
}

fn parsed_fence(value: Vec<u8>, name: &str) -> crate::Result<AuthorizationLeaseFence> {
    AuthorizationLeaseFence::from_bytes(bytes32(value, name)?)
        .map_err(|_| DbError::InvalidData(format!("authorization {name} is invalid")))
}

fn initial_publication_fence(
    community_id: CommunityId,
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
    lease_id: uuid::Uuid,
) -> crate::Result<AuthorizationLeaseFence> {
    let mut digest = Sha256::new();
    for field in [
        b"buzz:protected-publication-initial-fence:v1".as_slice(),
        community_id.as_uuid().as_bytes(),
        &(object_kind as i16).to_be_bytes(),
        object_key.as_slice(),
        lease_id.as_bytes(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    AuthorizationLeaseFence::from_bytes(digest.finalize().into())
        .map_err(|_| DbError::InvalidData("authorization initial fence is invalid".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concrete_resolver_is_redacted_and_declares_owner_bound_delegation() {
        assert_eq!(
            PostgresLocalBindingResolver {
                db: lazy_debug_test_db()
            }
            .capability(),
            LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
        );
        assert!(!format!(
            "{:?}",
            PostgresLocalBindingResolver {
                db: lazy_debug_test_db()
            }
        )
        .contains("postgres"));
    }

    fn lazy_debug_test_db() -> Db {
        // The production crate forbids unsafe code. Build a lazy pool through
        // the public configuration helper rather than fabricating a handle.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://localhost/buzz_debug_test")
            .expect("static URL is valid");
        Db::from_pool(pool)
    }
}
