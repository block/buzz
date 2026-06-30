//! OpenTelemetry metrics: provider setup, instrument handles, and HTTP middleware.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │  OTEL Meter API via global Metrics struct (pre-built instrument      │
//! │  handles — one allocation at startup, zero per call-site)            │
//! │          ↓                                                            │
//! │  SdkMeterProvider                                                     │
//! │     ├── PrometheusExporter → prometheus::Registry → HTTP :9102        │
//! │     └── (if OTEL_EXPORTER_OTLP_ENDPOINT set)                          │
//! │         PeriodicReader + OTLP MetricExporter → collector/DD agent     │
//! └──────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! All metric names, types, and label sets are preserved from the prior
//! `metrics-rs` implementation so existing Prometheus scrapers and Datadog
//! dashboards need no changes.
//!
//! Custom histogram bucket boundaries are registered as OTEL Views before the
//! provider is built.

use std::sync::OnceLock;
use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use opentelemetry::{
    metrics::{Counter, Histogram, Meter, UpDownCounter},
    KeyValue,
};
use opentelemetry_sdk::metrics::{Aggregation, Instrument, PeriodicReader, SdkMeterProvider, Stream};
use prometheus::Registry;

// ─── Bucket boundaries ───────────────────────────────────────────────────────

/// HTTP latency buckets (milliseconds) — only for `http_request_latency_ms`.
pub const LATENCY_BUCKETS_MS: &[f64] = &[
    5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
];

/// Seconds-scale buckets for internal processing histograms (event, search, audit).
pub const DURATION_BUCKETS_S: &[f64] =
    &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0];

/// Integer-count buckets for fan-out recipient histograms.
pub const FANOUT_BUCKETS: &[f64] = &[0.0, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0, 1000.0];

/// Scope name used when obtaining meters.
pub const METER_SCOPE: &str = "buzz-relay";

// ─── Global instrument handles ────────────────────────────────────────────────

/// Pre-built OTEL instrument handles.  One global instance; zero per-call-site
/// allocation.  Initialized by [`install`]; panics if accessed before that.
#[allow(missing_docs)] // field names are self-documenting metric names
pub struct Metrics {
    // HTTP framework
    pub http_requests_total: Counter<u64>,
    pub http_request_latency_ms: Histogram<f64>,

    // WebSocket connections
    pub ws_connections_total: Counter<u64>,
    pub ws_connections_active: UpDownCounter<i64>,
    pub ws_backpressure_disconnects_total: Counter<u64>,
    pub ws_auth_timeouts_total: Counter<u64>,

    // Subscriptions
    pub subscriptions_active: UpDownCounter<i64>,

    // Events
    pub events_received_total: Counter<u64>,
    pub events_stored_total: Counter<u64>,
    pub events_rejected_total: Counter<u64>,
    pub event_processing_seconds: Histogram<f64>,

    // Fan-out
    pub fanout_recipients: Histogram<f64>,
    pub multinode_fanout_total: Counter<u64>,
    pub multinode_fanout_lag_total: Counter<u64>,
    pub cache_invalidation_lag_total: Counter<u64>,

    // Search
    pub search_index_seconds: Histogram<f64>,
    pub search_index_errors_total: Counter<u64>,

    // Audit
    pub audit_log_seconds: Histogram<f64>,
    pub audit_log_errors_total: Counter<u64>,
    pub audit_send_errors_total: Counter<u64>,

    // Auth
    pub auth_attempts_total: Counter<u64>,
    pub auth_failures_total: Counter<u64>,

    // Media
    pub media_uploads_total: Counter<u64>,
    pub media_upload_rejections_total: Counter<u64>,

    // Workflows
    pub workflow_runs_total: Counter<u64>,

    // Cache
    pub membership_cache_hits_total: Counter<u64>,
    pub membership_cache_misses_total: Counter<u64>,
    pub accessible_channels_cache_hits_total: Counter<u64>,
    pub accessible_channels_cache_misses_total: Counter<u64>,

    // Count fallback
    pub count_fallback_rejections_total: Counter<u64>,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

/// Access the global [`Metrics`] instance.
///
/// If [`install`] has not yet been called (e.g. in unit tests that don't
/// start a full relay), returns a lazily-initialised set of no-op instruments
/// built from the global meter (which is a no-op provider by default).
/// This matches the prior `metrics-rs` behaviour where macros silently
/// did nothing without explicit initialisation.
pub fn metrics() -> &'static Metrics {
    METRICS.get_or_init(build_metrics)
}

/// Build the [`Metrics`] instrument handles from the current global meter.
///
/// Called once — either from [`install`] (real provider set) or lazily from
/// [`metrics`] (falls back to OTEL's built-in no-op meter).
fn build_metrics() -> Metrics {
    let m = opentelemetry::global::meter(METER_SCOPE);
    Metrics {
        http_requests_total: m.u64_counter("http_requests_total").build(),
        http_request_latency_ms: m.f64_histogram("http_request_latency_ms").build(),

        ws_connections_total: m.u64_counter("buzz_ws_connections_total").build(),
        ws_connections_active: m.i64_up_down_counter("buzz_ws_connections_active").build(),
        ws_backpressure_disconnects_total: m
            .u64_counter("buzz_ws_backpressure_disconnects_total")
            .build(),
        ws_auth_timeouts_total: m.u64_counter("buzz_ws_auth_timeouts_total").build(),

        subscriptions_active: m.i64_up_down_counter("buzz_subscriptions_active").build(),

        events_received_total: m.u64_counter("buzz_events_received_total").build(),
        events_stored_total: m.u64_counter("buzz_events_stored_total").build(),
        events_rejected_total: m.u64_counter("buzz_events_rejected_total").build(),
        event_processing_seconds: m.f64_histogram("buzz_event_processing_seconds").build(),

        fanout_recipients: m.f64_histogram("buzz_fanout_recipients").build(),
        multinode_fanout_total: m.u64_counter("buzz_multinode_fanout_total").build(),
        multinode_fanout_lag_total: m.u64_counter("buzz_multinode_fanout_lag_total").build(),
        cache_invalidation_lag_total: m
            .u64_counter("buzz_cache_invalidation_lag_total")
            .build(),

        search_index_seconds: m.f64_histogram("buzz_search_index_seconds").build(),
        search_index_errors_total: m.u64_counter("buzz_search_index_errors_total").build(),

        audit_log_seconds: m.f64_histogram("buzz_audit_log_seconds").build(),
        audit_log_errors_total: m.u64_counter("buzz_audit_log_errors_total").build(),
        audit_send_errors_total: m.u64_counter("buzz_audit_send_errors_total").build(),

        auth_attempts_total: m.u64_counter("buzz_auth_attempts_total").build(),
        auth_failures_total: m.u64_counter("buzz_auth_failures_total").build(),

        media_uploads_total: m.u64_counter("buzz_media_uploads_total").build(),
        media_upload_rejections_total: m
            .u64_counter("buzz_media_upload_rejections_total")
            .build(),

        workflow_runs_total: m.u64_counter("buzz_workflow_runs_total").build(),

        membership_cache_hits_total: m.u64_counter("buzz_membership_cache_hits_total").build(),
        membership_cache_misses_total: m
            .u64_counter("buzz_membership_cache_misses_total")
            .build(),
        accessible_channels_cache_hits_total: m
            .u64_counter("buzz_accessible_channels_cache_hits_total")
            .build(),
        accessible_channels_cache_misses_total: m
            .u64_counter("buzz_accessible_channels_cache_misses_total")
            .build(),

        count_fallback_rejections_total: m
            .u64_counter("buzz_count_fallback_rejections_total")
            .build(),
    }
}

/// Returns a [`Meter`] scoped to the relay.  Useful for one-off or dynamic
/// instruments (e.g. pool gauge task).
pub fn meter() -> Meter {
    opentelemetry::global::meter(METER_SCOPE)
}

// ─── Provider setup ───────────────────────────────────────────────────────────

/// Install the global OTEL meter provider and spawn the Prometheus HTTP exporter.
///
/// Returns the [`SdkMeterProvider`] so the caller can shut it down gracefully
/// on SIGTERM.
///
/// If `OTEL_EXPORTER_OTLP_ENDPOINT` is set, a second reader is attached that
/// pushes metrics via OTLP gRPC on the configured interval (default 60 s).
///
/// # Panics
/// Panics if called more than once or if the HTTP listener cannot bind to `port`.
pub fn install(port: u16) -> SdkMeterProvider {
    let registry = prometheus::Registry::new();

    // Build the Prometheus exporter (pull-based: no periodic push needed).
    let prom_exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        // Don't add unit suffixes — keep names identical to the old metrics-rs names.
        .without_units()
        // Don't add `_total` suffix — our counter names already end in `_total`.
        .without_counter_suffixes()
        // otel_scope_* labels add noise; the relay has a single scope.
        .without_scope_info()
        .build()
        .expect("Prometheus exporter must build exactly once");

    let mut provider_builder = SdkMeterProvider::builder()
        .with_reader(prom_exporter)
        .with_view(explicit_bucket_view);

    // Attach OTLP metric exporter only when the endpoint env var is set.
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        match opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()
        {
            Ok(exporter) => {
                let periodic = PeriodicReader::builder(exporter).build();
                provider_builder = provider_builder.with_reader(periodic);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to build OTLP metric exporter; OTLP metrics disabled");
            }
        }
    }

    let provider = provider_builder.build();

    // Store globally so `global::meter(...)` works throughout the codebase.
    // Must happen BEFORE build_metrics() so the real provider backs all handles.
    opentelemetry::global::set_meter_provider(provider.clone());

    // Build all instrument handles from the now-installed global meter.
    // `get_or_init` is safe here: in production `install()` runs before any
    // metric emission, so this is always the first init.  If somehow called
    // after a lazy noop init in tests, the noop handles are kept (acceptable
    // for test isolation — the Prometheus endpoint test calls install() first).
    METRICS.get_or_init(build_metrics);

    // Spawn the Prometheus HTTP listener on the metrics port.
    tokio::spawn(serve_prometheus(port, registry));

    provider
}

// ─── View ─────────────────────────────────────────────────────────────────────

/// OTEL View mapping named histograms to their explicit bucket boundaries.
///
/// Any histogram whose name doesn't match uses the SDK default buckets.
fn explicit_bucket_view(inst: &Instrument) -> Option<Stream> {
    let boundaries: &[f64] = if inst.name() == "http_request_latency_ms" {
        LATENCY_BUCKETS_MS
    } else if inst.name() == "buzz_event_processing_seconds"
        || inst.name() == "buzz_search_index_seconds"
        || inst.name() == "buzz_audit_log_seconds"
    {
        DURATION_BUCKETS_S
    } else if inst.name() == "buzz_fanout_recipients" {
        FANOUT_BUCKETS
    } else {
        return None; // use SDK default
    };

    Stream::builder()
        .with_aggregation(Aggregation::ExplicitBucketHistogram {
            boundaries: boundaries.to_vec(),
            record_min_max: false,
        })
        .build()
        .ok()
}

// ─── Prometheus HTTP endpoint ─────────────────────────────────────────────────

/// Serve the Prometheus `/metrics` endpoint on a bare TCP listener.
///
/// This is intentionally minimal — no middleware, no auth. Port access controls
/// are expected to be enforced at the network/mesh level (Istio excludes this
/// port from the mesh by default in the Blox deployment).
async fn serve_prometheus(port: u16, registry: Registry) {
    use axum::{routing::get, Router};
    use prometheus::{Encoder, TextEncoder};
    use std::net::SocketAddr;

    let app = Router::new().route(
        "/metrics",
        get(move || {
            let reg = registry.clone();
            async move {
                let encoder = TextEncoder::new();
                let families = reg.gather();
                let mut buf = Vec::new();
                encoder
                    .encode(&families, &mut buf)
                    .expect("encode prometheus metrics");
                let content_type = encoder.format_type().to_owned();
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        content_type,
                    )],
                    buf,
                )
            }
        }),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("metrics listener failed to bind :{port}: {e}"));
    axum::serve(listener, app).await.ok();
}

// ─── Cardinality control ──────────────────────────────────────────────────────

/// Map arbitrary Nostr event kinds to a bounded label value.
///
/// The allow-list matches the metrics observed in production; all other kinds
/// collapse to `"other"` to avoid cardinality explosion.
pub fn bounded_kind_label(kind: u16) -> &'static str {
    match kind {
        0 => "0",         // NIP-01 metadata
        1 => "1",         // short text note
        3 => "3",         // contact list
        4 => "4",         // encrypted DM
        5 => "5",         // deletion
        6 => "6",         // repost
        7 => "7",         // reaction
        40 => "40",       // channel create
        41 => "41",       // channel metadata
        42 => "42",       // channel message
        43 => "43",       // channel hide
        44 => "44",       // channel mute
        1059 => "1059",   // NIP-44 gift wrap
        1984 => "1984",   // report
        9734 => "9734",   // zap request
        9735 => "9735",   // zap
        10000 => "10000", // mute list
        10001 => "10001", // pin list
        _ => "other",
    }
}

// ─── HTTP metrics middleware ───────────────────────────────────────────────────

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

    let labels = [
        KeyValue::new("code", status),
        KeyValue::new("caller", caller),
        KeyValue::new("action", action),
    ];
    let m = metrics();
    m.http_requests_total.add(1, &labels);
    m.http_request_latency_ms.record(latency_ms, &labels);

    response
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_kind_label_known_kinds_are_stable() {
        assert_eq!(bounded_kind_label(1), "1");
        assert_eq!(bounded_kind_label(42), "42");
        assert_eq!(bounded_kind_label(9735), "9735");
    }

    #[test]
    fn bounded_kind_label_unknown_collapses_to_other() {
        assert_eq!(bounded_kind_label(12345), "other");
        assert_eq!(bounded_kind_label(0xFFFF), "other");
    }

    /// Verify that the Prometheus HTTP endpoint returns 200 and includes
    /// known metric names after installation.
    ///
    /// This test builds its own isolated Prometheus registry and meter
    /// provider (not the global one) so it is fully independent of any
    /// other test's metric state.
    #[tokio::test]
    async fn prometheus_endpoint_serves_known_metric_names() {
        use opentelemetry_sdk::metrics::SdkMeterProvider;
        use prometheus::Registry;

        // 1. Build an isolated registry + OTEL provider.
        let registry = Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .without_units()
            .without_counter_suffixes()
            .without_scope_info()
            .build()
            .expect("build test prometheus exporter");

        let provider = SdkMeterProvider::builder()
            .with_reader(exporter)
            .build();

        // 2. Create a meter from this isolated provider (not the global one).
        let meter = opentelemetry::metrics::MeterProvider::meter(&provider, METER_SCOPE);

        // 3. Build and record values for a sample of the relay's named instruments.
        //    These must appear in the Prometheus output.
        let ws_conn = meter.u64_counter("buzz_ws_connections_total").build();
        let events_recv = meter.u64_counter("buzz_events_received_total").build();
        let events_stored = meter.u64_counter("buzz_events_stored_total").build();
        let auth_attempts = meter.u64_counter("buzz_auth_attempts_total").build();

        ws_conn.add(1, &[]);
        events_recv.add(1, &[KeyValue::new("kind", "1")]);
        events_stored.add(1, &[KeyValue::new("kind", "42")]);
        auth_attempts.add(1, &[KeyValue::new("method", "nip42")]);

        // 4. Bind port 0 → let OS pick a free port, then release so the
        //    listener can bind it.  (Tiny TOCTOU gap acceptable in tests.)
        let port = {
            let sock = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
            sock.local_addr().expect("local_addr").port()
        };

        // 5. Spawn the Prometheus HTTP server with the isolated registry.
        tokio::spawn(serve_prometheus(port, registry));

        // Give the listener a moment to finish binding.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 6. Fetch /metrics and verify the expected names are present.
        let url = format!("http://127.0.0.1:{port}/metrics");
        let body = reqwest::get(&url)
            .await
            .expect("GET /metrics")
            .error_for_status()
            .expect("HTTP 200 from /metrics")
            .text()
            .await
            .expect("read response body");

        for expected in &[
            "buzz_ws_connections_total",
            "buzz_events_received_total",
            "buzz_events_stored_total",
            "buzz_auth_attempts_total",
        ] {
            assert!(
                body.contains(expected),
                "/metrics body is missing '{expected}';\nbody:\n{body}",
            );
        }

        // Keep the provider alive until the assertions complete so metrics
        // aren't flushed/dropped before the HTTP response arrives.
        drop(provider);
    }
}
