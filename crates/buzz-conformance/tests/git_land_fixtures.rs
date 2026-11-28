//! Land-race fixtures for the Git land seam.
//!
//! `docs/spec/GitGovernanceLand.tla` shows which commit protocols make the
//! judgment of a push and the move of its pointer one step. These traces are
//! the runtime shape of its row protocol: one honest trace that must pass,
//! one trace per seeded error of that protocol (`unlocked-read`,
//! `expiry-at-read`) that must fail for the reason named for it, and two
//! guards on what the trace itself must show: a pointer that moved with no
//! won land, and a land with no receipt.
//!
//! The traces are built here and compared byte for byte with the committed
//! JSONL under `tests/fixtures/`, so a schema change is a deliberate
//! re-commit (`BUZZ_CONFORMANCE_UPDATE=1`), never a silent break.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use buzz_conformance::git_land::{check_land_trace, CasOutcome, LandAction, LandStep, LandVerdict};
use buzz_conformance::transitions::TransitionError;
use buzz_conformance::{ActorLabel, OpaqueId};

const MAIN: &str = "refs/heads/main";
const ALL: &[&str] = &["decide", "land", "receipt", "pointer_observed"];

fn id(s: &str) -> OpaqueId {
    OpaqueId(s.to_string())
}

fn ids(xs: &[&str]) -> BTreeSet<OpaqueId> {
    xs.iter().map(|x| id(x)).collect()
}

fn main_at(tip: &str) -> BTreeMap<String, OpaqueId> {
    BTreeMap::from([(MAIN.to_string(), id(tip))])
}

fn observed(tip: &str) -> LandStep {
    LandStep::new(LandAction::PointerObserved {
        repo: id("r1"),
        refs: main_at(tip),
    })
}

/// A push by the agent `g`, which acts under a grant that lapses at
/// `valid_until`.
fn decide(push: &str, read_version: u64, valid_until: u64, verdict: LandVerdict) -> LandStep {
    LandStep::new(LandAction::Decide {
        push: id(push),
        repo: id("r1"),
        git_ref: MAIN.to_string(),
        new_tip: id("t1"),
        actor: ActorLabel("g".to_string()),
        read_version,
        verdict,
        valid_until: Some(valid_until),
        evidence: ids(&["approval-d"]),
        checks: ids(&["ci-pass-3"]),
    })
}

fn land(push: &str, cas_version: u64, swapped_at: u64, cas: CasOutcome) -> LandStep {
    LandStep::new(LandAction::Land {
        push: id(push),
        refs: main_at("t1"),
        cas_version,
        swapped_at,
        cas,
    })
}

fn receipt(push: &str, verdict: LandVerdict) -> LandStep {
    LandStep::new(LandAction::Receipt {
        push: id(push),
        git_ref: MAIN.to_string(),
        verdict,
        evidence: ids(&["approval-d"]),
        checks: ids(&["ci-pass-3"]),
    })
}

/// The row protocol, honestly traced. One transaction locks the repo's
/// authority version for share, judges at version 7, swaps the pointer row,
/// re-reads version 7 and records the receipt; the swap statement ran at
/// 5000, before the grant lapses at 9000. A revoke then bumps the version to
/// 8, and a second push by the same agent is judged at 8 and refused.
fn land_row() -> Vec<LandStep> {
    vec![
        observed("t0"),
        decide("p1", 7, 9_000, LandVerdict::Allow),
        land("p1", 7, 5_000, CasOutcome::Won),
        receipt("p1", LandVerdict::Allow),
        observed("t1"),
        decide("p2", 8, 9_000, LandVerdict::Deny),
        receipt("p2", LandVerdict::Deny),
        observed("t1"),
    ]
}

/// The stored pointer moved, and no won land in the trace moved it: a write
/// outside the land transaction. Only the pointer, read from storage, shows
/// it.
fn pointer_out_of_band() -> Vec<LandStep> {
    vec![
        observed("t0"),
        decide("p1", 7, 9_000, LandVerdict::Allow),
        observed("t1"),
    ]
}

/// The pointer moved by a won land that never recorded its receipt.
fn land_without_receipt() -> Vec<LandStep> {
    vec![
        observed("t0"),
        decide("p1", 7, 9_000, LandVerdict::Allow),
        land("p1", 7, 5_000, CasOutcome::Won),
        observed("t1"),
    ]
}

/// expiry-at-read: no authority write happened, so both versions are 7, but
/// the swap trusted the clock from the read: the agent's grant lapsed at
/// 5000 and the swap ran at 5000. Authority holds while the clock is
/// before the expiry, so the boundary itself is late.
fn expiry_at_read() -> Vec<LandStep> {
    vec![
        observed("t0"),
        decide("p1", 7, 5_000, LandVerdict::Allow),
        land("p1", 7, 5_000, CasOutcome::Won),
        receipt("p1", LandVerdict::Allow),
        observed("t1"),
    ]
}

/// unlocked-read: the read at version 7 took no lock, so a revoke committed
/// and moved the version to 8 before the swap.
fn unlocked_read() -> Vec<LandStep> {
    vec![
        observed("t0"),
        decide("p1", 7, 9_000, LandVerdict::Allow),
        land("p1", 8, 5_000, CasOutcome::Won),
        receipt("p1", LandVerdict::Allow),
        observed("t1"),
    ]
}

// ---- Fixture round-trip ------------------------------------------------

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn to_jsonl(trace: &[LandStep]) -> String {
    trace
        .iter()
        .map(|s| serde_json::to_string(s).expect("step serializes") + "\n")
        .collect()
}

/// The committed fixture, parsed, after checking it is byte for byte the
/// trace built above.
fn committed(name: &str, trace: &[LandStep]) -> Vec<LandStep> {
    let path = fixture_path(name);
    let expected = to_jsonl(trace);
    if std::env::var("BUZZ_CONFORMANCE_UPDATE").is_ok() {
        fs::write(&path, &expected).expect("write fixture");
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; regenerate with BUZZ_CONFORMANCE_UPDATE=1",
            path.display()
        )
    });
    assert_eq!(
        text, expected,
        "{name} drifted from the trace that defines it"
    );
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("{name} line {}: {e}", i + 1))
        })
        .collect()
}

fn fails_with(name: &str, trace: &[LandStep], at: Option<usize>, reason: &str) {
    match check_land_trace(&committed(name, trace), ALL) {
        Err(TransitionError::StateMismatch { step_index, detail }) if at.is_some() => {
            assert_eq!(Some(step_index), at, "{name}: {detail}");
            assert!(
                detail.contains(reason),
                "{name} failed for another reason: {detail}"
            );
        }
        Err(TransitionError::CoverageBreach { detail }) if at.is_none() => {
            assert!(
                detail.contains(reason),
                "{name} failed for another reason: {detail}"
            );
        }
        other => panic!("{name}: expected the failure named for it, got {other:?}"),
    }
}

#[test]
fn land_row_passes() {
    check_land_trace(&committed("git_land_row.jsonl", &land_row()), ALL).unwrap();
}

#[test]
fn a_pointer_moved_out_of_band_fails() {
    fails_with(
        "git_land_bad_pointer_out_of_band.jsonl",
        &pointer_out_of_band(),
        Some(2),
        "pointer moved with no won land",
    );
}

#[test]
fn a_land_without_receipt_fails() {
    fails_with(
        "git_land_bad_land_without_receipt.jsonl",
        &land_without_receipt(),
        None,
        "has no receipt",
    );
}

#[test]
fn expiry_at_read_fails() {
    fails_with(
        "git_land_bad_expiry_at_read.jsonl",
        &expiry_at_read(),
        Some(2),
        "lapsed at 5000",
    );
}

#[test]
fn unlocked_read_fails() {
    fails_with(
        "git_land_bad_unlocked_read.jsonl",
        &unlocked_read(),
        Some(2),
        "authority version 7 but landed at 8",
    );
}

/// An emitter that leaves out the expiry, or the swap time, must not
/// read as "nothing lapses".
#[test]
fn a_decision_without_valid_until_does_not_parse() {
    let mut v: serde_json::Value =
        serde_json::from_str(to_jsonl(&land_row()[1..2]).trim()).unwrap();
    assert!(
        serde_json::from_value::<LandStep>(v.clone()).is_ok(),
        "control must parse"
    );
    v["action"].as_object_mut().unwrap().remove("valid_until");
    assert!(serde_json::from_value::<LandStep>(v).is_err());
    let mut v: serde_json::Value =
        serde_json::from_str(to_jsonl(&land_row()[2..3]).trim()).unwrap();
    v["action"].as_object_mut().unwrap().remove("swapped_at");
    assert!(serde_json::from_value::<LandStep>(v).is_err());
}
