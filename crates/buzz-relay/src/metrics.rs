//! Prometheus metrics: recorder setup, upkeep task, and HTTP middleware.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │  metrics-rs facade (metrics::counter!, histogram!, etc.) │
//! │         ↓                                                │
//! │  PrometheusBuilder → HTTP listener on :9102              │
//! │         ↓                                                │
//! │  GET /metrics → Prometheus text format                   │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! Framework metrics (`http_requests_total`, `http_request_latency_ms`) are
//! recorded by [`track_metrics`] middleware on the app router. Sprout-specific
//! metrics are recorded inline at their call sites.

use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};

/// HTTP latency buckets (milliseconds) — only for `http_request_latency_ms`.
const LATENCY_BUCKETS_MS: [f64; 11] = [
    5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
];

/// Seconds-scale buckets for internal processing histograms (event, search, audit).
const DURATION_BUCKETS_S: [f64; 10] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0];

/// Integer-count buckets for fan-out recipient histograms.
const FANOUT_BUCKETS: [f64; 9] = [0.0, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0, 1000.0];

/// Install the global metrics recorder and spawn the Prometheus HTTP exporter.
///
/// `build()` returns the recorder + exporter future and internally spawns
/// the upkeep task, so no separate upkeep call is needed.
///
/// Must be called from within a Tokio runtime.
/// Panics if a recorder is already installed or the port is in use.
pub fn install(port: u16) {
    let (recorder, exporter) = PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], port))
        // Per-metric buckets: ms for HTTP latency, seconds for internal processing.
        .set_buckets_for_metric(
            Matcher::Full("http_request_latency_ms".to_owned()),
            &LATENCY_BUCKETS_MS,
        )
        .expect("valid ms bucket boundaries")
        .set_buckets_for_metric(Matcher::Suffix("_seconds".to_owned()), &DURATION_BUCKETS_S)
        .expect("valid seconds bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("sprout_fanout_recipients".to_owned()),
            &FANOUT_BUCKETS,
        )
        .expect("valid fanout bucket boundaries")
        .build()
        .expect("metrics exporter must build exactly once");

    metrics::set_global_recorder(recorder).expect("global recorder must be set exactly once");
    tokio::spawn(exporter);
}

/// Axum middleware that records CAKE framework HTTP metrics.
///
/// Emits:
/// - `http_requests_total{code, caller, action}` — counter
/// - `http_request_latency_ms{code, caller, action}` — histogram
///
/// Skips health/metrics paths (`/_*`, `/health`) to avoid polluting dashboards.
///
/// Labels:
/// - `code`: exact HTTP status code (e.g. "200", "404")
/// - `caller`: upstream service from Istio `x-envoy-downstream-service-cluster` header
/// - `action`: matched route pattern (e.g. `/api/channels/{channel_id}`)
pub async fn track_metrics(req: Request, next: Next) -> Response {
    // Use the route pattern (e.g. "/api/channels/{channel_id}"), NOT the raw URI.
    // Falling back to raw URI on 404s would create unbounded cardinality from scanners.
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned());

    // Skip health probes, metrics endpoint, and unmatched paths (404 scanners).
    match path.as_deref() {
        Some(p) if p.starts_with("/_") || p == "/health" || p == "/metrics" => {
            return next.run(req).await;
        }
        None => {
            // No matched route — 404/scanner traffic. Skip to avoid cardinality bomb.
            return next.run(req).await;
        }
        _ => {}
    }
    let action = path.unwrap(); // safe: None case returned above

    // Caller from Istio header. In CAKE, this is set by the mesh (trusted).
    // On the public TCP listener it's client-controlled, so validate format:
    // only accept short alphanumeric-with-hyphens service names.
    let caller = req
        .headers()
        .get("x-envoy-downstream-service-cluster")
        .and_then(|v| v.to_str().ok())
        .filter(|s| {
            s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .unwrap_or("unknown")
        .to_owned();

    let start = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    let labels = [("code", status), ("caller", caller), ("action", action)];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_latency_ms", &labels).record(latency_ms);

    response
}
