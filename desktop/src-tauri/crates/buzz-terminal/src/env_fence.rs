//! Environment fence for spawned PTY children.
//!
//! Buzz's own process holds `BUZZ_PRIVATE_KEY` (an nsec), `BUZZ_AUTH_TAG`, and
//! relay credentials. `portable_pty::CommandBuilder::new()` pre-seeds its env
//! map from `std::env::vars_os()` (`cmdbuilder.rs:218` -> `get_base_env()`
//! `:74`), so a shell spawned with the default builder inherits **all** of it:
//! the user types `env` and reads the signing key off the screen.
//!
//! The in-repo `feat/terminal` branch (`4f287d158`, abandoned 2026-05-22)
//! demonstrates the failure mode this module exists to prevent. It removed
//! seven Hermit/macOS keys by denylist under a comment promising "a clean
//! environment" and passed 68 variables — including the nsec — to the child.
//! A denylist is only as current as the last time someone remembered to
//! extend it; it was correct for the polluted-`PATH` threat it was written
//! for and became a key-disclosure bug when the app started holding secrets.
//!
//! So: **allowlist, never denylist.** Clear the inherited environment
//! wholesale, then rebuild only what a terminal legitimately needs.

use portable_pty::CommandBuilder;

/// Keys the child is allowed to inherit from Buzz's own environment.
///
/// Deliberately minimal: each entry is something a shell genuinely cannot
/// function without, or that visibly degrades the session by its absence.
/// Anything not listed here does not reach the child, including keys that do
/// not exist yet — which is the property a denylist cannot offer.
const INHERIT_ALLOWLIST: &[&str] = &[
    "HOME",     // shell startup files, ~ expansion
    "USER",     // prompt expansion, `whoami`-adjacent tooling
    "LOGNAME",  // POSIX companion to USER
    "LANG",     // UTF-8 decoding of the child's own output
    "LC_ALL",   // explicit locale override, when set
    "LC_CTYPE", // character classification; wide/emoji handling
    "TZ",       // timestamps in prompts and logs
    "TMPDIR",   // per-user temp dir; absence breaks many tools on macOS
];

/// Values Buzz sets on the child unconditionally, overriding any inherited
/// value. `TERM` in particular must describe *our* emulator, not whatever
/// terminal happened to launch the desktop app.
const OVERRIDES: &[(&str, &str)] = &[
    ("TERM", "xterm-256color"),
    ("TERM_PROGRAM", "Buzz"),
    ("COLORTERM", "truecolor"),
];

/// Applies the environment fence to `cmd`, returning it for chaining.
///
/// Ordering is load-bearing and the reverse fails silently: `env_clear()`
/// discards every accumulated entry, so clearing *after* populating yields a
/// child with an empty environment and no error anywhere. Clear first, then
/// rebuild.
///
/// `shell` is the *resolved* shell from [`crate::shell::resolve_shell`], and
/// it is injected rather than inherited. Buzz's own `SHELL` and the shell we
/// actually spawn are different values in exactly the cases the resolution
/// fallback exists for — a Finder-launched app with no `$SHELL`, or a
/// `$SHELL` that fails the executable-regular-file check — so inheriting it
/// would tell the child it is running something it is not.
pub fn fence_env(cmd: &mut CommandBuilder, path: &str, shell: &str) {
    // 1. Drop the inherited environment wholesale, secrets included.
    cmd.env_clear();

    // 2. Rebuild only the allowlisted keys that are actually present.
    for key in INHERIT_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }

    // 3. Apply Buzz's own terminal identity.
    for (key, value) in OVERRIDES {
        cmd.env(key, value);
    }

    // 4. PATH is supplied by the caller rather than inherited; see
    //    `path::user_shell_path`.
    cmd.env("PATH", path);

    // 5. The resolved shell, last. `CommandBuilder::as_command` writes its own
    //    `SHELL` before applying this map (`cmdbuilder.rs:528-536`), so our
    //    explicit entry is the one the child sees.
    cmd.env("SHELL", shell);
}
