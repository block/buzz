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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// Scheduling slack on top of a timeout before a test calls it unbounded.
    const SLACK: Duration = Duration::from_millis(400);
    /// The per-command bounds callers budget against (session-directory and
    /// audio permit budgets in buzz-relay). Literal on purpose: raising a
    /// timeout constant must fail these tests rather than move them with it.
    const PROMISED_CONNECT_BOUND: Duration = Duration::from_millis(1000);
    const PROMISED_RESPONSE_BOUND: Duration = Duration::from_millis(500);

    /// How a fake Redis peer treats each accepted socket.
    #[derive(Clone, Copy)]
    enum Peer {
        /// Accept, then never answer anything (stalls the connect handshake).
        Silent,
        /// Answer the handshake, never answer a command.
        WithholdReplies,
        /// Answer the handshake and one PING, then close the socket.
        OnePingThenClose,
    }

    /// Fake RESP peer on loopback. Returns its URL and an accept counter.
    async fn fake_redis(peer: Peer) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("redis://{}", listener.local_addr().expect("addr"));
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_ascii_uppercase();
                        let mut reply = String::new();
                        if !matches!(peer, Peer::Silent) {
                            // The handshake pipelines CLIENT SETINFO calls.
                            reply.push_str(&"+OK\r\n".repeat(chunk.matches("SETINFO").count()));
                        }
                        let pings = chunk.matches("PING").count();
                        if matches!(peer, Peer::OnePingThenClose) && pings > 0 {
                            reply.push_str("+PONG\r\n");
                            let _ = socket.write_all(reply.as_bytes()).await;
                            return; // dropping the socket closes it
                        }
                        if !reply.is_empty() && socket.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, accepted)
    }

    async fn timed_ping(conn: &RedisConnection) -> (redis::RedisResult<String>, Duration) {
        let started = Instant::now();
        let result = redis::cmd("PING")
            .query_async::<String>(&mut conn.clone())
            .await;
        (result, started.elapsed())
    }

    #[tokio::test]
    async fn stalled_handshake_fails_each_command_at_the_connect_timeout() {
        let (url, accepted) = fake_redis(Peer::Silent).await;
        let conn = redis_connection(&url).expect("lazy build never dials");
        for attempt in 1..=2 {
            let (result, elapsed) = timed_ping(&conn).await;
            assert!(result.is_err(), "attempt {attempt}: {result:?}");
            // The connect timeout fired (not an instant refusal), and no
            // in-command retry stacked a second attempt on top of it.
            assert!(
                elapsed >= REDIS_CONNECT_TIMEOUT - Duration::from_millis(50),
                "attempt {attempt} failed early: {elapsed:?}"
            );
            assert!(
                elapsed < PROMISED_CONNECT_BOUND + SLACK,
                "attempt {attempt} waited {elapsed:?}"
            );
        }
        assert_eq!(accepted.load(Ordering::SeqCst), 2, "one dial per command");
    }

    #[tokio::test]
    async fn withheld_reply_fails_the_command_at_the_response_timeout() {
        let (url, _) = fake_redis(Peer::WithholdReplies).await;
        let conn = redis_connection(&url).expect("lazy build never dials");
        let (result, elapsed) = timed_ping(&conn).await;
        let error = result.expect_err("no reply must fail the command");
        assert!(error.is_timeout(), "{error:?}");
        assert!(
            elapsed >= REDIS_RESPONSE_TIMEOUT - Duration::from_millis(50),
            "failed early: {elapsed:?}"
        );
        assert!(
            elapsed < PROMISED_RESPONSE_BOUND + SLACK,
            "waited {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn dropped_connection_is_redialed_by_a_later_command() {
        let (url, accepted) = fake_redis(Peer::OnePingThenClose).await;
        let conn = redis_connection(&url).expect("lazy build never dials");
        let (first, _) = timed_ping(&conn).await;
        assert_eq!(first.expect("first ping"), "PONG");
        // The peer closed the socket. Each later command is bounded, and the
        // shared connection redials instead of staying dead.
        let mut recovered = false;
        for _ in 0..5 {
            let (result, elapsed) = timed_ping(&conn).await;
            assert!(
                elapsed < PROMISED_CONNECT_BOUND + PROMISED_RESPONSE_BOUND + SLACK,
                "waited {elapsed:?}"
            );
            if result.is_ok() {
                recovered = true;
                break;
            }
        }
        assert!(recovered, "no later command reconnected");
        assert!(accepted.load(Ordering::SeqCst) >= 2, "never redialed");
    }

    #[tokio::test]
    async fn unreachable_redis_fails_each_command_within_the_bounds() {
        // Port 1 refuses (or times out) on every CI runner; nothing listens.
        let conn = redis_connection("redis://127.0.0.1:1").expect("lazy build never dials");
        for _ in 0..2 {
            let (result, elapsed) = timed_ping(&conn).await;
            assert!(result.is_err());
            assert!(
                elapsed < PROMISED_CONNECT_BOUND + PROMISED_RESPONSE_BOUND + SLACK,
                "command waited {elapsed:?}"
            );
        }
    }
}
