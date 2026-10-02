//! Redis-backed NIP-FI command replay seen-set.
//!
//! Implements [`CommandReplayGuard`] from `buzz-auth` with the same
//! `SET NX EX` shape and TTL clamp as [`crate::RedisNip98ReplayGuard`].

use buzz_auth::{
    command_replay_key, error::AuthError, CommandReplayGuard, DEFAULT_REPLAY_TTL_SECS,
    MAX_REPLAY_TTL_SECS,
};

/// Redis-backed NIP-FI command replay seen-set.
///
/// `try_claim` issues `SET buzz:nip-fi:command:{hash} 1 NX EX <ttl>`; `OK`
/// is a first claim and `nil` is a replay.
pub struct RedisCommandReplayGuard {
    pool: deadpool_redis::Pool,
}

impl RedisCommandReplayGuard {
    /// Create a new replay guard backed by the given Redis connection pool.
    pub fn new(pool: deadpool_redis::Pool) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<deadpool_redis::Connection, AuthError> {
        self.pool.get().await.map_err(|e| {
            tracing::warn!(error = %e, "nip-fi command replay: redis pool acquire failed");
            AuthError::Internal(format!("Redis pool: {e}"))
        })
    }
}

impl CommandReplayGuard for RedisCommandReplayGuard {
    fn try_claim<'a>(
        &'a self,
        issuer: &'a str,
        jti: &'a str,
        ttl_secs: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, AuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            let ttl = ttl_secs.clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS);
            let mut conn = self.conn().await?;
            let result: Option<String> = redis::cmd("SET")
                .arg(command_replay_key(issuer, jti))
                .arg("1")
                .arg("NX")
                .arg("EX")
                .arg(ttl)
                .query_async(&mut *conn)
                .await
                .map_err(|e| {
                    tracing::warn!(error = %e, "nip-fi command replay: redis SET NX EX failed");
                    AuthError::Internal(format!("Redis SET NX EX: {e}"))
                })?;
            match result.as_deref() {
                Some("OK") => Ok(true),
                None => Ok(false),
                Some(other) => Err(AuthError::Internal(format!(
                    "unexpected SET NX EX reply: {other}"
                ))),
            }
        })
    }
}
