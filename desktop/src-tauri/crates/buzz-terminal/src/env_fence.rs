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

/// Keys the child is allowed to inherit from Buzz's own environment, on Unix.
///
/// Deliberately minimal: each entry is something a shell genuinely cannot
/// function without, or that visibly degrades the session by its absence.
/// Anything not listed here does not reach the child, including keys that do
/// not exist yet — which is the property a denylist cannot offer.
pub const UNIX_INHERIT_ALLOWLIST: &[&str] = &[
    "HOME",     // shell startup files, ~ expansion
    "USER",     // prompt expansion, `whoami`-adjacent tooling
    "LOGNAME",  // POSIX companion to USER
    "LANG",     // UTF-8 decoding of the child's own output
    "LC_ALL",   // explicit locale override, when set
    "LC_CTYPE", // character classification; wide/emoji handling
    "TZ",       // timestamps in prompts and logs
    "TMPDIR",   // per-user temp dir; absence breaks many tools on macOS
];

/// The Windows counterpart. Same rule — allowlist, never denylist — but the
/// set a shell cannot function without is different, and two entries are
/// load-bearing for the spawn itself, not just the session:
///
/// - `ComSpec`: `portable-pty` resolves a default program by reading `ComSpec`
///   from the **builder's** env map (`cmdbuilder.rs:671-675`), not the process
///   environment, and hands it to `CreateProcessW` as `lpApplicationName` —
///   which performs no path search. Fencing it away turned every spawn into
///   `CreateProcessW "cmd.exe" ... (os error 2)`.
/// - `USERPROFILE`: same story for the working directory — `portable-pty`
///   falls back to the builder-env `USERPROFILE` (`cmdbuilder.rs:609-611`);
///   without it the child starts with `cwd None`.
///
/// None of these carry secrets: they are machine/user layout paths and shell
/// plumbing that every process on the machine can read. The keys the fence
/// exists to withhold (`BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG`, ...) stay
/// unlisted, exactly as on Unix.
/// The list is longer than the Unix one for a structural reason rather than a
/// permissive one: a Unix shell reads rc files and a Windows shell does not,
/// so anything omitted here is omitted for the whole session with no mechanism
/// to restore it. Every entry is machine or user *layout* — paths and counts
/// any process on the box can read — and the keys the fence exists to withhold
/// stay unlisted, exactly as on Unix.
pub const WINDOWS_INHERIT_ALLOWLIST: &[&str] = &[
    "ComSpec",                // default-program resolution (see above)
    "SystemRoot",             // required by much of Win32; DLL and WMI resolution
    "SystemDrive",            // `%SystemDrive%` in scripts and installers
    "windir",                 // legacy alias of SystemRoot; old scripts read it
    "PATHEXT",                // which extensions the shell treats as executable
    "USERPROFILE",            // ~ equivalent; portable-pty's cwd fallback (see above)
    "HOMEDRIVE",              // POSIX-ish home components used by ports and MSYS tools
    "HOMEPATH",               // companion to HOMEDRIVE
    "APPDATA",                // roaming config; PowerShell profiles, npm, git
    "LOCALAPPDATA",           // local config and caches
    "ProgramData",            // machine-wide app data; chocolatey, docker, certs
    "ALLUSERSPROFILE",        // legacy alias of ProgramData
    "ProgramFiles",           // install root many tool scripts resolve through
    "ProgramFiles(x86)",      // 32-bit install root on 64-bit Windows
    "ProgramW6432",           // 64-bit install root as seen from a 32-bit process
    "PUBLIC",                 // shared user profile; some installers write here
    "PSModulePath",           // how `powershell` finds its modules at all
    "TEMP",                   // temp dir; absence breaks cmd internals and most tools
    "TMP",                    // companion to TEMP
    "USERNAME",               // prompt expansion, `whoami`-adjacent tooling
    "USERDOMAIN",             // companion to USERNAME on domain-joined machines
    "COMPUTERNAME",           // prompt expansion; build scripts label output with it
    "NUMBER_OF_PROCESSORS",   // parallelism default for cargo, make, ninja
    "PROCESSOR_ARCHITECTURE", // which binaries a script picks to run
    "OS",                     // `%OS%`, still branched on by older scripts
];

#[cfg(unix)]
const INHERIT_ALLOWLIST: &[&str] = UNIX_INHERIT_ALLOWLIST;
#[cfg(windows)]
const INHERIT_ALLOWLIST: &[&str] = WINDOWS_INHERIT_ALLOWLIST;

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
    #[cfg(not(windows))]
    cmd.env("SHELL", shell);

    // 5. (Windows) The contract key is `ComSpec`, not `SHELL`, and it is
    //    doubly load-bearing: besides telling the child what its shell is, it
    //    is what `portable-pty` spawns for a default program — read from this
    //    builder's env map and passed to `CreateProcessW` as
    //    `lpApplicationName`, which does no path search. It must be the
    //    validated absolute path from `resolve_shell`, overriding whatever
    //    value step 2 inherited.
    #[cfg(windows)]
    cmd.env("ComSpec", shell);
}
