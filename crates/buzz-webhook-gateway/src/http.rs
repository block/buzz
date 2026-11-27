//! HTTP surface: the public webhook listener and the private router that
//! serves the management API, health probes, and metrics.
use crate::config::RelayScheme;
use crate::intake::RateLimiter;
use crate::secrets::SecretKeyring;
use crate::store::Store;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    /// Seals provider secrets at rest; only the current key seals.
    pub secrets: Arc<SecretKeyring>,
    /// Relays allowed to call the management API.
    pub relay_pubkeys: Arc<Vec<PublicKey>>,
    /// Public intake bounds from configuration.
    pub intake: IntakeLimits,
    /// Per-integration intake rate limiter.
    pub rate_limiter: Arc<RateLimiter>,
    /// Injectable clock so tests can drive secret grace windows.
    pub now: fn() -> DateTime<Utc>,
    /// Cleared on shutdown so readiness fails before listeners drain.
    pub accepting: Arc<AtomicBool>,
    /// Scheme used when reporting an integration's `delivery_url`.
    pub public_scheme: RelayScheme,
}

/// Intake bounds that shape the public router.
#[derive(Clone, Copy, Debug)]
pub struct IntakeLimits {
    /// Bodies above this are refused with 413 before any handler runs.
    pub max_body_bytes: usize,
    /// Window for provider timestamps where the provider supplies one.
    pub timestamp_tolerance_seconds: i64,
}

pub(crate) fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({ "error": code }))).into_response()
}

async fn live() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "alive" }))
}

async fn ready(State(s): State<AppState>) -> Response {
    if !s.accepting.load(Ordering::Relaxed) {
        crate::metrics::record_readiness_failure(crate::metrics::ReadinessFailure::NotAccepting);
        return error(StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    }
    if s.store.ready().await.is_err() {
        crate::metrics::record_readiness_failure(crate::metrics::ReadinessFailure::Store);
        return error(StatusCode::SERVICE_UNAVAILABLE, "not_ready");
    }
    Json(serde_json::json!({ "status": "ready" })).into_response()
}

pub fn router(state: AppState) -> (Router, Router) {
    router_with_metrics(state, None)
}

/// Build the public and private routers. When `metrics_handle` is provided,
/// the private router additionally serves `GET /metrics`. Metrics live only on
/// the private router, never on the public port.
pub fn router_with_metrics(
    state: AppState,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
) -> (Router, Router) {
    let public = Router::new()
        .merge(crate::intake::routes())
        .layer(RequestBodyLimitLayer::new(state.intake.max_body_bytes))
        .with_state(state.clone());
    let mut private = Router::new()
        .merge(crate::management::routes())
        .layer(RequestBodyLimitLayer::new(
            crate::management::MAX_MANAGEMENT_BODY_BYTES,
        ))
        .route("/_liveness", get(live))
        .route("/_readiness", get(ready))
        .with_state(state);
    if let Some(handle) = metrics_handle {
        private = private.route(
            "/metrics",
            get(move || {
                let handle = handle.clone();
                async move {
                    handle.run_upkeep();
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        handle.render(),
                    )
                }
            }),
        );
    }
    (public, private)
}
