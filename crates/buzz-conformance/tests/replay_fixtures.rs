//! Replay-fixture integration test.
//!
//! These fixtures are the load-bearing evidence that the runtime
//! conformance gate is **not decorative**. Each fixture is one
//! end-to-end JSONL trace, replayed through [`check_trace`], with the
//! expected verdict baked into the assertion.
//!
//! Eva's review (thread `06aaf3f7…`) green-lit cutting these as the
//! visible proof the gate bites. Coverage:
//!
//! - `good.jsonl` — a positive trace shaped like a real ingest:
//!   AuthCheck Allow → WriteInsert → ReadMessageRows with rows confined
//!   to the resolved community. `check_trace` returns `Ok(())`.
//! - `bad_host_channel_mismatch.jsonl` — a host/channel fence skip:
//!   the bound host is for community A, the write targets a channel in
//!   community B. The checker fails with `IllegalTransition`.
//! - `bad_coverage_breach.jsonl` — a trace that contains an `ImplBug`
//!   action (what `EmitGuard::Drop` emits when a critical seam exits
//!   without recording anything). The checker fails with
//!   `CoverageBreach`.
//!
//! The JSONL files are committed as "golden" artifacts under
//! `tests/fixtures/` for reviewer visibility, but this test also
//! round-trips: it constructs the trace in Rust, serializes it to a
//! temp file, reads it back, and asserts both the serialized form
//! matches the committed file AND the parsed form gives the expected
//! verdict. That way a schema change cannot silently desync the
//! committed JSONL from what the relay actually emits.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use buzz_conformance::checker::{check_trace, Scenario};
use buzz_conformance::transitions::TransitionError;
use buzz_conformance::{
    AbstractState, ActorLabel, ChannelLabel, CommunityLabel, HostLabel, OpaqueId, TraceAction,
    TraceStep, Verdict,
};
use uuid::Uuid;

// ---- Stable test-fixture labels ----------------------------------------
//
// These values are deterministic so the serialized JSONL is reproducible
// across runs. They are NOT secrets and they don't shadow any real
// community — they're test-only constants.

fn community_a() -> CommunityLabel {
    CommunityLabel::from_uuid(Uuid::from_u128(0xAAAA_0000_0000_0000_0000_0000_0000_0001))
}

fn community_b() -> CommunityLabel {
    CommunityLabel::from_uuid(Uuid::from_u128(0xBBBB_0000_0000_0000_0000_0000_0000_0002))
}

fn channel_in_a() -> ChannelLabel {
    ChannelLabel(Uuid::from_u128(0xCAFE_0000_0000_0000_0000_0000_0000_0010))
}

fn channel_in_b() -> ChannelLabel {
    ChannelLabel(Uuid::from_u128(0xDEAD_0000_0000_0000_0000_0000_0000_0020))
}

fn state_a() -> AbstractState {
    AbstractState {
        resolved_community: community_a(),
        bound_host: HostLabel("a.example.test".to_string()),
        actor: ActorLabel("0123456789abcdef".to_string()),
    }
}

// ---- Trace builders ----------------------------------------------------

/// A positive trace: bound to community A, all observations confined.
fn good_trace() -> Vec<TraceStep> {
    vec![
        TraceStep::new(
            TraceAction::AuthCheck {
                channel: channel_in_a(),
                claimed_community: Some(community_a()),
                verdict: Verdict::Allow,
            },
            state_a(),
        ),
        TraceStep::new(
            TraceAction::WriteInsert {
                msg_id: OpaqueId("d34db33fcafef00d".to_string()),
                channel: channel_in_a(),
                claimed_community: Some(community_a()),
            },
            state_a(),
        ),
        TraceStep::new(
            TraceAction::ReadMessageRows {
                channel: Some(channel_in_a()),
                row_communities: vec![community_a(), community_a()],
            },
            state_a(),
        ),
    ]
}

/// A bad trace: the host-channel fence was bypassed. The bound host
/// resolves to community A, but a WriteInsert targets a channel in
/// community B. The spec's `Inv_NonInterference` / channel-host coupling
/// rule rejects this as an illegal transition.
fn bad_host_channel_mismatch_trace() -> Vec<TraceStep> {
    vec![
        TraceStep::new(
            TraceAction::AuthCheck {
                channel: channel_in_b(),
                // Client claims B, host resolves A, fence was skipped:
                // AuthCheck recorded `verdict = Allow` despite the
                // mismatch. M2/M8 territory.
                claimed_community: Some(community_b()),
                verdict: Verdict::Allow,
            },
            state_a(),
        ),
        TraceStep::new(
            TraceAction::WriteInsert {
                msg_id: OpaqueId("badbadbad0000000".to_string()),
                channel: channel_in_b(),
                claimed_community: Some(community_b()),
            },
            state_a(),
        ),
    ]
}

/// A coverage-breach trace: an `ImplBug` step appears, meaning the
/// `EmitGuard` fired on Drop. The checker treats any `ImplBug` as a
/// hard coverage breach.
fn bad_coverage_breach_trace() -> Vec<TraceStep> {
    vec![TraceStep::new(
        TraceAction::ImplBug {
            kind: "ingest_exited_without_trace".to_string(),
        },
        state_a(),
    )]
}

// ---- Fixture round-trip ------------------------------------------------

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Serialize a trace to JSONL (one step per line).
fn to_jsonl(trace: &[TraceStep]) -> String {
    let mut out = String::new();
    for step in trace {
        let line = serde_json::to_string(step).expect("step serializes");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Parse a JSONL string into a trace, surfacing the offending line on
/// error so a misedited fixture is easy to fix.
fn from_jsonl(text: &str) -> Vec<TraceStep> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str::<TraceStep>(l)
                .unwrap_or_else(|e| panic!("fixture line {} did not parse: {e}", i + 1))
        })
        .collect()
}

/// Assert that the committed JSONL fixture for `name` round-trips to
/// `expected_trace` byte-exactly. Run with `BUZZ_CONFORMANCE_UPDATE=1`
/// to regenerate the fixture (so a schema change is a deliberate
/// re-commit, not a silent break).
fn assert_fixture_matches(name: &str, expected_trace: &[TraceStep]) {
    let expected = to_jsonl(expected_trace);
    let path = fixture_path(name);

    if std::env::var("BUZZ_CONFORMANCE_UPDATE").is_ok() {
        fs::create_dir_all(path.parent().expect("fixture dir")).expect("mkdir fixtures");
        fs::write(&path, &expected).expect("write fixture");
        return;
    }

    let actual = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "fixture {} missing or unreadable ({e}); run with \
             BUZZ_CONFORMANCE_UPDATE=1 to create it",
            path.display()
        )
    });

    assert_eq!(
        actual, expected,
        "committed fixture {} drifted from the typed builder; run with \
         BUZZ_CONFORMANCE_UPDATE=1 to refresh if the change is intentional",
        name
    );

    let parsed = from_jsonl(&actual);
    assert_eq!(parsed, *expected_trace, "fixture round-trip mismatched");
}

// ---- Tests --------------------------------------------------------------

#[test]
fn good_trace_passes_check() {
    let trace = good_trace();
    assert_fixture_matches("good.jsonl", &trace);

    let scenario = Scenario {
        trace,
        required_critical_actions: ["auth_check", "write_insert", "read_message_rows"]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>(),
    };
    check_trace(&scenario).expect("the good fixture must replay green");
}

#[test]
fn bad_host_channel_mismatch_is_illegal_transition() {
    let trace = bad_host_channel_mismatch_trace();
    assert_fixture_matches("bad_host_channel_mismatch.jsonl", &trace);

    let scenario = Scenario::unstructured(trace);
    let err = check_trace(&scenario)
        .expect_err("host/channel fence skip must be rejected by the checker");
    assert!(
        matches!(err, TransitionError::IllegalTransition { .. }),
        "host/channel mismatch must surface as IllegalTransition (M2/M8 bite), got {err:?}"
    );
}

#[test]
fn coverage_breach_is_caught() {
    let trace = bad_coverage_breach_trace();
    assert_fixture_matches("bad_coverage_breach.jsonl", &trace);

    let scenario = Scenario::unstructured(trace);
    let err = check_trace(&scenario)
        .expect_err("ImplBug in the trace must be rejected as a coverage breach");
    assert!(
        matches!(err, TransitionError::CoverageBreach { .. }),
        "ImplBug must surface as CoverageBreach, got {err:?}"
    );
}

#[test]
fn empty_trace_is_coverage_breach() {
    // Independent of the JSONL fixtures: the checker must fail closed on
    // an empty trace (no observations from a critical seam).
    let scenario = Scenario::unstructured(vec![]);
    let err = check_trace(&scenario).expect_err("empty trace must be CoverageBreach");
    assert!(
        matches!(err, TransitionError::CoverageBreach { .. }),
        "empty trace must be CoverageBreach, got {err:?}"
    );
}

#[test]
fn missing_required_action_is_coverage_breach() {
    // The good trace, but the scenario declares it must include
    // `read_by_id_rows` — which it does not. This is what the
    // "scenario-required action never appeared" coverage breach catches.
    let scenario = Scenario {
        trace: good_trace(),
        required_critical_actions: ["read_by_id_rows"]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>(),
    };
    let err = check_trace(&scenario)
        .expect_err("missing required critical action must be CoverageBreach");
    assert!(
        matches!(err, TransitionError::CoverageBreach { .. }),
        "missing required action must be CoverageBreach, got {err:?}"
    );
}
