/// Seconds since the last OS-wide user input (keyboard/mouse anywhere on the
/// machine), or `None` on platforms without a supported idle API (Linux
/// Wayland). Callers fall back to in-app activity tracking when `None`.
#[tauri::command]
pub fn get_os_idle_seconds() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        macos_idle_seconds()
    }
    #[cfg(windows)]
    {
        user_idle::UserIdle::get_time()
            .ok()
            .map(|idle| idle.as_seconds())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// `user-idle`'s macOS backend over-releases a Mach port name, which modern
/// macOS treats as a fatal guarded-port violation (EXC_GUARD) and kills the
/// process. Ask Quartz directly instead: same combined-session idle time, no
/// Mach ports involved.
#[cfg(target_os = "macos")]
fn macos_idle_seconds() -> Option<u64> {
    // CGEventSourceStateID kCGEventSourceStateCombinedSessionState
    const COMBINED_SESSION_STATE: i32 = 0;
    // CGEventType kCGAnyInputEventType ((CGEventType)(~0))
    const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            state_id: i32,
            event_type: u32,
        ) -> f64;
    }

    let seconds = unsafe {
        CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE)
    };
    seconds.is_finite().then(|| seconds.max(0.0) as u64)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_idle_seconds_returns_sane_value() {
        // Regression test for the EXC_GUARD crash: querying idle time must
        // not touch Mach ports. A day of idle time during a test run means
        // something is broken.
        let idle = macos_idle_seconds().expect("idle time should be available");
        assert!(idle < 86_400);
    }
}
