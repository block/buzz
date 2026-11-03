//! Private accessory API. Conversation authority remains signed Nostr events.

mod auth;
mod handlers;

use crate::state::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

/// Public accessory prefix, shared by routing and capability discovery.
pub const BASE_PATH: &str = "/buzz/v1";

/// Full path of the lifecycle state read, advertised in NIP-11.
pub const LIFECYCLE_STATE_PATH: &str = "/buzz/v1/lifecycle/state";

/// Narrow versioned router; unknown accessory paths never return SPA HTML.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/me/sidebar", get(handlers::sidebar))
        .route("/me/read-state", post(handlers::write))
        // Needs only `buzz_v1`: a client recovers its journal after the
        // lifecycle guard is turned off.
        .route("/lifecycle/state", post(handlers::lifecycle_state))
        .fallback(|| async { auth::Error::new(axum::http::StatusCode::NOT_FOUND, "not_found") })
        .with_state(state)
}

#[cfg(test)]
mod postgres_tests;
