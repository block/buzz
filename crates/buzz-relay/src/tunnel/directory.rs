//! Redis-backed fenced session directory for relay mesh tunnel sessions.
//!
//! Correctness law: mesh membership is only a routing hint. Redis is the
//! arbiter for session ownership, and every session-bearing frame must validate
//! its `{session_id, generation, owner_runtime_id}` fence against this directory
//! before it is accepted or forwarded.

use std::time::Duration;

use buzz_core::CommunityId;
use buzz_relay_mesh::{FencedHeader, MeshError, Profile, RuntimeId};
use redis::Script;
use uuid::Uuid;

const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

/// Redis key format used for tunnel fences during the staged cluster migration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionKeyFormat {
    /// Original untagged keys. This remains the default during the drain deploy.
    #[default]
    Legacy,
    /// Cluster-safe keys co-slotted by a first-position session hash tag.
    Tagged,
}

const ACQUIRE_SCRIPT: &str = r#"
local lease_key = KEYS[1]
local generation_key = KEYS[2]
local owner = ARGV[1]
local profile = ARGV[2]
local ttl_ms = tonumber(ARGV[3])
local legacy_lease = ARGV[4]
local legacy_generation = ARGV[5]

local current = redis.call('GET', lease_key)
if current then
    return {'exists', current, redis.call('GET', generation_key) or ''}
end

-- A live legacy lease is allowed to drain before this key format takes over.
-- Deployments must drain legacy writers before enabling the tagged format; the
-- separate legacy reads cannot be made atomic across Redis Cluster slots.
if not redis.call('GET', generation_key) and legacy_lease ~= '' then
    return {'exists', legacy_lease, legacy_generation}
end

-- Preserve the old non-expiring fence watermark on first use. SETNX accepts the
-- integer as an exact string, avoiding Lua-number precision loss.
if legacy_generation ~= '' then
    redis.call('SETNX', generation_key, legacy_generation)
end
local generation = redis.call('INCR', generation_key)
local value = owner .. '|' .. tostring(generation) .. '|' .. profile
redis.call('SET', lease_key, value, 'PX', ttl_ms)
return {'acquired', value, tostring(generation)}
"#;

const RENEW_SCRIPT: &str = r#"
local lease_key = KEYS[1]
local generation_key = KEYS[2]
local owner = ARGV[1]
local generation = ARGV[2]
local ttl_ms = tonumber(ARGV[3])

local current = redis.call('GET', lease_key)
if not current then
    return {'missing', '', redis.call('GET', generation_key) or ''}
end

local current_owner, current_generation, current_profile = string.match(current, '^([^|]+)|([^|]+)|([^|]+)$')
if current_owner == owner and current_generation == generation then
    redis.call('PEXPIRE', lease_key, ttl_ms)
    return {'renewed', current, redis.call('GET', generation_key) or current_generation}
end

return {'lost', current, redis.call('GET', generation_key) or current_generation or ''}
"#;

const RELEASE_SCRIPT: &str = r#"
local lease_key = KEYS[1]
local generation_key = KEYS[2]
local owner = ARGV[1]
local generation = ARGV[2]

local current = redis.call('GET', lease_key)
if not current then
    return {'missing', '', redis.call('GET', generation_key) or ''}
end

local current_owner, current_generation, current_profile = string.match(current, '^([^|]+)|([^|]+)|([^|]+)$')
if current_owner == owner and current_generation == generation then
    redis.call('DEL', lease_key)
    return {'released', current, redis.call('GET', generation_key) or current_generation}
end

return {'lost', current, redis.call('GET', generation_key) or current_generation or ''}
"#;

const VALIDATE_SCRIPT: &str = r#"
local lease_key = KEYS[1]
local generation_key = KEYS[2]
local legacy_lease = ARGV[1]
local legacy_generation = ARGV[2]

local current = redis.call('GET', lease_key) or ''
local known_generation = redis.call('GET', generation_key)
if known_generation then
    return {current, known_generation}
end
return {legacy_lease, legacy_generation}
"#;

/// Redis-backed owner directory for mesh tunnel sessions.
#[derive(Clone)]
pub struct SessionDirectory {
    pool: deadpool_redis::Pool,
    lease_ttl: Duration,
    key_format: SessionKeyFormat,
}

/// Active session ownership lease read from Redis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLease {
    /// Community/tenant scope for this session.
    pub community_id: CommunityId,
    /// Session id carried in every fenced frame.
    pub session_id: Uuid,
    /// Runtime currently allowed to own/send for this session generation.
    pub owner_runtime_id: RuntimeId,
    /// Monotonic Redis generation. Never derived from expiring lease state.
    pub generation: u64,
    /// Tunnel profile for the session.
    pub profile: Profile,
}

/// Result of attempting to acquire ownership for a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquireResult {
    /// This caller created the lease and owns the returned generation.
    Acquired(SessionLease),
    /// A live lease already exists; caller must route to that owner or retry.
    Exists(SessionLease),
}

/// Result of renewing an existing owned lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenewResult {
    /// Lease TTL was extended.
    Renewed(SessionLease),
    /// Lease was absent or owned by a different fenced tuple.
    Lost {
        /// Live lease currently in Redis, if the lease key still exists.
        current: Option<SessionLease>,
        /// Highest generation known from the non-expiring counter.
        known_generation: Option<u64>,
    },
}

/// Result of releasing an existing owned lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseResult {
    /// Lease was deleted.
    Released(SessionLease),
    /// Lease was absent or no longer matched this owner/generation.
    NotOwner {
        /// Live lease currently in Redis, if the lease key still exists.
        current: Option<SessionLease>,
        /// Highest generation known from the non-expiring counter.
        known_generation: Option<u64>,
    },
}

/// Errors from the Redis session directory.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    /// Redis pool checkout failed.
    #[error("redis pool: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    /// Redis command/script failed.
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    /// Redis contained a malformed lease value.
    #[error("malformed session lease for {community_id}/{session_id}: {value:?}")]
    MalformedLease {
        /// Community/tenant scope for the malformed lease.
        community_id: CommunityId,
        /// Session id whose Redis value was malformed.
        session_id: Uuid,
        /// Raw Redis lease value.
        value: String,
    },
    /// Redis contained a malformed generation counter value.
    #[error("malformed session generation for {community_id}/{session_id}: {value:?}")]
    MalformedGeneration {
        /// Community/tenant scope for the malformed counter.
        community_id: CommunityId,
        /// Session id whose Redis counter was malformed.
        session_id: Uuid,
        /// Raw Redis generation value.
        value: String,
    },
    /// Redis script returned an unexpected status string.
    #[error("unexpected session directory script status {status:?}")]
    UnexpectedScriptStatus {
        /// Raw status string returned by Lua.
        status: String,
    },
    /// Lease TTL cannot be represented in Redis milliseconds.
    #[error("lease ttl must be at least 1ms and fit in i64 milliseconds")]
    InvalidLeaseTtl,
}

impl SessionDirectory {
    /// Create a directory backed by `pool` with the default lease TTL.
    pub fn new(pool: deadpool_redis::Pool) -> Self {
        Self::with_key_format(pool, DEFAULT_LEASE_TTL, SessionKeyFormat::Legacy)
    }

    /// Create a directory backed by `pool` with an explicit lease TTL.
    pub fn with_lease_ttl(pool: deadpool_redis::Pool, lease_ttl: Duration) -> Self {
        Self::with_key_format(pool, lease_ttl, SessionKeyFormat::Legacy)
    }

    /// Create a directory using an explicitly selected migration key format.
    pub fn with_key_format(
        pool: deadpool_redis::Pool,
        lease_ttl: Duration,
        key_format: SessionKeyFormat,
    ) -> Self {
        Self {
            pool,
            lease_ttl,
            key_format,
        }
    }

    /// Attempt to create/take over the session lease.
    ///
    /// If no live lease exists, Redis atomically increments the companion
    /// non-expiring generation key and writes the lease with the new generation.
    /// If a lease exists, the generation key is not touched.
    pub async fn acquire(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner_runtime_id: RuntimeId,
        profile: Profile,
    ) -> Result<AcquireResult, DirectoryError> {
        let keys = SessionKeys::new(community_id, session_id);
        let ttl_ms = ttl_ms(self.lease_ttl)?;
        let mut conn = self.pool.get().await?;
        let (lease_key, generation_key, legacy) = match self.key_format {
            SessionKeyFormat::Legacy => (
                &keys.legacy_lease,
                &keys.legacy_generation,
                LegacyValues::default(),
            ),
            SessionKeyFormat::Tagged => (
                &keys.lease,
                &keys.generation,
                read_legacy_keys(&mut conn, &keys).await?,
            ),
        };
        let (status, value, _known_generation): (String, String, String) =
            Script::new(ACQUIRE_SCRIPT)
                .key(lease_key)
                .key(generation_key)
                .arg(owner_runtime_id.to_hex())
                .arg(profile.as_wire_str())
                .arg(ttl_ms)
                .arg(&legacy.lease)
                .arg(&legacy.generation)
                .invoke_async(&mut *conn)
                .await?;
        let lease = parse_lease(community_id, session_id, &value)?;
        match status.as_str() {
            "acquired" => Ok(AcquireResult::Acquired(lease)),
            "exists" => Ok(AcquireResult::Exists(lease)),
            _ => Err(DirectoryError::UnexpectedScriptStatus { status }),
        }
    }

    /// Attempt to take over a session whose previous lease is absent/expired.
    ///
    /// This uses the same atomic Redis path as [`Self::acquire`]: if no live
    /// lease exists, the non-expiring generation counter is incremented and the
    /// new lease is written in one Lua script. If a live lease exists, the
    /// caller receives [`AcquireResult::Exists`] and must not proceed as owner.
    pub async fn takeover(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
        owner_runtime_id: RuntimeId,
        profile: Profile,
    ) -> Result<AcquireResult, DirectoryError> {
        self.acquire(community_id, session_id, owner_runtime_id, profile)
            .await
    }

    /// Renew a lease only if the current Redis value exactly matches the
    /// caller's owner runtime and generation.
    pub async fn renew(&self, lease: &SessionLease) -> Result<RenewResult, DirectoryError> {
        let keys = SessionKeys::new(lease.community_id, lease.session_id);
        let ttl_ms = ttl_ms(self.lease_ttl)?;
        let mut conn = self.pool.get().await?;
        let (status, value, known_generation): (String, String, String) = Script::new(RENEW_SCRIPT)
            .key(keys.lease(self.key_format))
            .key(keys.generation(self.key_format))
            .arg(lease.owner_runtime_id.to_hex())
            .arg(lease.generation)
            .arg(ttl_ms)
            .invoke_async(&mut *conn)
            .await?;
        let current = parse_optional_lease(lease.community_id, lease.session_id, &value)?;
        match status.as_str() {
            "renewed" => Ok(RenewResult::Renewed(
                current.expect("renewed returns lease"),
            )),
            "missing" | "lost" => Ok(RenewResult::Lost {
                current,
                known_generation: parse_optional_generation(
                    lease.community_id,
                    lease.session_id,
                    &known_generation,
                )?,
            }),
            _ => Err(DirectoryError::UnexpectedScriptStatus { status }),
        }
    }

    /// Release a lease only if the current Redis value exactly matches the
    /// caller's owner runtime and generation.
    pub async fn release(&self, lease: &SessionLease) -> Result<ReleaseResult, DirectoryError> {
        let keys = SessionKeys::new(lease.community_id, lease.session_id);
        let mut conn = self.pool.get().await?;
        let (status, value, known_generation): (String, String, String) =
            Script::new(RELEASE_SCRIPT)
                .key(keys.lease(self.key_format))
                .key(keys.generation(self.key_format))
                .arg(lease.owner_runtime_id.to_hex())
                .arg(lease.generation)
                .invoke_async(&mut *conn)
                .await?;
        let current = parse_optional_lease(lease.community_id, lease.session_id, &value)?;
        match status.as_str() {
            "released" => Ok(ReleaseResult::Released(
                current.expect("released returns lease"),
            )),
            "missing" | "lost" => Ok(ReleaseResult::NotOwner {
                current,
                known_generation: parse_optional_generation(
                    lease.community_id,
                    lease.session_id,
                    &known_generation,
                )?,
            }),
            _ => Err(DirectoryError::UnexpectedScriptStatus { status }),
        }
    }

    /// Look up the current live lease, if any.
    pub async fn lookup(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<SessionLease>, DirectoryError> {
        let keys = SessionKeys::new(community_id, session_id);
        let mut conn = self.pool.get().await?;
        let (value, _known_generation) = read_keys(&mut conn, &keys, self.key_format).await?;
        parse_optional_lease(community_id, session_id, &value)
    }

    /// Read the non-expiring generation counter for a session, if it exists.
    pub async fn known_generation(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<u64>, DirectoryError> {
        let keys = SessionKeys::new(community_id, session_id);
        let mut conn = self.pool.get().await?;
        let (_lease, generation) = read_keys(&mut conn, &keys, self.key_format).await?;
        parse_optional_generation(community_id, session_id, &generation)
    }

    /// Validate a session-bearing mesh frame fence against Redis.
    ///
    /// This is the hop-by-hop guard: a frame is accepted only when a live lease
    /// exists and its owner/generation exactly match the frame. Fence-visible
    /// rejections return typed [`MeshError`] variants so Wren's chaos gate can
    /// distinguish `stale_generation`, `no_active_lease`, `owner_mismatch`, and
    /// `future_generation`.
    pub async fn validate_fenced_header(
        &self,
        community_id: CommunityId,
        fenced: &FencedHeader,
    ) -> Result<(), MeshError> {
        let keys = SessionKeys::new(community_id, fenced.session_id);
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| MeshError::Transport(format!("redis pool: {e}")))?;
        let (lease_value, known_generation) = read_keys(&mut conn, &keys, self.key_format)
            .await
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        let known_from_counter =
            parse_optional_generation(community_id, fenced.session_id, &known_generation)
                .map_err(|e| MeshError::Transport(e.to_string()))?
                .unwrap_or(0);
        let current = parse_optional_lease(community_id, fenced.session_id, &lease_value)
            .map_err(|e| MeshError::Transport(e.to_string()))?;
        let known = current
            .as_ref()
            .map(|lease| lease.generation)
            .unwrap_or(known_from_counter)
            .max(known_from_counter);

        if known > 0 && fenced.generation < known {
            record_fence_rejection("stale_generation");
            return Err(MeshError::StaleGeneration {
                session_id: fenced.session_id,
                frame_generation: fenced.generation,
                known_generation: known,
            });
        }

        let Some(current) = current else {
            tracing::warn!(
                community_id = %community_id,
                session_id = %fenced.session_id,
                frame_generation = fenced.generation,
                known_generation = known,
                frame_owner_runtime_id = %fenced.owner_runtime_id,
                "rejected fenced frame because no active session lease exists"
            );
            record_fence_rejection("no_active_lease");
            return Err(MeshError::NoActiveLease {
                session_id: fenced.session_id,
                frame_generation: fenced.generation,
                known_generation: known,
                frame_owner_runtime_id: fenced.owner_runtime_id,
            });
        };

        if fenced.generation != current.generation {
            tracing::warn!(
                community_id = %community_id,
                session_id = %fenced.session_id,
                frame_generation = fenced.generation,
                lease_generation = current.generation,
                frame_owner_runtime_id = %fenced.owner_runtime_id,
                "rejected fenced frame with generation that does not match active lease"
            );
            record_fence_rejection("future_generation");
            return Err(MeshError::FutureGeneration {
                session_id: fenced.session_id,
                frame_generation: fenced.generation,
                known_generation: current.generation,
            });
        }

        if fenced.owner_runtime_id != current.owner_runtime_id {
            tracing::warn!(
                community_id = %community_id,
                session_id = %fenced.session_id,
                generation = fenced.generation,
                frame_owner_runtime_id = %fenced.owner_runtime_id,
                lease_owner_runtime_id = %current.owner_runtime_id,
                "rejected fenced frame because owner runtime does not match active lease"
            );
            record_fence_rejection("owner_mismatch");
            return Err(MeshError::OwnerMismatch {
                session_id: fenced.session_id,
                generation: fenced.generation,
                frame_owner_runtime_id: fenced.owner_runtime_id,
                current_owner_runtime_id: current.owner_runtime_id,
            });
        }

        Ok(())
    }
}

impl SessionLease {
    /// Convert this lease to the fenced header carried by mesh frames.
    pub fn fenced_header(&self) -> FencedHeader {
        FencedHeader {
            session_id: self.session_id,
            generation: self.generation,
            owner_runtime_id: self.owner_runtime_id,
        }
    }
}

struct SessionKeys {
    lease: String,
    generation: String,
    legacy_lease: String,
    legacy_generation: String,
}

impl SessionKeys {
    fn lease(&self, format: SessionKeyFormat) -> &str {
        match format {
            SessionKeyFormat::Legacy => &self.legacy_lease,
            SessionKeyFormat::Tagged => &self.lease,
        }
    }

    fn generation(&self, format: SessionKeyFormat) -> &str {
        match format {
            SessionKeyFormat::Legacy => &self.legacy_generation,
            SessionKeyFormat::Tagged => &self.generation,
        }
    }

    fn new(community_id: CommunityId, session_id: Uuid) -> Self {
        // The session hash tag is deliberately the first `{...}` segment. Redis
        // Cluster hashes both script keys to this session's slot, while standard
        // Redis treats the braces as ordinary key bytes.
        let base = format!("buzz:{{{session_id}}}:{community_id}:tunnel");
        let legacy_base = format!("buzz:{community_id}:tunnel:{session_id}");
        Self {
            lease: format!("{base}:lease"),
            generation: format!("{base}:generation"),
            legacy_lease: format!("{legacy_base}:lease"),
            legacy_generation: format!("{legacy_base}:generation"),
        }
    }
}

#[derive(Default)]
struct LegacyValues {
    lease: String,
    generation: String,
}

async fn read_legacy_keys(
    conn: &mut deadpool_redis::Connection,
    keys: &SessionKeys,
) -> Result<LegacyValues, redis::RedisError> {
    // This pipeline is intentionally non-atomic: MULTI/EXEC would put the two
    // legacy keys in one cross-slot transaction on Redis Cluster. These reads
    // bridge the drain-aware key migration; tagged state always wins once its
    // non-expiring generation key has been initialized.
    let (lease, generation): (Option<String>, Option<String>) = redis::pipe()
        .cmd("GET")
        .arg(&keys.legacy_lease)
        .cmd("GET")
        .arg(&keys.legacy_generation)
        .query_async(&mut **conn)
        .await?;
    Ok(LegacyValues {
        lease: lease.unwrap_or_default(),
        generation: generation.unwrap_or_default(),
    })
}

async fn read_current_keys(
    conn: &mut deadpool_redis::Connection,
    keys: &SessionKeys,
    legacy: &LegacyValues,
) -> Result<(String, String), redis::RedisError> {
    read_current_keys_for(conn, keys, SessionKeyFormat::Tagged, legacy).await
}

async fn read_keys(
    conn: &mut deadpool_redis::Connection,
    keys: &SessionKeys,
    format: SessionKeyFormat,
) -> Result<(String, String), redis::RedisError> {
    match format {
        SessionKeyFormat::Legacy => {
            let empty = LegacyValues::default();
            read_current_keys_for(conn, keys, format, &empty).await
        }
        SessionKeyFormat::Tagged => read_keys_with_legacy_fallback(conn, keys).await,
    }
}

async fn read_current_keys_for(
    conn: &mut deadpool_redis::Connection,
    keys: &SessionKeys,
    format: SessionKeyFormat,
    legacy: &LegacyValues,
) -> Result<(String, String), redis::RedisError> {
    Script::new(VALIDATE_SCRIPT)
        .key(keys.lease(format))
        .key(keys.generation(format))
        .arg(&legacy.lease)
        .arg(&legacy.generation)
        .invoke_async(&mut **conn)
        .await
}

async fn read_keys_with_legacy_fallback(
    conn: &mut deadpool_redis::Connection,
    keys: &SessionKeys,
) -> Result<(String, String), redis::RedisError> {
    let empty = LegacyValues {
        lease: String::new(),
        generation: String::new(),
    };
    let current = read_current_keys(conn, keys, &empty).await?;
    if !current.1.is_empty() {
        return Ok(current);
    }
    let legacy = read_legacy_keys(conn, keys).await?;
    read_current_keys(conn, keys, &legacy).await
}

trait ProfileWireExt {
    fn as_wire_str(&self) -> &'static str;
}

impl ProfileWireExt for Profile {
    fn as_wire_str(&self) -> &'static str {
        match self {
            Profile::ReliableStream => "reliable-stream",
            Profile::RealtimeMedia => "realtime-media",
            Profile::HuddleControl => "huddle-control",
        }
    }
}

fn record_fence_rejection(reason: &'static str) {
    metrics::counter!("mesh_fence_rejections_total", "reason" => reason).increment(1);
}

fn profile_from_wire(value: &str) -> Option<Profile> {
    match value {
        "reliable-stream" => Some(Profile::ReliableStream),
        "realtime-media" => Some(Profile::RealtimeMedia),
        "huddle-control" => Some(Profile::HuddleControl),
        _ => None,
    }
}

fn parse_lease(
    community_id: CommunityId,
    session_id: Uuid,
    value: &str,
) -> Result<SessionLease, DirectoryError> {
    let malformed = || DirectoryError::MalformedLease {
        community_id,
        session_id,
        value: value.to_string(),
    };
    let mut parts = value.split('|');
    let owner_hex = parts.next().ok_or_else(malformed)?;
    let generation = parts
        .next()
        .ok_or_else(malformed)?
        .parse::<u64>()
        .map_err(|_| malformed())?;
    if generation == 0 {
        return Err(malformed());
    }
    let profile = parts
        .next()
        .and_then(profile_from_wire)
        .ok_or_else(malformed)?;
    if parts.next().is_some() {
        return Err(malformed());
    }
    let owner_bytes = hex::decode(owner_hex).map_err(|_| malformed())?;
    let owner_runtime_id = RuntimeId(owner_bytes.try_into().map_err(|_| malformed())?);
    Ok(SessionLease {
        community_id,
        session_id,
        owner_runtime_id,
        generation,
        profile,
    })
}

fn parse_optional_lease(
    community_id: CommunityId,
    session_id: Uuid,
    value: &str,
) -> Result<Option<SessionLease>, DirectoryError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_lease(community_id, session_id, value).map(Some)
    }
}

fn parse_optional_generation(
    community_id: CommunityId,
    session_id: Uuid,
    value: &str,
) -> Result<Option<u64>, DirectoryError> {
    if value.is_empty() {
        return Ok(None);
    }
    let generation = value
        .parse::<u64>()
        .map_err(|_| DirectoryError::MalformedGeneration {
            community_id,
            session_id,
            value: value.to_string(),
        })?;
    if generation == 0 {
        return Err(DirectoryError::MalformedGeneration {
            community_id,
            session_id,
            value: value.to_string(),
        });
    }
    Ok(Some(generation))
}

fn ttl_ms(ttl: Duration) -> Result<i64, DirectoryError> {
    i64::try_from(ttl.as_millis())
        .ok()
        .filter(|ms| *ms > 0)
        .ok_or(DirectoryError::InvalidLeaseTtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0xAAAA))
    }

    fn session() -> Uuid {
        Uuid::from_u128(0xBBBB)
    }

    fn runtime(byte: u8) -> RuntimeId {
        RuntimeId([byte; 32])
    }

    fn pool() -> deadpool_redis::Pool {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        deadpool_redis::Config::from_url(url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("create redis pool")
    }

    async fn redis_directory() -> SessionDirectory {
        let pool = pool();
        let mut conn = pool
            .get()
            .await
            .expect("REDIS_URL must be reachable for ignored tunnel directory tests");
        redis::cmd("PING")
            .query_async::<String>(&mut *conn)
            .await
            .expect("REDIS_URL must answer PING for ignored tunnel directory tests");
        drop(conn);
        SessionDirectory::with_key_format(
            pool,
            Duration::from_millis(150),
            SessionKeyFormat::Tagged,
        )
    }

    async fn clear_keys(directory: &SessionDirectory, community_id: CommunityId, session_id: Uuid) {
        let keys = SessionKeys::new(community_id, session_id);
        let mut conn = directory.pool.get().await.expect("redis conn");
        let _: () = redis::pipe()
            .cmd("DEL")
            .arg(keys.lease)
            .cmd("DEL")
            .arg(keys.generation)
            .cmd("DEL")
            .arg(keys.legacy_lease)
            .cmd("DEL")
            .arg(keys.legacy_generation)
            .query_async(&mut *conn)
            .await
            .expect("clear keys");
    }

    #[test]
    fn lease_value_roundtrips_profile_and_owner() {
        let value = format!("{}|42|huddle-control", runtime(7).to_hex());
        let lease = parse_lease(community(), session(), &value).expect("parse lease");
        assert_eq!(lease.owner_runtime_id, runtime(7));
        assert_eq!(lease.generation, 42);
        assert_eq!(lease.profile, Profile::HuddleControl);
        assert_eq!(lease.fenced_header().generation, 42);
    }

    #[test]
    fn malformed_lease_rejects_bad_owner_and_profile() {
        assert!(parse_lease(community(), session(), "not-hex|1|reliable-stream").is_err());
        assert!(parse_lease(
            community(),
            session(),
            &format!("{}|1|bogus", runtime(1).to_hex())
        )
        .is_err());
    }

    #[test]
    fn key_shape_is_community_scoped_and_separates_counter() {
        let keys = SessionKeys::new(community(), session());
        assert_eq!(
            keys.lease,
            format!("buzz:{{{}}}:{}:tunnel:lease", session(), community())
        );
        assert_eq!(
            keys.generation,
            format!("buzz:{{{}}}:{}:tunnel:generation", session(), community())
        );
        assert_eq!(
            keys.legacy_lease,
            format!("buzz:{}:tunnel:{}:lease", community(), session())
        );
        assert_eq!(
            keys.legacy_generation,
            format!("buzz:{}:tunnel:{}:generation", community(), session())
        );
        assert_eq!(keys.lease.find('{'), Some(5));
        assert_eq!(keys.generation.find('{'), Some(5));
        let tag = format!("{{{}}}", session());
        assert!(keys.lease.starts_with(&format!("buzz:{tag}:")));
        assert!(keys.generation.starts_with(&format!("buzz:{tag}:")));
        assert_ne!(keys.lease, keys.generation);
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn legacy_generation_and_live_lease_migrate_without_fence_regression() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;
        let keys = SessionKeys::new(community_id, session_id);
        let legacy_lease = format!("{}|41|reliable-stream", runtime(1).to_hex());
        let mut conn = directory.pool.get().await.expect("redis conn");
        let _: () = redis::pipe()
            .cmd("SET")
            .arg(&keys.legacy_generation)
            .arg(41_u64)
            .cmd("SET")
            .arg(&keys.legacy_lease)
            .arg(&legacy_lease)
            .arg("PX")
            .arg(5_000_u64)
            .query_async(&mut *conn)
            .await
            .expect("seed legacy keys");
        drop(conn);

        let existing = directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .expect("legacy lease remains authoritative");
        assert!(matches!(existing, AcquireResult::Exists(ref lease) if lease.generation == 41));
        let legacy = match existing {
            AcquireResult::Exists(lease) => lease,
            AcquireResult::Acquired(_) => unreachable!(),
        };
        assert_eq!(
            directory.lookup(community_id, session_id).await.unwrap(),
            Some(legacy)
        );
        let mut conn = directory.pool.get().await.expect("redis conn");
        let _: () = redis::cmd("DEL")
            .arg(&keys.legacy_lease)
            .query_async(&mut *conn)
            .await
            .expect("drain legacy lease");
        drop(conn);

        let migrated = match directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .expect("acquire tagged lease")
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("released legacy lease must drain"),
        };
        assert_eq!(migrated.generation, 42);
        assert_eq!(
            directory
                .known_generation(community_id, session_id)
                .await
                .unwrap(),
            Some(42)
        );
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn tagged_state_is_authoritative_when_both_formats_conflict() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;
        let keys = SessionKeys::new(community_id, session_id);
        let tagged = SessionLease {
            community_id,
            session_id,
            owner_runtime_id: runtime(2),
            generation: 42,
            profile: Profile::ReliableStream,
        };
        let legacy_value = format!("{}|99|reliable-stream", runtime(1).to_hex());
        let tagged_value = format!("{}|42|reliable-stream", runtime(2).to_hex());
        let mut conn = directory.pool.get().await.expect("redis conn");
        let _: () = redis::pipe()
            .cmd("SET")
            .arg(&keys.legacy_generation)
            .arg(99_u64)
            .cmd("SET")
            .arg(&keys.legacy_lease)
            .arg(&legacy_value)
            .arg("PX")
            .arg(5_000_u64)
            .cmd("SET")
            .arg(&keys.generation)
            .arg(42_u64)
            .cmd("SET")
            .arg(&keys.lease)
            .arg(&tagged_value)
            .arg("PX")
            .arg(5_000_u64)
            .query_async(&mut *conn)
            .await
            .expect("seed conflicting key formats");
        drop(conn);

        assert_eq!(
            directory.lookup(community_id, session_id).await.unwrap(),
            Some(tagged.clone())
        );
        assert_eq!(
            directory
                .known_generation(community_id, session_id)
                .await
                .unwrap(),
            Some(42)
        );
        assert!(directory
            .validate_fenced_header(community_id, &tagged.fenced_header())
            .await
            .is_ok());
        assert!(matches!(
            directory
                .validate_fenced_header(
                    community_id,
                    &FencedHeader {
                        session_id,
                        generation: 99,
                        owner_runtime_id: runtime(1),
                    },
                )
                .await,
            Err(MeshError::FutureGeneration {
                known_generation: 42,
                ..
            })
        ));
        assert!(matches!(
            directory
                .acquire(community_id, session_id, runtime(3), Profile::ReliableStream)
                .await
                .unwrap(),
            AcquireResult::Exists(ref lease) if *lease == tagged
        ));
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn acquire_conflict_renew_release_and_monotonic_takeover() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let first = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .expect("first acquire")
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("first acquire should win"),
        };
        assert_eq!(first.generation, 1);

        let conflict = directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .expect("conflict acquire");
        assert!(matches!(conflict, AcquireResult::Exists(ref lease) if *lease == first));
        assert_eq!(
            directory
                .known_generation(community_id, session_id)
                .await
                .unwrap(),
            Some(1)
        );

        assert!(matches!(
            directory.renew(&first).await.unwrap(),
            RenewResult::Renewed(_)
        ));
        assert!(matches!(
            directory.release(&first).await.unwrap(),
            ReleaseResult::Released(_)
        ));

        let second = match directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .expect("second acquire")
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("released lease should be acquirable"),
        };
        assert_eq!(second.generation, 2);

        assert!(matches!(
            directory.renew(&first).await.unwrap(),
            RenewResult::Lost { current: Some(ref lease), known_generation: Some(2) } if *lease == second
        ));
        assert!(matches!(
            directory.release(&first).await.unwrap(),
            ReleaseResult::NotOwner { current: Some(ref lease), known_generation: Some(2) } if *lease == second
        ));
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn takeover_after_ttl_expiry_increments_non_expiring_counter() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let first = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("first acquire should win"),
        };
        tokio::time::sleep(Duration::from_millis(220)).await;
        assert_eq!(
            directory.lookup(community_id, session_id).await.unwrap(),
            None
        );
        assert_eq!(
            directory
                .known_generation(community_id, session_id)
                .await
                .unwrap(),
            Some(first.generation)
        );

        let second = match directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("expired lease should be acquirable"),
        };
        assert!(second.generation > first.generation);
        assert_eq!(second.generation, 2);
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn validate_returns_typed_fence_rejections() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let first = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("first acquire should win"),
        };
        assert!(directory
            .validate_fenced_header(community_id, &first.fenced_header())
            .await
            .is_ok());

        assert!(matches!(
            directory
                .validate_fenced_header(
                    community_id,
                    &FencedHeader {
                        owner_runtime_id: runtime(2),
                        ..first.fenced_header()
                    },
                )
                .await,
            Err(MeshError::OwnerMismatch {
                generation: 1,
                frame_owner_runtime_id,
                current_owner_runtime_id,
                ..
            }) if frame_owner_runtime_id == runtime(2) && current_owner_runtime_id == runtime(1)
        ));

        assert!(matches!(
            directory
                .validate_fenced_header(
                    community_id,
                    &FencedHeader {
                        generation: first.generation + 1,
                        ..first.fenced_header()
                    },
                )
                .await,
            Err(MeshError::FutureGeneration {
                frame_generation: 2,
                known_generation: 1,
                ..
            })
        ));

        assert!(matches!(
            directory.release(&first).await.unwrap(),
            ReleaseResult::Released(_)
        ));
        let second = match directory
            .acquire(
                community_id,
                session_id,
                runtime(2),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("second acquire should win"),
        };
        assert!(matches!(
            directory
                .validate_fenced_header(community_id, &first.fenced_header())
                .await,
            Err(MeshError::StaleGeneration {
                frame_generation: 1,
                known_generation: 2,
                ..
            })
        ));
        assert!(directory
            .validate_fenced_header(community_id, &second.fenced_header())
            .await
            .is_ok());
    }

    #[tokio::test]
    #[ignore = "requires REDIS_URL; run by backend-integration"]
    async fn validate_returns_no_active_lease_after_expiry_before_takeover() {
        let directory = redis_directory().await;
        let community_id = community();
        let session_id = Uuid::new_v4();
        clear_keys(&directory, community_id, session_id).await;

        let lease = match directory
            .acquire(
                community_id,
                session_id,
                runtime(1),
                Profile::ReliableStream,
            )
            .await
            .unwrap()
        {
            AcquireResult::Acquired(lease) => lease,
            AcquireResult::Exists(_) => panic!("first acquire should win"),
        };

        tokio::time::sleep(Duration::from_millis(220)).await;
        assert_eq!(
            directory.lookup(community_id, session_id).await.unwrap(),
            None
        );
        assert!(matches!(
            directory
                .validate_fenced_header(community_id, &lease.fenced_header())
                .await,
            Err(MeshError::NoActiveLease {
                frame_generation: 1,
                known_generation: 1,
                frame_owner_runtime_id,
                ..
            }) if frame_owner_runtime_id == runtime(1)
        ));
    }
}
