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
    AuthenticatedClientPeer,
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

/// Atomically admit one optional client-status presentation across its domain,
/// authenticated actor, and authenticated end-client peer coordinates.
///
/// All keys share one Redis Cluster hash tag. The script validates every
/// counter before incrementing any of them, so denial cannot partially consume
/// another coordinate or refresh an existing window.
const CLIENT_STATUS_ADMISSION_SCRIPT: &str = r#"
local window = tonumber(ARGV[1])
local limits = {tonumber(ARGV[2]), tonumber(ARGV[3]), tonumber(ARGV[4])}
if not window or window < 1 then
  return {-1, -1}
end

local counts = {}
local ttls = {}
for i = 1, 3 do
  if not limits[i] or limits[i] < 1 then
    return {-1, -1}
  end
  local raw = redis.call('GET', KEYS[i])
  if raw then
    local count = tonumber(raw)
    local ttl = redis.call('TTL', KEYS[i])
    if not count or count < 0 or ttl < 0 then
      return {-1, -1}
    end
    counts[i] = count
    ttls[i] = ttl
  else
    counts[i] = 0
    ttls[i] = window
  end
end

local denied = false
local reset = 0
for i = 1, 3 do
  if ttls[i] > reset then
    reset = ttls[i]
  end
  if counts[i] >= limits[i] then
    denied = true
  end
end
if denied then
  return {0, reset}
end

for i = 1, 3 do
  local count = redis.call('INCR', KEYS[i])
  if count == 1 then
    redis.call('EXPIRE', KEYS[i], window)
  end
end
return {1, reset}
"#;

/// Result of the bounded client-status admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStatusAdmissionResult {
    /// Whether all three coordinates were atomically admitted.
    pub allowed: bool,
    /// Remaining fixed-window lifetime reported by Redis.
    pub reset_in_secs: u64,
}

/// Validated-by-caller limits for the three atomic status coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStatusAdmissionLimits {
    max_presentations_per_domain: u64,
    max_presentations_per_actor: u64,
    max_presentations_per_peer: u64,
}

impl ClientStatusAdmissionLimits {
    /// Bind the explicit domain, authenticated-actor, and authenticated-peer limits.
    pub const fn new(
        max_presentations_per_domain: u64,
        max_presentations_per_actor: u64,
        max_presentations_per_peer: u64,
    ) -> Self {
        Self {
            max_presentations_per_domain,
            max_presentations_per_actor,
            max_presentations_per_peer,
        }
    }
}

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

    /// Atomically check one optional current-binding status presentation.
    ///
    /// The peer type can only be produced by verified trusted-proxy provenance;
    /// raw socket and forwarding-header addresses are intentionally rejected by
    /// this API boundary.
    pub async fn check_client_status_admission(
        &self,
        tenant: &TenantContext,
        actor: &PublicKey,
        peer: &AuthenticatedClientPeer,
        limits: ClientStatusAdmissionLimits,
    ) -> Result<ClientStatusAdmissionResult, AuthError> {
        let window = client_status_positive_i64(
            buzz_core::client_binding_status::MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS,
            "window",
        )?;
        let domain_limit =
            client_status_positive_i64(limits.max_presentations_per_domain, "domain limit")?;
        let actor_limit =
            client_status_positive_i64(limits.max_presentations_per_actor, "actor limit")?;
        let peer_limit =
            client_status_positive_i64(limits.max_presentations_per_peer, "peer limit")?;
        let (domain_key, actor_key, peer_key) =
            client_status_admission_keys(tenant, actor, peer.admission_key());
        let mut connection = self
            .pool
            .get()
            .await
            .map_err(|error| AuthError::Internal(format!("Redis pool: {error}")))?;
        let (decision, reset): (i64, i64) = Script::new(CLIENT_STATUS_ADMISSION_SCRIPT)
            .key(domain_key)
            .key(actor_key)
            .key(peer_key)
            .arg(window)
            .arg(domain_limit)
            .arg(actor_limit)
            .arg(peer_limit)
            .invoke_async(&mut *connection)
            .await
            .map_err(|error| {
                AuthError::Internal(format!("Redis client status admission script: {error}"))
            })?;
        if !matches!(decision, 0 | 1) || reset < 0 {
            return Err(AuthError::Internal(
                "Redis client status admission response is invalid".to_owned(),
            ));
        }
        let reset_in_secs = u64::try_from(reset).map_err(|_| {
            AuthError::Internal("Redis client status admission reset is invalid".to_owned())
        })?;
        Ok(ClientStatusAdmissionResult {
            allowed: decision == 1,
            reset_in_secs,
        })
    }
}

fn client_status_positive_i64(value: u64, coordinate: &str) -> Result<i64, AuthError> {
    let value = i64::try_from(value).map_err(|_| {
        AuthError::Internal(format!("client status admission {coordinate} is invalid"))
    })?;
    if value < 1 {
        return Err(AuthError::Internal(format!(
            "client status admission {coordinate} is invalid"
        )));
    }
    Ok(value)
}

fn client_status_admission_keys(
    tenant: &TenantContext,
    actor: &PublicKey,
    authenticated_peer_key: &[u8; 32],
) -> (String, String, String) {
    // Literal braces create one Redis Cluster hash tag for the atomic script.
    let prefix = format!("buzz:{{{}}}:ratelimit:client-status", tenant.community());
    (
        format!("{prefix}:domain"),
        format!("{prefix}:actor:{}", actor.to_hex()),
        format!("{prefix}:peer:{}", lower_hex(authenticated_peer_key)),
    )
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
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
    use buzz_core::{CommunityId, TenantContext};
    use nostr::Keys;
    use uuid::Uuid;

    use super::*;

    fn tenant(id: u128) -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(Uuid::from_u128(id)),
            "status-admission.example",
        )
    }

    #[test]
    fn authenticated_peer_keys_preserve_proxy_fan_in_isolation() {
        let tenant = tenant(1);
        let actor_a = Keys::generate().public_key();
        let actor_b = Keys::generate().public_key();
        let peer_a = [0x11; 32];
        let peer_b = [0x22; 32];

        // The ingress socket is deliberately absent: authenticated clients
        // sharing one proxy retain independent peer coordinates.
        let a = client_status_admission_keys(&tenant, &actor_a, &peer_a);
        let b = client_status_admission_keys(&tenant, &actor_b, &peer_b);
        assert_eq!(a.0, b.0);
        assert_ne!(a.1, b.1);
        assert_ne!(a.2, b.2);

        let same_peer_other_actor = client_status_admission_keys(&tenant, &actor_b, &peer_a);
        assert_eq!(a.2, same_peer_other_actor.2);
        assert_ne!(a.1, same_peer_other_actor.1);
    }

    #[test]
    fn status_keys_share_cluster_slot_and_isolate_domains() {
        let actor = Keys::generate().public_key();
        let peer = [0x33; 32];
        let tenant_a = tenant(1);
        let tenant_b = tenant(2);
        let keys_a = client_status_admission_keys(&tenant_a, &actor, &peer);
        let keys_b = client_status_admission_keys(&tenant_b, &actor, &peer);
        let cluster_tag = format!("{{{}}}", tenant_a.community());
        for key in [&keys_a.0, &keys_a.1, &keys_a.2] {
            assert!(key.contains(&cluster_tag));
        }
        assert_ne!(keys_a.0, keys_b.0);
        assert_ne!(keys_a.1, keys_b.1);
        assert_ne!(keys_a.2, keys_b.2);
    }

    #[test]
    fn status_limits_reject_zero_and_overflow() {
        assert!(client_status_positive_i64(0, "test").is_err());
        assert!(client_status_positive_i64(u64::MAX, "test").is_err());
        assert_eq!(client_status_positive_i64(1, "test").ok(), Some(1));
    }

    #[tokio::test]
    #[ignore = "requires disposable Redis via REDIS_URL"]
    async fn live_redis_status_admission_is_atomic_across_domain_actor_and_peer() {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let pool = deadpool_redis::Config::from_url(redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("create disposable Redis pool");
        let limiter = RedisRateLimiter::new(pool.clone());
        let tenant = tenant(Uuid::new_v4().as_u128());
        let actor_a = Keys::generate().public_key();
        let actor_b = Keys::generate().public_key();
        let peer_a = AuthenticatedClientPeer::for_test([0x41; 32]);
        let peer_b = AuthenticatedClientPeer::for_test([0x42; 32]);
        let limits = ClientStatusAdmissionLimits::new(10, 1, 1);

        assert!(
            limiter
                .check_client_status_admission(&tenant, &actor_a, &peer_a, limits)
                .await
                .expect("admit first client")
                .allowed
        );
        assert!(
            !limiter
                .check_client_status_admission(&tenant, &actor_a, &peer_b, limits)
                .await
                .expect("deny repeated actor")
                .allowed
        );
        assert!(
            !limiter
                .check_client_status_admission(&tenant, &actor_b, &peer_a, limits)
                .await
                .expect("deny repeated peer")
                .allowed
        );
        assert!(
            limiter
                .check_client_status_admission(&tenant, &actor_b, &peer_b, limits)
                .await
                .expect("admit independent fan-in client")
                .allowed
        );

        let keys_a = client_status_admission_keys(&tenant, &actor_a, peer_a.admission_key());
        let keys_b = client_status_admission_keys(&tenant, &actor_b, peer_b.admission_key());
        let keys = [&keys_a.0, &keys_a.1, &keys_a.2, &keys_b.1, &keys_b.2];
        let mut connection = pool.get().await.expect("borrow disposable Redis");
        let counts: Vec<u64> = redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut *connection)
            .await
            .expect("read status counters");
        assert_eq!(counts, vec![2, 1, 1, 1, 1]);
        let _: usize = redis::cmd("DEL")
            .arg(&keys)
            .query_async(&mut *connection)
            .await
            .expect("remove exact disposable status counters");
    }
}
