//! B5 effort lifecycle tests split out of `spawn_snapshot/tests.rs` to hold
//! that file under the 1000-line file-size ratchet.
//!
//! Included as `mod ext` inside `tests.rs`, so `use super::*` gives access to
//! its `record`, `snap`, and `record_with_env_effort` helpers.

use super::*;

#[test]
fn effort_set_then_cleared_round_trips_to_no_effort_projection() {
    // Persist a canonical effort, then clear it: the projection must return to
    // the exact no-effort baseline, so the badge lights on set and clears on
    // clear rather than sticking.
    let baseline = snap(&record());
    let mut set = record();
    set.effort_level = Some("high".into());
    assert_ne!(baseline, snap(&set), "setting canonical effort must badge");
    // Clear the SAME record back to None — the projection must return to the
    // exact no-effort baseline, proving the round-trip clears rather than a
    // fresh record merely matching baseline.
    set.effort_level = None;
    assert_eq!(
        baseline,
        snap(&set),
        "clearing canonical effort restores the no-effort projection"
    );
}

#[test]
fn shadowed_user_env_effort_edit_under_canonical_is_empty_diff() {
    // Canonical `high` shadows the user env seed. Editing that seed low→medium
    // changes nothing effective (canonical wins and the env key is stripped),
    // so the projections are identical and no badge lights.
    let mut low_env = record_with_env_effort("low");
    low_env.effort_level = Some("high".into());
    let mut medium_env = record_with_env_effort("medium");
    medium_env.effort_level = Some("high".into());
    assert_eq!(
        snap(&low_env),
        snap(&medium_env),
        "editing a canonical-shadowed user env must not badge"
    );
}

#[test]
fn clearing_canonical_reveals_env_fallback_and_creates_a_diff() {
    // Canonical `high` over a user env seed `low`: clearing the canonical drops
    // the effective effort to the env fallback `low`, a real change that badges.
    let mut canonical = record_with_env_effort("low");
    canonical.effort_level = Some("high".into());
    let env_only = record_with_env_effort("low");
    assert_ne!(
        snap(&canonical),
        snap(&env_only),
        "clearing canonical must reveal the env fallback and badge"
    );
}
