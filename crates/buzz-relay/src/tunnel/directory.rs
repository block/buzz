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

const ACQUIRE_SCRIPT: &str = r#"
local lease_key = KEYS[1]
local generation_key = KEYS[2]
local owner = ARGV[1]
local profile = ARGV[2]
local ttl_ms = tonumber(ARGV[3])

local current = redis.call('GET', lease_key)
if current then
    return {'exists', current, redis.call('GET', generation_key) or ''}
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

local current = redis.call('GET', lease_key) or ''
local known_generation = redis.call('GET', generation_key) or ''
return {current, known_generation}
"#;

/// Redis-backed owner directory for mesh tunnel sessions.
#[derive(Clone)]
pub struct SessionDirectory {
    pool: deadpool_redis::Pool,
    lease_ttl: Duration,
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
        Self::with_lease_ttl(pool, DEFAULT_LEASE_TTL)
    }

    /// Create a directory backed by `pool` with an explicit lease TTL.
    pub fn with_lease_ttl(pool: deadpool_redis::Pool, lease_ttl: Duration) -> Self {
        Self { pool, lease_ttl }
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
        let (status, value, _known_generation): (String, String, String) =
            Script::new(ACQUIRE_SCRIPT)
                .key(&keys.lease)
                .key(&keys.generation)
                .arg(owner_runtime_id.to_hex())
                .arg(profile.as_wire_str())
                .arg(ttl_ms)
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
            .key(&keys.lease)
            .key(&keys.generation)
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
                .key(&keys.lease)
                .key(&keys.generation)
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
        let value: Option<String> = redis::cmd("GET")
            .arg(&keys.lease)
            .query_async(&mut *conn)
            .await?;
        value
            .as_deref()
            .map(|v| parse_lease(community_id, session_id, v))
            .transpose()
    }

    /// Read the non-expiring generation counter for a session, if it exists.
    pub async fn known_generation(
        &self,
        community_id: CommunityId,
        session_id: Uuid,
    ) -> Result<Option<u64>, DirectoryError> {
        let keys = SessionKeys::new(community_id, session_id);
        let mut conn = self.pool.get().await?;
        let value: Option<String> = redis::cmd("GET")
            .arg(&keys.generation)
            .query_async(&mut *conn)
            .await?;
        match value.as_deref() {
            Some(value) => parse_optional_generation(community_id, session_id, value),
            None => Ok(None),
        }
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
        let (lease_value, known_generation): (String, String) = Script::new(VALIDATE_SCRIPT)
            .key(&keys.lease)
            .key(&keys.generation)
            .invoke_async(&mut *conn)
            .await?;
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
}

impl SessionKeys {
    fn new(community_id: CommunityId, session_id: Uuid) -> Self {
        let base = format!("buzz:{}:tunnel:{}", community_id, session_id);
        Self {
            lease: format!("{base}:lease"),
            generation: format!("{base}:generation"),
        }
    }
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

    async fn redis_directory_if_available() -> Option<SessionDirectory> {
        let pool = pool();
        let mut conn = pool.get().await.ok()?;
        redis::cmd("PING")
            .query_async::<String>(&mut *conn)
            .await
            .ok()?;
        Some(SessionDirectory::with_lease_ttl(
            pool,
            Duration::from_millis(150),
        ))
    }

    async fn clear_keys(directory: &SessionDirectory, community_id: CommunityId, session_id: Uuid) {
        let keys = SessionKeys::new(community_id, session_id);
        let mut conn = directory.pool.get().await.expect("redis conn");
        let _: () = redis::cmd("DEL")
            .arg(keys.lease)
            .arg(keys.generation)
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
            format!("buzz:{}:tunnel:{}:lease", community(), session())
        );
        assert_eq!(
            keys.generation,
            format!("buzz:{}:tunnel:{}:generation", community(), session())
        );
        assert_ne!(keys.lease, keys.generation);
    }

    #[tokio::test]
    async fn acquire_conflict_renew_release_and_monotonic_takeover() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
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
    async fn takeover_after_ttl_expiry_increments_non_expiring_counter() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
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
    async fn validate_returns_typed_fence_rejections() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
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
    async fn validate_returns_no_active_lease_after_expiry_before_takeover() {
        let Some(directory) = redis_directory_if_available().await else {
            return;
        };
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
