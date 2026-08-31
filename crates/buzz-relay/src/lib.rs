#![deny(unsafe_code)]
#![warn(missing_docs)]
//! NIP-01 WebSocket relay for Buzz private team communication.

mod admission;
mod build_info;
mod rejection;

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

/// Build the default S3-backed media facade used by unit-test fixtures.
///
/// Provider construction belongs here, outside media and Git domain modules.
#[cfg(test)]
pub(crate) fn test_media_storage(
    config: &Config,
) -> std::result::Result<buzz_media::MediaStorage, buzz_object_store::ObjectStoreError> {
    let buzz_object_store::ObjectStoreConfig::S3(s3) = &config.object_store else {
        return Err(buzz_object_store::ObjectStoreError::Config(
            "synchronous test fixture requires the S3 test provider".to_string(),
        ));
    };
    let store = buzz_object_store::S3ObjectStore::new(s3)?;
    Ok(buzz_media::MediaStorage::with_store(std::sync::Arc::new(
        store,
    )))
}

/// Build an S3-backed Git facade for MinIO/unit-test fixtures.
#[cfg(test)]
pub(crate) fn test_git_store(
    endpoint: &str,
    access_key: &str,
    secret_key: &str,
    bucket: &str,
    region: &str,
    addressing_style: &str,
) -> std::result::Result<api::git::store::GitStore, buzz_object_store::ObjectStoreError> {
    let addressing_style = addressing_style
        .parse()
        .map_err(buzz_object_store::ObjectStoreError::Config)?;
    let store = buzz_object_store::S3ObjectStore::new(&buzz_object_store::S3StoreConfig {
        endpoint: endpoint.to_string(),
        access_key: access_key.to_string(),
        secret_key: secret_key.to_string(),
        bucket: bucket.to_string(),
        region: region.to_string(),
        addressing_style,
    })?;
    Ok(api::git::store::GitStore::new(std::sync::Arc::new(store)))
}
