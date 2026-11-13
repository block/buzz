//! Sanitized, bounded-cardinality Prometheus metrics for the push gateway.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │  metrics-rs facade (metrics::counter!, histogram!)        │
//! │         ↓                                                 │
//! │  PrometheusBuilder::install_recorder() → PrometheusHandle │
//! │         ↓                                                 │
//! │  GET /metrics on the PRIVATE health router (port 8081)    │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! Every label value emitted here is a compile-time `&'static str` drawn from a
//! closed set (the [`DeliveryOutcome`] variants, the gateway's fixed error
//! codes, and the handler stages). No endpoint, device token, relay pubkey,
//! request id, or any other request-scoped identifier is ever used as a label,
//! so metric cardinality is structurally bounded regardless of traffic.

use crate::apns::DeliveryOutcome;
use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder, PrometheusHandle};

/// Seconds-scale buckets for the APNs send round-trip histogram.
const APNS_LATENCY_BUCKETS_S: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 15.0,
];

/// Seconds-scale buckets for Postgres pool checkout waits. The top bucket
/// sits at sqlx's default 30-second acquire timeout.
const DB_POOL_ACQUIRE_WAIT_BUCKETS_S: [f64; 13] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// The exporter configuration shared by the process recorder and tests.
fn builder() -> Result<PrometheusBuilder, BuildError> {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("push_gateway_apns_delivery_seconds".to_owned()),
            &APNS_LATENCY_BUCKETS_S,
        )?
        .set_buckets_for_metric(
            Matcher::Full("push_gateway_db_pool_acquire_wait_seconds".to_owned()),
            &DB_POOL_ACQUIRE_WAIT_BUCKETS_S,
        )
}

/// Install the global metrics recorder and return the render handle.
///
/// Unlike the relay's exporter, this installs **no** HTTP listener: rendering is
/// served from the private health router so metrics never share the public port.
/// Must be called at most once per process, from within a Tokio runtime.
pub fn install() -> Result<PrometheusHandle, BuildError> {
    let handle = builder()?.install_recorder()?;
    // Publish the waiter gauge before any checkout so a quiet pod scrapes an
    // explicit zero rather than a missing series.
    metrics::gauge!("push_gateway_db_pool_waiters").set(0.0);
    Ok(handle)
}

/// Stable metric label for each sanitized delivery outcome. The mapping is total
/// over the closed [`DeliveryOutcome`] enum, so the `outcome` label can only take
/// these five values.
fn outcome_label(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Accepted => "accepted",
        DeliveryOutcome::InvalidEndpoint { .. } => "invalid_endpoint",
        DeliveryOutcome::Retry { .. } => "retry",
        DeliveryOutcome::ConfigurationFault => "configuration_fault",
        DeliveryOutcome::PermanentRequestFault => "permanent_request_fault",
    }
}

/// Record entry into the concrete APNs HTTP send seam. This counter is kept
/// separate from terminal outcomes so a control scrape can distinguish
/// "transport never reached" from "APNs send returned an error".
pub fn record_apns_send_attempt() {
    metrics::counter!("push_gateway_apns_send_attempts_total").increment(1);
}

/// Record the terminal APNs outcome and its send round-trip latency.
pub fn record_apns_delivery(outcome: DeliveryOutcome, seconds: f64) {
    metrics::counter!("push_gateway_apns_deliveries_total", "outcome" => outcome_label(outcome))
        .increment(1);
    metrics::histogram!("push_gateway_apns_delivery_seconds").record(seconds);
}

/// Delivery-admission result at the `authorize_delivery` seam.
#[derive(Debug, Clone, Copy)]
pub enum Admission {
    /// A delivery permit was issued.
    Admitted,
    /// The replay/quota/authority fence rejected the request.
    Rejected,
    /// The authority store was transiently unavailable.
    Unavailable,
}

/// Record the outcome of a delivery-admission attempt.
pub fn record_admission(result: Admission) {
    let label = match result {
        Admission::Admitted => "admitted",
        Admission::Rejected => "rejected",
        Admission::Unavailable => "unavailable",
    };
    metrics::counter!("push_gateway_admissions_total", "result" => label).increment(1);
}

/// Record a delivery-path error, tagged by the static failure class. This
/// counter covers only the `/v1/deliveries/apns` handler's post-admission exit
/// classes (admission rejection/unavailability, profile mismatch, token-custody
/// open failure, and detached finish/join failure); pre-admission request/auth/
/// attestation validation on the enrollment and delegation handlers is not
/// counted here. `class` is always a compile-time constant.
pub fn record_delivery_error(class: &'static str) {
    metrics::counter!("push_gateway_delivery_errors_total", "class" => class).increment(1);
}

/// Record a retention-reaper sweep failure.
pub fn record_reaper_failure() {
    metrics::counter!("push_gateway_reaper_failures_total").increment(1);
}

/// Why a readiness probe reported not-ready.
#[derive(Debug, Clone, Copy)]
pub enum ReadinessFailure {
    /// The process is draining and no longer accepting traffic.
    NotAccepting,
    /// The authority store readiness check failed.
    Authority,
}

/// Record a readiness-probe failure by cause.
pub fn record_readiness_failure(cause: ReadinessFailure) {
    let label = match cause {
        ReadinessFailure::NotAccepting => "not_accepting",
        ReadinessFailure::Authority => "authority",
    };
    metrics::counter!("push_gateway_readiness_failures_total", "cause" => label).increment(1);
}

/// Point-in-time utilization of the gateway's Postgres pool.
#[derive(Debug, Clone, Copy)]
pub struct DbPoolStats {
    /// Open connections, idle or checked out.
    pub size: u32,
    /// Open connections waiting in the pool.
    pub idle: u32,
    /// Configured connection ceiling.
    pub max: u32,
}

/// Record the pool's connection counts. The periodic sampler in `main.rs`
/// calls this; `state` takes only `idle` and `active`.
pub fn record_db_pool(stats: DbPoolStats) {
    let active = stats.size.saturating_sub(stats.idle);
    for (state, value) in [("idle", stats.idle), ("active", active)] {
        metrics::gauge!("push_gateway_db_pool_connections", "state" => state).set(f64::from(value));
    }
    metrics::gauge!("push_gateway_db_pool_max_connections").set(f64::from(stats.max));
}

/// How a pool checkout ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolAcquireOutcome {
    /// A connection was checked out.
    Success,
    /// The pool's acquire timeout elapsed first.
    Timeout,
    /// The pool returned any other error, such as a failed connect.
    Error,
    /// The caller dropped the checkout before it finished.
    Cancelled,
}

impl PoolAcquireOutcome {
    /// Classify a finished checkout.
    pub fn from_result<T>(result: &Result<T, sqlx::Error>) -> Self {
        match result {
            Ok(_) => Self::Success,
            Err(sqlx::Error::PoolTimedOut) => Self::Timeout,
            Err(_) => Self::Error,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One in-flight pool checkout. It counts in `push_gateway_db_pool_waiters`
/// from [`PoolAcquire::start`] until it finishes or is dropped, and records its
/// wait in `push_gateway_db_pool_acquire_wait_seconds` exactly once.
#[must_use = "dropping the checkout immediately records it as cancelled"]
pub struct PoolAcquire {
    started: std::time::Instant,
    finished: bool,
}

impl PoolAcquire {
    /// Begin timing a checkout and count it as a waiter.
    pub fn start() -> Self {
        metrics::gauge!("push_gateway_db_pool_waiters").increment(1.0);
        Self {
            started: std::time::Instant::now(),
            finished: false,
        }
    }

    /// Record the checkout's terminal outcome.
    pub fn finish(mut self, outcome: PoolAcquireOutcome) {
        self.record(outcome);
    }

    fn record(&mut self, outcome: PoolAcquireOutcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        metrics::gauge!("push_gateway_db_pool_waiters").decrement(1.0);
        metrics::histogram!(
            "push_gateway_db_pool_acquire_wait_seconds",
            "outcome" => outcome.as_str(),
        )
        .record(self.started.elapsed().as_secs_f64());
    }
}

impl Drop for PoolAcquire {
    fn drop(&mut self) {
        self.record(PoolAcquireOutcome::Cancelled);
    }
}

/// An uninstalled recorder with the production exporter configuration, for
/// tests that install it locally instead of process-wide.
#[cfg(test)]
pub(crate) fn test_recorder() -> metrics_exporter_prometheus::PrometheusRecorder {
    builder()
        .expect("exporter configuration is valid")
        .build_recorder()
}

/// The value of one exact rendered series, such as `name{label="x"}`.
#[cfg(test)]
pub(crate) fn rendered_sample(rendered: &str, series: &str) -> Option<f64> {
    rendered.lines().find_map(|line| {
        let value = line.strip_prefix(series)?.strip_prefix(' ')?;
        value.trim().parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_label_covers_every_variant_with_static_strings() {
        // Exhaustive over the closed enum; each arm is a compile-time constant,
        // so the `outcome` label is structurally bounded to these five values.
        for (outcome, expected) in [
            (DeliveryOutcome::Accepted, "accepted"),
            (
                DeliveryOutcome::InvalidEndpoint {
                    unregistered_at: Some(7),
                },
                "invalid_endpoint",
            ),
            (
                DeliveryOutcome::Retry {
                    retry_after_seconds: Some(30),
                },
                "retry",
            ),
            (DeliveryOutcome::ConfigurationFault, "configuration_fault"),
            (
                DeliveryOutcome::PermanentRequestFault,
                "permanent_request_fault",
            ),
        ] {
            assert_eq!(outcome_label(outcome), expected);
        }
    }

    // The global metrics recorder can be installed only once per process, so a
    // single test owns the install and exercises every helper end-to-end,
    // asserting the rendered exposition is sanitized and bounded-cardinality.
    #[test]
    fn recorder_renders_sanitized_bounded_series() {
        let handle = install().expect("recorder installs exactly once per test process");

        record_apns_send_attempt();
        record_apns_delivery(DeliveryOutcome::Accepted, 0.012);
        record_apns_delivery(
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            },
            0.030,
        );
        record_admission(Admission::Admitted);
        record_admission(Admission::Rejected);
        record_admission(Admission::Unavailable);
        record_delivery_error("invalid_grant");
        record_delivery_error("finish_failed");
        record_reaper_failure();
        record_readiness_failure(ReadinessFailure::NotAccepting);
        record_readiness_failure(ReadinessFailure::Authority);
        record_db_pool(DbPoolStats {
            size: 2,
            idle: 1,
            max: 20,
        });
        PoolAcquire::start().finish(PoolAcquireOutcome::Success);

        let rendered = handle.render();

        // All expected series are present.
        for needle in [
            "push_gateway_apns_send_attempts_total",
            "push_gateway_apns_deliveries_total",
            "push_gateway_apns_delivery_seconds",
            "push_gateway_admissions_total",
            "push_gateway_delivery_errors_total",
            "push_gateway_reaper_failures_total",
            "push_gateway_readiness_failures_total",
            "push_gateway_db_pool_connections",
            "push_gateway_db_pool_max_connections",
            "push_gateway_db_pool_waiters",
            "push_gateway_db_pool_acquire_wait_seconds",
        ] {
            assert!(rendered.contains(needle), "missing series {needle}");
        }
        // Labels are the closed static sets only.
        for needle in [
            "outcome=\"accepted\"",
            "outcome=\"invalid_endpoint\"",
            "result=\"admitted\"",
            "result=\"rejected\"",
            "result=\"unavailable\"",
            "class=\"invalid_grant\"",
            "cause=\"not_accepting\"",
            "cause=\"authority\"",
            "state=\"idle\"",
            "state=\"active\"",
            "outcome=\"success\"",
        ] {
            assert!(rendered.contains(needle), "missing label {needle}");
        }
    }

    #[test]
    fn pool_checkouts_balance_waiters_and_record_bucketed_waits() {
        let recorder = test_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            record_db_pool(DbPoolStats {
                size: 7,
                idle: 3,
                max: 20,
            });
            let finished = PoolAcquire::start();
            let abandoned = PoolAcquire::start();
            assert_eq!(
                rendered_sample(&handle.render(), "push_gateway_db_pool_waiters"),
                Some(2.0),
                "both in-flight checkouts count as waiters"
            );
            finished.finish(PoolAcquireOutcome::Success);
            drop(abandoned);
            PoolAcquire::start().finish(PoolAcquireOutcome::Timeout);
        });

        let rendered = handle.render();
        for (series, expected) in [
            ("push_gateway_db_pool_waiters", 0.0),
            ("push_gateway_db_pool_connections{state=\"idle\"}", 3.0),
            ("push_gateway_db_pool_connections{state=\"active\"}", 4.0),
            ("push_gateway_db_pool_max_connections", 20.0),
            (
                "push_gateway_db_pool_acquire_wait_seconds_count{outcome=\"success\"}",
                1.0,
            ),
            (
                "push_gateway_db_pool_acquire_wait_seconds_count{outcome=\"cancelled\"}",
                1.0,
            ),
            (
                "push_gateway_db_pool_acquire_wait_seconds_count{outcome=\"timeout\"}",
                1.0,
            ),
        ] {
            assert_eq!(
                rendered_sample(&rendered, series),
                Some(expected),
                "{series} in:\n{rendered}"
            );
        }
        // Configured buckets render a histogram, not the exporter's default
        // summary, so the scrape keeps a 30-second top bucket.
        assert!(
            rendered.contains(
                "push_gateway_db_pool_acquire_wait_seconds_bucket{outcome=\"success\",le=\"30\"}"
            ),
            "acquire wait must render as a bucketed histogram:\n{rendered}"
        );
    }

    #[test]
    fn pool_acquire_outcome_classifies_sqlx_results() {
        assert_eq!(
            PoolAcquireOutcome::from_result(&Ok::<(), sqlx::Error>(())),
            PoolAcquireOutcome::Success
        );
        assert_eq!(
            PoolAcquireOutcome::from_result(&Err::<(), _>(sqlx::Error::PoolTimedOut)),
            PoolAcquireOutcome::Timeout
        );
        assert_eq!(
            PoolAcquireOutcome::from_result(&Err::<(), _>(sqlx::Error::PoolClosed)),
            PoolAcquireOutcome::Error
        );
    }
}
