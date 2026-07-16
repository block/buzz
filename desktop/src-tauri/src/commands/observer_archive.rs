//! Build-time flag and runtime dev-nest check for observer-feed archive policy.
//!
//! `observer_archive_default_enabled()` returns `true` when either:
//! - `BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT` was set at build time (internal
//!   builds bake in the flag via `build.rs`), **or**
//! - the running binary is using the dev nest (`~/.buzz-dev`), which is the
//!   case for all dev builds launched with `just staging` or `just dev`.
//!
//! When `true`, the frontend reconciles the observer archive subscription
//! every startup — unconditionally ensuring kind 24200 exists in the DB
//! regardless of stale localStorage markers.
//!
//! OSS prod builds (baked flag unset, prod nest `~/.buzz`) return `false` —
//! no reconciliation; the user manages the subscription via Settings.

/// Returns `true` when observer-feed archive policy is enforced.
///
/// True when the build has the internal baked flag set, or when the running
/// binary is using the dev nest (`~/.buzz-dev`). The frontend calls this
/// every startup to decide whether to reconcile the `owner_p` subscription.
#[tauri::command]
pub fn observer_archive_default_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_OBSERVER_ARCHIVE_DEFAULT").is_some()
        || crate::managed_agents::nest_is_dev()
}

#[cfg(test)]
mod tests {
    use super::*;

    // `nest_is_dev()` is deterministic-false in unit tests: NEST_DIR OnceLock
    // is uninitialized → falls back to prod `~/.buzz` (nest.rs:101-106), so
    // the compiled flag is the sole variable. No runner normalization needed.
    //
    // #[ignore]: requires BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT to be
    // set — `just desktop-tauri-test-compiled-flags` runs it explicitly with
    // `--ignored` under both compile states; general `cargo test` skips it.
    #[test]
    #[ignore]
    fn test_observer_archive_default_enabled_matches_expected() {
        let result = observer_archive_default_enabled();
        let expected_str = std::env::var("BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT").expect(
            "BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT must be set — \
                 the dual-compile CI step supplies it; bare `cargo test` is \
                 not sufficient to validate compiled-flag behavior",
        );
        let expected = expected_str == "true" || expected_str == "1";
        assert_eq!(
            result, expected,
            "observer_archive_default_enabled() returned {result}, \
             expected {expected} (BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT={expected_str:?})"
        );
    }
}
