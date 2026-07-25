//! Authenticated Nostr WebSocket transport used by Buzz clients.
//!
//! Most applications should use `buzz-client`; this lower-level crate exposes
//! the connection and relay-message primitives used by that shared client.

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Authenticated WebSocket connection primitives.
pub mod connection;
/// WebSocket transport errors.
pub mod error;
/// Typed Nostr relay message parsing and authentication helpers.
pub mod message;

pub use connection::{publish_event, NostrWsConnection};
pub use error::WsClientError;
pub use message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};
