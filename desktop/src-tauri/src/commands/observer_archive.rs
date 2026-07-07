//! Build-time flag for observer-feed archive default.
//!
//! When `BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT` is set at build time (internal
//! builds), `observer_archive_default_enabled()` returns `true` and the
//! frontend auto-seeds an `owner_p` save subscription for the current identity
//! on first run.
//!
//! OSS builds (env var unset) return `false` — no auto-seeding, user opts in
//! manually via the Local Archive settings card.

/// Returns `true` when an internal build has observer-feed archive default-on.
///
/// The frontend calls this once at startup to decide whether to seed the
/// `owner_p` save subscription.  The result is stable for the lifetime of the
/// binary — it is baked at compile time.
#[tauri::command]
pub fn observer_archive_default_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_OBSERVER_ARCHIVE_DEFAULT").is_some()
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
