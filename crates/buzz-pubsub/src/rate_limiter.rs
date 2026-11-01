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
    rate_limit::{
        AgentBudgetScope, AgentEventBudget, AgentEventVerdict, LimitType, RateLimitResult,
        RateLimiter,
    },
};
use buzz_core::TenantContext;
use nostr::PublicKey;
use redis::Script;

/// Atomically INCR the key, set EXPIRE on first call, and return (count, ttl).
///
/// Using a Lua script ensures INCR and EXPIRE are executed atomically —
/// a crash between them can no longer leave a key without a TTL.
const RATE_LIMIT_SCRIPT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
    redis.call('EXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('TTL', KEYS[1])
return {count, ttl}
"#;

/// Admit one event against an agent owner's channel counter (`KEYS[1]`) and
/// overall counter (`KEYS[2]`), all or nothing.
///
/// `ARGV` is the window in seconds, then the channel and owner limits. Returns
/// `{refused, reset}`: `refused` has bit 1 set when the channel budget is full
/// and bit 2 when the owner budget is full, and `reset` is the longest
/// remaining window among the full budgets. When neither is full, both
/// counters advance and `{0, 0}` is returned. A refused event advances
/// neither counter. A counter found without an expiry gets one, so it cannot
/// refuse forever.
const AGENT_EVENT_SCRIPT: &str = r#"
local window = tonumber(ARGV[1])
local limits = {tonumber(ARGV[2]), tonumber(ARGV[3])}
local refused, reset = 0, 0
for i, key in ipairs(KEYS) do
    local count = tonumber(redis.call('GET', key) or '0')
    if count >= limits[i] then
        refused = refused + i
        local ttl = redis.call('TTL', key)
        if ttl < 0 then
            if ttl == -1 then
                redis.call('EXPIRE', key, window)
            end
            ttl = window
        end
        reset = math.max(reset, ttl)
    end
end
if refused ~= 0 then
    return {refused, reset}
end
for _, key in ipairs(KEYS) do
    if redis.call('INCR', key) == 1 or redis.call('TTL', key) < 0 then
        redis.call('EXPIRE', key, window)
    end
end
return {0, 0}
"#;

/// Run the atomic rate-limit Lua script against `key` and return a
/// [`RateLimitResult`].
///
/// If the TTL comes back negative (key exists without expiry — broken state
/// from a prior crash), the key is repaired with a fresh EXPIRE and a warning
/// is logged.
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

    let script = Script::new(RATE_LIMIT_SCRIPT);
    let (count, ttl): (u64, i64) = script
        .key(key)
        .arg(window_secs as i64)
        .invoke_async(&mut *conn)
        .await
        .map_err(|e| AuthError::Internal(format!("Redis rate limit script: {e}")))?;

    // ttl == -1 means the key exists but has no expiry — broken state from a
    // prior crash between INCR and EXPIRE. Repair it now.
    let reset_in_secs = if ttl < 0 {
        tracing::warn!(key = %key, "rate limit key has no TTL — repairing");
        let _: () = redis::cmd("EXPIRE")
            .arg(key)
            .arg(window_secs as i64)
            .query_async(&mut *conn)
            .await
            .map_err(|e| AuthError::Internal(format!("Redis EXPIRE repair: {e}")))?;
        // After repair, the window resets to the full duration.
        window_secs
    } else {
        ttl.max(0) as u64
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

    async fn check_agent_event(
        &self,
        ctx: &TenantContext,
        owner: &PublicKey,
        channel: Option<uuid::Uuid>,
        budget: AgentEventBudget,
    ) -> Result<AgentEventVerdict, AuthError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AuthError::Internal(format!("Redis pool: {e}")))?;
        let (refused, reset_in_secs): (u8, u64) = Script::new(AGENT_EVENT_SCRIPT)
            .key(buzz_auth::rate_limit::agent_channel_budget_key(
                ctx, owner, channel,
            ))
            .key(buzz_auth::rate_limit::agent_owner_budget_key(ctx, owner))
            .arg(budget.window_secs)
            .arg(budget.channel_limit)
            .arg(budget.owner_limit)
            .invoke_async(&mut *conn)
            .await
            .map_err(|e| AuthError::Internal(format!("Redis agent event script: {e}")))?;
        Ok(match refused {
            0 => AgentEventVerdict::Admitted,
            1 => AgentEventVerdict::Exceeded {
                scope: AgentBudgetScope::Channel,
                reset_in_secs,
            },
            _ => AgentEventVerdict::Exceeded {
                scope: AgentBudgetScope::Owner,
                reset_in_secs,
            },
        })
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
