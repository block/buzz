/// Seconds since the last OS-wide user input (keyboard/mouse anywhere on the
/// machine), or `None` on platforms without a supported idle API (Linux
/// Wayland). Callers fall back to in-app activity tracking when `None`.
#[tauri::command]
pub fn get_os_idle_seconds() -> Option<u64> {
    #[cfg(any(target_os = "macos", windows))]
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
