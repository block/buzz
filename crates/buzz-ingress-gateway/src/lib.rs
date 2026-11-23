//! Webhook ingress gateway: verifies provider deliveries at the edge, stores
//! them durably, and forwards them to the owning community relay as signed
//! Nostr events.
pub mod config;
pub mod http;
pub mod metrics;
pub mod postgres;
pub use http::{router, router_with_metrics, AppState};
