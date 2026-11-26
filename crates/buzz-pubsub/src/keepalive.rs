//! Dead-socket detection for Redis pub/sub connections.
//!
//! A pub/sub connection only reads, so a socket the network silently dropped
//! looks like a quiet channel forever. redis-rs puts no deadline on pub/sub
//! requests, so each subscriber bounds its setup and control requests here
//! and polls [`Keepalive::heartbeat`] next to its message stream. When either
//! fails, the subscriber returns, dropping its sink and stream (which closes
//! the socket), and its reconnect loop takes over.

use std::future::Future;
use std::io;
use std::time::Duration;

use redis::aio::{PubSubSink, PubSubStream};
use redis::{RedisError, RedisResult, Value};

/// Deadlines for one pub/sub connection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keepalive {
    /// Pause between PINGs.
    pub(crate) interval: Duration,
    /// Longest wait for a reply to any request, PING included.
    pub(crate) reply_deadline: Duration,
}

impl Keepalive {
    /// A dead socket is detected within `interval + reply_deadline` (20 s).
    pub(crate) const DEFAULT: Self = Self {
        interval: Duration::from_secs(15),
        reply_deadline: Duration::from_secs(5),
    };

    /// Open a pub/sub connection, bounding connect and handshake.
    pub(crate) async fn connect(&self, redis_url: &str) -> RedisResult<(PubSubSink, PubSubStream)> {
        let client = redis::Client::open(redis_url)?;
        Ok(self.bounded(client.get_async_pubsub()).await?.split())
    }

    /// Run a request, failing with `TimedOut` if no reply arrives in time.
    pub(crate) async fn bounded<T>(
        &self,
        request: impl Future<Output = RedisResult<T>>,
    ) -> RedisResult<T> {
        tokio::time::timeout(self.reply_deadline, request)
            .await
            .map_err(|_| {
                RedisError::from(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Redis pub/sub reply deadline elapsed",
                ))
            })?
    }

    /// PING forever; resolves with the first failure. Poll the same pinned
    /// future alongside the message stream, so messages keep flowing while a
    /// PING is pending and cannot reset its deadline.
    pub(crate) async fn heartbeat(self, mut sink: PubSubSink) -> RedisError {
        loop {
            tokio::time::sleep(self.interval).await;
            if let Err(e) = self.bounded(sink.ping::<Value>()).await {
                return e;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod fake_redis {
    //! A scripted RESP2 server for pub/sub tests: answers SUBSCRIBE, PSUBSCRIBE
    //! and PING like Redis, can withhold replies, and pushes messages on demand.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, Notify};

    use super::Keepalive;

    /// Short deadlines so tests finish quickly.
    pub(crate) const FAST: Keepalive = Keepalive {
        interval: Duration::from_millis(100),
        reply_deadline: Duration::from_millis(200),
    };

    pub(crate) struct FakeRedis {
        pub(crate) url: String,
        /// When set, PING gets no reply.
        pub(crate) hold_pong: Arc<AtomicBool>,
        /// When set, SUBSCRIBE/PSUBSCRIBE get no reply.
        pub(crate) hold_subscribe: Arc<AtomicBool>,
        /// Raw RESP frames to push to the current connection.
        pub(crate) push: mpsc::UnboundedSender<Vec<u8>>,
        /// Notified for every PING received.
        pub(crate) pinged: Arc<Notify>,
        /// Notified when a client connection closes.
        pub(crate) closed: Arc<Notify>,
    }

    pub(crate) fn message(channel: &str, payload: &str) -> Vec<u8> {
        resp(&["message", channel, payload])
    }

    fn resp(parts: &[&str]) -> Vec<u8> {
        let mut out = format!("*{}\r\n", parts.len());
        for p in parts {
            out.push_str(&format!("${}\r\n{p}\r\n", p.len()));
        }
        out.into_bytes()
    }

    impl FakeRedis {
        pub(crate) async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("redis://{}", listener.local_addr().unwrap());
            let (push, push_rx) = mpsc::unbounded_channel();
            let fake = Self {
                url,
                hold_pong: Arc::default(),
                hold_subscribe: Arc::default(),
                push,
                pinged: Arc::default(),
                closed: Arc::default(),
            };
            let state = (
                fake.hold_pong.clone(),
                fake.hold_subscribe.clone(),
                fake.pinged.clone(),
                fake.closed.clone(),
            );
            tokio::spawn(async move {
                let push_rx = Arc::new(tokio::sync::Mutex::new(push_rx));
                loop {
                    let (sock, _) = listener.accept().await.unwrap();
                    let state = state.clone();
                    let push_rx = push_rx.clone();
                    tokio::spawn(async move {
                        serve(sock, state.0, state.1, state.2, push_rx).await;
                        state.3.notify_one();
                    });
                }
            });
            fake
        }
    }

    async fn serve(
        sock: TcpStream,
        hold_pong: Arc<AtomicBool>,
        hold_subscribe: Arc<AtomicBool>,
        pinged: Arc<Notify>,
        push_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>>,
    ) {
        let (mut rd, mut wr) = sock.into_split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer = tokio::spawn(async move {
            while let Some(bytes) = out_rx.recv().await {
                if wr.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });
        let pusher = {
            let out_tx = out_tx.clone();
            tokio::spawn(async move {
                let mut rx = push_rx.lock().await;
                while let Some(bytes) = rx.recv().await {
                    let _ = out_tx.send(bytes);
                }
            })
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            while let Some((cmd, used)) = parse_command(&buf) {
                buf.drain(..used);
                let name = cmd[0].to_ascii_uppercase();
                let reply = match name.as_str() {
                    "PING" => {
                        pinged.notify_one();
                        (!hold_pong.load(Ordering::SeqCst)).then(|| resp(&["pong", ""]))
                    }
                    "SUBSCRIBE" | "PSUBSCRIBE" => {
                        (!hold_subscribe.load(Ordering::SeqCst)).then(|| {
                            let kind = name.to_ascii_lowercase();
                            let mut out = Vec::new();
                            for (i, ch) in cmd[1..].iter().enumerate() {
                                out.extend(
                                    format!(
                                        "*3\r\n${}\r\n{kind}\r\n${}\r\n{ch}\r\n:{}\r\n",
                                        kind.len(),
                                        ch.len(),
                                        i + 1
                                    )
                                    .into_bytes(),
                                );
                            }
                            out
                        })
                    }
                    _ => Some(b"+OK\r\n".to_vec()),
                };
                if let Some(reply) = reply {
                    let _ = out_tx.send(reply);
                }
            }
            match rd.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        pusher.abort();
        writer.abort();
    }

    /// Parse one RESP array of bulk strings; returns the parts and bytes used.
    fn parse_command(buf: &[u8]) -> Option<(Vec<String>, usize)> {
        fn line(buf: &[u8], at: usize) -> Option<(&str, usize)> {
            let end = buf[at..].windows(2).position(|w| w == b"\r\n")? + at;
            Some((std::str::from_utf8(&buf[at..end]).ok()?, end + 2))
        }
        let (head, mut at) = line(buf, 0)?;
        let count: usize = head.strip_prefix('*')?.parse().ok()?;
        let mut parts = Vec::with_capacity(count);
        for _ in 0..count {
            let (len, next) = line(buf, at)?;
            let len: usize = len.strip_prefix('$')?.parse().ok()?;
            if buf.len() < next + len + 2 {
                return None;
            }
            parts.push(String::from_utf8_lossy(&buf[next..next + len]).into_owned());
            at = next + len + 2;
        }
        Some((parts, at))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::fake_redis::{FakeRedis, FAST};

    #[tokio::test]
    async fn heartbeat_fails_when_pong_withheld() {
        let fake = FakeRedis::start().await;
        fake.hold_pong.store(true, Ordering::SeqCst);
        let (sink, _stream) = FAST.connect(&fake.url).await.unwrap();
        let err = tokio::time::timeout(Duration::from_secs(2), FAST.heartbeat(sink))
            .await
            .expect("heartbeat must fail within interval + reply deadline");
        assert_eq!(err.kind(), redis::ErrorKind::Io);
    }

    #[tokio::test]
    async fn heartbeat_keeps_running_while_pongs_arrive() {
        let fake = FakeRedis::start().await;
        let (sink, _stream) = FAST.connect(&fake.url).await.unwrap();
        let outcome = tokio::time::timeout(Duration::from_millis(800), FAST.heartbeat(sink)).await;
        assert!(
            outcome.is_err(),
            "heartbeat failed on a live socket: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn bounded_fails_stalled_subscribe() {
        let fake = FakeRedis::start().await;
        fake.hold_subscribe.store(true, Ordering::SeqCst);
        let (mut sink, _stream) = FAST.connect(&fake.url).await.unwrap();
        let err = FAST.bounded(sink.subscribe("ch")).await.unwrap_err();
        assert_eq!(err.kind(), redis::ErrorKind::Io);
    }
}
