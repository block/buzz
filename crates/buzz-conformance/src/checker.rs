//! Replay engine: validate a sequence of [`TraceStep`]s against the spec's
//! transition relation (re-implemented in [`crate::transitions`]).
//!
//! The checker is intentionally minimal — it walks the trace, bootstraps
//! its model on the first step, and runs [`transitions::check_step`] for
//! each subsequent step. The first failure stops the trace (fail-closed).
//!
//! The other half of the checker's job is **coverage breach**: declaring
//! up-front which critical actions a scenario MUST exercise, and failing
//! the trace if any are missing. Without this, a regression that silently
//! removed an emit site would still pass conformance — the trace would
//! just be shorter. The skill is explicit: this mode is mandatory.

use std::collections::HashSet;

use crate::{
    transitions::{check_step, ModelState, TransitionError},
    TraceStep,
};

/// A scenario the checker is validating: the recorded trace plus the set
/// of critical actions the scenario asserts must appear.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Trace steps in emission order. Stamps and worker ids are NOT
    /// modeled — observations are unordered in the spec, so the only
    /// invariant the order enforces is "within one request, observations
    /// share the same `state_after`".
    pub trace: Vec<TraceStep>,
    /// Action kinds that this scenario must include at least once. If any
    /// are missing the checker returns a coverage breach.
    ///
    /// Use [`crate::TraceAction::kind`] to get the canonical strings:
    /// `"write_insert"`, `"write_insert_global"`, `"write_duplicate"`,
    /// `"sanitized_error"`, `"auth_check"`, `"read_message_rows"`,
    /// `"read_by_id_rows"`, `"read_host_feed_rows"`.
    pub required_critical_actions: HashSet<String>,
}

impl Scenario {
    /// Build a scenario with no required actions — used for traces where
    /// the only thing being asserted is "every observation is consistent
    /// with non-interference". Most ingest fixtures need explicit
    /// requirements; this helper is for replays of unstructured traffic.
    pub fn unstructured(trace: Vec<TraceStep>) -> Self {
        Self {
            trace,
            required_critical_actions: HashSet::new(),
        }
    }

    /// Builder helper: add a required critical action kind. Returns self
    /// for chaining.
    pub fn require(mut self, kind: &str) -> Self {
        self.required_critical_actions.insert(kind.to_string());
        self
    }
}

/// Check one scenario. Returns `Ok(())` on conformance; returns the first
/// transition error on any failure.
///
/// Stages:
/// 1. **Bootstrap.** Read the first step's `state_after` as the model.
///    A trace with zero steps fails as a coverage breach (the seam was
///    reached and emitted nothing).
/// 2. **Schema-version check.** Each step's `schema_version` must equal
///    [`crate::SCHEMA_VERSION`] — a divergence means the relay and the
///    checker speak different schemas. We treat that as an illegal
///    transition because no transition rule applies.
/// 3. **Per-step transition check.** [`check_step`] runs on each step.
/// 4. **Coverage check.** After all steps pass, every entry in
///    `required_critical_actions` must appear in the trace.
pub fn check_trace(scenario: &Scenario) -> Result<(), TransitionError> {
    if scenario.trace.is_empty() {
        return Err(TransitionError::CoverageBreach {
            detail: "trace is empty — seam reached without emitting any action; \
                     this is the no-trace coverage breach"
                .to_string(),
        });
    }

    let first = &scenario.trace[0];
    if first.schema_version != crate::SCHEMA_VERSION {
        return Err(TransitionError::IllegalTransition {
            step_index: 0,
            detail: format!(
                "trace schema_version={} but checker schema_version={}",
                first.schema_version,
                crate::SCHEMA_VERSION
            ),
        });
    }
    let model = ModelState::bootstrap(&first.state_after);

    for (i, step) in scenario.trace.iter().enumerate() {
        if step.schema_version != crate::SCHEMA_VERSION {
            return Err(TransitionError::IllegalTransition {
                step_index: i,
                detail: format!(
                    "trace schema_version={} but checker schema_version={}",
                    step.schema_version,
                    crate::SCHEMA_VERSION
                ),
            });
        }
        check_step(i, &model, step)?;
    }

    // Coverage breach: required actions missing.
    let mut seen: HashSet<String> = HashSet::with_capacity(scenario.trace.len());
    for step in &scenario.trace {
        seen.insert(step.action.kind().to_string());
    }
    let missing: Vec<&String> = scenario
        .required_critical_actions
        .iter()
        .filter(|k| !seen.contains(*k))
        .collect();
    if !missing.is_empty() {
        let mut sorted: Vec<&&String> = missing.iter().collect();
        sorted.sort();
        return Err(TransitionError::CoverageBreach {
            detail: format!(
                "scenario required actions never emitted: {:?}",
                sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>()
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommunityLabel;
    use crate::{
        AbstractState, ActorLabel, ChannelLabel, HostLabel, OpaqueId, SanitizedReason, TraceAction,
        Verdict,
    };
    use uuid::Uuid;

    fn cid(u: u128) -> CommunityLabel {
        CommunityLabel::from_uuid(Uuid::from_u128(u))
    }

    fn ch(u: u128) -> ChannelLabel {
        ChannelLabel(Uuid::from_u128(u))
    }

    fn state(c: CommunityLabel) -> AbstractState {
        AbstractState {
            resolved_community: c,
            bound_host: HostLabel("h_local".into()),
            actor: ActorLabel("a_alice".into()),
        }
    }

    fn step(action: TraceAction, c: CommunityLabel) -> TraceStep {
        TraceStep::new(action, state(c))
    }

    #[test]
    fn empty_trace_is_coverage_breach() {
        let sc = Scenario::unstructured(vec![]);
        let err = check_trace(&sc).unwrap_err();
        assert!(matches!(err, TransitionError::CoverageBreach { .. }));
    }

    #[test]
    fn write_insert_then_read_with_only_resolved_rows_passes() {
        let c = cid(1);
        let trace = vec![
            step(
                TraceAction::AuthCheck {
                    channel: ch(10),
                    claimed_community: Some(c),
                    verdict: Verdict::Allow,
                },
                c,
            ),
            step(
                TraceAction::WriteInsert {
                    msg_id: OpaqueId("m1".into()),
                    channel: ch(10),
                    claimed_community: Some(c),
                },
                c,
            ),
            step(
                TraceAction::ReadMessageRows {
                    channel: Some(ch(10)),
                    row_communities: vec![c, c],
                },
                c,
            ),
        ];
        let sc = Scenario {
            trace,
            required_critical_actions: ["auth_check", "write_insert", "read_message_rows"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        check_trace(&sc).expect("trace should conform");
    }

    #[test]
    fn cross_community_row_bites_non_interference() {
        let c = cid(1);
        let foreign = cid(2);
        let trace = vec![step(
            TraceAction::ReadMessageRows {
                channel: Some(ch(10)),
                row_communities: vec![c, foreign],
            },
            c,
        )];
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        assert!(
            matches!(err, TransitionError::NonInterference { .. }),
            "expected NonInterference, got {err:?}"
        );
    }

    #[test]
    fn auth_allow_with_foreign_claim_bites_m2() {
        let c = cid(1);
        let foreign = cid(2);
        let trace = vec![step(
            TraceAction::AuthCheck {
                channel: ch(10),
                claimed_community: Some(foreign),
                verdict: Verdict::Allow,
            },
            c,
        )];
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        assert!(
            matches!(err, TransitionError::IllegalTransition { .. }),
            "expected IllegalTransition for M2 bite, got {err:?}"
        );
    }

    #[test]
    fn auth_deny_with_foreign_claim_is_fine() {
        let c = cid(1);
        let foreign = cid(2);
        let trace = vec![step(
            TraceAction::AuthCheck {
                channel: ch(10),
                claimed_community: Some(foreign),
                verdict: Verdict::Deny,
            },
            c,
        )];
        check_trace(&Scenario::unstructured(trace)).expect("deny with foreign claim is in-spec");
    }

    #[test]
    fn state_after_changing_mid_request_is_state_mismatch() {
        let c1 = cid(1);
        let c2 = cid(2);
        let trace = vec![
            step(
                TraceAction::AuthCheck {
                    channel: ch(10),
                    claimed_community: Some(c1),
                    verdict: Verdict::Allow,
                },
                c1,
            ),
            step(
                TraceAction::ReadMessageRows {
                    channel: Some(ch(10)),
                    row_communities: vec![c2],
                },
                c2,
            ),
        ];
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        assert!(
            matches!(err, TransitionError::StateMismatch { .. }),
            "expected StateMismatch, got {err:?}"
        );
    }

    #[test]
    fn impl_bug_action_bites_coverage_breach() {
        let c = cid(1);
        let trace = vec![step(
            TraceAction::ImplBug {
                kind: "ingest_exited_without_trace".into(),
            },
            c,
        )];
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        assert!(
            matches!(err, TransitionError::CoverageBreach { .. }),
            "expected CoverageBreach from ImplBug, got {err:?}"
        );
    }

    #[test]
    fn required_critical_action_missing_bites_coverage_breach() {
        let c = cid(1);
        let trace = vec![step(
            TraceAction::SanitizedError {
                reason: SanitizedReason::Restricted,
            },
            c,
        )];
        let sc = Scenario {
            trace,
            required_critical_actions: ["auth_check".to_string()].into_iter().collect(),
        };
        let err = check_trace(&sc).unwrap_err();
        assert!(
            matches!(err, TransitionError::CoverageBreach { ref detail } if detail.contains("auth_check")),
            "expected CoverageBreach naming auth_check, got {err:?}"
        );
    }

    #[test]
    fn sanitized_error_alone_is_well_formed() {
        let c = cid(1);
        for reason in [
            SanitizedReason::Restricted,
            SanitizedReason::Invalid,
            SanitizedReason::ServerError,
        ] {
            let trace = vec![step(TraceAction::SanitizedError { reason }, c)];
            check_trace(&Scenario::unstructured(trace)).expect("sanitized_error alone is in-spec");
        }
    }
}
