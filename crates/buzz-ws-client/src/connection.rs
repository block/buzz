use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventId, Keys, Tag};
use serde_json::{json, Value};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::debug;

use crate::error::WsClientError;
use crate::message::{
    build_auth_event, parse_relay_message, validate_auth_challenge, OkResponse, RelayMessage,
};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

// AUTH events can contain reusable NIP-OA authority. Tungstenite 0.29 logs
// complete message/frame payloads at `trace` and relay-controlled Close reasons
// at `debug`, with no payload-redaction feature. `buzz-ws-client` therefore
// activates log/max_level_info as a compile-time dependency boundary. Keep the
// outbound private event below Tungstenite as defense in depth, with an explicit
// cap that still exercises every RFC 6455 payload-length encoding.
const MAX_PRIVATE_AUTH_FRAME_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthTransportState {
    Ready,
    Writing,
    Poisoned,
}

struct AuthWriteGuard<'a> {
    state: &'a mut AuthTransportState,
    completed: bool,
}

impl<'a> AuthWriteGuard<'a> {
    fn begin(state: &'a mut AuthTransportState) -> Result<Self, WsClientError> {
        if *state != AuthTransportState::Ready {
            return Err(WsClientError::AuthTransportPoisoned);
        }
        *state = AuthTransportState::Writing;
        Ok(Self {
            state,
            completed: false,
        })
    }

    fn complete(mut self) {
        *self.state = AuthTransportState::Ready;
        self.completed = true;
    }
}

impl Drop for AuthWriteGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            *self.state = AuthTransportState::Poisoned;
        }
    }
}

#[derive(Debug)]
enum AuthChallengeState {
    Empty,
    Pending(String),
    InFlight,
}

fn encode_masked_client_text_frame(
    payload: &[u8],
    mask: [u8; 4],
) -> Result<Vec<u8>, WsClientError> {
    let size = u64::try_from(payload.len()).map_err(|_| WsClientError::AuthFrameTooLarge {
        size: u64::MAX,
        max: MAX_PRIVATE_AUTH_FRAME_BYTES,
    })?;
    if size > MAX_PRIVATE_AUTH_FRAME_BYTES {
        return Err(WsClientError::AuthFrameTooLarge {
            size,
            max: MAX_PRIVATE_AUTH_FRAME_BYTES,
        });
    }

    // FIN plus text opcode. AUTH is never fragmented by this boundary, and no
    // WebSocket extensions are requested.
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x81);
    match size {
        0..=125 => frame.push(0x80 | size as u8),
        126..=65_535 => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(size as u16).to_be_bytes());
        }
        _ => {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&size.to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    Ok(frame)
}

async fn write_private_text_frame<S>(
    stream: &mut S,
    state: &mut AuthTransportState,
    payload: &[u8],
) -> Result<(), WsClientError>
where
    S: AsyncWrite + Unpin,
{
    // `rand::random` uses the thread-local CSPRNG; every client frame gets a
    // fresh unpredictable masking key as required by RFC 6455 section 5.3.
    let frame = encode_masked_client_text_frame(payload, rand::random())?;
    // Cancellation or I/O failure after the first byte may leave an incomplete
    // frame on the wire. The guard permanently poisons this connection unless
    // both the write and flush complete, preventing a later Tungstenite write
    // from being appended to a partial private frame.
    let guard = AuthWriteGuard::begin(state)?;
    stream.write_all(&frame).await.map_err(|error| {
        WsClientError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(error))
    })?;
    stream.flush().await.map_err(|error| {
        WsClientError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(error))
    })?;
    guard.complete();
    Ok(())
}

fn log_outbound(value: &Value, text: &str) {
    if is_auth_message(value) {
        debug!("→ relay: AUTH <redacted>");
    } else {
        debug!("→ relay: {text}");
    }
}

fn is_auth_message(value: &Value) -> bool {
    value
        .as_array()
        .and_then(|parts| parts.first())
        .and_then(Value::as_str)
        == Some("AUTH")
}

fn contains_private_marker(text: &str, marker: &str) -> bool {
    if text.contains(marker) {
        return true;
    }

    // Canonicalize valid JSON once more so escape sequences cannot disguise a
    // reflected signature from the raw-byte check. Malformed frames are still
    // returned only through the static post-auth parse error below.
    serde_json::from_str::<Value>(text)
        .ok()
        .map(|value| value.to_string())
        .is_some_and(|normalized| normalized.contains(marker))
}

#[cfg(test)]
fn contains_private_auth_marker(text: &str, markers: &[String]) -> bool {
    markers
        .iter()
        .any(|marker| contains_private_marker(text, marker))
}

fn is_expected_verified_authority_event(
    parsed: &Result<RelayMessage, WsClientError>,
    text: &str,
    expected_event_id: Option<&EventId>,
    authority_markers: &[String],
) -> bool {
    let (
        Ok(RelayMessage::Event {
            subscription_id,
            event,
            raw_event_json,
        }),
        Some(expected_event_id),
    ) = (parsed, expected_event_id)
    else {
        return false;
    };

    let reflected_markers_stay_inside_event = authority_markers.iter().all(|marker| {
        !contains_private_marker(text, marker)
            || (contains_private_marker(raw_event_json, marker)
                && !contains_private_marker(subscription_id, marker))
    });
    let raw_matches_typed = serde_json::from_str::<Value>(raw_event_json)
        .ok()
        .zip(serde_json::to_value(event.as_ref()).ok())
        .is_some_and(|(raw, typed)| raw == typed);

    reflected_markers_stay_inside_event
        && raw_matches_typed
        && event.id == *expected_event_id
        && event.verify_id()
        && event.verify_signature()
}

/// Seconds to wait for the relay to send the NIP-42 AUTH challenge after connecting.
pub const AUTH_CHALLENGE_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to the AUTH event.
pub const AUTH_OK_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to a published event.
pub const PUBLISH_OK_TIMEOUT_SECS: u64 = 30;

/// A NIP-42-capable WebSocket connection to a Nostr relay.
pub struct NostrWsConnection {
    ws: WsStream,
    buffer: VecDeque<RelayMessage>,
    auth_challenge: AuthChallengeState,
    auth_transport: AuthTransportState,
    private_auth_started: bool,
    private_auth_markers: Vec<String>,
    relay_url: String,
}

impl NostrWsConnection {
    /// Connects to the relay at `url` and performs NIP-42 authentication with `keys`.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn connect_authenticated(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<Self, WsClientError> {
        let mut conn = Self::connect(url).await?;
        conn.authenticate(keys, auth_tag).await?;
        Ok(conn)
    }

    /// Connects to the relay at `url` without performing authentication.
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;

        let (ws, _response) =
            connect_async(parsed.as_str())
                .await
                .map_err(|error| match error {
                    tokio_tungstenite::tungstenite::Error::Http(ref response)
                        if matches!(response.status().as_u16(), 401 | 403) =>
                    {
                        WsClientError::AuthFailed(format!(
                            "relay rejected WebSocket upgrade with HTTP {}",
                            response.status()
                        ))
                    }
                    other => WsClientError::WebSocket(other),
                })?;

        debug!("connected to relay at {url}");

        Ok(Self {
            ws,
            buffer: VecDeque::new(),
            auth_challenge: AuthChallengeState::Empty,
            auth_transport: AuthTransportState::Ready,
            private_auth_started: false,
            private_auth_markers: Vec::new(),
            relay_url: url.to_string(),
        })
    }

    /// Performs NIP-42 authentication using `keys` against the connected relay.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn authenticate(
        &mut self,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<(), WsClientError> {
        self.ensure_auth_transport_ready()?;
        let challenge = self
            .wait_for_auth_challenge(Duration::from_secs(AUTH_CHALLENGE_TIMEOUT_SECS))
            .await?;

        let auth_event = build_auth_event(&challenge, &self.relay_url, keys, auth_tag)?;
        let event_id = auth_event.id.to_hex();

        // Retain only the two signatures that uniquely identify reflected
        // private authority: the one-time AUTH event signature and, when
        // present, the reusable NIP-OA authority signature. Checking these
        // before returning any parsed message also prevents an exact reflected
        // AUTH event from reaching a caller's output path.
        self.private_auth_markers.clear();
        self.private_auth_markers.push(auth_event.sig.to_string());
        if let Some(authority_signature) = auth_tag
            .and_then(|tag| tag.as_slice().last())
            .filter(|value| !value.is_empty())
        {
            self.private_auth_markers
                .push(authority_signature.to_string());
        }

        self.send_private_auth(&auth_event).await?;

        let ok = self
            .wait_for_auth_ok(&event_id, Duration::from_secs(AUTH_OK_TIMEOUT_SECS))
            .await?;
        self.auth_challenge = AuthChallengeState::Empty;
        if !ok.accepted {
            // Relay-controlled text must not be allowed to echo the signed
            // AUTH event or its reusable authority into caller logs/stderr.
            return Err(WsClientError::AuthFailed(
                "relay rejected NIP-42 authentication".into(),
            ));
        }

        debug!("NIP-42 authentication successful");
        Ok(())
    }

    /// Sends a signed event to the relay and waits for the OK response.
    pub async fn send_event(&mut self, event: Event) -> Result<OkResponse, WsClientError> {
        let event_id = event.id.to_hex();
        self.send_raw(&json!(["EVENT", event])).await?;
        self.wait_for_ok(&event_id, Duration::from_secs(PUBLISH_OK_TIMEOUT_SECS))
            .await
    }

    /// Reports whether this connection has begun a private AUTH write.
    ///
    /// This becomes `true` before awaiting the write and does not depend on a
    /// relay `OK`, so callers can select non-reflective error reporting even if
    /// authentication fails or is cancelled after transmission starts.
    pub fn private_auth_started(&self) -> bool {
        self.private_auth_started
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn next_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, WsClientError> {
        self.ensure_auth_transport_ready()?;
        if let Some(msg) = self.buffer.pop_front() {
            // Messages enter the buffer only through `buffer_message`, which
            // records an AUTH challenge before storing it.
            return Ok(msg);
        }
        self.recv_one(timeout_dur, None).await
    }

    /// Receives the next relay message for an exact-event lookup.
    ///
    /// A stored event signed by a delegated agent legitimately carries the
    /// same reusable NIP-OA authority tag used during connection AUTH. This
    /// method permits that authority signature only inside an `EVENT` whose
    /// raw object is semantically identical to its locally ID-and-signature-
    /// verified typed form and matches `expected_event_id`. The one-time AUTH
    /// event signature remains forbidden in every frame, and all other
    /// authority reflections fail closed.
    pub async fn next_event_for_exact_id(
        &mut self,
        timeout_dur: Duration,
        expected_event_id: &EventId,
    ) -> Result<RelayMessage, WsClientError> {
        self.ensure_auth_transport_ready()?;
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }
        self.recv_one(timeout_dur, Some(expected_event_id)).await
    }

    /// Closes the WebSocket connection gracefully.
    pub async fn disconnect(mut self) -> Result<(), WsClientError> {
        self.ensure_auth_transport_ready()?;
        self.ws.close(None).await?;
        Ok(())
    }

    /// Sends a raw JSON value as a WebSocket text frame.
    ///
    /// Raw `AUTH` envelopes are rejected. Use [`Self::authenticate`] so the
    /// signed event is registered as private state and written below
    /// Tungstenite's payload-bearing trace boundary.
    pub async fn send_raw(&mut self, value: &Value) -> Result<(), WsClientError> {
        self.ensure_auth_transport_ready()?;
        if is_auth_message(value) {
            return Err(WsClientError::AuthFailed(
                "raw AUTH messages are not accepted; use authenticate".into(),
            ));
        }
        let text = serde_json::to_string(value)?;
        log_outbound(value, &text);
        self.ws.send(Message::Text(text.into())).await?;
        Ok(())
    }

    async fn send_private_auth(&mut self, event: &Event) -> Result<(), WsClientError> {
        let value = json!(["AUTH", event]);
        let text = serde_json::to_string(&value)?;
        log_outbound(&value, &text);
        // Tungstenite 0.29 traces plaintext frame payloads. Drain all prior
        // writes, then keep the registered private AUTH event below that
        // logger. This path is intentionally unavailable through `send_raw`.
        self.ws.flush().await?;
        self.private_auth_started = true;
        write_private_text_frame(self.ws.get_mut(), &mut self.auth_transport, text.as_bytes()).await
    }

    fn ensure_auth_transport_ready(&self) -> Result<(), WsClientError> {
        if self.auth_transport == AuthTransportState::Ready {
            Ok(())
        } else {
            Err(WsClientError::AuthTransportPoisoned)
        }
    }

    fn observe_auth_challenge(&mut self, challenge: &str) -> Result<(), WsClientError> {
        validate_auth_challenge(challenge)?;
        if !matches!(self.auth_challenge, AuthChallengeState::Empty) {
            return Err(WsClientError::AmbiguousAuthChallenge);
        }
        self.auth_challenge = AuthChallengeState::Pending(challenge.to_string());
        Ok(())
    }

    fn parse_inbound_message(
        &self,
        text: &str,
        authenticating: bool,
        expected_private_event_id: Option<&EventId>,
    ) -> Result<RelayMessage, WsClientError> {
        let auth_event_signature_reflected = self
            .private_auth_markers
            .first()
            .is_some_and(|marker| contains_private_marker(text, marker));
        let authority_signature_reflected = self
            .private_auth_markers
            .iter()
            .skip(1)
            .any(|marker| contains_private_marker(text, marker));
        let parsed = parse_relay_message(text);

        if auth_event_signature_reflected {
            return Err(WsClientError::ReflectedAuthMaterial);
        }
        if authority_signature_reflected
            && !is_expected_verified_authority_event(
                &parsed,
                text,
                expected_private_event_id,
                &self.private_auth_markers[1..],
            )
        {
            return Err(WsClientError::ReflectedAuthMaterial);
        }

        match parsed {
            Ok(message) => Ok(message),
            Err(WsClientError::AuthChallengeTooLarge { .. }) if authenticating => {
                Err(WsClientError::AmbiguousAuthChallenge)
            }
            Err(_) if !self.private_auth_markers.is_empty() => {
                let phase = if authenticating {
                    "during authentication"
                } else {
                    "after authentication"
                };
                Err(WsClientError::UnexpectedMessage(format!(
                    "malformed relay response {phase}"
                )))
            }
            Err(error) => Err(error),
        }
    }

    fn begin_auth_transaction(&mut self) -> Result<Option<String>, WsClientError> {
        match std::mem::replace(&mut self.auth_challenge, AuthChallengeState::Empty) {
            AuthChallengeState::Empty => Ok(None),
            AuthChallengeState::Pending(challenge) => {
                self.auth_challenge = AuthChallengeState::InFlight;
                if let Some(index) = self
                    .buffer
                    .iter()
                    .position(|message| matches!(message, RelayMessage::Auth { .. }))
                {
                    let _ = self.buffer.remove(index);
                }
                Ok(Some(challenge))
            }
            AuthChallengeState::InFlight => {
                self.auth_challenge = AuthChallengeState::InFlight;
                Err(WsClientError::AmbiguousAuthChallenge)
            }
        }
    }

    fn buffer_message(&mut self, message: RelayMessage) -> Result<(), WsClientError> {
        if let RelayMessage::Auth { ref challenge } = message {
            self.observe_auth_challenge(challenge)?;
        }
        self.buffer.push_back(message);
        Ok(())
    }

    fn deliver_message(&mut self, message: RelayMessage) -> Result<RelayMessage, WsClientError> {
        if let RelayMessage::Auth { ref challenge } = message {
            self.observe_auth_challenge(challenge)?;
        }
        Ok(message)
    }

    async fn recv_one(
        &mut self,
        timeout_dur: Duration,
        expected_private_event_id: Option<&EventId>,
    ) -> Result<RelayMessage, WsClientError> {
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }

        loop {
            let raw = timeout(timeout_dur, self.ws.next())
                .await
                .map_err(|_| WsClientError::Timeout)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg =
                        self.parse_inbound_message(&text, false, expected_private_event_id)?;
                    return self.deliver_message(msg);
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_auth_challenge(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, WsClientError> {
        if let Some(challenge) = self.begin_auth_transaction()? {
            return Ok(challenge);
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
                    let msg = self.parse_inbound_message(&text, false, None)?;
                    match msg {
                        RelayMessage::Auth { challenge } => {
                            self.observe_auth_challenge(&challenge)?;
                            if let Some(challenge) = self.begin_auth_transaction()? {
                                return Ok(challenge);
                            }
                        }
                        other => self.buffer_message(other)?,
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_auth_ok(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, WsClientError> {
        self.wait_for_ok_inner(event_id, timeout_dur, true).await
    }

    async fn wait_for_ok(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, WsClientError> {
        self.wait_for_ok_inner(event_id, timeout_dur, false).await
    }

    async fn wait_for_ok_inner(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
        authenticating: bool,
    ) -> Result<OkResponse, WsClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|m| matches!(m, RelayMessage::Ok(ok) if ok.event_id == event_id))
        {
            match self.buffer.remove(idx).unwrap() {
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
                    let msg = self.parse_inbound_message(&text, authenticating, None)?;
                    match msg {
                        RelayMessage::Ok(ok) if ok.event_id == event_id => return Ok(ok),
                        other => self.buffer_message(other)?,
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }
}

/// One-shot helper: connect, authenticate, send one event, disconnect.
///
/// Establishes a fresh WebSocket connection, completes NIP-42 authentication,
/// publishes `event`, waits for the relay's OK response, then closes the
/// connection. The entire operation is bounded by `timeout_secs`.
pub async fn publish_event(
    relay_url: &str,
    event: Event,
    keys: &Keys,
    auth_tag: Option<&Tag>,
    timeout_secs: u64,
) -> Result<OkResponse, WsClientError> {
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut conn = NostrWsConnection::connect(relay_url).await?;
        conn.authenticate(keys, auth_tag).await?;
        let ok = conn.send_event(event).await?;
        let _ = conn.disconnect().await;
        Ok::<_, WsClientError>(ok)
    })
    .await
    .map_err(|_| WsClientError::Timeout)?;
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use nostr::{EventBuilder, Kind};

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

    #[test]
    fn dependency_payload_log_levels_are_compiled_out() {
        assert_eq!(log::STATIC_MAX_LEVEL, log::LevelFilter::Info);
    }

    #[test]
    fn json_escapes_cannot_disguise_private_auth_markers() {
        let markers = vec!["signature-marker".to_string()];
        let escaped = r#"["CLOSED","subscription","signature\u002dmarker"]"#;

        assert!(!escaped.contains(&markers[0]));
        assert!(contains_private_auth_marker(escaped, &markers));
    }

    #[test]
    fn delegated_authority_is_allowed_only_inside_the_exact_signed_event() {
        let authority_signature = "a".repeat(128);
        let authority_markers = vec![authority_signature.clone()];
        let auth_tag = Tag::parse([
            "auth",
            &"b".repeat(64),
            "kind=1",
            authority_signature.as_str(),
        ])
        .unwrap();
        let event = EventBuilder::new(Kind::TextNote, "delegated")
            .tags([auth_tag])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let raw_event_json = serde_json::to_string(&event).unwrap();
        let frame = serde_json::json!(["EVENT", "exact", event]).to_string();
        let parsed = parse_relay_message(&frame);

        assert!(is_expected_verified_authority_event(
            &parsed,
            &frame,
            Some(&event.id),
            &authority_markers,
        ));

        let reflected_subscription = serde_json::json!([
            "EVENT",
            authority_signature,
            serde_json::from_str::<Value>(&raw_event_json).unwrap()
        ])
        .to_string();
        let parsed = parse_relay_message(&reflected_subscription);
        assert!(!is_expected_verified_authority_event(
            &parsed,
            &reflected_subscription,
            Some(&event.id),
            &authority_markers,
        ));

        let mut raw_with_unknown_field = serde_json::from_str::<Value>(&raw_event_json).unwrap();
        raw_with_unknown_field["relay-controlled"] = Value::String(authority_markers[0].clone());
        let reflected_unknown_field =
            serde_json::json!(["EVENT", "exact", raw_with_unknown_field]).to_string();
        let parsed = parse_relay_message(&reflected_unknown_field);
        assert!(!is_expected_verified_authority_event(
            &parsed,
            &reflected_unknown_field,
            Some(&event.id),
            &authority_markers,
        ));
    }

    fn assert_masked_text_frame_at_boundary(size: usize, length_marker: u8, header_len: usize) {
        let payload: Vec<u8> = (0..size).map(|index| index as u8).collect();
        let mask = [0x12, 0x34, 0x56, 0x78];
        let frame = encode_masked_client_text_frame(&payload, mask).unwrap();

        assert_eq!(frame[0], 0x81, "AUTH frame must be FIN + text");
        assert_eq!(frame[1] & 0x80, 0x80, "client MASK bit must be set");
        assert_eq!(frame[1] & 0x7f, length_marker);
        let encoded_size = match length_marker {
            0..=125 => u64::from(length_marker),
            126 => u64::from(u16::from_be_bytes([frame[2], frame[3]])),
            127 => u64::from_be_bytes(frame[2..10].try_into().unwrap()),
            _ => unreachable!(),
        };
        assert_eq!(encoded_size, size as u64);
        assert_eq!(&frame[header_len - 4..header_len], &mask);

        let unmasked: Vec<u8> = frame[header_len..]
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()])
            .collect();
        assert_eq!(unmasked, payload);
    }

    #[test]
    fn private_auth_frame_has_canonical_lengths_across_rfc6455_boundaries() {
        assert_masked_text_frame_at_boundary(125, 125, 6);
        assert_masked_text_frame_at_boundary(126, 126, 8);
        assert_masked_text_frame_at_boundary(65_535, 126, 8);
        assert_masked_text_frame_at_boundary(65_536, 127, 14);
    }

    #[test]
    fn private_auth_frame_has_an_explicit_allocation_and_length_cap() {
        let oversized = vec![0_u8; MAX_PRIVATE_AUTH_FRAME_BYTES as usize + 1];
        assert!(matches!(
            encode_masked_client_text_frame(&oversized, [1, 2, 3, 4]),
            Err(WsClientError::AuthFrameTooLarge {
                size: 1_048_577,
                max: 1_048_576
            })
        ));
    }

    struct PartialThenPending {
        bytes_written: usize,
    }

    impl AsyncWrite for PartialThenPending {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.bytes_written == 0 && !bytes.is_empty() {
                self.bytes_written = 1;
                Poll::Ready(Ok(1))
            } else {
                Poll::Pending
            }
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn cancelled_partial_auth_write_permanently_poisons_transport() {
        let mut state = AuthTransportState::Ready;
        let mut writer = PartialThenPending { bytes_written: 0 };

        let cancelled = timeout(
            Duration::from_millis(10),
            write_private_text_frame(&mut writer, &mut state, b"private-auth"),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "partial AUTH write unexpectedly completed"
        );
        assert_eq!(writer.bytes_written, 1);
        assert_eq!(state, AuthTransportState::Poisoned);

        assert!(matches!(
            write_private_text_frame(&mut writer, &mut state, b"later-frame").await,
            Err(WsClientError::AuthTransportPoisoned)
        ));
        assert_eq!(writer.bytes_written, 1, "poisoned transport was reused");
    }

    #[test]
    fn auth_event_payload_is_redacted_from_debug_logs() {
        #[derive(Clone)]
        struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

        impl Write for CaptureWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&captured);
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || CaptureWriter(Arc::clone(&writer)))
            .finish();
        let auth = json!([
            "AUTH",
            {
                "id": "private-auth-event-bytes",
                "tags": [["auth", "reusable-auth-tag-secret"]],
                "sig": "private-auth-signature"
            }
        ]);
        let serialized = serde_json::to_string(&auth).unwrap();

        tracing::subscriber::with_default(subscriber, || log_outbound(&auth, &serialized));

        let logs = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("AUTH <redacted>"));
        for secret in [
            "private-auth-event-bytes",
            "reusable-auth-tag-secret",
            "private-auth-signature",
        ] {
            assert!(!logs.contains(secret), "AUTH log leaked {secret}: {logs}");
        }
        assert!(!logs.contains(&serialized), "AUTH log leaked its raw frame");
    }
}
