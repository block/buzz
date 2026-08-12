//! Read-only enrollment preparation and transaction-local binding creation.
//!
//! Federated assertions are verified by `buzz-auth`; this module reads only
//! provider-free local policy and writes only immutable binding generations.
//! Assertion expiry bounds the resulting authorization lease and is never
//! copied into `identity_bindings.expires_at`.

use std::fmt;

use buzz_auth::{
    ActiveLocalBinding, DirectEnrollmentMode, DirectEnrollmentProposal, LocalEnrollmentAuthority,
    VerifiedFederatedAssertion, VerifiedNostrProof,
};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::identity_lifecycle::{
    lock_identity_lifecycle_transaction, IdentityLifecycleLockCoordinates,
};

/// Stable result of transaction-local enrollment convergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentDisposition {
    /// This transaction created the one immutable binding generation.
    Created,
    /// A concurrent transaction already created the exact eligible generation.
    ConvergedExisting,
}

/// Credential-free enrollment state retained until canonical admission commits.
pub(crate) struct EnrollmentTransactionOutcome {
    pub(crate) binding: ActiveLocalBinding,
    pub(crate) disposition: EnrollmentDisposition,
}

/// Direct-enrollment proposal bound to the exact immutable local policy row.
///
/// The policy digest is credential-free and remains separate from the
/// assertion digest stored as binding provenance. Canonical admission retains
/// both until the authoritative transaction proves that the proposal is still
/// the highest currently effective policy.
pub struct PreparedDirectEnrollment {
    proposal: DirectEnrollmentProposal,
    policy_digest: [u8; 32],
}

impl PreparedDirectEnrollment {
    /// Consume the preparation into its frozen S2 proposal and policy digest.
    pub fn into_parts(self) -> (DirectEnrollmentProposal, [u8; 32]) {
        (self.proposal, self.policy_digest)
    }
}

impl fmt::Debug for PreparedDirectEnrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedDirectEnrollment([REDACTED])")
    }
}

impl fmt::Debug for EnrollmentTransactionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EnrollmentTransactionOutcome([REDACTED])")
    }
}

/// Fail-closed enrollment preparation and transaction errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IdentityEnrollmentError {
    /// No current local enrollment policy was readable.
    #[error("identity enrollment policy is unavailable")]
    PolicyUnavailable,
    /// The current policy requires a separately authorized provisioning action.
    #[error("identity binding must be provisioned")]
    ProvisioningRequired,
    /// Lifecycle state, an administrative expiry, or the partial bijection denied creation.
    #[error("identity enrollment denied")]
    Denied,
    /// A server-generated identifier or sealed digest was invalid.
    #[error("identity enrollment input is invalid")]
    InvalidInput,
    /// PostgreSQL state could not be read or committed.
    #[error("identity enrollment storage is unavailable")]
    StorageUnavailable,
}

/// Read current local authority and seal an Attested or risk-labelled TOFU proposal.
///
/// This function is read-only. Provisioned mode intentionally returns
/// [`IdentityEnrollmentError::ProvisioningRequired`]; only an authenticated
/// operator transition may create that binding provenance.
pub async fn prepare_direct_enrollment(
    pool: &PgPool,
    assertion: VerifiedFederatedAssertion,
    proof: VerifiedNostrProof,
    authoritative_now: DateTime<Utc>,
) -> Result<PreparedDirectEnrollment, IdentityEnrollmentError> {
    if assertion.authorization_domain() != proof.authorization_domain() {
        return Err(IdentityEnrollmentError::InvalidInput);
    }
    let domain = assertion.authorization_domain();
    let row = sqlx::query(
        r#"
        SELECT policy_revision, enrollment_mode, policy_digest
        FROM identity_enrollment_policies
        WHERE community_id = $1
          AND effective_at <= $2
          AND (expires_at IS NULL OR $2 < expires_at)
        ORDER BY policy_revision DESC
        LIMIT 1
        "#,
    )
    .bind(domain.as_uuid())
    .bind(authoritative_now)
    .fetch_optional(pool)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?
    .ok_or(IdentityEnrollmentError::PolicyUnavailable)?;

    let policy_revision = positive_i64(row.try_get("policy_revision")?)?;
    let mode_code: i16 = row
        .try_get("enrollment_mode")
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
    // Read and validate policy lineage, but never copy it into binding
    // evidence. Attested/TOFU bindings retain the exact assertion digest
    // sealed into the Nostr proof.
    let policy_digest = digest_column(&row, "policy_digest")?;
    let evidence_digest = proof
        .bound_assertion_fingerprint()
        .copied()
        .ok_or(IdentityEnrollmentError::Denied)?;
    let _mode = direct_enrollment_mode(mode_code)?;
    let principal = assertion.principal_storage_key();
    let authority = LocalEnrollmentAuthority::from_database_parts(
        domain,
        principal.issuer().to_owned(),
        principal.subject().to_owned(),
        proof.actor_pubkey(),
        mode_code,
        policy_revision,
        evidence_digest,
    )
    .ok_or(IdentityEnrollmentError::InvalidInput)?;
    let proposal = DirectEnrollmentProposal::from_verified_evidence(
        authority,
        assertion,
        proof,
        authoritative_now,
    )
    .map_err(|_| IdentityEnrollmentError::Denied)?;
    Ok(PreparedDirectEnrollment {
        proposal,
        policy_digest,
    })
}

/// Create or converge on one exact eligible enrollment generation.
///
/// The caller retains this transaction through final authorization recheck,
/// replay claim, receipt, audit append, and any admitted mutation.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_authoritative_enrollment_tx(
    transaction: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    proposal: &DirectEnrollmentProposal,
    prepared_policy_digest: [u8; 32],
    assertion: &VerifiedFederatedAssertion,
    proof: &VerifiedNostrProof,
    authoritative_now: DateTime<Utc>,
) -> Result<EnrollmentTransactionOutcome, IdentityEnrollmentError> {
    if operation_id.is_nil()
        || request_fingerprint == [0; 32]
        || proposal.authorization_domain() != assertion.authorization_domain()
        || proposal.authorization_domain() != proof.authorization_domain()
        || prepared_policy_digest == [0; 32]
    {
        return Err(IdentityEnrollmentError::InvalidInput);
    }
    let domain = proposal.authorization_domain();
    let principal = assertion.principal_storage_key();
    let issuer = principal.issuer();
    let subject = principal.subject();
    let event_author = proof.actor_pubkey();
    let event_author_bytes = event_author.to_bytes();
    let principal_fingerprint = principal_fingerprint(domain, issuer, subject);
    let (policy_revision, evidence_digest) = proposal.local_authority();
    let policy_revision_i64 =
        i64::try_from(policy_revision).map_err(|_| IdentityEnrollmentError::InvalidInput)?;

    let community_exists =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM communities WHERE id=$1 FOR UPDATE")
            .bind(domain.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?
            .is_some();
    if !community_exists {
        return Err(IdentityEnrollmentError::PolicyUnavailable);
    }

    let coordinates = IdentityLifecycleLockCoordinates::new(
        domain,
        Some(principal_fingerprint),
        None,
        Some(event_author),
    )
    .map_err(|_| IdentityEnrollmentError::InvalidInput)?;
    lock_identity_lifecycle_transaction(transaction, &coordinates)
        .await
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;

    let policy = sqlx::query(
        r#"
        SELECT policy_revision, enrollment_mode, policy_digest, expires_at
        FROM identity_enrollment_policies
        WHERE community_id = $1
          AND effective_at <= $2
          AND (expires_at IS NULL OR $2 < expires_at)
        ORDER BY policy_revision DESC
        LIMIT 1
        FOR SHARE
        "#,
    )
    .bind(domain.as_uuid())
    .bind(authoritative_now)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?
    .ok_or(IdentityEnrollmentError::PolicyUnavailable)?;
    let current_revision = positive_i64(policy.try_get("policy_revision")?)?;
    let mode_code: i16 = policy
        .try_get("enrollment_mode")
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
    let _mode = direct_enrollment_mode(mode_code)?;
    let stored_policy_digest = digest_column(&policy, "policy_digest")?;
    let binding_expires_at: Option<DateTime<Utc>> = policy
        .try_get("expires_at")
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
    if current_revision != policy_revision
        || mode_code != proposal.mode().database_code()
        || stored_policy_digest != prepared_policy_digest
        || proof.bound_assertion_fingerprint() != Some(evidence_digest)
        || binding_expires_at.is_some_and(|expires_at| authoritative_now >= expires_at)
    {
        return Err(IdentityEnrollmentError::Denied);
    }

    let rows = sqlx::query(
        r#"
        SELECT binding_id, binding_version, issuer, subject,
               principal_fingerprint, event_author_pubkey, binding_state,
               binding_provenance, policy_revision,
               enrollment_evidence_digest, expires_at
        FROM identity_bindings
        WHERE community_id = $1
          AND binding_state = 1
          AND ((issuer = $2 AND subject = $3) OR event_author_pubkey = $4)
        ORDER BY binding_version
        FOR UPDATE
        "#,
    )
    .bind(domain.as_uuid())
    .bind(issuer)
    .bind(subject)
    .bind(event_author_bytes.as_slice())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;

    if !rows.is_empty() {
        if rows.len() != 1 {
            return Err(IdentityEnrollmentError::Denied);
        }
        let row = &rows[0];
        let row_issuer: String = row
            .try_get("issuer")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let row_subject: String = row
            .try_get("subject")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let row_key: Vec<u8> = row
            .try_get("event_author_pubkey")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let row_state: i16 = row
            .try_get("binding_state")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let row_mode: i16 = row
            .try_get("binding_provenance")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let row_policy = positive_i64(row.try_get("policy_revision")?)?;
        let row_evidence = digest_column(row, "enrollment_evidence_digest")?;
        let row_expires_at: Option<DateTime<Utc>> = row
            .try_get("expires_at")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        if row_issuer != issuer
            || row_subject != subject
            || row_key.as_slice() != event_author_bytes
            || row_state != 1
            || row_mode != mode_code
            || row_policy != policy_revision
            || row_evidence != *evidence_digest
            || row_expires_at.is_some_and(|expires_at| authoritative_now >= expires_at)
        {
            return Err(IdentityEnrollmentError::Denied);
        }
        let binding_id: Uuid = row
            .try_get("binding_id")
            .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
        let binding_version = positive_i64(row.try_get("binding_version")?)?;
        let binding = ActiveLocalBinding::from_enrollment_storage_parts(
            domain,
            row_issuer,
            row_subject,
            binding_id,
            binding_version,
            event_author,
            row_expires_at,
            row_mode,
            row_policy,
            row_evidence,
        )
        .ok_or(IdentityEnrollmentError::StorageUnavailable)?;
        return Ok(EnrollmentTransactionOutcome {
            binding,
            disposition: EnrollmentDisposition::ConvergedExisting,
        });
    }

    let blocked: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM identity_lifecycle_selectors selector
            LEFT JOIN identity_lifecycle_selector_consumptions consumption
              ON consumption.community_id = selector.community_id
             AND consumption.selector_id = selector.selector_id
            WHERE selector.community_id = $1
              AND (
                (selector.selector_kind = 1
                  AND selector.principal_fingerprint = $2
                  AND selector.event_author_pubkey = $3)
                OR (selector.selector_kind = 2
                  AND selector.principal_fingerprint = $2
                  AND consumption.selector_id IS NULL)
                OR (selector.selector_kind = 3
                  AND selector.event_author_pubkey = $3)
                OR (selector.selector_kind = 4
                  AND selector.principal_fingerprint = $2
                  AND consumption.selector_id IS NULL)
              )
        )
        "#,
    )
    .bind(domain.as_uuid())
    .bind(principal_fingerprint.as_slice())
    .bind(event_author_bytes.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
    if blocked {
        return Err(IdentityEnrollmentError::Denied);
    }

    let binding_id = Uuid::new_v4();
    let history_id = Uuid::new_v4();
    let transition_digest = enrollment_transition_digest(
        domain,
        operation_id,
        request_fingerprint,
        principal_fingerprint,
        event_author_bytes,
        policy_revision,
        mode_code,
    );
    let binding_version: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO identity_bindings (
            community_id, binding_id, issuer, subject, principal_fingerprint,
            event_author_pubkey, binding_state, lifecycle_revision,
            binding_provenance, policy_revision, enrollment_evidence_digest,
            expires_at, birth_history_id, creation_operation_id,
            creation_request_fingerprint
        ) VALUES ($1, $2, $3, $4, $5, $6, 1, 1, $7, $8, $9, $10, $11, $12, $13)
        RETURNING binding_version
        "#,
    )
    .bind(domain.as_uuid())
    .bind(binding_id)
    .bind(issuer)
    .bind(subject)
    .bind(principal_fingerprint.as_slice())
    .bind(event_author_bytes.as_slice())
    .bind(mode_code)
    .bind(policy_revision_i64)
    .bind(evidence_digest.as_slice())
    .bind(binding_expires_at)
    .bind(history_id)
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;

    sqlx::query(
        r#"
        INSERT INTO identity_lifecycle_history (
            community_id, history_id, transition_kind, outcome_code,
            successor_binding_id, successor_binding_version,
            successor_lifecycle_revision, successor_state,
            operation_id, request_fingerprint, transition_digest
        ) VALUES ($1, $2, 1, 1, $3, $4, 1, 1, $5, $6, $7)
        "#,
    )
    .bind(domain.as_uuid())
    .bind(history_id)
    .bind(binding_id)
    .bind(binding_version)
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(transition_digest.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;

    let binding_version = positive_i64(binding_version)?;
    let binding = ActiveLocalBinding::from_enrollment_storage_parts(
        domain,
        issuer.to_owned(),
        subject.to_owned(),
        binding_id,
        binding_version,
        event_author,
        binding_expires_at,
        mode_code,
        policy_revision,
        *evidence_digest,
    )
    .ok_or(IdentityEnrollmentError::StorageUnavailable)?;
    Ok(EnrollmentTransactionOutcome {
        binding,
        disposition: EnrollmentDisposition::Created,
    })
}

/// Domain-separated pseudonymous principal coordinate shared by lifecycle code.
pub(crate) fn principal_fingerprint(domain: CommunityId, issuer: &str, subject: &str) -> [u8; 32] {
    framed_digest(
        b"buzz:nip-fi:principal:v1",
        &[
            domain.as_uuid().as_bytes(),
            issuer.as_bytes(),
            subject.as_bytes(),
        ],
    )
}

/// Domain-separated pseudonymous actor coordinate used in durable audit rows.
pub(crate) fn actor_fingerprint(domain: CommunityId, key: &[u8; 32]) -> [u8; 32] {
    framed_digest(b"buzz:nip-fi:actor:v1", &[domain.as_uuid().as_bytes(), key])
}

pub(crate) fn framed_digest(label: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((label.len() as u64).to_be_bytes());
    digest.update(label);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

fn enrollment_transition_digest(
    domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    principal_fingerprint: [u8; 32],
    event_author: [u8; 32],
    policy_revision: u64,
    mode_code: i16,
) -> [u8; 32] {
    framed_digest(
        b"buzz:nip-fi:enrollment-transition:v1",
        &[
            domain.as_uuid().as_bytes(),
            operation_id.as_bytes(),
            &request_fingerprint,
            &principal_fingerprint,
            &event_author,
            &policy_revision.to_be_bytes(),
            &mode_code.to_be_bytes(),
        ],
    )
}

fn positive_i64(value: i64) -> Result<u64, IdentityEnrollmentError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(IdentityEnrollmentError::StorageUnavailable)
}

fn direct_enrollment_mode(mode_code: i16) -> Result<DirectEnrollmentMode, IdentityEnrollmentError> {
    match DirectEnrollmentMode::from_database_code(mode_code) {
        Some(DirectEnrollmentMode::Provisioned) => {
            Err(IdentityEnrollmentError::ProvisioningRequired)
        }
        Some(mode @ (DirectEnrollmentMode::Attested | DirectEnrollmentMode::RiskLabelledTofu)) => {
            Ok(mode)
        }
        None => Err(IdentityEnrollmentError::InvalidInput),
    }
}

fn digest_column(
    row: &sqlx::postgres::PgRow,
    column: &str,
) -> Result<[u8; 32], IdentityEnrollmentError> {
    let bytes: Vec<u8> = row
        .try_get(column)
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)?;
    bytes
        .try_into()
        .map_err(|_| IdentityEnrollmentError::StorageUnavailable)
}

impl From<sqlx::Error> for IdentityEnrollmentError {
    fn from(_: sqlx::Error) -> Self {
        Self::StorageUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_domain_and_tuple_separated() {
        let first = CommunityId::from_uuid(Uuid::from_u128(1));
        let second = CommunityId::from_uuid(Uuid::from_u128(2));
        assert_ne!(
            principal_fingerprint(first, "issuer-a", "subject"),
            principal_fingerprint(second, "issuer-a", "subject")
        );
        assert_ne!(
            principal_fingerprint(first, "issuer-a", "subject"),
            principal_fingerprint(first, "issuer", "a-subject")
        );
        assert_ne!(actor_fingerprint(first, &[1; 32]), [0; 32]);
    }

    #[test]
    fn public_direct_enrollment_denies_provisioned_mode() {
        assert_eq!(
            direct_enrollment_mode(DirectEnrollmentMode::Attested.database_code()),
            Ok(DirectEnrollmentMode::Attested)
        );
        assert_eq!(
            direct_enrollment_mode(DirectEnrollmentMode::RiskLabelledTofu.database_code()),
            Ok(DirectEnrollmentMode::RiskLabelledTofu)
        );
        assert_eq!(
            direct_enrollment_mode(DirectEnrollmentMode::Provisioned.database_code()),
            Err(IdentityEnrollmentError::ProvisioningRequired)
        );
    }
}
