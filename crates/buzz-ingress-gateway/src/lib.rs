//! Webhook ingress gateway: verifies provider deliveries at the edge, stores
//! them durably, and forwards them to the owning community relay as signed
//! Nostr events.
pub mod config;
pub mod http;
pub mod intake;
pub mod management;
pub mod metrics;
pub mod postgres;
pub mod providers;
pub mod secrets;
pub mod store;
pub use http::{router, router_with_metrics, AppState};
