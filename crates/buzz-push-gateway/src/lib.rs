#![warn(missing_docs)]
//! Stateful, capability-gated APNs last hop for NIP-PL.
pub mod apns;
/// App Attest enrollment verification.
pub mod app_attest;
/// Durable installation authority and relay delegation state.
pub mod authority;
/// Environment-driven configuration.
pub mod config;
/// Authenticated, expiring endpoint grants.
pub mod grant;
/// Stateful installation, delegation, delivery, and health HTTP API.
pub mod http;
/// Metrics counters and labels.
pub mod metrics;
/// Closed wire types for the stateful gateway.
pub mod model;
/// PostgreSQL authority store.
pub mod postgres;
pub(crate) mod strict_json;
/// APNs token custody.
pub mod token;
pub use http::{router, router_with_metrics, AppState};
