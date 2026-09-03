#![deny(unsafe_code)]
#![warn(missing_docs)]
//! NIP-01 WebSocket relay for Buzz private team communication.

mod admission;
mod build_info;
mod rejection;

/// NIP-FI session admission gate — per-connection effect-permit and quiescence barrier.
pub(crate) mod nip_fi_gate;
pub(crate) mod nip_fi_session;
/// NIP-FI test hooks — production barriers for deterministic B1/B2 witnesses.
#[cfg(test)]
pub(crate) mod nip_fi_test_hooks;
/// NIP-FI assertion validation at WebSocket upgrade.
pub(crate) mod nip_fi_upgrade;

/// REST API route handlers.
pub mod api;
/// WebSocket audio relay for huddle voice channels.
pub mod audio;
/// Relay configuration from environment variables.
pub mod config;
/// Runtime conformance harness — abstract trace emission at the
/// ingest/read accept-reject boundary, replayed against
/// `docs/spec/MultiTenantRelay.tla` by the independent `buzz-conformance`
/// checker.
pub mod conformance;
/// WebSocket connection lifecycle and state.
pub mod connection;
/// Relay error types.
pub mod error;
/// WebSocket message handlers for NIP-01 client commands.
pub mod handlers;
/// Stateless HMAC-signed relay invite tokens (mint/verify).
pub mod invite_token;
/// Inter-relay mesh startup wiring (`BUZZ_MESH` seam).
pub mod mesh_boot;
/// Prometheus metrics: recorder, upkeep, HTTP middleware.
pub mod metrics;
/// NIP-11 relay information document.
pub mod nip11;
/// NIP-FI relay-level configuration (S4): issuer registry, JWKS, and
/// admin-command fields.  Parsed from `BUZZ_NIP_FI_MODE` and
/// `BUZZ_NIP_FI_ISSUERS` at startup.
pub(crate) mod nip_fi_config;
/// NIP-01 client/relay message parsing.
pub mod protocol;
/// Durable NIP-PL matcher and delivery worker.
pub mod push_runtime;
mod readiness;
/// Axum router construction.
pub mod router;
/// Shared application state.
pub mod state;
pub mod storage_sweep;
/// Subscription registry with (channel, kind) fan-out index.
pub mod subscription;
/// OpenTelemetry tracing initialisation (tracer provider + OTLP exporter).
pub mod telemetry;
/// Row-zero host binding: resolve the request community from the connection host.
pub mod tenant;
#[cfg(test)]
mod test_support;
/// Relay-side tunnel session directory and routing.
pub mod tunnel;
/// Webhook secret generation and constant-time comparison.
pub mod webhook_secret;
/// Workflow action sink — relay-side implementation of [`buzz_workflow::ActionSink`].
pub mod workflow_sink;

pub use config::Config;
pub use error::{RelayError, Result};
pub use state::AppState;
