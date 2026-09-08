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
//! recorded by [`track_metrics`] middleware on the app router. Buzz-specific
//! metrics are recorded inline at their call sites.

use std::time::{Duration, Instant};

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder};
use metrics_util::MetricKindMask;

/// HTTP latency buckets (milliseconds) — only for `http_request_latency_ms`.
const LATENCY_BUCKETS_MS: [f64; 11] = [
    5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
];

/// Seconds-scale buckets for internal processing histograms (event, search, audit).
const DURATION_BUCKETS_S: [f64; 10] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0];

/// Readiness buckets concentrate resolution near the two-second failure budget.
const READINESS_DURATION_BUCKETS_S: [f64; 15] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5,
];

/// Pool checkout buckets: dense around normal sub-100ms waits, with explicit
/// coverage of the reader's 150ms and writer's default three-second budgets.
const DB_POOL_ACQUIRE_DURATION_BUCKETS_S: [f64; 9] =
    [0.001, 0.005, 0.01, 0.025, 0.05, 0.15, 0.5, 1.0, 3.0];
const DB_POOL_ACQUIRE_DURATION_UNIT: metrics::Unit = metrics::Unit::Seconds;

/// Seconds-scale buckets for Git hydration and pack streams.
const GIT_DURATION_BUCKETS_S: [f64; 13] = [
    0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
];

/// Byte buckets for hydrated repositories and streamed clone/fetch responses.
const GIT_BYTES_BUCKETS: [f64; 9] = [
    0.0,
    64.0 * 1024.0,
    1024.0 * 1024.0,
    10.0 * 1024.0 * 1024.0,
    50.0 * 1024.0 * 1024.0,
    100.0 * 1024.0 * 1024.0,
    250.0 * 1024.0 * 1024.0,
    500.0 * 1024.0 * 1024.0,
    1024.0 * 1024.0 * 1024.0,
];

/// Pack-count buckets bounded by the manifest's maximum pack count.
const GIT_PACK_BUCKETS: [f64; 9] = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

/// Integer-count buckets for fan-out recipient histograms.
const FANOUT_BUCKETS: [f64; 9] = [0.0, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0, 1000.0];

fn configured_prometheus_builder(gauge_idle_timeout_secs: u64) -> PrometheusBuilder {
    PrometheusBuilder::new()
        // Remove gauge series that the relay intentionally stops emitting.
        .idle_timeout(
            MetricKindMask::GAUGE,
            Some(Duration::from_secs(gauge_idle_timeout_secs)),
        )
        // Per-metric buckets: ms for HTTP latency, seconds for internal processing.
        .set_buckets_for_metric(
            Matcher::Full("http_request_latency_ms".to_owned()),
            &LATENCY_BUCKETS_MS,
        )
        .expect("valid ms bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_hydrate_seconds".to_owned()),
            &GIT_DURATION_BUCKETS_S,
        )
        .expect("valid git hydration duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_upload_pack_stream_seconds".to_owned()),
            &GIT_DURATION_BUCKETS_S,
        )
        .expect("valid git stream duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_cache_populate_seconds".to_owned()),
            &GIT_DURATION_BUCKETS_S,
        )
        .expect("valid git cache population duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_cache_population_wait_seconds".to_owned()),
            &GIT_DURATION_BUCKETS_S,
        )
        .expect("valid git cache population wait bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_compaction_seconds".to_owned()),
            &GIT_DURATION_BUCKETS_S,
        )
        .expect("valid git compaction duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_readiness_check_duration_seconds".to_owned()),
            &READINESS_DURATION_BUCKETS_S,
        )
        .expect("valid readiness duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_db_pool_acquire_duration_seconds".to_owned()),
            &DB_POOL_ACQUIRE_DURATION_BUCKETS_S,
        )
        .expect("valid DB pool acquisition duration bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_hydrate_bytes".to_owned()),
            &GIT_BYTES_BUCKETS,
        )
        .expect("valid git hydration byte bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_upload_pack_stream_bytes".to_owned()),
            &GIT_BYTES_BUCKETS,
        )
        .expect("valid git stream byte bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_compaction_bytes".to_owned()),
            &GIT_BYTES_BUCKETS,
        )
        .expect("valid git compaction byte bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_hydrate_packs".to_owned()),
            &GIT_PACK_BUCKETS,
        )
        .expect("valid git pack-count bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_compaction_packs_before".to_owned()),
            &GIT_PACK_BUCKETS,
        )
        .expect("valid git compaction input pack-count bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_git_pack_compaction_packs_after".to_owned()),
            &GIT_PACK_BUCKETS,
        )
        .expect("valid git compaction output pack-count bucket boundaries")
        .set_buckets_for_metric(Matcher::Suffix("_seconds".to_owned()), &DURATION_BUCKETS_S)
        .expect("valid seconds bucket boundaries")
        .set_buckets_for_metric(
            Matcher::Full("buzz_fanout_recipients".to_owned()),
            &FANOUT_BUCKETS,
        )
        .expect("valid fanout bucket boundaries")
}

/// A bounded class of metrics installation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricsInstallFailure {
    /// The Prometheus listener could not bind.
    Bind,
    /// Another component already installed a global recorder.
    RecorderConflict,
    /// The exporter could not be built for another reason.
    ExporterBuild,
}

/// An error returned while installing Prometheus metrics.
#[derive(Debug, thiserror::Error)]
pub enum MetricsInstallError {
    /// Prometheus exporter construction failed.
    #[error("failed to build Prometheus exporter: {0}")]
    Build(#[source] BuildError),
    /// Another component already installed the process-global recorder.
    #[error("the global metrics recorder is already installed")]
    RecorderConflict,
}

impl MetricsInstallError {
    /// Return the secret-safe lifecycle classification.
    pub const fn failure(&self) -> MetricsInstallFailure {
        match self {
            Self::Build(BuildError::FailedToCreateHTTPListener(_)) => MetricsInstallFailure::Bind,
            Self::Build(_) => MetricsInstallFailure::ExporterBuild,
            Self::RecorderConflict => MetricsInstallFailure::RecorderConflict,
        }
    }
}

/// Try to install the global metrics recorder and spawn the Prometheus HTTP exporter.
///
/// `build()` returns the recorder + exporter future and internally spawns
/// the upkeep task, so no separate upkeep call is needed.
///
/// Must be called from within a Tokio runtime.
/// Listener and global-recorder failures are returned rather than panicking.
/// A later exporter exit remains detached from relay service; external scrape
/// coverage is authoritative for exporter availability.
pub fn try_install(port: u16, gauge_idle_timeout_secs: u64) -> Result<(), MetricsInstallError> {
    let (recorder, exporter) = configured_prometheus_builder(gauge_idle_timeout_secs)
        .with_http_listener(([0, 0, 0, 0], port))
        .build()
        .map_err(MetricsInstallError::Build)?;

    metrics::set_global_recorder(recorder)
        .map_err(|_error| MetricsInstallError::RecorderConflict)?;
    describe_readiness_metrics();
    describe_db_pool_metrics();
    tokio::spawn(exporter);
    Ok(())
}

/// Install the global metrics recorder and spawn the Prometheus HTTP exporter.
///
/// This compatibility entry point preserves the original panic-on-failure API.
/// New startup code should use [`try_install`] to report typed failures.
pub fn install(port: u16, gauge_idle_timeout_secs: u64) {
    try_install(port, gauge_idle_timeout_secs)
        .unwrap_or_else(|error| panic!("metrics exporter must install exactly once: {error}"));
}

/// Register the frozen readiness metric descriptions with the active recorder.
pub(crate) fn describe_readiness_metrics() {
    metrics::describe_counter!(
        "buzz_readiness_checks_total",
        "Kubernetes health-listener readiness probes by terminal bounded reason"
    );
    metrics::describe_counter!(
        "buzz_readiness_dependency_checks_total",
        "Completed readiness dependency attempts by dependency and bounded outcome"
    );
    metrics::describe_histogram!(
        "buzz_readiness_check_duration_seconds",
        metrics::Unit::Seconds,
        "Completed readiness check duration without outcome label multiplication"
    );
    metrics::describe_gauge!(
        "buzz_readiness_state",
        "Latest publishable readiness state by check, where 1 is ready and 0 is not ready"
    );
}

/// Register the frozen operation-aware pool-acquisition contract.
pub(crate) fn describe_db_pool_metrics() {
    metrics::describe_histogram!(
        "buzz_db_pool_acquire_duration_seconds",
        DB_POOL_ACQUIRE_DURATION_UNIT,
        "Database pool checkout duration by valid pool role and operation"
    );
    metrics::describe_counter!(
        "buzz_db_pool_acquire_attempts_total",
        "Database pool checkout terminals by valid pool role, operation, and outcome"
    );
    metrics::describe_gauge!(
        "buzz_db_pool_waiters",
        "Current tracked-operation database pool checkout attempts in progress by valid pool role and operation"
    );
}

#[cfg(test)]
pub(crate) fn readiness_test_recorder() -> (
    metrics_exporter_prometheus::PrometheusRecorder,
    metrics_exporter_prometheus::PrometheusHandle,
) {
    let recorder = configured_prometheus_builder(300).build_recorder();
    let handle = recorder.handle();
    (recorder, handle)
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
#[cfg(test)]
mod contract_tests {
    use std::collections::BTreeSet;

    const OUTCOMES: [&str; 4] = ["success", "timeout", "error", "cancelled"];

    fn label_keys(line: &str) -> BTreeSet<&str> {
        line.split_once('{')
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(labels, _)| {
                labels
                    .split(',')
                    .filter_map(|label| label.split_once('=').map(|(key, _)| key))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn production_builder_exports_frozen_db_pool_contract_and_187_series_budget() {
        let (recorder, handle) = super::readiness_test_recorder();
        metrics::with_local_recorder(&recorder, || {
            super::describe_db_pool_metrics();
            for (pool_role, operation) in buzz_db::DB_POOL_ACQUIRE_VALID_PAIRS {
                metrics::histogram!(
                    "buzz_db_pool_acquire_duration_seconds",
                    "pool_role" => pool_role,
                    "operation" => operation,
                )
                .record(0.02);
                metrics::gauge!(
                    "buzz_db_pool_waiters",
                    "pool_role" => pool_role,
                    "operation" => operation,
                )
                .set(0.0);
                for outcome in OUTCOMES {
                    metrics::counter!(
                        "buzz_db_pool_acquire_attempts_total",
                        "pool_role" => pool_role,
                        "operation" => operation,
                        "outcome" => outcome,
                    )
                    .increment(1);
                }
            }
        });

        let scrape = handle.render();
        assert!(scrape.contains("# TYPE buzz_db_pool_acquire_duration_seconds histogram"));
        assert!(scrape.contains("# TYPE buzz_db_pool_acquire_attempts_total counter"));
        assert!(scrape.contains("# TYPE buzz_db_pool_waiters gauge"));
        assert!(scrape.contains("# HELP buzz_db_pool_acquire_duration_seconds Database pool checkout duration by valid pool role and operation"));
        assert!(scrape.contains("# HELP buzz_db_pool_acquire_attempts_total Database pool checkout terminals by valid pool role, operation, and outcome"));
        assert!(scrape.contains("# HELP buzz_db_pool_waiters Current tracked-operation database pool checkout attempts in progress by valid pool role and operation"));
        assert_eq!(super::DB_POOL_ACQUIRE_DURATION_UNIT, metrics::Unit::Seconds);
        let readiness_buckets = scrape
            .lines()
            .filter(|line| {
                line.starts_with("buzz_db_pool_acquire_duration_seconds_bucket{")
                    && line.contains("pool_role=\"writer\"")
                    && line.contains("operation=\"readiness\"")
            })
            .map(|line| {
                line.split(",le=\"")
                    .nth(1)
                    .and_then(|rest| rest.split_once('"').map(|(bucket, _)| bucket))
                    .expect("duration bucket carries le label")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            readiness_buckets,
            ["0.001", "0.005", "0.01", "0.025", "0.05", "0.15", "0.5", "1", "3", "+Inf",],
            "duration bucket contract drifted:\n{scrape}"
        );

        let raw_series = scrape
            .lines()
            .filter(|line| {
                line.starts_with("buzz_db_pool_acquire_duration_seconds")
                    || line.starts_with("buzz_db_pool_acquire_attempts_total")
                    || line.starts_with("buzz_db_pool_waiters{")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            raw_series.len(),
            buzz_db::DB_POOL_ACQUIRE_RAW_SERIES_PER_POD,
            "unexpected raw scrape:\n{scrape}"
        );

        for line in raw_series {
            let keys = label_keys(line);
            if line.starts_with("buzz_db_pool_acquire_duration_seconds_bucket") {
                assert_eq!(keys, BTreeSet::from(["le", "operation", "pool_role"]));
            } else if line.starts_with("buzz_db_pool_acquire_duration_seconds") {
                assert_eq!(keys, BTreeSet::from(["operation", "pool_role"]));
            } else if line.starts_with("buzz_db_pool_acquire_attempts_total") {
                assert_eq!(keys, BTreeSet::from(["operation", "outcome", "pool_role"]));
            } else {
                assert_eq!(keys, BTreeSet::from(["operation", "pool_role"]));
            }
            assert!(!line.contains("operation=\"other\""));
            assert!(!line.contains("result="));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn occupied_listener_is_classified_as_bind() {
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("bind occupied port");
        let port = listener.local_addr().expect("occupied address").port();
        let error = try_install(port, 300).expect_err("occupied listener must fail");
        assert_eq!(error.failure(), MetricsInstallFailure::Bind);
    }

    #[tokio::test]
    async fn recorder_conflict_is_typed_in_an_isolated_process() {
        const CHILD_ENV: &str = "BUZZ_TEST_METRICS_RECORDER_CONFLICT";
        if std::env::var_os(CHILD_ENV).is_some() {
            let recorder = configured_prometheus_builder(300).build_recorder();
            metrics::set_global_recorder(recorder).expect("install first recorder");
            let error = try_install(0, 300).expect_err("second recorder must fail");
            assert_eq!(error.failure(), MetricsInstallFailure::RecorderConflict);
            return;
        }

        crate::test_support::run_exact_test_child(
            "metrics::tests::recorder_conflict_is_typed_in_an_isolated_process",
            CHILD_ENV,
        );
    }
}
