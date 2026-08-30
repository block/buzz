//! Optional local sink for accepted inbound Buzz events.
//!
//! The sink is disabled unless an absolute Unix socket path is configured. It
//! carries only public event/provenance data and never receives relay or agent
//! credentials.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

const FRAME_VERSION: u8 = 1;
const ACK: u8 = 0x06;
const MAX_FRAME_BYTES: usize = 512 * 1024;

#[derive(Debug, Error)]
pub enum EventSinkError {
    #[error("event sink socket path must be absolute")]
    RelativePath,
    #[error("event sink socket path exceeds the platform bound")]
    PathTooLong,
    #[cfg(not(unix))]
    #[error("event sink is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("event sink frame exceeds the bounded size")]
    FrameTooLarge,
    #[error("event sink timeout must be between 1ms and 5000ms")]
    InvalidTimeout,
    #[error("event sink timed out")]
    Timeout,
    #[error("event sink I/O failed")]
    Io,
    #[error("event sink refused the frame")]
    Refused,
    #[error("event sink frame serialization failed")]
    Serialization,
}

#[derive(Clone, Debug)]
pub struct EventSink {
    socket_path: PathBuf,
    relay_origin: String,
    timeout: Duration,
}

#[derive(Serialize)]
struct EventSinkFrame<'a> {
    version: u8,
    relay_origin: &'a str,
    channel_id: Uuid,
    event_b64: String,
}

impl EventSink {
    /// Build a local event sink. The path must be absolute so the destination
    /// cannot drift with the harness working directory.
    pub fn new(
        socket_path: PathBuf,
        relay_origin: String,
        timeout: Duration,
    ) -> Result<Self, EventSinkError> {
        if !socket_path.is_absolute() {
            return Err(EventSinkError::RelativePath);
        }
        if socket_path.as_os_str().as_encoded_bytes().len() > 100 {
            return Err(EventSinkError::PathTooLong);
        }
        if timeout.is_zero() || timeout > Duration::from_secs(5) {
            return Err(EventSinkError::InvalidTimeout);
        }
        #[cfg(not(unix))]
        {
            let _ = (socket_path, relay_origin, timeout);
            return Err(EventSinkError::UnsupportedPlatform);
        }
        #[cfg(unix)]
        Ok(Self {
            socket_path,
            relay_origin,
            timeout,
        })
    }

    /// Deliver one event and wait for a one-byte acknowledgement. Every phase
    /// shares the same deadline, bounding connection, write, and response time.
    #[cfg(unix)]
    pub async fn deliver(
        &self,
        channel_id: Uuid,
        raw_event_json: &[u8],
    ) -> Result<(), EventSinkError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;

        let frame = EventSinkFrame {
            version: FRAME_VERSION,
            relay_origin: &self.relay_origin,
            channel_id,
            event_b64: base64::engine::general_purpose::STANDARD.encode(raw_event_json),
        };
        let payload = serde_json::to_vec(&frame).map_err(|_| EventSinkError::Serialization)?;
        if payload.len() > MAX_FRAME_BYTES || payload.len() > u32::MAX as usize {
            return Err(EventSinkError::FrameTooLarge);
        }
        let socket_path = self.socket_path.clone();
        let timeout = self.timeout;
        tokio::time::timeout(timeout, async move {
            let mut stream = UnixStream::connect(socket_path)
                .await
                .map_err(|_| EventSinkError::Io)?;
            stream
                .write_all(&(payload.len() as u32).to_be_bytes())
                .await
                .map_err(|_| EventSinkError::Io)?;
            stream
                .write_all(&payload)
                .await
                .map_err(|_| EventSinkError::Io)?;
            stream.flush().await.map_err(|_| EventSinkError::Io)?;
            let mut ack = [0u8; 1];
            stream
                .read_exact(&mut ack)
                .await
                .map_err(|_| EventSinkError::Io)?;
            if ack[0] != ACK {
                return Err(EventSinkError::Refused);
            }
            Ok(())
        })
        .await
        .map_err(|_| EventSinkError::Timeout)?
    }

    /// Expose the configured socket path for diagnostics without exposing any
    /// credentials (none are retained by this type).
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    fn socket_path(label: &str) -> PathBuf {
        let suffix = Uuid::new_v4().simple().to_string();
        PathBuf::from(format!("/tmp/bz-{label}-{}.sock", &suffix[..8]))
    }

    #[test]
    fn relative_socket_path_refuses() {
        let result = EventSink::new(
            PathBuf::from("relative.sock"),
            "wss://relay.example.test".into(),
            Duration::from_millis(50),
        );
        assert!(matches!(result, Err(EventSinkError::RelativePath)));
    }

    #[test]
    fn zero_and_excessive_timeouts_refuse() {
        for timeout in [Duration::ZERO, Duration::from_millis(5_001)] {
            let result = EventSink::new(
                PathBuf::from("/tmp/bz-timeout.sock"),
                "wss://relay.example.test".into(),
                timeout,
            );
            assert!(matches!(result, Err(EventSinkError::InvalidTimeout)));
        }
    }

    #[tokio::test]
    async fn exact_event_bytes_and_public_provenance_are_delivered() {
        let path = socket_path("exact");
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept test connection");
            let mut size = [0u8; 4];
            stream.read_exact(&mut size).await.expect("read size");
            let mut payload = vec![0u8; u32::from_be_bytes(size) as usize];
            stream.read_exact(&mut payload).await.expect("read payload");
            stream.write_all(&[ACK]).await.expect("write ack");
            payload
        });
        let sink = EventSink::new(
            path.clone(),
            "wss://relay.example.test".into(),
            Duration::from_secs(1),
        )
        .expect("valid sink");
        let raw = br#"{ "content" : "exact bytes" }"#;
        let channel_id = Uuid::new_v4();
        sink.deliver(channel_id, raw).await.expect("deliver frame");
        let payload = server.await.expect("server task");
        let frame: serde_json::Value = serde_json::from_slice(&payload).expect("parse frame");
        let keys = frame
            .as_object()
            .expect("frame object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            ["channel_id", "event_b64", "relay_origin", "version"]
                .into_iter()
                .collect()
        );
        assert_eq!(frame["version"], FRAME_VERSION);
        assert_eq!(frame["relay_origin"], "wss://relay.example.test");
        assert_eq!(frame["channel_id"], channel_id.to_string());
        let restored = base64::engine::general_purpose::STANDARD
            .decode(frame["event_b64"].as_str().expect("event bytes"))
            .expect("decode event");
        assert_eq!(restored, raw);
        assert!(!payload.windows(11).any(|w| w == b"private_key"));
        std::fs::remove_file(path).expect("remove socket");
    }

    #[tokio::test]
    async fn backpressure_without_acknowledgement_is_bounded() {
        let path = socket_path("timeout");
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept test connection");
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let sink = EventSink::new(
            path.clone(),
            "wss://relay.example.test".into(),
            Duration::from_millis(25),
        )
        .expect("valid sink");
        let result = sink.deliver(Uuid::new_v4(), b"{}").await;
        assert!(matches!(result, Err(EventSinkError::Timeout)));
        server.abort();
        std::fs::remove_file(path).expect("remove socket");
    }

    #[tokio::test]
    async fn wrong_ack_refuses() {
        let path = socket_path("refuse");
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept test connection");
            let mut size = [0u8; 4];
            stream.read_exact(&mut size).await.expect("read size");
            let mut payload = vec![0u8; u32::from_be_bytes(size) as usize];
            stream.read_exact(&mut payload).await.expect("read payload");
            stream.write_all(&[0x15]).await.expect("write refusal");
        });
        let sink = EventSink::new(
            path.clone(),
            "wss://relay.example.test".into(),
            Duration::from_secs(1),
        )
        .expect("valid sink");
        let result = sink.deliver(Uuid::new_v4(), b"{}").await;
        assert!(matches!(result, Err(EventSinkError::Refused)));
        server.await.expect("server task");
        std::fs::remove_file(path).expect("remove socket");
    }
}
