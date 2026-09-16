#![deny(unsafe_code)]

pub mod connection;
pub mod error;
pub mod message;

pub use connection::{publish_event, NostrWsConnection};
pub use error::WsClientError;
pub use message::{
    build_auth_event, build_auth_event_with_signer, parse_relay_message, OkResponse, RelayMessage,
};

#[cfg(test)]
mod signer_tests;
