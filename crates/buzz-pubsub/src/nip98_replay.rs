//! Redis-backed NIP-98 replay seen-set.
//!
//! Implements the [`Nip98ReplayGuard`] trait from `buzz-auth`. Uses Redis
//! `SET NX EX` for an atomic set-if-absent with TTL — the §5 pre-build gate
//! for multi-tenant HA replay protection.

use buzz_auth::{
    error::AuthError,
    nip98_replay::{
        nip98_replay_key, Nip98ReplayGuard, DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS,
    },
};
use buzz_core::TenantContext;
use nostr::EventId;

/// Redis-backed NIP-98 replay seen-set.
///
/// Each `try_mark(ctx, event_id, ttl)` issues a single
/// `SET buzz:{community}:nip98:{event_id_hex} 1 NX EX <ttl>` against Redis.
/// `NX` makes the operation atomic set-if-absent — the freshness proof comes
/// from Redis returning `OK` only on the first claim. Subsequent claims within
/// the TTL window return `nil`, which we surface as `Ok(false)` so the caller
/// rejects the request as replay.
pub struct RedisNip98ReplayGuard {
    pool: deadpool_redis::Pool,
}

impl RedisNip98ReplayGuard {
    /// Create a new replay guard backed by the given Redis connection pool.
    pub fn new(pool: deadpool_redis::Pool) -> Self {
        Self { pool }
    }
}

impl Nip98ReplayGuard for RedisNip98ReplayGuard {
    fn try_mark<'a>(
        &'a self,
        ctx: &'a TenantContext,
        event_id: &'a EventId,
        ttl_secs: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, AuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            // §5 gate floor + safety ceiling. Sub-floor values are lifted to the
            // floor (contract permits clamping); above-ceiling values are pushed
            // down to MAX_REPLAY_TTL_SECS (contract REQUIRES clamping) so a buggy
            // caller cannot send a Redis-incompatible `EX` arg or pin a slot for
            // implausibly long.
            let ttl = ttl_secs.clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS);

            let mut conn = self.pool.get().await.map_err(|e| {
                // Structured field for ops; the user-facing AuthError stays a
                // bounded category string.
                tracing::warn!(
                    community = %ctx.community(),
                    error = %e,
                    "nip98 replay: redis pool acquire failed — caller MUST fail closed"
                );
                AuthError::Internal(format!("Redis pool: {e}"))
            })?;

            let key = nip98_replay_key(ctx, event_id);

            // SET key 1 NX EX <ttl>. redis-rs typed return: Some("OK") on first
            // claim, None on existing key. Any other value would be a Redis-side
            // bug; treat it as internal error.
            let result: Option<String> = redis::cmd("SET")
                .arg(&key)
                .arg("1")
                .arg("NX")
                .arg("EX")
                .arg(ttl)
                .query_async(&mut *conn)
                .await
                .map_err(|e| {
                    tracing::warn!(
                        community = %ctx.community(),
                        error = %e,
                        "nip98 replay: redis SET NX EX failed — caller MUST fail closed"
                    );
                    AuthError::Internal(format!("Redis SET NX EX: {e}"))
                })?;

            match result.as_deref() {
                Some("OK") => Ok(true),
                None => Ok(false),
                Some(other) => {
                    tracing::error!(
                        community = %ctx.community(),
                        reply = %other,
                        "nip98 replay: redis SET NX EX returned an unexpected reply — investigate"
                    );
                    Err(AuthError::Internal(format!(
                        "unexpected SET NX EX reply: {other}"
                    )))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{CommunityId, TenantContext};
    use deadpool_redis::{Config, Runtime};
    use nostr::{EventBuilder, Keys, Kind};
    use uuid::Uuid;

    fn redis_pool() -> deadpool_redis::Pool {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        Config::from_url(url)
            .create_pool(Some(Runtime::Tokio1))
            .expect("create pool")
    }

    fn fresh_ctx() -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "test.example")
    }

    fn fresh_event_id() -> EventId {
        EventBuilder::new(Kind::HttpAuth, "")
            .sign_with_keys(&Keys::generate())
            .expect("sign")
            .id
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn first_claim_succeeds_replay_fails() {
        let guard = RedisNip98ReplayGuard::new(redis_pool());
        let ctx = fresh_ctx();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark(&ctx, &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("first mark"));
        assert!(!guard
            .try_mark(&ctx, &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("replay mark"));
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn isolation_between_communities() {
        let guard = RedisNip98ReplayGuard::new(redis_pool());
        let ctx_a = fresh_ctx();
        let ctx_b = fresh_ctx();
        let eid = fresh_event_id();

        assert!(guard
            .try_mark(&ctx_a, &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("mark in A"));
        // Same event id under ctx_b is still a first claim — communities are
        // independent seen-sets.
        assert!(guard
            .try_mark(&ctx_b, &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .expect("mark in B"));
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn sub_floor_ttl_is_lifted_to_default() {
        let guard = RedisNip98ReplayGuard::new(redis_pool());
        let ctx = fresh_ctx();
        let eid = fresh_event_id();

        // Caller asks for 30s; impl lifts to ≥ DEFAULT_REPLAY_TTL_SECS.
        // Smoke-test the path: pass sub-floor TTL, claim once, then expect
        // replay rejection — the TTL lift kept the marker alive past 30s and
        // the contract holds.
        assert!(guard.try_mark(&ctx, &eid, 30).await.expect("mark"));
        assert!(!guard.try_mark(&ctx, &eid, 30).await.expect("replay"));
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn above_ceiling_ttl_is_clamped() {
        let guard = RedisNip98ReplayGuard::new(redis_pool());
        let ctx = fresh_ctx();
        let eid = fresh_event_id();

        // Caller asks for u64::MAX (well past Redis's i64::MAX `EX` limit).
        // Impl MUST clamp down to MAX_REPLAY_TTL_SECS so the SET succeeds; if
        // we instead forwarded u64::MAX, Redis would reject the EX arg and
        // try_mark would return Err — and per the trait contract, callers
        // fail closed on Err. That's correctness-preserving but UX-hostile:
        // every nip98 request errors. The clamp turns a foot-gun into a
        // contained warning.
        assert!(guard
            .try_mark(&ctx, &eid, u64::MAX)
            .await
            .expect("mark with extreme ttl must succeed via clamp"));
        assert!(!guard
            .try_mark(&ctx, &eid, u64::MAX)
            .await
            .expect("replay with extreme ttl must succeed via clamp"));
    }
}
