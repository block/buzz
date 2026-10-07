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

// ── B5 effort: single canonical representation ───────────────────────────
//
// `effective_effort` and the snapshot's `effort_level` field are the sole
// carrier of startup effort. `BUZZ_ACP_EFFORT_LEVEL` is stripped from the
// snapshot `env` so an authority handoff at an unchanged effective value
// (canonical replacing a user-env seed, or the reverse) raises no spurious
// restart badge, while a genuine effort change surfaces exactly once.

/// Look up the `env.BUZZ_ACP_EFFORT_LEVEL` leaf of a canonical snapshot, if any.
fn effort_env_leaf(canonical: &serde_json::Value) -> Option<&serde_json::Value> {
    canonical
        .get("env")
        .and_then(|env| env.get("BUZZ_ACP_EFFORT_LEVEL"))
}

/// A record whose user env seeds `BUZZ_ACP_EFFORT_LEVEL` (the pre-canonical
/// authority: no persisted `effort_level`, effort comes from user env_vars).
fn record_with_env_effort(value: &str) -> ManagedAgentRecord {
    let mut rec = record();
    rec.env_vars
        .insert("BUZZ_ACP_EFFORT_LEVEL".into(), value.into());
    rec
}

#[test]
fn effective_effort_prefers_persisted_canonical_over_user_env() {
    // Canonical wins, mirroring spawn's `apply_effort_env` (written after the
    // user env layer). The env value is ignored when a canonical is present.
    let mut rec = record();
    rec.effort_level = Some("high".into());
    let env = BTreeMap::from([("BUZZ_ACP_EFFORT_LEVEL".to_string(), "low".to_string())]);
    assert_eq!(effective_effort(&rec, &env).as_deref(), Some("high"));
}

#[test]
fn effective_effort_falls_back_to_user_env_when_no_canonical() {
    // No persisted canonical → the user-seeded env value is the effective
    // startup effort, exactly what a spawn would leave in place.
    let rec = record();
    let env = BTreeMap::from([("BUZZ_ACP_EFFORT_LEVEL".to_string(), "low".to_string())]);
    assert_eq!(effective_effort(&rec, &env).as_deref(), Some("low"));
}

#[test]
fn effective_effort_is_none_without_canonical_or_env() {
    assert_eq!(effective_effort(&record(), &BTreeMap::new()), None);
}

#[test]
fn snapshot_carries_effort_in_field_not_env() {
    // Always-canonicalize: a user-seeded effort reaches the snapshot ONLY as
    // the `effort_level` field; the raw env key is stripped so effort has one
    // representation, never two.
    let canonical = snap(&record_with_env_effort("low"));
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        Some("low"),
        "effective effort must land in the effort_level field"
    );
    assert_eq!(
        effort_env_leaf(&canonical),
        None,
        "BUZZ_ACP_EFFORT_LEVEL must be stripped from the snapshot env"
    );
}

#[test]
fn equal_value_effort_authority_handoff_env_to_canonical_is_no_op() {
    // User env `low` (no canonical) → persisted canonical `low` while the env
    // seed remains: the effective effort is `low` either way, so a restart
    // would change nothing. Old raw-env snapshots would have shown drift; the
    // single canonical representation makes the projections identical.
    let env_authority = record_with_env_effort("low");
    let mut canonical_authority = record_with_env_effort("low");
    canonical_authority.effort_level = Some("low".into());
    assert_eq!(
        snap(&env_authority),
        snap(&canonical_authority),
        "an authority handoff at the same effort value must not badge"
    );
}

#[test]
fn equal_value_effort_authority_handoff_canonical_to_env_is_no_op() {
    // The reverse direction: canonical `low` (env seed present) → env `low`
    // only (canonical cleared). Effective effort stays `low`; no badge.
    let mut canonical_authority = record_with_env_effort("low");
    canonical_authority.effort_level = Some("low".into());
    let env_authority = record_with_env_effort("low");
    assert_eq!(
        snap(&canonical_authority),
        snap(&env_authority),
        "clearing the canonical while the env seed holds the same value must not badge"
    );
}

#[test]
fn env_only_effort_edit_changes_effort_level_not_env() {
    // An env-only effort edit (no canonical) moves the single `effort_level`
    // representation and never reintroduces an `env.BUZZ_ACP_EFFORT_LEVEL`
    // leaf, so the diff names `effort_level` once rather than duplicating it.
    let low = snap(&record_with_env_effort("low"));
    let high = snap(&record_with_env_effort("high"));
    assert_ne!(
        low, high,
        "an env-only effort edit must change the snapshot"
    );
    assert_eq!(
        low.get("effort_level").and_then(|v| v.as_str()),
        Some("low")
    );
    assert_eq!(
        high.get("effort_level").and_then(|v| v.as_str()),
        Some("high")
    );
    assert_eq!(effort_env_leaf(&low), None);
    assert_eq!(effort_env_leaf(&high), None);
}

#[test]
fn canonical_effort_edit_changes_snapshot() {
    let mut low = record();
    low.effort_level = Some("low".into());
    let mut high = record();
    high.effort_level = Some("high".into());
    assert_ne!(
        snap(&low),
        snap(&high),
        "a canonical effort edit must trip the restart badge"
    );
}
