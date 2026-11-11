use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, NostrSigner, Tag};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::{header::USER_AGENT, HeaderMap, HeaderValue};
use tokio_tungstenite::{
    client_async, connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

use crate::error::WsClientError;
use crate::message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Seconds to wait for the relay to send the NIP-42 AUTH challenge after connecting.
pub const AUTH_CHALLENGE_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to the AUTH event.
pub const AUTH_OK_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to a published event.
pub const PUBLISH_OK_TIMEOUT_SECS: u64 = 30;

/// Cap on a graceful close.
pub const CLOSE_TIMEOUT_SECS: u64 = 5;

/// Run one network phase, writes included, within its read `wait` plus a
/// second, so a read that times out reports its own error first.
async fn phase<T>(
    wait: Duration,
    io: impl std::future::Future<Output = Result<T, WsClientError>>,
) -> Result<T, WsClientError> {
    tokio::time::timeout(wait + Duration::from_secs(1), io)
        .await
        .map_err(|_| WsClientError::Timeout)?
}

/// `User-Agent` sent on the upgrade unless the caller's headers supply one.
const DEFAULT_USER_AGENT: &str = concat!("buzz-ws-client/", env!("CARGO_PKG_VERSION"));

/// The upgrade request for `url`: the default `User-Agent`, then `headers`.
fn upgrade_request(url: &url::Url, headers: HeaderMap) -> Result<Request, WsClientError> {
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    request.headers_mut().extend(headers);
    Ok(request)
}

/// A NIP-42-capable WebSocket connection to a Nostr relay.
pub struct NostrWsConnection {
    ws: WsStream,
    /// Messages read while waiting for something else, each with when it
    /// was read.
    buffer: VecDeque<(Instant, RelayMessage)>,
    pending_challenge: Option<String>,
    relay_url: String,
}

impl NostrWsConnection {
    /// Connects to the relay at `url` and performs NIP-42 authentication with `signer`.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn connect_authenticated<S: NostrSigner>(
        url: &str,
        signer: &S,
        auth_tag: Option<&Tag>,
    ) -> Result<Self, WsClientError> {
        Self::connect_authenticated_with_headers(url, signer, auth_tag, HeaderMap::new()).await
    }

    /// [`Self::connect_authenticated`], sending `headers` on the WebSocket upgrade request.
    ///
    /// Use this for credentials the relay checks at upgrade time, such as a NIP-FI
    /// `Nostr-Federated-Identity` assertion that must pair with the NIP-42 AUTH key.
    pub async fn connect_authenticated_with_headers<S: NostrSigner>(
        url: &str,
        signer: &S,
        auth_tag: Option<&Tag>,
        headers: HeaderMap,
    ) -> Result<Self, WsClientError> {
        let mut conn = Self::connect_with_headers(url, headers).await?;
        conn.authenticate(signer, auth_tag).await?;
        Ok(conn)
    }

    /// Connects to the relay at `url` without performing authentication.
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        Self::connect_with_headers(url, HeaderMap::new()).await
    }

    /// [`Self::connect`], sending `headers` on the WebSocket upgrade request.
    ///
    /// The headers are added after the handshake headers, so supplying a handshake
    /// header such as `Sec-WebSocket-Key` replaces it and breaks the upgrade. A
    /// `User-Agent` among them replaces the default `buzz-ws-client/<version>`.
    pub async fn connect_with_headers(
        url: &str,
        headers: HeaderMap,
    ) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;

        let request = upgrade_request(&parsed, headers)?;

        let (ws, _response) = connect_async(request)
            .await
            .map_err(WsClientError::WebSocket)?;

        debug!("connected to relay at {url}");

        Ok(Self::from_stream(ws, url))
    }

    /// [`Self::connect`], but opens a plaintext TCP connection to `addr`
    /// (`host:port`) instead of resolving the URL's host.
    ///
    /// The URL still supplies the `Host` header and NIP-42 relay tag, even when
    /// it is `wss://`, so a client inside the relay's cluster can reach a
    /// community through the relay's plaintext service address and still
    /// authenticate for the community's public URL.
    pub async fn connect_to_addr(url: &str, addr: &str) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;
        let request = upgrade_request(&parsed, HeaderMap::new())?;
        let tcp = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| WsClientError::WebSocket(e.into()))?;
        let (ws, _response) = client_async(request, MaybeTlsStream::Plain(tcp))
            .await
            .map_err(WsClientError::WebSocket)?;

        debug!("connected to relay at {url} via {addr}");

        Ok(Self::from_stream(ws, url))
    }

    fn from_stream(ws: WsStream, url: &str) -> Self {
        Self {
            ws,
            buffer: VecDeque::new(),
            pending_challenge: None,
            relay_url: url.to_string(),
        }
    }

    /// Performs NIP-42 authentication using `signer` against the connected relay.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn authenticate<S: NostrSigner>(
        &mut self,
        signer: &S,
        auth_tag: Option<&Tag>,
    ) -> Result<(), WsClientError> {
        // Each network phase is capped as a whole, writes (and Pongs sent
        // while reading) included. Signing between them is the signer's to
        // bound: a remote signer may hold it for a person's decision.
        let wait = Duration::from_secs(AUTH_CHALLENGE_TIMEOUT_SECS);
        let challenge = phase(wait, self.wait_for_auth_challenge(wait)).await?;

        let auth_event = build_auth_event(&challenge, &self.relay_url, signer, auth_tag).await?;
        let event_id = auth_event.id.to_hex();

        let wait = Duration::from_secs(AUTH_OK_TIMEOUT_SECS);
        let ok = phase(wait, async {
            self.send_raw(&json!(["AUTH", auth_event])).await?;
            self.wait_for_ok(&event_id, wait).await
        })
        .await?;
        if !ok.accepted {
            return Err(WsClientError::AuthFailed(ok.message));
        }

        debug!("NIP-42 authentication successful");
        Ok(())
    }

    /// Sends a signed event to the relay and waits for the OK response.
    pub async fn send_event(&mut self, event: Event) -> Result<OkResponse, WsClientError> {
        let event_id = event.id.to_hex();
        let wait = Duration::from_secs(PUBLISH_OK_TIMEOUT_SECS);
        phase(wait, async {
            self.send_raw(&json!(["EVENT", event])).await?;
            self.wait_for_ok(&event_id, wait).await
        })
        .await
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn next_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, WsClientError> {
        Ok(self.next_event_received(timeout_dur).await?.1)
    }

    /// [`Self::next_event`], with when the message was read off the socket.
    ///
    /// A message that arrived while another call, such as
    /// [`Self::send_event`], waited for something else is returned later,
    /// with the instant it arrived rather than the instant it is returned.
    pub async fn next_event_received(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<(Instant, RelayMessage), WsClientError> {
        if let Some(entry) = self.buffer.pop_front() {
            return Ok(entry);
        }
        let msg = self.recv_one(timeout_dur).await?;
        Ok((Instant::now(), msg))
    }

    /// Closes the WebSocket connection gracefully, within [`CLOSE_TIMEOUT_SECS`].
    pub async fn disconnect(mut self) -> Result<(), WsClientError> {
        tokio::time::timeout(Duration::from_secs(CLOSE_TIMEOUT_SECS), self.ws.close(None))
            .await
            .map_err(|_| WsClientError::Timeout)??;
        Ok(())
    }

    /// Sends a raw JSON value as a WebSocket text frame.
    pub async fn send_raw(&mut self, value: &Value) -> Result<(), WsClientError> {
        let text = serde_json::to_string(value)?;
        debug!("→ relay: {text}");
        self.ws.send(Message::Text(text.into())).await?;
        Ok(())
    }

    async fn recv_one(&mut self, timeout_dur: Duration) -> Result<RelayMessage, WsClientError> {
        loop {
            let raw = timeout(timeout_dur, self.ws.next())
                .await
                .map_err(|_| WsClientError::Timeout)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = parse_relay_message(&text)?;
                    if let RelayMessage::Auth { ref challenge } = msg {
                        self.pending_challenge = Some(challenge.clone());
                    }
                    return Ok(msg);
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(frame) => return Err(closed(frame)),
                _ => {}
            }
        }
    }

    async fn wait_for_auth_challenge(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, WsClientError> {
        if let Some(challenge) = self.pending_challenge.take() {
            return Ok(challenge);
        }

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|(_, m)| matches!(m, RelayMessage::Auth { .. }))
        {
            match self.buffer.remove(idx).unwrap().1 {
                RelayMessage::Auth { challenge } => return Ok(challenge),
                _ => unreachable!(),
            }
        }

        let deadline = tokio::time::Instant::now() + timeout_dur;

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(WsClientError::NoAuthChallenge);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| WsClientError::NoAuthChallenge)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = parse_relay_message(&text)?;
                    match msg {
                        RelayMessage::Auth { challenge } => {
                            if challenge.len() > 1024 {
                                return Err(WsClientError::AuthFailed(
                                    "challenge exceeds 1024 bytes".into(),
                                ));
                            }
                            return Ok(challenge);
                        }
                        other => self.buffer.push_back((Instant::now(), other)),
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(frame) => return Err(closed(frame)),
                _ => {}
            }
        }
    }

    async fn wait_for_ok(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, WsClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|(_, m)| matches!(m, RelayMessage::Ok(ok) if ok.event_id == event_id))
        {
            match self.buffer.remove(idx).unwrap().1 {
                RelayMessage::Ok(ok) => return Ok(ok),
                _ => unreachable!(),
            }
        }

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(WsClientError::Timeout);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| WsClientError::Timeout)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = parse_relay_message(&text)?;
                    match msg {
                        RelayMessage::Ok(ok) if ok.event_id == event_id => return Ok(ok),
                        RelayMessage::Auth { ref challenge } => {
                            self.pending_challenge = Some(challenge.clone());
                            self.buffer.push_back((Instant::now(), msg));
                        }
                        other => self.buffer.push_back((Instant::now(), other)),
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(frame) => return Err(closed(frame)),
                _ => {}
            }
        }
    }
}

/// One-shot helper: connect, authenticate, send one event, disconnect.
///
/// Establishes a fresh WebSocket connection carrying `headers` on the upgrade
/// (e.g. a NIP-FI `Nostr-Federated-Identity` assertion), completes NIP-42
/// authentication with `signer`, publishes `event`, waits for the relay's OK
/// response, then closes the connection. The entire operation is bounded by
/// `timeout_secs`.
pub async fn publish_event<S: NostrSigner>(
    relay_url: &str,
    event: Event,
    signer: &S,
    auth_tag: Option<&Tag>,
    headers: HeaderMap,
    timeout_secs: u64,
) -> Result<OkResponse, WsClientError> {
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut conn = NostrWsConnection::connect_authenticated_with_headers(
            relay_url, signer, auth_tag, headers,
        )
        .await?;
        let ok = conn.send_event(event).await?;
        let _ = conn.disconnect().await;
        Ok::<_, WsClientError>(ok)
    })
    .await
    .map_err(|_| WsClientError::Timeout)?;
    result
}

/// The error for a close frame: a policy close keeps its reason, since a
/// relay denial is not an ordinary connection loss.
fn closed(frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) -> WsClientError {
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    match frame {
        Some(frame) if frame.code == CloseCode::Policy => {
            WsClientError::PolicyClose(frame.reason.to_string())
        }
        _ => WsClientError::ConnectionClosed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_challenge_timeout_meets_floor() {
        const { assert!(AUTH_CHALLENGE_TIMEOUT_SECS >= 20) };
    }

    #[test]
    fn auth_ok_timeout_meets_floor() {
        const { assert!(AUTH_OK_TIMEOUT_SECS >= 20) };
    }

    #[test]
    fn publish_ok_timeout_meets_floor() {
        const { assert!(PUBLISH_OK_TIMEOUT_SECS >= 30) };
    }

    // The handshake callback's `Result<Response, ErrorResponse>` is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn connect_with_headers_sends_headers_on_upgrade() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut received = None;
            let _ws = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    received = request.headers().get("nostr-federated-identity").cloned();
                    Ok(response)
                },
            )
            .await
            .unwrap();
            received
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            "Nostr-Federated-Identity",
            HeaderValue::from_static("Bearer header.payload.signature"),
        );
        let _conn = NostrWsConnection::connect_with_headers(&url, headers)
            .await
            .unwrap();

        assert_eq!(
            server.await.unwrap(),
            Some(HeaderValue::from_static("Bearer header.payload.signature"))
        );
    }

    /// The `User-Agent` a relay sees on an upgrade sent with `headers`, through
    /// `connect_with_headers` or, with `via_addr`, `connect_to_addr`.
    #[allow(clippy::result_large_err)]
    async fn upgrade_user_agent(headers: HeaderMap, via_addr: bool) -> Option<HeaderValue> {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut received = None;
            let _ws = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    received = request.headers().get(USER_AGENT).cloned();
                    Ok(response)
                },
            )
            .await
            .unwrap();
            received
        });
        let url = format!("ws://{addr}");
        let _conn = if via_addr {
            NostrWsConnection::connect_to_addr(&url, &addr).await
        } else {
            NostrWsConnection::connect_with_headers(&url, headers).await
        }
        .unwrap();
        server.await.unwrap()
    }

    #[tokio::test]
    async fn upgrade_sends_default_user_agent() {
        let expected = Some(HeaderValue::from_static(DEFAULT_USER_AGENT));
        assert_eq!(upgrade_user_agent(HeaderMap::new(), false).await, expected);
        assert_eq!(upgrade_user_agent(HeaderMap::new(), true).await, expected);
        assert!(DEFAULT_USER_AGENT.starts_with("buzz-ws-client/"));
    }

    #[tokio::test]
    async fn caller_user_agent_replaces_default() {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("example/1.0"));
        assert_eq!(
            upgrade_user_agent(headers, false).await,
            Some(HeaderValue::from_static("example/1.0"))
        );
    }

    /// A relay that answers NIP-42 AUTH by closing with `code`, as the relay
    /// does on a NIP-FI key mismatch (1008).
    async fn close_after_auth(code: u16) -> WsClientError {
        use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::Text(r#"["AUTH","challenge"]"#.into()))
                .await
                .unwrap();
            let _auth = ws.next().await;
            ws.send(Message::Close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: "auth-required: key mismatch".into(),
            })))
            .await
            .unwrap();
        });
        NostrWsConnection::connect_authenticated(&url, &nostr::Keys::generate(), None)
            .await
            .err()
            .unwrap()
    }

    #[tokio::test]
    async fn a_policy_close_keeps_its_reason_and_other_closes_do_not() {
        let err = close_after_auth(1008).await;
        assert!(
            matches!(&err, WsClientError::PolicyClose(r) if r.contains("key mismatch")),
            "{err:?}"
        );
        let err = close_after_auth(1000).await;
        assert!(matches!(err, WsClientError::ConnectionClosed), "{err:?}");
    }
}
