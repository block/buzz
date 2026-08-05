//! In-process circuit breaker for the provider failover chain.
//!
//! Without it, every turn re-discovers an outage the hard way: the primary is
//! tried, waits out its retries, and only then cuts over. A quota wall lasts
//! days, so that cost is paid on every turn for as long as it lasts. The
//! breaker remembers, and sends the next turn straight to a provider that
//! works.
//!
//! State machine per endpoint:
//!
//! ```text
//! closed  --2 consecutive cutover failures-->  open (cooldown starts)
//! open, now <  open_until                      blocked: skipped by the chain
//! open, now >= open_until                      half-open: one probe allowed
//!   probe fails                                cooldown doubles, stays open
//!   probe succeeds                             closed, counters reset
//! ```
//!
//! Cooldown starts at 60s and doubles per failed probe to a 15min cap.
//! Quota and auth failures start at 5min instead: a weekly quota window or a
//! revoked key does not heal inside a minute, so probing that fast is waste.
//!
//! Deliberately in-process and not persisted. The state is a latency
//! optimization, not a source of truth — a restarted agent re-learning an
//! outage costs one slow turn, which is not worth a file to corrupt. It is
//! also reactive by construction: a circuit opens only after real traffic has
//! failed, so it cannot predict an exhausted quota before anything tries.
//!
//! Fail-soft throughout: the breaker never reports an endpoint as down unless
//! it has seen it fail, and the chain walk ignores it entirely when every
//! candidate is blocked. It must never be the reason a turn has nowhere to go.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::types::CutoverKind;

/// Consecutive cutover-class failures before an endpoint is taken out of
/// rotation. Two, not one: a single 503 is noise, and opening on it would
/// bounce a healthy provider out over a blip.
const FAILURE_THRESHOLD: u32 = 2;

/// Opening cooldown for failures that usually clear quickly (5xx, transport).
const BASE_COOLDOWN: Duration = Duration::from_secs(60);

/// Opening cooldown for failures that do not (quota windows, bad credentials).
const SLOW_HEAL_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Ceiling on the doubling, so a long outage still gets probed periodically.
const MAX_COOLDOWN: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
struct Circuit {
    consecutive_failures: u32,
    /// `None` while closed. `Some(t)` once opened — the circuit is blocking
    /// until `t`, and half-open (one probe allowed) from `t` onward.
    open_until: Option<Instant>,
    cooldown: Duration,
}

impl Circuit {
    fn closed() -> Self {
        Self {
            consecutive_failures: 0,
            open_until: None,
            cooldown: BASE_COOLDOWN,
        }
    }
}

/// Per-endpoint availability memory shared across the turns of one agent
/// process. Keyed by [`Endpoint::circuit_key`](crate::config::Endpoint::circuit_key).
#[derive(Debug, Default)]
pub struct Breaker {
    circuits: Mutex<HashMap<String, Circuit>>,
}

impl Breaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recover the guard rather than propagating a poisoned lock: a panic in
    /// some other turn's bookkeeping must not take the failover path down with
    /// it. Worst case the map holds slightly stale counters.
    fn circuits(&self) -> std::sync::MutexGuard<'_, HashMap<String, Circuit>> {
        self.circuits.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `key` is currently blocked. False for an endpoint that has
    /// never failed, and false once the cooldown has elapsed (the half-open
    /// probe).
    pub fn is_open(&self, key: &str) -> bool {
        self.is_open_at(key, Instant::now())
    }

    fn is_open_at(&self, key: &str, now: Instant) -> bool {
        self.circuits()
            .get(key)
            .and_then(|c| c.open_until)
            .is_some_and(|until| now < until)
    }

    /// Record a cutover-class failure. Never call this for a stop-class error
    /// (a 400): a malformed request says nothing about whether the endpoint is
    /// up, and would take a healthy provider out of rotation.
    pub fn record_failure(&self, key: &str, kind: CutoverKind) {
        self.record_failure_at(key, kind, Instant::now());
    }

    fn record_failure_at(&self, key: &str, kind: CutoverKind, now: Instant) {
        let mut circuits = self.circuits();
        let circuit = circuits
            .entry(key.to_owned())
            .or_insert_with(Circuit::closed);

        if circuit.open_until.is_some() {
            // A failed probe, or an attempt forced through because every
            // candidate was blocked. Back off further; already open, so this
            // is not a transition.
            circuit.cooldown = (circuit.cooldown * 2).min(MAX_COOLDOWN);
            circuit.open_until = Some(now + circuit.cooldown);
            return;
        }

        circuit.consecutive_failures += 1;
        if circuit.consecutive_failures < FAILURE_THRESHOLD {
            return;
        }
        circuit.cooldown = if kind.is_slow_to_heal() {
            SLOW_HEAL_COOLDOWN
        } else {
            BASE_COOLDOWN
        };
        circuit.open_until = Some(now + circuit.cooldown);
        tracing::warn!(
            endpoint = key,
            kind = kind.as_str(),
            cooldown_secs = circuit.cooldown.as_secs(),
            failures = circuit.consecutive_failures,
            "llm: endpoint taken out of rotation"
        );
    }

    /// Record a success, closing the circuit and clearing the backoff.
    pub fn record_success(&self, key: &str) {
        let mut circuits = self.circuits();
        if let Some(circuit) = circuits.get(key) {
            if circuit.open_until.is_some() {
                tracing::info!(endpoint = key, "llm: endpoint recovered");
            }
            circuits.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "anthropic|https://api.z.ai/api/anthropic";

    #[test]
    fn unknown_endpoint_is_never_blocked() {
        let breaker = Breaker::new();
        assert!(!breaker.is_open(KEY));
    }

    #[test]
    fn one_failure_does_not_open_the_circuit() {
        let breaker = Breaker::new();
        breaker.record_failure(KEY, CutoverKind::Server);
        assert!(
            !breaker.is_open(KEY),
            "a single blip must not take an endpoint out of rotation"
        );
    }

    #[test]
    fn threshold_failures_open_the_circuit() {
        let breaker = Breaker::new();
        breaker.record_failure(KEY, CutoverKind::Server);
        breaker.record_failure(KEY, CutoverKind::Server);
        assert!(breaker.is_open(KEY));
    }

    #[test]
    fn quota_opens_with_the_slow_heal_cooldown() {
        let breaker = Breaker::new();
        let now = Instant::now();
        breaker.record_failure_at(KEY, CutoverKind::Quota, now);
        breaker.record_failure_at(KEY, CutoverKind::Quota, now);

        // Still blocked well past the 60s a transport failure would have used.
        assert!(breaker.is_open_at(KEY, now + Duration::from_secs(61)));
        assert!(breaker.is_open_at(KEY, now + SLOW_HEAL_COOLDOWN - Duration::from_secs(1)));
        // Half-open once the 5min window elapses.
        assert!(!breaker.is_open_at(KEY, now + SLOW_HEAL_COOLDOWN));
    }

    #[test]
    fn server_failures_open_with_the_short_cooldown() {
        let breaker = Breaker::new();
        let now = Instant::now();
        breaker.record_failure_at(KEY, CutoverKind::Server, now);
        breaker.record_failure_at(KEY, CutoverKind::Server, now);

        assert!(breaker.is_open_at(KEY, now + Duration::from_secs(59)));
        assert!(!breaker.is_open_at(KEY, now + BASE_COOLDOWN));
    }

    #[test]
    fn a_failed_probe_doubles_the_cooldown() {
        let breaker = Breaker::new();
        let now = Instant::now();
        breaker.record_failure_at(KEY, CutoverKind::Server, now);
        breaker.record_failure_at(KEY, CutoverKind::Server, now);

        // Cooldown elapses, the probe is allowed, and it fails too.
        let probe_at = now + BASE_COOLDOWN;
        assert!(!breaker.is_open_at(KEY, probe_at));
        breaker.record_failure_at(KEY, CutoverKind::Server, probe_at);

        // Next window is 120s, not another 60s.
        assert!(breaker.is_open_at(KEY, probe_at + Duration::from_secs(119)));
        assert!(!breaker.is_open_at(KEY, probe_at + Duration::from_secs(120)));
    }

    #[test]
    fn cooldown_doubling_is_capped() {
        let breaker = Breaker::new();
        let mut now = Instant::now();
        breaker.record_failure_at(KEY, CutoverKind::Server, now);
        breaker.record_failure_at(KEY, CutoverKind::Server, now);

        // Fail probe after probe; the window must never exceed the cap.
        for _ in 0..12 {
            now += MAX_COOLDOWN;
            breaker.record_failure_at(KEY, CutoverKind::Server, now);
        }
        assert!(!breaker.is_open_at(KEY, now + MAX_COOLDOWN));
    }

    #[test]
    fn success_closes_an_open_circuit() {
        let breaker = Breaker::new();
        breaker.record_failure(KEY, CutoverKind::Quota);
        breaker.record_failure(KEY, CutoverKind::Quota);
        assert!(breaker.is_open(KEY));

        breaker.record_success(KEY);
        assert!(!breaker.is_open(KEY));
    }

    #[test]
    fn success_resets_the_failure_count() {
        let breaker = Breaker::new();
        breaker.record_failure(KEY, CutoverKind::Server);
        breaker.record_success(KEY);
        // The earlier failure must not combine with this one to hit the
        // threshold — otherwise a slow trickle of unrelated blips eventually
        // opens the circuit on a healthy endpoint.
        breaker.record_failure(KEY, CutoverKind::Server);
        assert!(!breaker.is_open(KEY));
    }

    #[test]
    fn circuits_are_independent_per_endpoint() {
        let breaker = Breaker::new();
        let other = "openrouter|https://openrouter.ai/api/v1";
        breaker.record_failure(KEY, CutoverKind::Quota);
        breaker.record_failure(KEY, CutoverKind::Quota);

        assert!(breaker.is_open(KEY));
        assert!(
            !breaker.is_open(other),
            "one provider's quota wall must not bench the rest of the chain"
        );
    }
}
