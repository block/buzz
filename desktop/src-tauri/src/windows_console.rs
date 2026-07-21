//! Windows GUI helpers for child processes.
//!
//! Desktop is a GUI app: spawning console-subsystem tools without
//! `CREATE_NO_WINDOW` steals focus with a flash of conhost. Prefer this helper
//! over inlining the flag at every call site.

/// Suppress the console window for a `std::process::Command` on Windows.
///
/// No-op on other platforms so call sites stay uniform.
#[inline]
pub(crate) fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW — do not allocate a new console for the child.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
