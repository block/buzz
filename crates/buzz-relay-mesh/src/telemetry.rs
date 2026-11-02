//! Prometheus metrics for the mesh reconcile loop and peer table.
//!
//! These answer two operational questions without exec-ing into a pod:
//!
//! - **Where is a reconcile pass spending its time?** A pass first rescans the
//!   ready registry, then dials every peer it is not yet connected to, one at
//!   a time. `buzz_mesh_reconcile_candidates` is set once the scan finishes,
//!   so a pass stuck before it is stuck in the scan; the dial counter and
//!   histogram show progress and cost inside the dial loop.
//! - **How big and how stale is the peer table?** `buzz_mesh_peer_records`
//!   and `buzz_mesh_peer_record_max_age_seconds` are refreshed every pass;
//!   `buzz_mesh_peer_records_expired_total` counts records dropped for age.
//!
//! Label values are closed vocabularies: `outcome` ∈ {`ok`, `error`},
//! `state` ∈ [`ConnectionState`] in snake_case, `draining` ∈ {`true`,
//! `false`}, `reason` ∈ {`pruned`, `rejected`}.

use std::time::Duration;

use crate::status::ConnectionState;

/// Gauge: peers the current reconcile pass will dial.
pub const RECONCILE_CANDIDATES: &str = "buzz_mesh_reconcile_candidates";
/// Histogram: wall-clock duration of one full reconcile pass.
pub const RECONCILE_PASS_SECONDS: &str = "buzz_mesh_reconcile_pass_seconds";
/// Counter: outbound connect attempts by `outcome`.
pub const DIAL_TOTAL: &str = "buzz_mesh_dial_total";
/// Histogram: duration of one outbound connect attempt.
pub const DIAL_DURATION_SECONDS: &str = "buzz_mesh_dial_duration_seconds";
/// Gauge: peer-table records by effective `state` and `draining`.
pub const PEER_RECORDS: &str = "buzz_mesh_peer_records";
/// Gauge: age of the oldest peer-table heartbeat, in seconds.
pub const PEER_RECORD_MAX_AGE_SECONDS: &str = "buzz_mesh_peer_record_max_age_seconds";
/// Counter: records dropped for an expired heartbeat, by `reason`.
pub const PEER_RECORDS_EXPIRED_TOTAL: &str = "buzz_mesh_peer_records_expired_total";

/// Every effective connection state, in `state` label order.
pub(crate) const CONNECTION_STATES: [ConnectionState; 4] = [
    ConnectionState::Disconnected,
    ConnectionState::Connecting,
    ConnectionState::Connected,
    ConnectionState::Suspect,
];

/// Register metric descriptions. Call once after the global recorder exists.
pub fn describe_metrics() {
    metrics::describe_gauge!(
        RECONCILE_CANDIDATES,
        "Mesh peers the current reconcile pass will dial (set after the registry scan)"
    );
    metrics::describe_histogram!(
        RECONCILE_PASS_SECONDS,
        metrics::Unit::Seconds,
        "Wall-clock duration of one mesh reconcile pass (registry scan plus dials)"
    );
    metrics::describe_counter!(
        DIAL_TOTAL,
        "Outbound mesh connect attempts, by outcome (ok, error)"
    );
    metrics::describe_histogram!(
        DIAL_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "Duration of one outbound mesh connect attempt"
    );
    metrics::describe_gauge!(
        PEER_RECORDS,
        "Mesh peer-table records, by effective connection state and draining flag"
    );
    metrics::describe_gauge!(
        PEER_RECORD_MAX_AGE_SECONDS,
        metrics::Unit::Seconds,
        "Age of the oldest heartbeat in the mesh peer table"
    );
    metrics::describe_counter!(
        PEER_RECORDS_EXPIRED_TOTAL,
        "Mesh peer records dropped for an expired heartbeat (pruned from the table, or rejected on arrival)"
    );
}

pub(crate) const fn state_label(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Suspect => "suspect",
    }
}

/// Position of `state` in [`CONNECTION_STATES`].
const fn state_index(state: ConnectionState) -> usize {
    match state {
        ConnectionState::Disconnected => 0,
        ConnectionState::Connecting => 1,
        ConnectionState::Connected => 2,
        ConnectionState::Suspect => 3,
    }
}

const fn draining_label(draining: bool) -> &'static str {
    if draining {
        "true"
    } else {
        "false"
    }
}

/// Peer-table shape at one instant, published as gauges.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PeerTableStats {
    /// `counts[state][draining as usize]`, indexed like [`CONNECTION_STATES`].
    pub counts: [[u64; 2]; CONNECTION_STATES.len()],
    /// Oldest heartbeat age; zero for an empty table.
    pub max_age: Duration,
}

impl PeerTableStats {
    pub(crate) fn record(&mut self, state: ConnectionState, draining: bool, age: Duration) {
        self.counts[state_index(state)][usize::from(draining)] += 1;
        self.max_age = self.max_age.max(age);
    }

    /// Set every `(state, draining)` series, zeros included, so a series
    /// that empties reads 0 instead of holding its last non-zero value.
    pub(crate) fn publish(&self) {
        for (index, state) in CONNECTION_STATES.iter().enumerate() {
            for draining in [false, true] {
                metrics::gauge!(
                    PEER_RECORDS,
                    "state" => state_label(*state),
                    "draining" => draining_label(draining),
                )
                .set(self.counts[index][usize::from(draining)] as f64);
            }
        }
        metrics::gauge!(PEER_RECORD_MAX_AGE_SECONDS).set(self.max_age.as_secs_f64());
    }
}

pub(crate) fn record_dial(ok: bool, elapsed: Duration) {
    let outcome = if ok { "ok" } else { "error" };
    metrics::counter!(DIAL_TOTAL, "outcome" => outcome).increment(1);
    metrics::histogram!(DIAL_DURATION_SECONDS).record(elapsed.as_secs_f64());
}

pub(crate) fn record_expired(reason: &'static str, count: u64) {
    if count > 0 {
        metrics::counter!(PEER_RECORDS_EXPIRED_TOTAL, "reason" => reason).increment(count);
    }
}
