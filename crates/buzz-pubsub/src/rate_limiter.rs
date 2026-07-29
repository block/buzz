//! Redis-backed rate limiter using a sliding-window log (Sorted Set).
//!
//! Implements the [`RateLimiter`] trait from `buzz-auth`.
//! Uses a single Lua script to atomically trim old entries, count the
//! remaining entries in the current window, conditionally add the new
//! request timestamp, and set a TTL for automatic key cleanup.
//!
//! ## Algorithm: sliding-window log
//!
//! Each request timestamp is stored as a member in a Redis Sorted Set (ZSET)
//! scored by its millisecond timestamp. On every check the script:
//!
//! 1. Removes entries older than the window (`ZREMRANGEBYSCORE`).
//! 2. Counts the surviving entries (`ZCARD`).
//! 3. If under the limit, adds the new timestamp (`ZADD`) and increments the
//!    count.
//! 4. Refreshes the key TTL so the ZSET is garbage-collected after the window.
//! 5. Returns `(count, reset_in_secs)`.
//!
//! Unlike a fixed-window counter, this guarantees that at most `limit` requests
//! are allowed in *any* trailing `window_secs` interval — there is no 2× burst
//! at window boundaries.

use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use buzz_auth::{
    error::AuthError,
    rate_limit::{LimitType, RateLimitResult, RateLimiter},
};
use buzz_core::TenantContext;
use nostr::PublicKey;
use redis::Script;

/// Sliding-window log rate-limit script.
///
/// Operates on a ZSET keyed at `KEYS[1]` whose members are request timestamps
/// (scored by the timestamp in milliseconds).
///
/// # Arguments (ARGV)
///
/// - `ARGV[1]` — `now`: current time in milliseconds.
/// - `ARGV[2]` — `window_secs`: the sliding window length in seconds.
/// - `ARGV[3]` — `limit`: maximum requests allowed in the window.
/// - `ARGV[4]` — `suffix`: unique suffix to disambiguate same-millisecond
///   members (prevents `ZADD` collisions for concurrent requests).
///
/// # Returns
///
/// `{count, reset_in_secs}` where `count` is the number of entries in the
/// window *after* this request (incremented only if admitted), and
/// `reset_in_secs` is the seconds remaining until the oldest entry expires out
/// of the window.
const RATE_LIMIT_SCRIPT: &str = r#"
local now = tonumber(ARGV[1])
local window_secs = tonumber(ARGV[2])
local limit = tonumber(ARGV[3])
local window_ms = window_secs * 1000
local cutoff = now - window_ms

-- Trim entries that have aged out of the sliding window.
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', cutoff)

local count = redis.call('ZCARD', KEYS[1])

if count < limit then
    -- Admit: add the new request timestamp. The suffix disambiguates members
    -- that share the same millisecond score (concurrent requests).
    redis.call('ZADD', KEYS[1], now, now .. '-' .. ARGV[4])
    count = count + 1
end

-- Refresh the TTL so the ZSET is cleaned up after the window passes.
redis.call('EXPIRE', KEYS[1], window_secs)

-- reset_in = time until the oldest surviving entry exits the window.
local reset_in = window_secs
local oldest = redis.call('ZRANGE', KEYS[1], 0, 0, 'WITHSCORES')
if oldest[2] ~= nil then
    local oldest_score = tonumber(oldest[2])
    local reset_ms = oldest_score + window_ms - now
    if reset_ms > 0 then
        reset_in = math.ceil(reset_ms / 1000)
    else
        reset_in = 0
    end
end

return {count, reset_in}
"#;

/// Current time in milliseconds since the Unix epoch.
///
/// Centralized here so the timestamp handed to the Lua script and the one
/// used for any Rust-side logic are the same source.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run the sliding-window rate-limit Lua script against `key` and return a
/// [`RateLimitResult`].
async fn run_rate_limit(
    pool: &deadpool_redis::Pool,
    key: &str,
    window_secs: u64,
    limit: u64,
) -> Result<RateLimitResult, AuthError> {
    let mut conn = pool
        .get()
        .await
        .map_err(|e| AuthError::Internal(format!("Redis pool: {e}")))?;

    let now = now_millis();
    // Unique suffix so concurrent requests in the same millisecond don't
    // collide as ZSET members (a collision would silently drop one request
    // from the count).
    let suffix = uuid::Uuid::new_v4().simple().to_string();

    let script = Script::new(RATE_LIMIT_SCRIPT);
    let (count, reset_in_secs): (u64, u64) = script
        .key(key)
        .arg(now)
        .arg(window_secs)
        .arg(limit)
        .arg(&suffix)
        .invoke_async(&mut *conn)
        .await
        .map_err(|e| AuthError::Internal(format!("Redis rate limit script: {e}")))?;

    if count <= limit {
        Ok(RateLimitResult::allowed(count, limit, reset_in_secs))
    } else {
        Ok(RateLimitResult::denied(count, limit, reset_in_secs))
    }
}

/// Redis-backed rate limiter using a sliding-window log (Sorted Set).
///
/// Pubkey keys are community-scoped via `&TenantContext`:
/// `buzz:{community}:ratelimit:{pubkey_hex}:{suffix}`. IP keys remain
/// operator-global: `buzz:ratelimit:ip:{ip}:conn`. The ZSET trim, count,
/// conditional add, and TTL refresh are all performed atomically inside a
/// single Lua script — no crash window can leave the key in an inconsistent
/// state.
pub struct RedisRateLimiter {
    pool: deadpool_redis::Pool,
}

impl RedisRateLimiter {
    /// Create a new `RedisRateLimiter` backed by the given connection pool.
    pub fn new(pool: deadpool_redis::Pool) -> Self {
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
    use buzz_auth::rate_limit::{ip_rate_limit_key, LimitType, RateLimiter};
    use buzz_core::{CommunityId, TenantContext};
    use nostr::Keys;
    use uuid::Uuid;

    /// Connect to a local Redis for integration testing. Returns `None` if
    /// Redis is unavailable, so `#[ignore]` tests are no-ops without infra.
    async fn test_pool() -> Option<deadpool_redis::Pool> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let cfg = deadpool_redis::Config::from_url(&url);
        cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?
            .status()
            .available
            .max(1);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        // Verify connectivity.
        let mut conn = pool.get().await.ok()?;
        let _: String = redis::cmd("PING").query_async(&mut *conn).await.ok()?;
        Some(pool)
    }

    fn fixture_ctx() -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "test.example")
    }

    /// Delete a rate-limit key so each test starts clean.
    async fn cleanup(pool: &deadpool_redis::Pool, key: &str) {
        if let Ok(mut conn) = pool.get().await {
            let _: redis::RedisResult<()> =
                redis::cmd("DEL").arg(key).query_async(&mut *conn).await;
        }
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn admits_requests_under_limit() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let limiter = RedisRateLimiter::new(pool.clone());
        let ctx = fixture_ctx();
        let keys = Keys::generate();

        let r1 = limiter
            .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 60, 5)
            .await
            .unwrap();
        assert!(r1.allowed, "first request should be allowed");
        assert_eq!(r1.current, 1);

        let r2 = limiter
            .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 60, 5)
            .await
            .unwrap();
        assert!(r2.allowed);
        assert_eq!(r2.current, 2);

        cleanup(
            &pool,
            &buzz_auth::rate_limit::rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages),
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn denies_requests_over_limit() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let limiter = RedisRateLimiter::new(pool.clone());
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let key =
            buzz_auth::rate_limit::rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages);

        // Exhaust the limit (3 per 60s).
        for _ in 0..3 {
            assert!(
                limiter
                    .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 60, 3)
                    .await
                    .unwrap()
                    .allowed
            );
        }

        // 4th request must be denied.
        let r = limiter
            .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 60, 3)
            .await
            .unwrap();
        assert!(!r.allowed, "4th request should be denied");
        assert_eq!(r.current, 3, "count should not exceed limit when denied");

        cleanup(&pool, &key).await;
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn sliding_window_admits_after_old_entries_expire() {
        // THE key test for sliding-window correctness: after the window passes,
        // old entries are trimmed and new requests are admitted. With a
        // fixed-window counter, a request just past the window boundary would
        // see count=0 and admit — but so would a burst straddling the boundary.
        // Here we verify the *delayed* re-admission works.
        let Some(pool) = test_pool().await else {
            return;
        };
        let limiter = RedisRateLimiter::new(pool.clone());
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let key =
            buzz_auth::rate_limit::rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages);

        // Use a short 2-second window for a fast test.
        for _ in 0..2 {
            assert!(
                limiter
                    .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 2, 2)
                    .await
                    .unwrap()
                    .allowed
            );
        }
        // At limit now.
        assert!(
            !limiter
                .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 2, 2)
                .await
                .unwrap()
                .allowed
        );

        // Wait for the window to pass.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        // Should be admitted again — old entries trimmed.
        let r = limiter
            .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 2, 2)
            .await
            .unwrap();
        assert!(r.allowed, "should be admitted after window expires");

        cleanup(&pool, &key).await;
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn no_double_burst_at_boundary() {
        // The fixed-window weakness: if you fire `limit` requests just before
        // a window boundary and `limit` more just after, the counter resets and
        // you get 2× burst. A sliding window must reject the second batch
        // because the first batch's entries are still in the window.
        //
        // We simulate this with a 10s window: exhaust the limit, then without
        // waiting, verify additional requests are still denied (the entries
        // haven't aged out).
        let Some(pool) = test_pool().await else {
            return;
        };
        let limiter = RedisRateLimiter::new(pool.clone());
        let ctx = fixture_ctx();
        let keys = Keys::generate();
        let key =
            buzz_auth::rate_limit::rate_limit_key(&ctx, &keys.public_key(), &LimitType::Messages);

        // Exhaust limit=3 in a 10s window.
        for _ in 0..3 {
            assert!(
                limiter
                    .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 10, 3)
                    .await
                    .unwrap()
                    .allowed
            );
        }

        // Immediately try 3 more — all must be denied. Under a fixed window,
        // if the boundary had just passed, these would be incorrectly admitted.
        for i in 0..3 {
            let r = limiter
                .check_and_increment(&ctx, &keys.public_key(), LimitType::Messages, 10, 3)
                .await
                .unwrap();
            assert!(
                !r.allowed,
                "request {} after limit must be denied (no boundary burst)",
                i + 1
            );
        }

        cleanup(&pool, &key).await;
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn ip_connection_limit_works() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let limiter = RedisRateLimiter::new(pool.clone());
        let ip: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        let key = ip_rate_limit_key(&ip);

        let r = limiter.check_ip_connection(&ip, 60, 2).await.unwrap();
        assert!(r.allowed);
        assert_eq!(r.current, 1);

        let r = limiter.check_ip_connection(&ip, 60, 2).await.unwrap();
        assert!(r.allowed);
        assert_eq!(r.current, 2);

        let r = limiter.check_ip_connection(&ip, 60, 2).await.unwrap();
        assert!(!r.allowed, "3rd IP connection over limit should be denied");

        cleanup(&pool, &key).await;
    }
}
