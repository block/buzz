//! Bounded-cardinality Prometheus metrics for the ingress gateway.
//!
//! Every label value is a compile-time `&'static str` from a closed set. No
//! integration id, delivery id, community host, or other request-scoped value
//! is ever used as a label, so cardinality is bounded regardless of traffic.
//! Rendering is served from the private health router, never the public port.

use metrics_exporter_prometheus::{BuildError, PrometheusBuilder, PrometheusHandle};

/// Install the global metrics recorder and return the render handle.
///
/// Installs no HTTP listener: the private health router serves `GET /metrics`.
/// Must be called at most once per process, from within a Tokio runtime.
pub fn install() -> Result<PrometheusHandle, BuildError> {
    PrometheusBuilder::new().install_recorder()
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
        let rendered = handle.render();
        for needle in [
            "ingress_gateway_readiness_failures_total",
            "ingress_gateway_reaper_failures_total",
            "ingress_gateway_intake_total",
            "cause=\"not_accepting\"",
            "cause=\"store\"",
            "outcome=\"queued\"",
            "outcome=\"unauthorized\"",
        ] {
            assert!(rendered.contains(needle), "missing {needle}: {rendered}");
        }
    }
}
