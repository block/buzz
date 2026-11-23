use std::sync::Mutex;

use super::*;

const ALL: &[&str] = &["decide", "land", "receipt"];

fn id(s: &str) -> OpaqueId {
    OpaqueId(s.to_string())
}

fn ids(xs: &[&str]) -> BTreeSet<OpaqueId> {
    xs.iter().map(|x| id(x)).collect()
}

fn decide(git_ref: &str, tip: &str, read_version: u64, verdict: LandVerdict) -> LandStep {
    LandStep::new(LandAction::Decide {
        push: id("p1"),
        repo: id("r1"),
        git_ref: git_ref.to_string(),
        new_tip: id(tip),
        actor: ActorLabel("alice".to_string()),
        read_version,
        verdict,
        valid_until: None,
        evidence: ids(&["a1"]),
        checks: ids(&["ci1"]),
    })
}

fn land(refs: &[(&str, &str)], cas_version: u64, cas: CasOutcome) -> LandStep {
    LandStep::new(LandAction::Land {
        push: id("p1"),
        refs: refs.iter().map(|(r, t)| (r.to_string(), id(t))).collect(),
        cas_version,
        swapped_at: 5_000,
        cas,
    })
}

fn receipt(git_ref: &str, verdict: LandVerdict) -> LandStep {
    LandStep::new(LandAction::Receipt {
        push: id("p1"),
        git_ref: git_ref.to_string(),
        verdict,
        evidence: ids(&["a1"]),
        checks: ids(&["ci1"]),
    })
}

const MAIN: &str = "refs/heads/main";
const TAG: &str = "refs/tags/v1";

fn good() -> Vec<LandStep> {
    vec![
        decide(MAIN, "t1", 7, LandVerdict::Allow),
        land(&[(MAIN, "t1")], 7, CasOutcome::Won),
        receipt(MAIN, LandVerdict::Allow),
    ]
}

fn illegal_at(r: Result<(), TransitionError>, at: usize) {
    match r {
        Err(TransitionError::IllegalTransition { step_index, .. }) => assert_eq!(step_index, at),
        other => panic!("expected IllegalTransition at {at}, got {other:?}"),
    }
}

fn is_breach(r: Result<(), TransitionError>) {
    assert!(
        matches!(r, Err(TransitionError::CoverageBreach { .. })),
        "expected CoverageBreach, got {r:?}"
    );
}

#[test]
fn good_land_passes() {
    check_land_trace(&good(), ALL).unwrap();
}

#[test]
fn good_deny_with_receipt_passes() {
    let t = vec![
        decide(MAIN, "t1", 7, LandVerdict::Deny),
        receipt(MAIN, LandVerdict::Deny),
    ];
    check_land_trace(&t, &["decide", "receipt"]).unwrap();
}

#[test]
fn good_lost_cas_is_not_a_land() {
    // A lost swap published nothing, so neither its version nor a deny matters.
    let t = vec![
        decide(MAIN, "t1", 7, LandVerdict::Deny),
        land(&[(MAIN, "t1")], 9, CasOutcome::Lost),
        receipt(MAIN, LandVerdict::Deny),
    ];
    check_land_trace(&t, ALL).unwrap();
}

#[test]
fn authority_change_between_decide_and_cas_is_a_state_mismatch() {
    let mut t = good();
    t[1] = land(&[(MAIN, "t1")], 8, CasOutcome::Won);
    match check_land_trace(&t, ALL) {
        Err(TransitionError::StateMismatch { step_index, detail }) => {
            assert_eq!(step_index, 1);
            assert!(
                detail.contains("version 7") && detail.contains("at 8"),
                "{detail}"
            );
        }
        other => panic!("expected StateMismatch, got {other:?}"),
    }
}

#[test]
fn land_without_decision_is_illegal() {
    let t = vec![land(&[(MAIN, "t1")], 7, CasOutcome::Won)];
    illegal_at(check_land_trace(&t, ALL), 0);
}

#[test]
fn land_after_deny_is_illegal() {
    let t = vec![
        decide(MAIN, "t1", 7, LandVerdict::Deny),
        land(&[(MAIN, "t1")], 7, CasOutcome::Won),
        receipt(MAIN, LandVerdict::Deny),
    ];
    illegal_at(check_land_trace(&t, ALL), 1);
}

#[test]
fn landing_an_undecided_tip_is_illegal() {
    let mut t = good();
    t[1] = land(&[(MAIN, "t2")], 7, CasOutcome::Won);
    illegal_at(check_land_trace(&t, ALL), 1);
}

#[test]
fn multi_ref_land_needs_every_ref_allowed() {
    let t = vec![
        decide(MAIN, "t1", 7, LandVerdict::Allow),
        decide(TAG, "t1", 7, LandVerdict::Deny),
        land(&[(MAIN, "t1"), (TAG, "t1")], 7, CasOutcome::Won),
        receipt(MAIN, LandVerdict::Allow),
        receipt(TAG, LandVerdict::Deny),
    ];
    illegal_at(check_land_trace(&t, ALL), 2);
}

#[test]
fn empty_won_land_is_illegal() {
    let mut t = good();
    t[1] = land(&[], 7, CasOutcome::Won);
    illegal_at(check_land_trace(&t, ALL), 1);
}

#[test]
fn duplicate_decision_is_illegal() {
    let mut t = good();
    t.insert(1, decide(MAIN, "t1", 7, LandVerdict::Allow));
    illegal_at(check_land_trace(&t, ALL), 1);
}

#[test]
fn receipt_must_match_its_decision() {
    let mut t = good();
    t[2] = receipt(MAIN, LandVerdict::Deny);
    illegal_at(check_land_trace(&t, ALL), 2);

    let mut t = good();
    t.push(receipt(MAIN, LandVerdict::Allow));
    illegal_at(check_land_trace(&t, ALL), 3);

    let mut t = good();
    t[2] = receipt(TAG, LandVerdict::Allow);
    illegal_at(check_land_trace(&t, ALL), 2);
}

/// receipt-omits-evidence, receipt-omits-checks: the record must carry
/// what the decision counted, not merely a matching verdict.
#[test]
fn receipt_must_record_the_evidence_and_checks_counted() {
    for omit_evidence in [true, false] {
        let mut t = good();
        let LandAction::Receipt {
            evidence, checks, ..
        } = &mut t[2].action
        else {
            unreachable!()
        };
        if omit_evidence {
            evidence.clear();
        } else {
            checks.clear();
        }
        illegal_at(check_land_trace(&t, ALL), 2);
    }
}

#[test]
fn a_receipt_without_evidence_fields_does_not_parse() {
    let line = r#"{"schema_version":1,"action":{"type":"receipt","push":"p1","git_ref":"refs/heads/main","verdict":"allow"}}"#;
    assert!(serde_json::from_str::<LandStep>(line).is_err());
}

#[test]
fn deny_without_receipt_is_a_coverage_breach() {
    let t = vec![decide(MAIN, "t1", 7, LandVerdict::Deny)];
    is_breach(check_land_trace(&t, &["decide"]));
}

#[test]
fn wrong_schema_version_is_illegal() {
    let mut t = good();
    t[0].schema_version = LAND_SCHEMA_VERSION + 1;
    illegal_at(check_land_trace(&t, ALL), 0);
}

#[test]
fn missing_required_kind_and_empty_trace_are_breaches() {
    let t = vec![
        decide(MAIN, "t1", 7, LandVerdict::Deny),
        receipt(MAIN, LandVerdict::Deny),
    ];
    is_breach(check_land_trace(&t, ALL));
    is_breach(check_land_trace(&[], &[]));
}

#[test]
fn unknown_action_fails_to_parse() {
    let line = r#"{"schema_version":1,"action":{"type":"teleport","push":"p1"}}"#;
    assert!(serde_json::from_str::<LandStep>(line).is_err());
}

#[test]
fn steps_round_trip_through_json() {
    for step in good() {
        let line = serde_json::to_string(&step).unwrap();
        assert_eq!(serde_json::from_str::<LandStep>(&line).unwrap(), step);
    }
}

#[derive(Default)]
struct Collect(Mutex<Vec<LandStep>>);

impl LandTracer for Collect {
    fn record(&self, step: LandStep) {
        self.0.lock().unwrap().push(step);
    }
}

#[test]
fn guard_records_impl_bug_when_the_seam_is_silent() {
    let sink = Arc::new(Collect::default());
    {
        let (_guard, _tracer) = LandGuard::arm(sink.clone(), "land");
    }
    let trace = sink.0.lock().unwrap().clone();
    assert_eq!(
        trace,
        vec![LandStep::new(LandAction::ImplBug {
            seam: "land".to_string()
        })]
    );
    is_breach(check_land_trace(&trace, &[]));
}

#[test]
fn guard_is_silent_when_the_seam_records() {
    let sink = Arc::new(Collect::default());
    {
        let (_guard, tracer) = LandGuard::arm(sink.clone(), "land");
        for step in good() {
            tracer.record(step);
        }
    }
    let trace = sink.0.lock().unwrap().clone();
    assert_eq!(trace, good());
    check_land_trace(&trace, ALL).unwrap();
}

#[test]
fn guard_reports_the_inner_tracer_enabled_bit() {
    let (_g, on) = LandGuard::arm(Arc::new(Collect::default()), "land");
    assert!(on.enabled());
    let (_g, off) = LandGuard::arm(Arc::new(NoopLandTracer), "land");
    assert!(!off.enabled());
}
