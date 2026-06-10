//! Channel access enforcement.
//!
//! Defines [`ChannelAccessChecker`] so `sprout-auth` can enforce access
//! without depending on `sprout-db` directly.

use std::collections::HashSet;
use std::future::Future;

use nostr::PublicKey;
use uuid::Uuid;

use crate::error::AuthError;
use crate::scope::Scope;

/// Async trait for checking channel membership.
///
/// Implemented by the database layer (`sprout-db`) in production. The `sprout-auth`
/// crate defines the trait so it can enforce access rules without a direct dependency
/// on `sprout-db`.
pub trait ChannelAccessChecker: Send + Sync {
    /// Return the set of channel UUIDs accessible to `pubkey`.
    fn accessible_channel_ids(
        &self,
        pubkey: &PublicKey,
    ) -> impl Future<Output = Result<HashSet<Uuid>, AuthError>> + Send;

    /// Returns `true` if `pubkey` is a member of `channel_id`.
    ///
    /// Default implementation calls [`Self::accessible_channel_ids`] and checks membership.
    /// Implementations may override this with a more efficient point-lookup query.
    fn can_access(
        &self,
        pubkey: &PublicKey,
        channel_id: Uuid,
    ) -> impl Future<Output = Result<bool, AuthError>> + Send {
        async move {
            let ids = self.accessible_channel_ids(pubkey).await?;
            Ok(ids.contains(&channel_id))
        }
    }
}

/// Check that `scopes` contains the required scope.
pub fn require_scope(scopes: &[Scope], required: Scope) -> Result<(), AuthError> {
    if scopes.contains(&required) {
        Ok(())
    } else {
        Err(AuthError::InsufficientScope {
            required: required.as_str().to_string(),
            have: scopes.iter().map(|s| s.as_str().to_string()).collect(),
        })
    }
}

/// Verify read access: scope + membership.
pub async fn check_read_access(
    checker: &impl ChannelAccessChecker,
    pubkey: &PublicKey,
    channel_id: Uuid,
    scopes: &[Scope],
) -> Result<(), AuthError> {
    require_scope(scopes, Scope::MessagesRead)?;
    if checker.can_access(pubkey, channel_id).await? {
        Ok(())
    } else {
        Err(AuthError::ChannelAccessDenied)
    }
}

/// Verify write access: scope + membership.
pub async fn check_write_access(
    checker: &impl ChannelAccessChecker,
    pubkey: &PublicKey,
    channel_id: Uuid,
    scopes: &[Scope],
) -> Result<(), AuthError> {
    require_scope(scopes, Scope::MessagesWrite)?;
    if checker.can_access(pubkey, channel_id).await? {
        Ok(())
    } else {
        Err(AuthError::ChannelAccessDenied)
    }
}

// ── Test-only mock ───────────────────────────────────────────────────────────

/// In-memory [`ChannelAccessChecker`] for unit tests.
#[cfg(any(test, feature = "test-utils"))]
pub struct MockAccessChecker {
    allowed: HashSet<(String, Uuid)>,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockAccessChecker {
    /// Create an empty checker (all access denied by default).
    pub fn new() -> Self {
        Self {
            allowed: HashSet::new(),
        }
    }

    /// Grant `pubkey` access to `channel_id`.
    pub fn allow(&mut self, pubkey: &PublicKey, channel_id: Uuid) {
        self.allowed.insert((pubkey.to_hex(), channel_id));
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MockAccessChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl ChannelAccessChecker for MockAccessChecker {
    async fn accessible_channel_ids(&self, pubkey: &PublicKey) -> Result<HashSet<Uuid>, AuthError> {
        let hex = pubkey.to_hex();
        Ok(self
            .allowed
            .iter()
            .filter(|(pk, _)| pk == &hex)
            .map(|(_, id)| *id)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[tokio::test]
    async fn mock_checker_allow_and_deny() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let allowed_ch = Uuid::new_v4();
        let denied_ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&pk, allowed_ch);

        assert!(checker.can_access(&pk, allowed_ch).await.unwrap());
        assert!(!checker.can_access(&pk, denied_ch).await.unwrap());
    }

    #[tokio::test]
    async fn read_access_denied_by_scope() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&pk, ch);

        assert!(matches!(
            check_read_access(&checker, &pk, ch, &[]).await,
            Err(AuthError::InsufficientScope { .. })
        ));
    }

    #[tokio::test]
    async fn read_access_denied_by_membership() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();
        let checker = MockAccessChecker::new();

        assert!(matches!(
            check_read_access(&checker, &pk, ch, &[Scope::MessagesRead]).await,
            Err(AuthError::ChannelAccessDenied)
        ));
    }

    #[tokio::test]
    async fn read_access_granted() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&pk, ch);

        assert!(check_read_access(&checker, &pk, ch, &[Scope::MessagesRead])
            .await
            .is_ok());
    }
}
