//! Redis-backed rate limiter using atomic Lua script (INCR + EXPIRE).
//!
//! Implements the [`RateLimiter`] trait from `buzz-auth`.
//! Uses a single Lua script to atomically INCR and conditionally EXPIRE,
//! eliminating the crash window where a key could exist without a TTL.
//!
//! ⚠️ Fixed windows allow up to 2× burst at boundaries. Upgrade to sliding
//! window or token bucket for strict limiting.

use std::net::IpAddr;

use buzz_auth::{
    error::AuthError,
    rate_limit::{LimitType, RateLimitResult, RateLimiter},
};
use buzz_core::TenantContext;
use nostr::PublicKey;
use redis::Script;

/// Atomically INCR the key, set EXPIRE on first call, and return
/// (count, ttl_ms). The TTL is in milliseconds because `TTL` truncates to
/// whole seconds and would report 0 while part of a second remains.
///
/// Using a Lua script ensures INCR and EXPIRE are executed atomically —
/// a crash between them can no longer leave a key without a TTL.
const RATE_LIMIT_SCRIPT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
    redis.call('EXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('PTTL', KEYS[1])
return {count, ttl}
"#;

/// Run the atomic rate-limit Lua script against `key` and return a
/// [`RateLimitResult`].
///
/// If the TTL comes back negative (key exists without expiry — broken state
/// from a prior crash), the key is repaired with a fresh EXPIRE and a warning
/// is logged.
async fn run_rate_limit(
    pool: &crate::RedisConnection,
    key: &str,
    window_secs: u64,
    limit: u64,
) -> Result<RateLimitResult, AuthError> {
    let mut conn = pool.clone();

    let script = Script::new(RATE_LIMIT_SCRIPT);
    let (count, ttl_ms): (u64, i64) = script
        .key(key)
        .arg(window_secs as i64)
        .invoke_async(&mut conn)
        .await
        .map_err(|e| AuthError::Internal(format!("Redis rate limit script: {e}")))?;

    // ttl == -1 means the key exists but has no expiry — broken state from a
    // prior crash between INCR and EXPIRE. Repair it now.
    let reset_in_secs = if ttl_ms < 0 {
        tracing::warn!(key = %key, "rate limit key has no TTL — repairing");
        let _: () = redis::cmd("EXPIRE")
            .arg(key)
            .arg(window_secs as i64)
            .query_async(&mut conn)
            .await
            .map_err(|e| AuthError::Internal(format!("Redis EXPIRE repair: {e}")))?;
        // After repair, the window resets to the full duration.
        window_secs
    } else {
        // Round up so the retry hint is never 0 while the window is open.
        (ttl_ms as u64).div_ceil(1000)
    };

    if count <= limit {
        Ok(RateLimitResult::allowed(count, limit, reset_in_secs))
    } else {
        Ok(RateLimitResult::denied(count, limit, reset_in_secs))
    }
}

/// Redis-backed rate limiter using fixed-window counters.
///
/// Pubkey keys are community-scoped via `&TenantContext`:
/// `buzz:{community}:ratelimit:{pubkey_hex}:{suffix}`. IP keys remain
/// operator-global: `buzz:ratelimit:ip:{ip}:conn`. The counter and its TTL are
/// managed atomically via a Lua script to prevent keys from persisting without
/// expiry.
pub struct RedisRateLimiter {
    pool: crate::RedisConnection,
}

impl RedisRateLimiter {
    /// Create a new `RedisRateLimiter` backed by the given connection pool.
    pub fn new(pool: crate::RedisConnection) -> Self {
        Self { pool }
    }
}

impl RateLimiter for RedisRateLimiter {
    async fn check_and_increment(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
        limit_type: LimitType,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        let key = buzz_auth::rate_limit::rate_limit_key(ctx, pubkey, &limit_type);
        run_rate_limit(&self.pool, &key, window_secs, limit).await
    }

    async fn check_ip_connection(
        &self,
        ip: &IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, AuthError> {
        let key = buzz_auth::rate_limit::ip_rate_limit_key(ip);
        run_rate_limit(&self.pool, &key, window_secs, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redis_pool() -> crate::RedisConnection {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        crate::redis_connection(&url).expect("redis connection")
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn retry_hint_rounds_a_partial_second_up() {
        let pool = redis_pool();
        let key = format!("buzz:test:ratelimit:{}", uuid::Uuid::new_v4());
        let _: () = redis::cmd("SET")
            .arg(&key)
            .arg(1)
            .arg("PX")
            .arg(1400)
            .query_async(&mut pool.clone())
            .await
            .expect("seed exhausted window");

        let result = run_rate_limit(&pool, &key, 60, 1)
            .await
            .expect("rate limit");

        assert!(!result.allowed);
        assert_eq!(result.reset_in_secs, 2);
    }
}
