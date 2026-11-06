//! Operator-tunable concurrency caps, re-read from feature flags while the
//! relay runs.
//!
//! Each cap is a [`ConcurrencyGate`] with a ceiling fixed at startup and an
//! integer flag evaluated in the process context:
//!
//! | Flag | Gate | Ceiling |
//! |------|------|---------|
//! | `relay.concurrency.history-reads` | subscription-history reads on the reader pool | the reader-pool share [`buzz_db`] reserves for history |
//! | `relay.concurrency.auth` | verified NIP-42 AUTH frames in their database checks | none |
//!
//! A positive value sets the limit, clamped to the ceiling; zero, a negative
//! value, or no value leaves the gate at its ceiling. A flag can therefore
//! only tighten a cap, never loosen one past what the relay starts with, and
//! a provider outage or a deleted flag puts every gate back at its ceiling.
//! A lowered gate never cancels admitted work: it admits nothing new until
//! the count in flight falls under the new limit.
//!
//! The flags are re-read every [`REFRESH_INTERVAL`], so a change reaches every
//! pod within that interval plus the provider's own propagation delay, with no
//! restart. `buzz_concurrency_limit{gate}` reports each applied limit, with
//! `0` meaning no limit.

use std::time::Duration;

use buzz_db::ConcurrencyGate;
use buzz_feature_flags::{EvaluationContext, IntegerFlag};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::feature_flags::FeatureFlags;

/// Cap on subscription-history reads holding a reader connection.
pub const HISTORY_READS_FLAG: IntegerFlag = IntegerFlag::new("relay.concurrency.history-reads", 0);
/// Cap on verified AUTH frames running their database checks.
pub const AUTH_FLAG: IntegerFlag = IntegerFlag::new("relay.concurrency.auth", 0);

/// How often the flags are re-read.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// One gate whose limit a flag controls.
#[derive(Clone, Debug)]
pub struct ManagedGate {
    name: &'static str,
    flag: IntegerFlag,
    gate: ConcurrencyGate,
}

impl ManagedGate {
    /// `gate`, controlled by `flag` and reported as `name`.
    pub fn new(name: &'static str, flag: IntegerFlag, gate: ConcurrencyGate) -> Self {
        Self { name, flag, gate }
    }
}

/// The relay's managed gates: history reads and AUTH.
pub fn relay_gates(state: &crate::state::AppState) -> Vec<ManagedGate> {
    vec![
        ManagedGate::new(
            "history_reads",
            HISTORY_READS_FLAG,
            state.db.history_read_gate().clone(),
        ),
        ManagedGate::new("auth", AUTH_FLAG, state.auth_gate.clone()),
    ]
}

/// The limit a flag value asks for on a gate with `ceiling`.
fn requested_limit(value: i64, ceiling: usize) -> usize {
    if value <= 0 {
        return ceiling;
    }
    usize::try_from(value).map_or(ceiling, |value| value.min(ceiling))
}

/// Evaluate every gate's flag once and apply the result.
pub fn apply(flags: &FeatureFlags, gates: &[ManagedGate]) {
    let context = EvaluationContext::for_process();
    for managed in gates {
        let ceiling = managed.gate.ceiling();
        let value = flags.evaluator().evaluate_int(managed.flag, &context);
        let previous = managed.gate.limit();
        let applied = managed.gate.set_limit(requested_limit(value, ceiling));
        if applied != previous {
            info!(
                gate = managed.name,
                limit = applied,
                previous,
                ceiling,
                "concurrency limit changed"
            );
        }
        let reported = if applied >= tokio::sync::Semaphore::MAX_PERMITS {
            0.0
        } else {
            applied as f64
        };
        metrics::gauge!("buzz_concurrency_limit", "gate" => managed.name).set(reported);
    }
    flags.report();
}

/// Apply the flags every [`REFRESH_INTERVAL`] (the first time immediately)
/// until `cancel` fires.
pub async fn run(
    flags: std::sync::Arc<FeatureFlags>,
    gates: Vec<ManagedGate>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = interval.tick() => apply(&flags, &gates),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_feature_flags::{BooleanFlag, FlagEvaluator};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A provider whose values the test changes while the relay "runs".
    #[derive(Default)]
    struct LiveValues(Mutex<HashMap<&'static str, i64>>);

    impl LiveValues {
        fn set(&self, flag: IntegerFlag, value: Option<i64>) {
            let mut values = self.0.lock().expect("values");
            match value {
                Some(value) => values.insert(flag.key(), value),
                None => values.remove(flag.key()),
            };
        }
    }

    impl FlagEvaluator for LiveValues {
        fn evaluate_bool(&self, flag: BooleanFlag, _: &EvaluationContext) -> bool {
            flag.default()
        }

        fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64 {
            assert_eq!(
                context.community(),
                None,
                "caps evaluate in the process context"
            );
            self.0
                .lock()
                .expect("values")
                .get(flag.key())
                .copied()
                .unwrap_or(flag.default())
        }
    }

    #[test]
    fn flag_values_only_tighten_and_reset_to_the_ceiling() {
        let values = Arc::new(LiveValues::default());
        let flags = FeatureFlags::from_evaluator("test", values.clone());
        let history = ConcurrencyGate::new(20);
        let auth = ConcurrencyGate::unbounded();
        let gates = vec![
            ManagedGate::new("history_reads", HISTORY_READS_FLAG, history.clone()),
            ManagedGate::new("auth", AUTH_FLAG, auth.clone()),
        ];

        // Each row: the flag values, then the limits applied.
        let rows: [(Option<i64>, Option<i64>, usize, usize); 6] = [
            (None, None, 20, auth.ceiling()),
            (Some(8), Some(50), 8, 50),
            (Some(500), Some(i64::MAX), 20, auth.ceiling()),
            (Some(0), Some(-3), 20, auth.ceiling()),
            (Some(1), Some(1), 1, 1),
            (None, None, 20, auth.ceiling()),
        ];
        for (history_value, auth_value, history_limit, auth_limit) in rows {
            values.set(HISTORY_READS_FLAG, history_value);
            values.set(AUTH_FLAG, auth_value);
            apply(&flags, &gates);
            assert_eq!(history.limit(), history_limit, "history {history_value:?}");
            assert_eq!(auth.limit(), auth_limit, "auth {auth_value:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_refresh_loop_applies_changes_without_a_restart() {
        let values = Arc::new(LiveValues::default());
        let flags = Arc::new(FeatureFlags::from_evaluator("test", values.clone()));
        let gate = ConcurrencyGate::new(10);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(
            flags,
            vec![ManagedGate::new(
                "history_reads",
                HISTORY_READS_FLAG,
                gate.clone(),
            )],
            cancel.clone(),
        ));

        values.set(HISTORY_READS_FLAG, Some(3));
        tokio::time::sleep(REFRESH_INTERVAL + Duration::from_millis(1)).await;
        assert_eq!(gate.limit(), 3, "the next refresh applies the new value");

        values.set(HISTORY_READS_FLAG, None);
        tokio::time::sleep(REFRESH_INTERVAL).await;
        assert_eq!(gate.limit(), 10, "removing the flag restores the ceiling");

        cancel.cancel();
        task.await.expect("the loop stops on cancel");
    }
}
