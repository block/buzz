//! Fail-closed transaction interfaces for the pre-finalization stack.
//!
//! Protected transports need stable types before the owning finalization and
//! invalidation slices land. This compatibility module never creates a permit
//! or starts a transaction; the later owning slice replaces it with the
//! transaction-owned implementation.

use std::fmt;

use buzz_core::CommunityId;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::transport::{
    LeaseCurrentStateError, ProtectedEnrollmentAuthority, ProtectedOperationAuthority,
    ProtectedTransportError,
};

/// Opaque commit fence that cannot be constructed in this review unit.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationCommitFence {
    _private: (),
}

impl fmt::Debug for AuthorizationCommitFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationCommitFence([unavailable])")
    }
}

/// Opaque ephemeral claim that cannot be constructed in this review unit.
#[allow(dead_code)]
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct EphemeralAuthorityClaim {
    _private: (),
}

#[allow(dead_code)]
impl EphemeralAuthorityClaim {
    pub(super) fn from_authority(
        _authority: &ProtectedOperationAuthority,
        _event_id: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        Err(ProtectedTransportError::AtomicMutationFenceUnavailable)
    }
}

/// Stable retry identity for one protected operation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtectedOperationId(Uuid);

impl ProtectedOperationId {
    /// Derive a deterministic UUID from a domain-separated stable key.
    pub fn derive(
        authorization_domain: CommunityId,
        operation_kind: &'static str,
        stable_key: &[u8],
    ) -> Result<Self, AuthorizationExecutionError> {
        if operation_kind.is_empty() || operation_kind.len() > 128 || stable_key.is_empty() {
            return Err(AuthorizationExecutionError::InvalidOperationIdentity);
        }
        let mut digest = Sha256::new();
        digest.update(b"buzz-protected-operation-id-v1");
        digest.update(authorization_domain.as_uuid().as_bytes());
        digest.update((operation_kind.len() as u64).to_be_bytes());
        digest.update(operation_kind.as_bytes());
        digest.update((stable_key.len() as u64).to_be_bytes());
        digest.update(stable_key);
        let digest: [u8; 32] = digest.finalize().into();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self(Uuid::from_bytes(bytes)))
    }

    pub(crate) const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Debug for ProtectedOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedOperationId([redacted])")
    }
}

/// Permit placeholder that cannot be minted before finalization lands.
pub struct SealedOperationPermit {
    _private: (),
}

impl SealedOperationPermit {
    pub(super) fn from_authority(
        _authority: &ProtectedOperationAuthority,
        _operation_id: ProtectedOperationId,
        _operation_kind: &'static str,
        _request_fingerprint: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        Err(ProtectedTransportError::AtomicMutationFenceUnavailable)
    }
}

/// Enrollment permit placeholder that cannot be minted before finalization.
pub struct SealedEnrollmentPermit {
    _private: (),
}

impl SealedEnrollmentPermit {
    pub(super) fn from_authority(
        _authority: &ProtectedEnrollmentAuthority,
        _operation_id: ProtectedOperationId,
        _operation_kind: &'static str,
        _request_fingerprint: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        Err(ProtectedTransportError::AtomicMutationFenceUnavailable)
    }
}

/// Start result retained for protected operation call-site compatibility.
pub enum AuthorizedOperationStart {
    /// Previously committed bounded response.
    Replay(Vec<u8>),
    /// Transaction owned by the authorization executor.
    Execute(Box<AuthorizedOperation>),
}

/// Transaction handle that can only be produced by the owning later slice.
pub struct AuthorizedOperation {
    transaction: Transaction<'static, Postgres>,
}

impl AuthorizedOperation {
    /// Borrow the executor-owned transaction.
    pub fn transaction(&mut self) -> &mut Transaction<'static, Postgres> {
        &mut self.transaction
    }

    /// Refuse to commit while the durable executor is unavailable.
    pub async fn commit(
        self,
        _result_payload: &[u8],
    ) -> Result<Vec<u8>, AuthorizationExecutionError> {
        self.transaction.rollback().await?;
        Err(AuthorizationExecutionError::RestoreUnavailable)
    }
}

/// Fail closed until the finalization slice installs transaction execution.
pub async fn begin_authorized_operation(
    _state: &crate::state::AppState,
    _permit: SealedOperationPermit,
) -> Result<AuthorizedOperationStart, AuthorizationExecutionError> {
    Err(AuthorizationExecutionError::RestoreUnavailable)
}

/// Fail-closed protected mutation execution error.
#[derive(Debug, Error)]
pub enum AuthorizationExecutionError {
    /// Database transaction failed.
    #[error("protected operation database transaction failed")]
    Database(#[from] sqlx::Error),
    /// Database wrapper failed.
    #[error("protected operation database transaction failed")]
    Db(#[from] buzz_db::DbError),
    /// Independent restore witness failed.
    #[error("protected operation restore witness failed")]
    Restore(#[from] super::restore::RestoreProtectionError),
    /// Mandatory restore witness is unavailable.
    #[error("protected operation restore witness is unavailable")]
    RestoreUnavailable,
    /// Captured authorization fence is incomplete.
    #[error("protected operation commit fence is invalid")]
    InvalidCommitFence,
    /// Stable operation identity is malformed.
    #[error("protected operation identity is invalid")]
    InvalidOperationIdentity,
    /// Stable identity was reused for different input.
    #[error("protected operation retry conflicts with the committed request")]
    ConflictingRetry,
    /// Durable invalidation state denies the operation.
    #[error("protected operation authority was invalidated")]
    Invalidated,
    /// The active binding changed or disappeared.
    #[error("protected operation binding is no longer active")]
    InvalidBinding,
    /// Authorization expired before commit.
    #[error("protected operation authorization expired before commit")]
    Expired,
    /// Replay result exceeded its bounded payload.
    #[error("protected operation result is too large")]
    ResultTooLarge,
}

impl From<AuthorizationExecutionError> for LeaseCurrentStateError {
    fn from(_error: AuthorizationExecutionError) -> Self {
        Self::Unavailable
    }
}
