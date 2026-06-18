#![deny(unsafe_code)]
#![warn(missing_docs)]
//! NIP-01 WebSocket relay for Buzz private team communication.

/// REST API route handlers.
pub mod api;
/// WebSocket audio relay for huddle voice channels.
pub mod audio;
/// Relay configuration from environment variables.
pub mod config;
/// WebSocket connection lifecycle and state.
pub mod connection;
/// Relay error types.
pub mod error;
/// WebSocket message handlers for NIP-01 client commands.
pub mod handlers;
/// Relay-signed mesh-LLM status publisher.
pub mod mesh_status_publisher;
/// Prometheus metrics: recorder, upkeep, HTTP middleware.
pub mod metrics;
/// NIP-11 relay information document.
pub mod nip11;
/// NIP-01 client/relay message parsing.
pub mod protocol;
/// Axum router construction.
pub mod router;
/// Shared application state.
pub mod state;
/// Subscription registry with (channel, kind) fan-out index.
pub mod subscription;
/// Webhook secret generation and constant-time comparison.
pub mod webhook_secret;
/// Workflow action sink — relay-side implementation of [`buzz_workflow::ActionSink`].
pub mod workflow_sink;

pub use config::Config;
pub use error::{RelayError, Result};
pub use state::AppState;
