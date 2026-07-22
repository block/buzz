//! Suppress console windows for child processes on Windows GUI hosts.
//!
//! Buzz desktop launches this MCP server windowless; children that allocate a
//! console flash a conhost. Use these helpers instead of inlining
//! `CREATE_NO_WINDOW` at each spawn site.

/// Hide the console for a `std::process::Command` (no-op on non-Windows).
pub(crate) fn hide_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Hide the console for a `tokio::process::Command` (no-op on non-Windows).
pub(crate) fn hide_tokio(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
