//! Webhook ingress gateway: verifies provider deliveries at the edge, stores
//! them durably, fans each out to the channel subscriptions of its
//! integration, and sends the matches to the owning community relay as signed
//! kind-9 channel messages.
pub mod config;
pub mod fanout;
pub mod forwarder;
pub mod http;
pub mod intake;
pub mod management;
pub mod metrics;
pub mod postgres;
pub mod providers;
pub mod relay_client;
pub mod secrets;
pub mod signer;
pub mod store;
pub use http::{router, router_with_metrics, AppState};
