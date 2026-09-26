//! Private accessory API. Conversation authority remains signed Nostr events.

mod auth;
mod handlers;

use crate::state::AppState;
use axum::{routing::get, Router};
use std::sync::Arc;

/// Narrow versioned router; unknown accessory paths never return SPA HTML.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/me/sidebar", get(handlers::sidebar))
        .route(
            "/me/read-state",
            get(handlers::contexts).post(handlers::write),
        )
        .fallback(|| async { auth::Error::new(axum::http::StatusCode::NOT_FOUND, "not_found") })
        .with_state(state)
}

#[cfg(test)]
mod postgres_tests;
