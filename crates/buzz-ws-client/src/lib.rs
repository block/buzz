#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Shared NIP-42 WebSocket client used by `buzz-cli` and other tools that need
//! to connect to a Buzz relay, perform the AUTH handshake, and publish events.
//!
//! This crate wraps the low-level WebSocket transport with the relay-specific
//! protocol: NIP-42 challenge/response authentication, event submission with
//! OK confirmation, and relay message parsing.

/// WebSocket connection management — connect, authenticate (NIP-42), publish.
pub mod connection;
/// Error types for WebSocket client operations.
pub mod error;
/// Relay message types and event builders.
pub mod message;

pub use connection::{publish_event, NostrWsConnection};
pub use error::WsClientError;
pub use message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};
