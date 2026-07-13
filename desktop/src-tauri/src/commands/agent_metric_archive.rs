//! Build-time flag for agent-turn-metric archive default.
//!
//! When `BUZZ_BUILD_AGENT_METRIC_ARCHIVE_DEFAULT` is set at build time
//! (internal builds), `agent_metric_archive_default_enabled()` returns `true`
//! and the frontend auto-seeds an `owner_p` save subscription for kind 44200
//! (agent turn metrics) on first run for the current identity.
//!
//! OSS builds (env var unset) return `false` — no auto-seeding, user opts in
//! manually via the Local Archive settings card.

/// Returns `true` when an internal build has agent-turn-metric archive
/// default-on.
///
/// The frontend calls this once at startup to decide whether to seed the
/// `owner_p` [44200] save subscription. The result is stable for the lifetime
/// of the binary — it is baked at compile time.
#[tauri::command]
pub fn agent_metric_archive_default_enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_AGENT_METRIC_ARCHIVE_DEFAULT").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_metric_archive_default_enabled_returns_false_in_oss_build() {
        // In a standard OSS/test build (no BUZZ_DESKTOP_BUILD_AGENT_METRIC_ARCHIVE_DEFAULT
        // baked in), this must return false.
        assert!(
            !agent_metric_archive_default_enabled(),
            "expected false in OSS/test build"
        );
    }
}
