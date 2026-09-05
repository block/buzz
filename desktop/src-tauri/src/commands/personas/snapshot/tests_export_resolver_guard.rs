//! Source guard: every export resolver folds the private-config overlay.
//!
//! Kept in a sibling file so `snapshot/tests.rs` stays under the 1000-line
//! gate; `#[path]`-included from `snapshot.rs` as a child module.
//!
//! The behavioural tests in `tests.rs`/`tests_overlay_fold.rs` model the fold
//! by calling `resolved_records` / `resolve_from_lists` directly. They CANNOT
//! reach the production call sites: all three live inside `#[tauri::command]`
//! bodies needing a live `AppHandle` + `State<AppState>`. Measured before this
//! guard existed — deleting any one of the three production folds left the
//! full workspace suite green (2283 passed, 0 failed).
//!
//! Same weakness, same remedy, as `write_site_resolve_guard` in
//! `private_config_overlay.rs`: assert the call exists at each site.
//!
//! The fold lives in one helper, `load_effective_managed_agents`, so the
//! guarded invariant has three legs:
//!   1. every resolver site calls the helper (exact count per file);
//!   2. no resolver file reads raw `load_managed_agents(` outside the helper;
//!   3. the helper itself still performs the fold.

/// Call-shaped needle for the helper. A resolver that reads raw disk instead
/// fails with "agent not found" for a relay-only agent that has never started
/// on this device, and bakes stale disk values into the exported manifest on
/// a follower device.
const HELPER_CALL: &str = "load_effective_managed_agents(&app, &state)?";

/// The fold inside the helper.
const FOLD_CALL: &str = ".resolved_records(&instances)";

/// Raw-disk read. Exact counts per file: the only permitted occurrence is the
/// one inside the helper in `snapshot.rs`.
const RAW_LOAD_CALL: &str = "load_managed_agents(";

/// `(file, source, expected_helper_calls, allowed_raw_loads)` — every export
/// resolver that turns an id into a record destined for a snapshot manifest
/// or a card.
fn sites() -> Vec<(&'static str, &'static str, usize, usize)> {
    vec![
        // `materialize_snapshot_bytes` (JSON + PNG + send-to-channel): 1
        // helper call. 1 raw load allowed: the helper's own body.
        (
            "commands/personas/snapshot.rs",
            include_str!("../snapshot.rs"),
            1,
            1,
        ),
        // Card precheck (`card_mint_key_status`) + mint resolver
        // (`mint_agent_card`): 2 helper calls, 0 raw loads. Exact, not
        // `>= 1`: a lower bound would not notice one site losing its call
        // while the other gained a second.
        (
            "commands/personas/card.rs",
            include_str!("../card.rs"),
            2,
            0,
        ),
    ]
}

#[test]
fn every_export_resolver_folds_the_private_config_overlay() {
    for (file, source, expected, allowed_raw) in sites() {
        let found = source.matches(HELPER_CALL).count();
        assert_eq!(
            found, expected,
            "{file}: expected {expected} `{HELPER_CALL}` call(s), found {found}. \
             An export resolver that reads raw disk fails with \"agent not found\" \
             for a relay-only agent that has never started on this device, and \
             bakes stale disk values into the exported manifest on a follower."
        );
        let raw = source.matches(RAW_LOAD_CALL).count();
        assert_eq!(
            raw, allowed_raw,
            "{file}: expected {allowed_raw} raw `{RAW_LOAD_CALL}` call(s), found \
             {raw}. A resolver bypassing `load_effective_managed_agents` reads \
             raw disk and skips the kind:30179 overlay fold."
        );
    }
}

/// The helper must actually fold — the resolver-site counts above would pass
/// vacuously if `load_effective_managed_agents` degraded to a plain load.
#[test]
fn the_helper_itself_performs_the_fold() {
    let source = include_str!("../snapshot.rs");
    let found = source.matches(FOLD_CALL).count();
    assert_eq!(
        found, 1,
        "snapshot.rs: expected exactly 1 `{FOLD_CALL}` call (inside \
         `load_effective_managed_agents`), found {found}"
    );
}

/// The guards above are substring counts, so prove they can FAIL: a source
/// with the calls removed must not satisfy them. Without this, a typo in a
/// needle would make every row pass vacuously.
#[test]
fn guard_detects_missing_calls() {
    for (file, source, expected, _) in sites() {
        let stripped = source.replace(HELPER_CALL, "REMOVED(&app, &state)?");
        assert_eq!(
            stripped.matches(HELPER_CALL).count(),
            0,
            "{file}: helper needle never matched, so its {expected} expected \
             call(s) passed vacuously"
        );
    }
    let source = include_str!("../snapshot.rs");
    let stripped = source.replace(FOLD_CALL, ".REMOVED(&instances)");
    assert_eq!(
        stripped.matches(FOLD_CALL).count(),
        0,
        "snapshot.rs: fold needle never matched, so the helper check passed \
         vacuously"
    );
}
