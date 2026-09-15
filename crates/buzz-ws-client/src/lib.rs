#![deny(unsafe_code)]

pub mod connection;
pub mod error;
pub mod federated_identity;
pub mod identity_adapter;
pub mod identity_git;
pub mod identity_socket;
pub mod message;

pub use connection::{publish_event, NostrWsConnection};
pub use error::WsClientError;
pub use message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};
