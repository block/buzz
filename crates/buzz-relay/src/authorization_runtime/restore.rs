//! Fail-closed restore-witness interfaces for the lower review unit.
//!
//! The invalidation slice replaces this module with the independent durable
//! witness. Until then no caller can begin, commit, or abort a protected
//! mutation through this interface.

use buzz_core::CommunityId;
use thiserror::Error;
use uuid::Uuid;

/// Disabled restore witness placeholder.
pub struct RestoreProtectionRuntime {
    _private: (),
}

impl RestoreProtectionRuntime {
    /// Refuse to begin a protected mutation before the witness is installed.
    pub async fn begin(
        &self,
        _domain: CommunityId,
        _operation_id: Uuid,
        _request_fingerprint: [u8; 32],
    ) -> Result<RestoreMutationGuard, RestoreProtectionError> {
        Err(RestoreProtectionError::DomainNotConfigured)
    }
}

/// Unconstructible mutation-witness guard retained for call-site typing.
pub struct RestoreMutationGuard {
    _private: (),
}

impl RestoreMutationGuard {
    /// Refuse a commit while the independent witness is absent.
    pub async fn commit(self) -> Result<(), RestoreProtectionError> {
        Err(RestoreProtectionError::DomainNotConfigured)
    }

    /// Refuse an abort while the independent witness is absent.
    pub async fn abort(self) -> Result<(), RestoreProtectionError> {
        Err(RestoreProtectionError::DomainNotConfigured)
    }
}

/// Fail-closed restore-witness error.
#[derive(Debug, Error)]
pub enum RestoreProtectionError {
    /// The exact domain has no installed witness.
    #[error("restore protection domain is not configured")]
    DomainNotConfigured,
}
