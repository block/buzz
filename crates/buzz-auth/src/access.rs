//! Channel access enforcement.
//!
//! Defines [`ChannelAccessChecker`] so `buzz-auth` can enforce access
//! without depending on `buzz-db` directly.

use std::collections::HashSet;
use std::future::Future;

use buzz_core::TenantContext;
use nostr::PublicKey;
use uuid::Uuid;

use crate::error::AuthError;
use crate::scope::Scope;

/// Async trait for checking channel membership.
///
/// Implemented by the database layer (`buzz-db`) in production. The `buzz-auth`
/// crate defines the trait so it can enforce access rules without a direct dependency
/// on `buzz-db`.
///
/// ## Tenant scoping
///
/// Every method takes `&TenantContext`. Channel UUIDs are not globally unique under
/// multi-tenant — the frozen schema's `channels` PK is `(community_id, id)`, so the
/// same UUID can legitimately exist in two communities. A bare `WHERE id = $1`
/// implementation would be a cross-community existence oracle and could return
/// `true` for a B-community membership when the request bound community is A.
/// Implementations MUST scope every query by `ctx.community()` (S1 cross-community
/// fence at the access layer).
pub trait ChannelAccessChecker: Send + Sync {
    /// Return the set of channel UUIDs in `ctx`'s community accessible to `pubkey`.
    ///
    /// Channels in other communities, even with the same UUID, MUST NOT appear.
    fn accessible_channel_ids(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
    ) -> impl Future<Output = Result<HashSet<Uuid>, AuthError>> + Send;

    /// Returns `true` if `pubkey` is a member of `(ctx.community, channel_id)`.
    ///
    /// Default implementation calls [`Self::accessible_channel_ids`] and checks
    /// membership. Implementations may override this with a more efficient
    /// scoped point-lookup query.
    fn can_access(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
        channel_id: Uuid,
    ) -> impl Future<Output = Result<bool, AuthError>> + Send {
        async move {
            let ids = self.accessible_channel_ids(ctx, pubkey).await?;
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

/// Verify read access: scope + membership in `ctx`'s community.
pub async fn check_read_access(
    checker: &impl ChannelAccessChecker,
    ctx: &TenantContext,
    pubkey: &PublicKey,
    channel_id: Uuid,
    scopes: &[Scope],
) -> Result<(), AuthError> {
    require_scope(scopes, Scope::MessagesRead)?;
    if checker.can_access(ctx, pubkey, channel_id).await? {
        Ok(())
    } else {
        Err(AuthError::ChannelAccessDenied)
    }
}

/// Verify write access: scope + membership in `ctx`'s community.
pub async fn check_write_access(
    checker: &impl ChannelAccessChecker,
    ctx: &TenantContext,
    pubkey: &PublicKey,
    channel_id: Uuid,
    scopes: &[Scope],
) -> Result<(), AuthError> {
    require_scope(scopes, Scope::MessagesWrite)?;
    if checker.can_access(ctx, pubkey, channel_id).await? {
        Ok(())
    } else {
        Err(AuthError::ChannelAccessDenied)
    }
}

/// In-memory [`ChannelAccessChecker`] for unit tests.
///
/// Membership is keyed on the full `(community_id, pubkey, channel_id)` tuple
/// so the mock can't accidentally model a non-tenant-scoped checker.
#[cfg(any(test, feature = "test-utils"))]
pub struct MockAccessChecker {
    allowed: HashSet<(uuid::Uuid, String, Uuid)>,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockAccessChecker {
    /// Create an empty checker (all access denied by default).
    pub fn new() -> Self {
        Self {
            allowed: HashSet::new(),
        }
    }

    /// Grant `pubkey` access to `channel_id` inside `ctx`'s community.
    pub fn allow(&mut self, ctx: &TenantContext, pubkey: &PublicKey, channel_id: Uuid) {
        self.allowed
            .insert((*ctx.community().as_uuid(), pubkey.to_hex(), channel_id));
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
    async fn accessible_channel_ids(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
    ) -> Result<HashSet<Uuid>, AuthError> {
        let community = *ctx.community().as_uuid();
        let hex = pubkey.to_hex();
        Ok(self
            .allowed
            .iter()
            .filter(|(c, pk, _)| *c == community && pk == &hex)
            .map(|(_, _, id)| *id)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::Keys;

    fn fixture_ctx() -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "test.example")
    }

    #[tokio::test]
    async fn mock_checker_allow_and_deny() {
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let pk = keys.public_key();
        let allowed_ch = Uuid::new_v4();
        let denied_ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&ctx, &pk, allowed_ch);

        assert!(checker.can_access(&ctx, &pk, allowed_ch).await.unwrap());
        assert!(!checker.can_access(&ctx, &pk, denied_ch).await.unwrap());
    }

    #[tokio::test]
    async fn read_access_denied_by_scope() {
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&ctx, &pk, ch);

        assert!(matches!(
            check_read_access(&checker, &ctx, &pk, ch, &[]).await,
            Err(AuthError::InsufficientScope { .. })
        ));
    }

    #[tokio::test]
    async fn read_access_denied_by_membership() {
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();
        let checker = MockAccessChecker::new();

        assert!(matches!(
            check_read_access(&checker, &ctx, &pk, ch, &[Scope::MessagesRead]).await,
            Err(AuthError::ChannelAccessDenied)
        ));
    }

    #[tokio::test]
    async fn read_access_granted() {
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&ctx, &pk, ch);

        assert!(
            check_read_access(&checker, &ctx, &pk, ch, &[Scope::MessagesRead])
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn access_does_not_cross_communities() {
        // S1 fence at the access layer: same pubkey, same channel UUID, two
        // communities. A grant in A MUST NOT show up under B's TenantContext.
        // This bites the existence-oracle direction a bare `WHERE id=$1`
        // checker would have left open.
        let ctx_a = fixture_ctx();
        let ctx_b = fixture_ctx();
        let keys = Keys::generate();
        let pk = keys.public_key();
        let ch = Uuid::new_v4();

        let mut checker = MockAccessChecker::new();
        checker.allow(&ctx_a, &pk, ch);

        assert!(checker.can_access(&ctx_a, &pk, ch).await.unwrap());
        assert!(
            !checker.can_access(&ctx_b, &pk, ch).await.unwrap(),
            "access in community A must NOT leak into community B for same (pubkey, channel_id)"
        );
        assert!(checker
            .accessible_channel_ids(&ctx_b, &pk)
            .await
            .unwrap()
            .is_empty());
    }
}
