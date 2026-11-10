//! `PATH` derivation for spawned PTY children.
//!
//! Buzz's own process runs under Hermit activation, so its `PATH` leads with
//! the repo's hermit `bin` and the hermit cache. Inheriting that verbatim
//! hands the user a shell whose `cargo`, `node`, and `python` are Buzz's
//! pinned build toolchain rather than the ones they installed. That is a
//! product defect, not merely untidy: `⌘J` then `cargo --version` should
//! answer for the user's machine, not for Buzz's build.
//!
//! The abandoned `feat/terminal` branch tried to solve this by subtracting
//! hermit roots from the inherited `PATH` (`terminal.rs:504-536`). The
//! subtraction never ran: `spawn_session` calls `env_remove` on `HERMIT_ENV`
//! and `ACTIVE_HERMIT` at `:339-344`, *before* `scrub_hermit_path` reads
//! those same keys at `:505-506` to learn what to strip. With both keys
//! already gone the roots list is empty and the function returns early,
//! leaving the hermit entries in place. Verified by reproduction: in that
//! order the child's `PATH` is unchanged; reversed, the hermit entries are
//! removed. A subtractive fence depends on evidence of what to subtract, and
//! that evidence is exactly what the preceding cleanup destroys.
//!
//! So `PATH` is *constructed*, not filtered. The child gets the platform's
//! standard user path, which is what a login shell would have produced had
//! Buzz never been in the picture.

/// The default user `PATH` for a spawned shell.
///
/// This intentionally does not consult Buzz's own `PATH`. A login shell reads
/// the user's rc files, which prepend their own entries (homebrew, asdf, mise,
/// `~/.local/bin`); starting from the platform default lets that happen
/// normally instead of layering it on top of Buzz's build toolchain.
#[cfg(unix)]
pub fn user_shell_path() -> String {
    // Mirrors the `_PATH_DEFPATH`/`login(1)` default: standard system
    // binaries only. `/usr/local/bin` is included because it is the
    // conventional prefix on both macOS and Linux for user-installed tools
    // that rc files expect to already be present.
    "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
}

#[cfg(windows)]
pub fn user_shell_path() -> String {
    // On Windows the system directories are derived from the environment
    // rather than fixed, and `cmd.exe`/PowerShell resolution depends on them.
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\system32;{root};{root}\system32\Wbem")
}
