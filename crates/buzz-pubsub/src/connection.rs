//! The relay's shared Redis command connection.
//!
//! Every non-pub/sub Redis command (PUBLISH, rate-limit scripts, replay
//! guards, presence, the session directory, the mesh registry) goes through
//! one auto-reconnecting multiplexed connection. Redis pipelines concurrent
//! commands on a single socket and executes them one at a time anyway, so a
//! pool of exclusively checked-out sockets only added a queue to wait in and a
//! PING round trip per checkout.

use std::time::Duration;

use crate::error::PubSubError;

/// Shared Redis command connection. Cloning is an `Arc` bump; every clone
/// uses the same socket and reconnects together.
pub type RedisConnection = redis::aio::ConnectionManager;

/// Bound on one connect attempt. A command issued while the connection is
/// down waits at most this long for it before failing.
pub const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Bound on one command's response (the redis 1.x driver default, pinned
/// here because callers budget against it).
pub const REDIS_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Build the shared connection without dialing; the first command connects.
///
/// There are no connect retries inside a command: a failed attempt is that
/// command's error, and the next command starts a fresh attempt. So no caller
/// waits longer than [`REDIS_CONNECT_TIMEOUT`] plus
/// [`REDIS_RESPONSE_TIMEOUT`] per command, during an outage or otherwise.
///
/// Must be called inside a Tokio runtime.
pub fn redis_connection(redis_url: &str) -> Result<RedisConnection, PubSubError> {
    let client = redis::Client::open(redis_url)?;
    let config = redis::aio::ConnectionManagerConfig::new()
        .set_connection_timeout(Some(REDIS_CONNECT_TIMEOUT))
        .set_response_timeout(Some(REDIS_RESPONSE_TIMEOUT))
        .set_number_of_retries(0);
    Ok(redis::aio::ConnectionManager::new_lazy_with_config(
        client, config,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_redis_fails_each_command_within_the_bounds() {
        // Port 1 refuses (or times out) on every CI runner; nothing listens.
        let conn = redis_connection("redis://127.0.0.1:1").expect("lazy build never dials");
        for _ in 0..2 {
            let started = std::time::Instant::now();
            let result = redis::cmd("PING")
                .query_async::<String>(&mut conn.clone())
                .await;
            assert!(result.is_err());
            assert!(
                started.elapsed()
                    < REDIS_CONNECT_TIMEOUT + REDIS_RESPONSE_TIMEOUT + Duration::from_millis(500),
                "command waited {:?}",
                started.elapsed()
            );
        }
    }
}
