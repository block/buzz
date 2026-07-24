//! buzz-transport — the pluggable event-transport seam.
//!
//! Buzz's native wire is signed events over a relay WebSocket. This crate
//! captures what that layer *provides* as one small trait — a bidirectional
//! stream of the same signed events — so the relay connection becomes one
//! implementation among many:
//!
//! - [`Transport`] — the seam. Outbound: signed events published into the
//!   stream. Inbound: channel-tagged events delivered by the other side.
//! - [`remote::RemoteTransport`] — dials an operator-supplied bridge
//!   endpoint (WebSocket or Unix socket, optionally through a SOCKS5
//!   proxy) speaking the small JSON protocol documented in
//!   [`PROTOCOL.md`](https://github.com/block/buzz/blob/main/crates/buzz-transport/PROTOCOL.md),
//!   so anyone can bridge Buzz onto their own network (Slack, a private
//!   mesh, …) in any language that can serve a socket.
//! - [`memory::InMemoryHub`] — the reference implementation: a working
//!   in-process hub fanning signed events out between participants.
//! - `MockTransport` (feature `test-utils`) — scripted test double for
//!   call-log assertions in tests of transport consumers.
//!
//! The unit that flows through the seam is [`SignedEvent`]: plain data owned
//! by this crate, deliberately not a Nostr-library type. It is structurally
//! the NIP-01 event Buzz already speaks (so the relay path converts
//! losslessly), but a transport or bridge implementor only ever sees six
//! JSON fields and a signature — no Nostr dependency required.
//!
//! Everything above raw delivery — channel membership, discovery, presence,
//! typing — already *is* events in Buzz (kind 39000/39002 metadata, the
//! ephemeral kinds), so it rides the same stream. The trait deliberately has
//! no side-channel API: if a bridge wants the agent to see a channel, it
//! delivers that channel's events; if it wants to announce membership, it
//! delivers the membership event.

#![deny(unsafe_code)]

pub mod memory;
pub mod protocol;
pub mod remote;
mod socks;

#[cfg(any(test, feature = "test-utils"))]
pub mod mock;

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Boxed future used across the seam trait. Public because [`Transport`]
/// implementors outside this crate must name it.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Errors from transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Dial, TLS, handshake, or socket-level failure.
    #[error("connection: {0}")]
    Connection(String),

    /// The transport (or its background task) has shut down.
    #[error("transport closed")]
    Closed,

    /// The remote endpoint violated the bridge protocol.
    #[error("protocol: {0}")]
    Protocol(String),

    /// A frame or event failed to encode or decode.
    #[error("codec: {0}")]
    Codec(String),

    /// An event's signature or ID failed verification.
    #[error("invalid event: {0}")]
    InvalidEvent(String),

    /// Plaintext `ws://` to a non-loopback host without the explicit opt-in.
    #[error("insecure transport rejected: {0}")]
    Insecure(String),
}

/// A signed event — the unit that flows through every transport.
///
/// Plain data: six fields plus the ID, all JSON-friendly. Structurally this
/// is the [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md)
/// event Buzz already speaks — `id` is the SHA-256 of the canonical
/// serialization, `sig` a Schnorr signature over it — so events convert
/// losslessly to and from the Nostr wire form ([`SignedEvent::from_nostr`],
/// [`SignedEvent::to_nostr`]). Transports and bridges, however, only ever
/// need this struct and its JSON: no Nostr library required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEvent {
    /// Event ID: SHA-256 of the canonical serialization, hex-encoded.
    pub id: String,
    /// Author public key (secp256k1 x-only), hex-encoded.
    pub pubkey: String,
    /// Creation time, Unix seconds.
    pub created_at: u64,
    /// Event kind — the only dispatch switch in Buzz.
    pub kind: u32,
    /// Structured metadata: an array of string arrays.
    pub tags: Vec<Vec<String>>,
    /// Payload: plain text or JSON, depending on `kind`.
    pub content: String,
    /// Schnorr signature over `id`, hex-encoded.
    pub sig: String,
}

impl SignedEvent {
    /// Convert from the Nostr wire type. Lossless: both serialize to the
    /// same NIP-01 JSON.
    pub fn from_nostr(event: &nostr::Event) -> Result<Self, TransportError> {
        serde_json::to_value(event)
            .and_then(serde_json::from_value)
            .map_err(|e| TransportError::Codec(format!("event conversion failed: {e}")))
    }

    /// Convert to the Nostr wire type. Fails only if a field is out of the
    /// Nostr type's domain (e.g. a malformed hex ID).
    pub fn to_nostr(&self) -> Result<nostr::Event, TransportError> {
        serde_json::to_value(self)
            .and_then(serde_json::from_value)
            .map_err(|e| TransportError::Codec(format!("event conversion failed: {e}")))
    }

    /// Verify the event ID hash and Schnorr signature.
    pub fn verify(&self) -> Result<(), TransportError> {
        self.to_nostr()?
            .verify()
            .map_err(|e| TransportError::InvalidEvent(e.to_string()))
    }
}

/// A subscription: which events the consumer wants delivered for a channel.
///
/// Subscriptions are *advisory* for transports whose far side decides what to
/// push (a Slack bridge already knows which conversations the agent is in);
/// the Nostr relay transport translates them into NIP-01 `REQ` filters
/// verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// The channel whose events should be delivered.
    pub channel_id: Uuid,
    /// Event kinds to deliver. `None` = all kinds. Per the NIP-01 edge
    /// case, an explicit empty list matches nothing.
    pub kinds: Option<Vec<u32>>,
    /// Only deliver events that mention (`p`-tag) the transport's own
    /// identity — the pubkey the transport connected as.
    pub require_mention: bool,
    /// Replay stored events created at or after this Unix timestamp before
    /// switching to live delivery. `None` = live events only.
    pub replay_since: Option<u64>,
}

impl Subscription {
    /// Subscription for all kinds, live-only, without a mention gate.
    pub fn all(channel_id: Uuid) -> Self {
        Self {
            channel_id,
            kinds: None,
            require_mention: false,
            replay_since: None,
        }
    }
}

/// An inbound event, tagged with the channel it belongs to.
#[derive(Debug, Clone)]
pub struct TransportEvent {
    /// Which channel this event belongs to.
    pub channel_id: Uuid,
    /// The underlying signed event.
    pub event: SignedEvent,
}

/// A bidirectional stream of signed events.
///
/// This is the seam between Buzz consumers (the ACP harness, custom bots)
/// and whatever network carries their events. The contract is deliberately
/// the same shape in both directions: signed events, tagged with a channel.
/// The relay WebSocket is one implementation; a remote bridge to a private
/// network is another.
///
/// Methods return [`BoxFuture`] for dyn-compatibility — consumers hold a
/// `Box<dyn Transport>` chosen at startup.
///
/// # Delivery contract
///
/// - [`next_event`](Transport::next_event) returning `None` signals
///   connection loss; the caller should invoke
///   [`reconnect`](Transport::reconnect) and continue polling.
/// - [`publish`](Transport::publish) resolves when the event is accepted for
///   sending (queued on the connection), not when the far side acknowledges
///   it. Rejections are surfaced by implementations through logging; the
///   stream stays fire-and-forget like the underlying relay socket.
/// - Implementations must not deliver events that fail
///   [`SignedEvent::verify`] — either by verifying inbound events
///   themselves (the remote bridge and in-memory transports do) or by
///   relying on an upstream that already verified them (the relay verifies
///   every event on ingest).
pub trait Transport: Send {
    /// Declare interest in a channel's events.
    ///
    /// Re-subscribing an already-subscribed channel replaces the previous
    /// subscription. Implementations re-establish active subscriptions after
    /// [`reconnect`](Transport::reconnect).
    fn subscribe(
        &mut self,
        subscription: Subscription,
    ) -> BoxFuture<'_, Result<(), TransportError>>;

    /// Withdraw interest in a channel's events.
    fn unsubscribe(&mut self, channel_id: Uuid) -> BoxFuture<'_, Result<(), TransportError>>;

    /// Wait for the next inbound event from any subscribed channel.
    ///
    /// Returns `None` on connection loss — the caller should call
    /// [`reconnect`](Transport::reconnect).
    fn next_event(&mut self) -> BoxFuture<'_, Option<TransportEvent>>;

    /// Publish a signed event into the stream.
    ///
    /// Waits for queue capacity; use [`try_publish`](Transport::try_publish)
    /// for ephemeral events that may be dropped instead.
    fn publish(&self, event: SignedEvent) -> BoxFuture<'_, Result<(), TransportError>>;

    /// Fire-and-forget publish — never blocks the caller.
    ///
    /// Suitable for ephemeral events (typing indicators, presence) where
    /// dropping on a full queue is acceptable.
    fn try_publish(&self, event: SignedEvent) -> Result<(), TransportError>;

    /// Re-establish the stream after connection loss, restoring active
    /// subscriptions.
    fn reconnect(&mut self) -> BoxFuture<'_, Result<(), TransportError>>;

    /// Gracefully shut the transport down.
    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()>;
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind};

    use super::*;

    // Compile-time proof that the trait stays dyn-compatible: consumers hold
    // a `Box<dyn Transport>` chosen at startup.
    fn _assert_dyn_compatible(_: Box<dyn Transport>) {}

    #[test]
    fn subscription_all_is_wildcard_live_only() {
        let id = Uuid::new_v4();
        let sub = Subscription::all(id);
        assert_eq!(sub.channel_id, id);
        assert_eq!(sub.kinds, None);
        assert!(!sub.require_mention);
        assert_eq!(sub.replay_since, None);
    }

    #[test]
    fn signed_event_round_trips_through_nostr() {
        let keys = Keys::generate();
        let nostr_event = EventBuilder::new(Kind::Custom(9), "hello transport")
            .sign_with_keys(&keys)
            .unwrap();

        let event = SignedEvent::from_nostr(&nostr_event).unwrap();
        // Same JSON both ways — the seam type is the wire form, verbatim.
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::to_value(&nostr_event).unwrap()
        );
        assert_eq!(event.to_nostr().unwrap(), nostr_event);
        assert_eq!(event.kind, 9);
        assert_eq!(event.content, "hello transport");
        event.verify().unwrap();
    }

    #[test]
    fn tampered_event_fails_verification() {
        let keys = Keys::generate();
        let nostr_event = EventBuilder::new(Kind::Custom(9), "original")
            .sign_with_keys(&keys)
            .unwrap();
        let mut event = SignedEvent::from_nostr(&nostr_event).unwrap();
        event.content = "tampered".to_string();
        assert!(matches!(
            event.verify(),
            Err(TransportError::InvalidEvent(_))
        ));
    }

    #[test]
    fn malformed_event_fails_conversion() {
        let event = SignedEvent {
            id: "not-hex".into(),
            pubkey: "also-not-hex".into(),
            created_at: 0,
            kind: 9,
            tags: vec![],
            content: String::new(),
            sig: "nope".into(),
        };
        assert!(matches!(event.verify(), Err(TransportError::Codec(_))));
    }
}
