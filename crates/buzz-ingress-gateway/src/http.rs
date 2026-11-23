//! HTTP surface: the public webhook listener and the private router that
//! serves the management API, health probes, and metrics.
use crate::postgres::PostgresStore;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<PostgresStore>,
    /// Cleared on shutdown so readiness fails before listeners drain.
    pub accepting: Arc<AtomicBool>,
}

fn error(status: StatusCode, code: &'static str) -> Response {
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
    let public = Router::new().with_state(state.clone());
    let mut private = Router::new()
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
