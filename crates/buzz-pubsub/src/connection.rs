//! The relay's shared Redis command connection.
//!
//! Every non-pub/sub Redis command (PUBLISH, rate-limit scripts, replay
//! guards, presence, the session directory, the mesh registry) goes through
//! one multiplexed connection. Redis pipelines concurrent commands on a single
//! socket and executes them one at a time anyway, so a pool of exclusively
//! checked-out sockets only added a queue to wait in and a PING round trip per
//! checkout.
//!
//! Recovery lives here rather than in redis-rs's `ConnectionManager`, which
//! keeps a socket that stops answering and caches a failed AUTH handshake
//! until the process restarts. This connection retires its socket on any
//! setup failure, response timeout or dropped connection, and the next command
//! dials a fresh one. A failed command is never replayed: a timed-out mutation
//! may already have run.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::{AsyncConnectionConfig, Client, Cmd, Pipeline, RedisFuture, RedisResult, Value};

use crate::error::PubSubError;

/// Bound on one connect attempt, handshake included. A command issued while
/// the connection is down waits at most this long for it before failing.
pub const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Bound on one command's response (the redis 1.x driver default, pinned
/// here because callers budget against it).
pub const REDIS_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Shared Redis command connection. Cloning is an `Arc` bump; every clone
/// sends on the same socket and moves to its replacement together.
#[derive(Clone)]
pub struct RedisConnection(Arc<Inner>);

struct Inner {
    client: Client,
    config: AsyncConnectionConfig,
    /// The current socket's dial. Every command shares it; a failed or
    /// retired dial is swapped for a fresh one.
    socket: ArcSwap<Dial>,
}

/// One dial of the shared socket, awaited by every command that arrives while
/// it runs. Lazy: nothing connects until a command polls it.
type Dial = Shared<BoxFuture<'static, RedisResult<MultiplexedConnection>>>;

fn dial(client: &Client, config: &AsyncConnectionConfig) -> Arc<Dial> {
    let (client, config) = (client.clone(), config.clone());
    let dial = async move {
        client
            .get_multiplexed_async_connection_with_config(&config)
            .await
    };
    Arc::new(dial.boxed().shared())
}

/// Build the shared connection without dialing; the first command connects.
///
/// There are no retries inside a command: a failed connect or command is that
/// command's error, and the next command dials afresh if the socket was
/// retired. So no caller waits longer than [`REDIS_CONNECT_TIMEOUT`] plus
/// [`REDIS_RESPONSE_TIMEOUT`] per command, during an outage or otherwise.
pub fn redis_connection(redis_url: &str) -> Result<RedisConnection, PubSubError> {
    let client = Client::open(redis_url)?;
    let config = AsyncConnectionConfig::new()
        .set_connection_timeout(Some(REDIS_CONNECT_TIMEOUT))
        .set_response_timeout(Some(REDIS_RESPONSE_TIMEOUT));
    let socket = ArcSwap::new(dial(&client, &config));
    Ok(RedisConnection(Arc::new(Inner {
        client,
        config,
        socket,
    })))
}

impl RedisConnection {
    async fn socket(&self) -> RedisResult<(Arc<Dial>, MultiplexedConnection)> {
        let current = self.0.socket.load_full();
        match (*current).clone().await {
            Ok(socket) => Ok((current, socket)),
            Err(err) => {
                self.retire(&current);
                Err(err)
            }
        }
    }

    /// Replace `current` unless another command already did, so one failure
    /// wave costs one dial. The retired socket closes once the commands still
    /// waiting on it finish or time out, which frees their reply slots.
    fn retire(&self, current: &Arc<Dial>) {
        self.0
            .socket
            .compare_and_swap(current, dial(&self.0.client, &self.0.config));
    }

    fn settle<T>(&self, current: &Arc<Dial>, result: RedisResult<T>) -> RedisResult<T> {
        if let Err(err) = &result {
            // A timeout can't tell a slow server from a socket that will never
            // answer again, so it retires the socket too: on a healthy server
            // that costs one handshake.
            if err.is_timeout() || err.is_unrecoverable_error() {
                self.retire(current);
            }
        }
        result
    }
}

impl ConnectionLike for RedisConnection {
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, Value> {
        async move {
            let (current, mut socket) = self.socket().await?;
            let result = socket.send_packed_command(cmd).await;
            self.settle(&current, result)
        }
        .boxed()
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        cmd: &'a Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<Value>> {
        async move {
            let (current, mut socket) = self.socket().await?;
            let result = socket.send_packed_commands(cmd, offset, count).await;
            self.settle(&current, result)
        }
        .boxed()
    }

    fn get_db(&self) -> i64 {
        self.0.client.get_connection_info().redis_settings().db()
    }
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

    /// How a fake Redis peer treats one accepted socket.
    #[derive(Clone, Copy)]
    enum Peer {
        /// Accept, then never answer anything (stalls the connect handshake).
        Silent,
        /// Answer the handshake, never answer a command.
        WithholdReplies,
        /// Answer the handshake and one PING, then close the socket.
        OnePingThenClose,
        /// Answer the handshake and one PING, then stay open and silent.
        OnePingThenSilent,
        /// Reject AUTH in the handshake.
        RejectAuth,
        /// Answer everything.
        Healthy,
    }

    /// Fake RESP peer on loopback. `script(n)` picks how the n-th accepted
    /// socket (from 0) behaves. Returns its URL and an accept counter.
    async fn fake_redis(script: fn(usize) -> Peer) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("redis://{}", listener.local_addr().expect("addr"));
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let peer = script(counter.fetch_add(1, Ordering::SeqCst));
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let mut answered_ping = false;
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        if matches!(peer, Peer::Silent) {
                            continue;
                        }
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_ascii_uppercase();
                        // The handshake pipelines AUTH (with a password) and
                        // CLIENT SETINFO calls, in that order.
                        let auth = if matches!(peer, Peer::RejectAuth) {
                            "-WRONGPASS invalid username-password pair\r\n"
                        } else {
                            "+OK\r\n"
                        };
                        let mut reply = auth.repeat(chunk.matches("AUTH").count());
                        reply.push_str(&"+OK\r\n".repeat(chunk.matches("SETINFO").count()));
                        let pings = chunk.matches("PING").count();
                        match peer {
                            Peer::Healthy => reply.push_str(&"+PONG\r\n".repeat(pings)),
                            Peer::OnePingThenClose if pings > 0 => {
                                reply.push_str("+PONG\r\n");
                                let _ = socket.write_all(reply.as_bytes()).await;
                                return; // dropping the socket closes it
                            }
                            Peer::OnePingThenSilent if pings > 0 && !answered_ping => {
                                reply.push_str("+PONG\r\n");
                                answered_ping = true;
                            }
                            _ => {}
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
        let (url, accepted) = fake_redis(|_| Peer::Silent).await;
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
        let (url, _) = fake_redis(|_| Peer::WithholdReplies).await;
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
        let (url, accepted) = fake_redis(|_| Peer::OnePingThenClose).await;
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
    async fn silent_socket_is_replaced_for_every_clone() {
        // The first socket answers one PING, then stays open but never answers
        // again (a dropped NAT entry, a failed-over endpoint). New sockets work.
        let (url, accepted) = fake_redis(|n| {
            if n == 0 {
                Peer::OnePingThenSilent
            } else {
                Peer::Healthy
            }
        })
        .await;
        let conn = redis_connection(&url).expect("lazy build never dials");
        let existing_clone = conn.clone();
        let (first, _) = timed_ping(&conn).await;
        assert_eq!(first.expect("first ping"), "PONG");
        let (stalled, elapsed) = timed_ping(&conn).await;
        let error = stalled.expect_err("the silent socket must time out");
        assert!(error.is_timeout(), "{error:?}");
        assert!(
            elapsed < PROMISED_RESPONSE_BOUND + SLACK,
            "waited {elapsed:?}"
        );
        // The timeout retired the socket for every clone: the next command,
        // through a handle cloned before the stall, dials once and succeeds.
        let (recovered, _) = timed_ping(&existing_clone).await;
        assert_eq!(recovered.expect("ping on a fresh socket"), "PONG");
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_auth_handshake_is_retried_by_a_later_command() {
        // The serving socket drops, the reconnect's AUTH is rejected (a
        // credential rotation caught mid-way), then the server accepts again.
        let (url, accepted) = fake_redis(|n| match n {
            0 => Peer::OnePingThenClose,
            1 => Peer::RejectAuth,
            _ => Peer::Healthy,
        })
        .await;
        let url = url.replace("redis://", "redis://:pw@");
        let conn = redis_connection(&url).expect("lazy build never dials");
        let (first, _) = timed_ping(&conn).await;
        assert_eq!(first.expect("first ping"), "PONG");
        let mut errors = Vec::new();
        let mut recovered = false;
        for _ in 0..5 {
            match timed_ping(&conn).await.0 {
                Ok(pong) => {
                    assert_eq!(pong, "PONG");
                    recovered = true;
                    break;
                }
                Err(error) => errors.push(error),
            }
        }
        assert!(
            errors
                .iter()
                .any(|e| e.kind() == redis::ErrorKind::AuthenticationFailed),
            "the rejected handshake never surfaced: {errors:?}"
        );
        // The AUTH failure was not cached: the same handle redialed past it.
        assert!(recovered, "never recovered: {errors:?}");
        assert_eq!(accepted.load(Ordering::SeqCst), 3, "{errors:?}");
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
