//! Build-time flag and runtime dev-nest check for observer-feed archive default.
//!
//! `observer_archive_default_enabled()` returns `true` when either:
//! - `BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT` was set at build time (internal
//!   builds bake in the flag via `build.rs`), **or**
//! - the running binary is using the dev nest (`~/.buzz-dev`), which is the
//!   case for all dev builds launched with `just staging` or `just dev`.
//!
//! When `true`, the frontend auto-seeds an `owner_p` save subscription for the
//! current identity on first run, so the observer-feed archive is on by default.
//!
//! OSS prod builds (baked flag unset, prod nest `~/.buzz`) return `false` —
//! no auto-seeding; the user opts in manually via the Local Archive settings card.

/// Returns `true` when observer-feed archive should default to on.
///
/// True when the build has the internal baked flag set, or when the running
/// binary is using the dev nest (`~/.buzz-dev`). The frontend calls this once
/// at startup to decide whether to seed the `owner_p` save subscription.
#[tauri::command]
pub fn observer_archive_default_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_OBSERVER_ARCHIVE_DEFAULT").is_some()
        || crate::managed_agents::nest_is_dev()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observer_archive_default_enabled_returns_bool() {
        // The command must return a plain bool without panicking.
        // Whether it's true or false depends on the build environment;
        // what we assert here is just that the return type is correct and
        // the function is callable.
        let result = observer_archive_default_enabled();
        // In a standard OSS/test build (no BUZZ_DESKTOP_BUILD_OBSERVER_ARCHIVE_DEFAULT
        // baked in), this should be false.
        assert!(!result, "expected false in OSS/test build");
    }
}
