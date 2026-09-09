//! Bounded OpenAI Realtime WebSocket transport. One connection belongs to one
//! ACP session; this does not own MCP execution or audio playback.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

use crate::AgentError;

const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_AUDIO_BYTES: usize = 24_000 * 2; // One second of mono PCM16.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Persistent, single-owner provider connection. No implicit reconnect or replay:
/// a failed transport must not duplicate committed inputs or tool effects.
pub struct RealtimeConnection {
    sender: RealtimeSender,
    receiver: RealtimeReceiver,
    session: Value,
}

/// Write half. Keep this in the session actor so capture/cancel/tool events do
/// not wait for a provider read; reads run independently in the receive task.
pub struct RealtimeSender {
    socket: SplitSink<Socket, Message>,
    failed: bool,
}

/// Read half. A bounded caller-owned channel supplies backpressure to the
/// session actor. Dropping a pending read is terminal, not implicit reconnect.
pub struct RealtimeReceiver {
    socket: SplitStream<Socket>,
    failed: bool,
}

fn error(message: &str) -> AgentError {
    AgentError::Llm(format!("realtime: {message}"))
}

impl RealtimeConnection {
    /// Connect to an explicit ws(s) Realtime URL and await session readiness.
    /// Plain WS is restricted to loopback; credentials must not cross cleartext networks.
    pub async fn connect(endpoint: &str, token: &str) -> Result<Self, AgentError> {
        let url = url::Url::parse(endpoint).map_err(|_| error("invalid endpoint URL"))?;
        let loopback = match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !(url.scheme() == "wss" || url.scheme() == "ws" && loopback)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(error(
                "use wss (or loopback ws) without URL credentials or fragment",
            ));
        }
        let mut request = endpoint
            .into_client_request()
            .map_err(|_| error("invalid websocket request"))?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| error("invalid authorization header"))?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_EVENT_BYTES))
            .max_frame_size(Some(MAX_EVENT_BYTES))
            .max_write_buffer_size(MAX_EVENT_BYTES * 2);
        let (socket, _) = tokio::time::timeout(
            IO_TIMEOUT,
            connect_async_with_config(request, Some(config), false),
        )
        .await
        .map_err(|_| error("connect timed out"))?
        .map_err(|_| error("websocket handshake failed"))?;
        let (write, read) = socket.split();
        let mut connection = Self {
            sender: RealtimeSender {
                socket: write,
                failed: false,
            },
            receiver: RealtimeReceiver {
                socket: read,
                failed: false,
            },
            session: Value::Null,
        };
        let event = tokio::time::timeout(IO_TIMEOUT, connection.receiver.next_event())
            .await
            .map_err(|_| error("session readiness timed out"))??;
        if event["type"] != "session.created" {
            return Err(error("expected session.created"));
        }
        connection.session = event["session"].clone();
        Ok(connection)
    }

    /// Provider session configuration returned at readiness.
    pub fn session(&self) -> &Value {
        &self.session
    }

    /// Transfer the two halves to the session's independent write/read owners.
    pub fn split(self) -> (RealtimeSender, RealtimeReceiver) {
        (self.sender, self.receiver)
    }
}

impl RealtimeSender {
    /// Send a GA session update. Reject unsupported settings at the server,
    /// rather than translating them into model-specific wire fields.
    pub async fn update_session(&mut self, session: Value) -> Result<(), AgentError> {
        if session["type"] != "realtime" {
            return Err(error("session.type must be realtime"));
        }
        self.send(json!({"type":"session.update", "session":session}))
            .await
    }

    /// Append at most one second of 24 kHz mono signed little-endian PCM16.
    /// The caller must configure the matching audio/pcm session format first.
    pub async fn append_audio(&mut self, pcm: &[u8]) -> Result<(), AgentError> {
        if pcm.is_empty() || pcm.len() > MAX_AUDIO_BYTES || !pcm.len().is_multiple_of(2) {
            return Err(error("invalid PCM16 chunk length"));
        }
        self.send(json!({"type":"input_audio_buffer.append", "audio":STANDARD.encode(pcm)}))
            .await
    }

    /// Commit buffered input without implicitly requesting a response.
    pub async fn commit_audio(&mut self) -> Result<(), AgentError> {
        self.send(json!({"type":"input_audio_buffer.commit"})).await
    }

    /// Discard uncommitted capture.
    pub async fn clear_audio(&mut self) -> Result<(), AgentError> {
        self.send(json!({"type":"input_audio_buffer.clear"})).await
    }

    /// Add a standard conversation item, including function_call_output.
    /// Tool authorization and execution remain the ACP/MCP caller's job.
    pub async fn create_item(&mut self, item: Value) -> Result<(), AgentError> {
        self.send(json!({"type":"conversation.item.create", "item":item}))
            .await
    }

    /// Request output using the current session configuration.
    pub async fn create_response(&mut self) -> Result<(), AgentError> {
        self.send(json!({"type":"response.create"})).await
    }

    /// Cancel a named response. This is not tool cancellation or playback truncation.
    pub async fn cancel_response(&mut self, response_id: &str) -> Result<(), AgentError> {
        self.send(json!({"type":"response.cancel", "response_id":response_id}))
            .await
    }

    /// Reconcile history with the number of audio milliseconds actually rendered.
    pub async fn truncate_audio(
        &mut self,
        item_id: &str,
        content_index: u32,
        audio_end_ms: u32,
    ) -> Result<(), AgentError> {
        self.send(
            json!({"type":"conversation.item.truncate", "item_id":item_id,
            "content_index":content_index, "audio_end_ms":audio_end_ms}),
        )
        .await
    }

    async fn send(&mut self, event: Value) -> Result<(), AgentError> {
        if self.failed {
            return Err(error("connection is terminal"));
        }
        let text = serde_json::to_string(&event).map_err(|_| error("cannot encode event"))?;
        if text.len() > MAX_EVENT_BYTES {
            return Err(error("outgoing event exceeds byte limit"));
        }
        // Cancellation during a partial write leaves the connection terminal too.
        self.failed = true;
        tokio::time::timeout(IO_TIMEOUT, self.socket.send(Message::Text(text.into())))
            .await
            .map_err(|_| error("write timed out"))?
            .map_err(|_| error("websocket write failed"))?;
        self.failed = false;
        Ok(())
    }

    /// Close explicitly. Drop both halves to release the socket without reconnecting.
    pub async fn close(mut self) -> Result<(), AgentError> {
        tokio::time::timeout(IO_TIMEOUT, self.socket.close())
            .await
            .map_err(|_| error("close timed out"))?
            .map_err(|_| error("websocket close failed"))
    }
}

impl RealtimeReceiver {
    /// Read one bounded JSON event. Unknown event types remain available to the
    /// session driver; errors propagate, and binary audio is not this protocol.
    /// The owner sets idle/session deadlines and must not log raw events.
    pub async fn next_event(&mut self) -> Result<Value, AgentError> {
        if self.failed {
            return Err(error("connection is terminal"));
        }
        // A dropped read future cannot silently resume a partially consumed event.
        self.failed = true;
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or_else(|| error("websocket closed"))?
                .map_err(|_| error("websocket read failed"))?;
            match message {
                Message::Text(text) => {
                    let event: Value =
                        serde_json::from_str(&text).map_err(|_| error("invalid JSON event"))?;
                    if event["type"].as_str().is_none() {
                        return Err(error("event type missing"));
                    }
                    if event["type"] == "error" {
                        return Err(error("provider returned an error event"));
                    }
                    self.failed = false;
                    return Ok(event);
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => return Err(error("websocket closed")),
                _ => return Err(error("expected JSON text frame")),
            }
        }
    }
}
