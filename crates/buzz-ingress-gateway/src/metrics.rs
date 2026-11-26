//! Bounded-cardinality Prometheus metrics for the ingress gateway.
//!
//! Every label value is a compile-time `&'static str` from a closed set. No
//! integration id, delivery id, community host, or other request-scoped value
//! is ever used as a label, so cardinality is bounded regardless of traffic.
//! Rendering is served from the private health router, never the public port.

use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder, PrometheusHandle};

/// Install the global metrics recorder and return the render handle.
///
/// Installs no HTTP listener: the private health router serves `GET /metrics`.
/// Must be called at most once per process, from within a Tokio runtime.
pub fn install() -> Result<PrometheusHandle, BuildError> {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("ingress_gateway_forward_seconds".to_owned()),
            &FORWARD_LATENCY_BUCKETS_S,
        )?
        .install_recorder()
}

const FORWARD_LATENCY_BUCKETS_S: [f64; 10] =
    [0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

/// How one forward attempt ended.
#[derive(Debug, Clone, Copy)]
pub enum ForwardResult {
    Accepted,
    Failed,
    Retry,
}

/// Record one forward attempt and, when the relay was contacted, its
/// round-trip latency.
pub fn record_forward(result: ForwardResult, seconds: Option<f64>) {
    let label = match result {
        ForwardResult::Accepted => "accepted",
        ForwardResult::Failed => "failed",
        ForwardResult::Retry => "retry",
    };
    metrics::counter!("ingress_gateway_forward_total", "outcome" => label).increment(1);
    if let Some(seconds) = seconds {
        metrics::histogram!("ingress_gateway_forward_seconds").record(seconds);
    }
}

/// How fanning out one delivery ended.
#[derive(Debug, Clone, Copy)]
pub enum FanoutResult {
    /// Settled into this many messages (failed ones included).
    Fanned(u64),
    Failed,
    Retry,
}

/// Record one delivery fan-out.
pub fn record_fanout(result: FanoutResult) {
    let label = match result {
        FanoutResult::Fanned(messages) => {
            metrics::counter!("ingress_gateway_fanout_messages_total").increment(messages);
            "fanned"
        }
        FanoutResult::Failed => "failed",
        FanoutResult::Retry => "retry",
    };
    metrics::counter!("ingress_gateway_fanout_total", "outcome" => label).increment(1);
}

/// Record a forwarder or fan-out poll that could not reach the store.
pub fn record_forwarder_failure() {
    metrics::counter!("ingress_gateway_forwarder_failures_total").increment(1);
}

/// Why a readiness probe reported not-ready.
#[derive(Debug, Clone, Copy)]
pub enum ReadinessFailure {
    /// The process is draining and no longer accepting traffic.
    NotAccepting,
    /// The store readiness check failed.
    Store,
}

/// Record a readiness-probe failure by cause.
pub fn record_readiness_failure(cause: ReadinessFailure) {
    let label = match cause {
        ReadinessFailure::NotAccepting => "not_accepting",
        ReadinessFailure::Store => "store",
    };
    metrics::counter!("ingress_gateway_readiness_failures_total", "cause" => label).increment(1);
}

/// How an intake request ended. Every label is a compile-time constant.
#[derive(Debug, Clone, Copy)]
pub enum IntakeOutcome {
    Queued,
    Duplicate,
    NotFound,
    Disabled,
    PendingSecret,
    Invalid,
    Unauthorized,
    RateLimited,
    Unavailable,
}

/// Record the outcome of one `POST /webhooks/{id}` request.
pub fn record_intake(outcome: IntakeOutcome) {
    let label = match outcome {
        IntakeOutcome::Queued => "queued",
        IntakeOutcome::Duplicate => "duplicate",
        IntakeOutcome::NotFound => "not_found",
        IntakeOutcome::Disabled => "disabled",
        IntakeOutcome::PendingSecret => "pending_secret",
        IntakeOutcome::Invalid => "invalid",
        IntakeOutcome::Unauthorized => "unauthorized",
        IntakeOutcome::RateLimited => "rate_limited",
        IntakeOutcome::Unavailable => "unavailable",
    };
    metrics::counter!("ingress_gateway_intake_total", "outcome" => label).increment(1);
}

/// Record a retention-reaper sweep failure.
pub fn record_reaper_failure() {
    metrics::counter!("ingress_gateway_reaper_failures_total").increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The global recorder installs once per process, so one test owns the
    // install and exercises every helper.
    #[test]
    fn recorder_renders_bounded_series() {
        let handle = install().expect("recorder installs exactly once per test process");
        record_readiness_failure(ReadinessFailure::NotAccepting);
        record_readiness_failure(ReadinessFailure::Store);
        record_reaper_failure();
        record_intake(IntakeOutcome::Queued);
        record_intake(IntakeOutcome::Unauthorized);
        record_forward(ForwardResult::Accepted, Some(0.05));
        record_forward(ForwardResult::Retry, Some(2.0));
        record_forward(ForwardResult::Failed, None);
        record_forwarder_failure();
        record_fanout(FanoutResult::Fanned(2));
        record_fanout(FanoutResult::Retry);
        let rendered = handle.render();
        for needle in [
            "ingress_gateway_readiness_failures_total",
            "ingress_gateway_reaper_failures_total",
            "ingress_gateway_intake_total",
            "ingress_gateway_forward_total",
            "ingress_gateway_forward_seconds",
            "ingress_gateway_forwarder_failures_total",
            "ingress_gateway_fanout_total",
            "ingress_gateway_fanout_messages_total",
            "outcome=\"fanned\"",
            "outcome=\"accepted\"",
            "outcome=\"retry\"",
            "cause=\"not_accepting\"",
            "cause=\"store\"",
            "outcome=\"queued\"",
            "outcome=\"unauthorized\"",
        ] {
            assert!(rendered.contains(needle), "missing {needle}: {rendered}");
        }
    }
}
