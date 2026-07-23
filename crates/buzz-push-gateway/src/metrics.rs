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

/// Install the global metrics recorder and return the render handle.
///
/// Unlike the relay's exporter, this installs **no** HTTP listener: rendering is
/// served from the private health router so metrics never share the public port.
/// Must be called at most once per process, from within a Tokio runtime.
pub fn install() -> Result<PrometheusHandle, BuildError> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("push_gateway_apns_delivery_seconds".to_owned()),
            &APNS_LATENCY_BUCKETS_S,
        )?
        .install_recorder()?;
    Ok(handle)
}

/// Stable metric label for each sanitized delivery outcome. The mapping is total
/// over the closed [`DeliveryOutcome`] enum, so the `outcome` label can only take
/// these six values.
fn outcome_label(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Accepted => "accepted",
        DeliveryOutcome::InvalidEndpoint { .. } => "invalid_endpoint",
        DeliveryOutcome::Retry { .. } => "retry",
        DeliveryOutcome::RefreshCredential => "refresh_credential",
        DeliveryOutcome::ConfigurationFault => "configuration_fault",
        DeliveryOutcome::PermanentRequestFault => "permanent_request_fault",
    }
}

/// Record the terminal APNs outcome and its send round-trip latency.
pub fn record_apns_delivery(outcome: DeliveryOutcome, seconds: f64) {
    metrics::counter!("push_gateway_apns_deliveries_total", "outcome" => outcome_label(outcome))
        .increment(1);
    metrics::histogram!("push_gateway_apns_delivery_seconds").record(seconds);
}

/// Record that a cached provider credential was refreshed after APNs reported expiry.
pub fn record_credential_refresh() {
    metrics::counter!("push_gateway_apns_credential_refreshes_total").increment(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_label_covers_every_variant_with_static_strings() {
        // Exhaustive over the closed enum; each arm is a compile-time constant,
        // so the `outcome` label is structurally bounded to these six values.
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
            (DeliveryOutcome::RefreshCredential, "refresh_credential"),
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

        record_apns_delivery(DeliveryOutcome::Accepted, 0.012);
        record_apns_delivery(
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            },
            0.030,
        );
        record_credential_refresh();
        record_admission(Admission::Admitted);
        record_admission(Admission::Rejected);
        record_admission(Admission::Unavailable);
        record_delivery_error("invalid_grant");
        record_delivery_error("finish_failed");
        record_reaper_failure();
        record_readiness_failure(ReadinessFailure::NotAccepting);
        record_readiness_failure(ReadinessFailure::Authority);

        let rendered = handle.render();

        // All expected series are present.
        for needle in [
            "push_gateway_apns_deliveries_total",
            "push_gateway_apns_delivery_seconds",
            "push_gateway_apns_credential_refreshes_total",
            "push_gateway_admissions_total",
            "push_gateway_delivery_errors_total",
            "push_gateway_reaper_failures_total",
            "push_gateway_readiness_failures_total",
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
        ] {
            assert!(rendered.contains(needle), "missing label {needle}");
        }
    }
}
