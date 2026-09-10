const ONBOARDING_PREVIEW_ENV: &str = "BUZZ_ONBOARDING_PREVIEW";

fn enabled_for(debug_build: bool, value: Option<&str>) -> bool {
    debug_build && value == Some("1")
}

/// Whether the development-only native onboarding workshop is active.
pub(crate) fn enabled() -> bool {
    enabled_for(
        cfg!(debug_assertions),
        std::env::var(ONBOARDING_PREVIEW_ENV).ok().as_deref(),
    )
}

/// Fail closed before app state could generate a placeholder identity.
pub(crate) fn require_fixed_identity<T>(identity: Option<T>) -> Option<T> {
    assert!(
        identity.is_some() || !enabled(),
        "BUZZ_ONBOARDING_PREVIEW requires fixed mock identity data"
    );
    identity
}

/// Stop native setup before migrations, persistence, networking, or agents.
pub(crate) fn skip_native_setup() -> bool {
    if !enabled() {
        return false;
    }
    eprintln!(
        "buzz-desktop: onboarding preview safety mode; skipped migrations, identity persistence, relay sync, and agent restore"
    );
    true
}

/// Preview does not wait for the normal app's first-render event.
pub(crate) fn reveal_window<R: tauri::Runtime>(window: &tauri::Window<R>) -> bool {
    if !enabled() {
        return false;
    }
    crate::initial_window::reveal_initial_window(window);
    true
}

#[cfg(test)]
mod tests {
    use super::enabled_for;

    #[test]
    fn preview_is_debug_only_and_requires_exact_opt_in() {
        assert!(enabled_for(true, Some("1")));
        assert!(!enabled_for(true, Some("true")));
        assert!(!enabled_for(true, None));
        assert!(!enabled_for(false, Some("1")));
    }
}
