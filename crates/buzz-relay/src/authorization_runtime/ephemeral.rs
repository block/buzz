//! Fail-closed ephemeral-authority interfaces for the lower review unit.
//!
//! The session-conformance slice installs signed, durable authority. Production
//! verification here always denies; test-only constructors preserve the
//! existing realtime fixture surface without creating production authority.

use std::{fmt, sync::Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_core::CommunityId;
use thiserror::Error;

use super::transport::ProtectedAuthorization;
use crate::state::AppState;

const PRESENCE_PREFIX: &str = "pa1:";

/// Protected presence envelope decoded before authority verification.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct ProtectedPresenceValue {
    pub(crate) status: String,
    pub(crate) context_id: [u8; 32],
    pub(crate) authority: String,
}

impl fmt::Debug for ProtectedPresenceValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPresenceValue")
            .field("status", &"[redacted]")
            .field("context_id", &"[redacted]")
            .field("authority", &"[redacted]")
            .finish()
    }
}

/// Decode an envelope while retaining fail-closed authority verification.
pub(crate) fn decode_presence(
    value: &str,
) -> Result<Option<ProtectedPresenceValue>, EphemeralAuthorityError> {
    let Some(value) = value.strip_prefix(PRESENCE_PREFIX) else {
        return Ok(None);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| EphemeralAuthorityError::InvalidEnvelope)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Refuse to seal ephemeral authority before the session slice is installed.
pub(crate) fn seal_context(
    _state: &AppState,
    _authority: &ProtectedAuthorization,
    _context_id: [u8; 32],
) -> Result<String, EphemeralAuthorityError> {
    Err(EphemeralAuthorityError::AuthorityRequired)
}

/// Authority verifier that denies production input until its owning slice.
#[derive(Clone)]
pub(crate) enum AuthorityTokenVerifier {
    ProductionDeny,
    #[cfg(test)]
    Test(Arc<std::sync::atomic::AtomicBool>),
}

/// Test-compatible retained authority with bounded synchronous expiry checks.
#[derive(Clone)]
pub(crate) struct RetainedEphemeralAuthority {
    gate: Arc<std::sync::atomic::AtomicBool>,
    expires_at: u64,
}

impl RetainedEphemeralAuthority {
    pub(crate) fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub(crate) async fn release(&self) -> bool {
        self.is_time_valid() && self.gate.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn is_time_valid(&self) -> bool {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .is_ok_and(|duration| duration.as_secs() < self.expires_at)
    }
}

impl AuthorityTokenVerifier {
    /// Build a production verifier that remains unavailable in this slice.
    pub(crate) fn new(_db: buzz_db::Db, _relay_secret: &[u8]) -> Self {
        Self::ProductionDeny
    }

    #[cfg(test)]
    pub(crate) fn allow_for_test() -> Self {
        Self::Test(Arc::new(std::sync::atomic::AtomicBool::new(true)))
    }

    #[cfg(test)]
    pub(crate) fn deny_for_test() -> Self {
        Self::Test(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    #[cfg(test)]
    pub(crate) fn conditional_for_test(gate: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::Test(gate)
    }

    pub(crate) async fn verify_context(
        &self,
        _community_id: CommunityId,
        _context_id: [u8; 32],
        _token: &str,
    ) -> Result<RetainedEphemeralAuthority, EphemeralAuthorityError> {
        self.retained_test_authority()
    }

    pub(crate) async fn verify_actor_context(
        &self,
        _community_id: CommunityId,
        _context_id: [u8; 32],
        _actor_pubkey: [u8; 32],
        _token: &str,
    ) -> Result<RetainedEphemeralAuthority, EphemeralAuthorityError> {
        self.retained_test_authority()
    }

    fn retained_test_authority(
        &self,
    ) -> Result<RetainedEphemeralAuthority, EphemeralAuthorityError> {
        match self {
            Self::ProductionDeny => Err(EphemeralAuthorityError::ExpiredOrInvalidated),
            #[cfg(test)]
            Self::Test(gate) if gate.load(std::sync::atomic::Ordering::SeqCst) => {
                Ok(RetainedEphemeralAuthority {
                    gate: Arc::clone(gate),
                    expires_at: u64::MAX,
                })
            }
            #[cfg(test)]
            Self::Test(_) => Err(EphemeralAuthorityError::ExpiredOrInvalidated),
        }
    }
}

/// Fail-closed ephemeral-authority error.
#[derive(Debug, Error)]
pub(crate) enum EphemeralAuthorityError {
    #[error("ephemeral sender authority is required")]
    AuthorityRequired,
    #[error("ephemeral sender authority envelope is invalid")]
    InvalidEnvelope,
    #[error("ephemeral sender authority is expired or invalidated")]
    ExpiredOrInvalidated,
    #[error("ephemeral sender authority serialization failed")]
    Serialization(#[from] serde_json::Error),
}
