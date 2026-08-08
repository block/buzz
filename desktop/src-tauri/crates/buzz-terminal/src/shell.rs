//! Login-shell resolution for spawned PTY children.
//!
//! Tyler asked for the user's shell of choice, so the resolution order is the
//! user's own: `$SHELL`, then the passwd entry, then `/bin/sh`. What matters
//! is the *validity* test applied at each step, and it is not "the path
//! exists".
//!
//! `portable-pty` gates both steps on `access(X_OK)` (`cmdbuilder.rs:545-553`
//! for `$SHELL`, `:43-71` for passwd). `access(X_OK)` answers "may I execute
//! this" for *any* file type, and a directory carries the execute bit to mean
//! "may I traverse it" — so `access("/tmp", X_OK)` returns 0. Verified by C
//! repro and end-to-end through a real PTY: with `SHELL=/tmp`,
//! `CommandBuilder::get_shell()` returns `"/tmp"`, `spawn_command` returns
//! `Ok`, and the child dies with exit code 1 after printing
//! `fatal runtime error: assertion failed: output.write(&bytes).is_ok()`.
//! The user gets a terminal that opens and instantly dies with a Rust runtime
//! panic, and every layer above reported success.
//!
//! So we require an **executable regular file**, following symlinks: `stat`
//! rather than `lstat` semantics, because `/bin/sh` is legitimately a symlink
//! on many systems. A directory or a non-executable file falls through to the
//! next candidate instead of becoming an unspawnable child.

#[cfg(unix)]
use std::path::Path;

/// Last-resort shell. POSIX guarantees `/bin/sh`; if this is not executable
/// the machine has bigger problems than our terminal.
pub const FALLBACK_SHELL: &str = "/bin/sh";

/// Returns true if `path` is a regular file this process may execute.
///
/// The conjunction is load-bearing and neither half suffices:
///
/// - `access(X_OK)` alone accepts a **directory** — the execute bit means
///   *traverse* there, so `access("/tmp", X_OK) == 0`. That is the bug
///   inherited from `portable-pty` (`cmdbuilder.rs:545-553`): with
///   `SHELL=/tmp` the child aborts with a Rust runtime panic while every
///   layer reports success.
/// - Raw `mode & 0o111` alone accepts a file the caller **cannot** execute.
///   The bits say *some* class has execute permission, not the applicable
///   one, and they do not evaluate ACLs. Verified with a self-owned regular
///   file at mode `0o010`: `mode & 0o111` is true, `access(X_OK)` is -1, and
///   running it gives `Permission denied`.
///
/// So: regular-file metadata (following symlinks, because `/bin/sh -> dash`
/// is legitimate) **and** effective executability via `access(X_OK)`.
#[cfg(unix)]
pub fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file() && can_execute(path)
}

/// `access(path, X_OK)`: does the *effective* user have execute permission,
/// accounting for the applicable permission class and ACLs?
#[cfg(unix)]
fn can_execute(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false; // interior NUL: not a path we can ask about
    };
    // SAFETY: `c_path` is a valid NUL-terminated C string for the duration of
    // the call, and `access` only reads it.
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
}

/// Resolves the shell to spawn: `$SHELL`, then the passwd entry, then
/// [`FALLBACK_SHELL`]. Each candidate must pass [`is_executable_file`].
///
/// `shell_env` is the caller's view of `$SHELL` so the resolution order is
/// testable without mutating process-global state; production passes
/// `std::env::var_os("SHELL")`.
#[cfg(unix)]
pub fn resolve_shell(shell_env: Option<&str>) -> String {
    // One validation path for every candidate, deliberately. Validating each
    // branch separately leaves the passwd branch's check untestable on any
    // machine whose passwd shell happens to be valid — a mutant that deletes
    // it survives because nothing can distinguish it. Sharing `validated`
    // means the `$SHELL` arm's coverage is the passwd arm's coverage.
    let candidates = [shell_env.map(str::to_owned), passwd_shell()];
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| validated(candidate))
        .unwrap_or_else(|| FALLBACK_SHELL.to_owned())
}

/// The single validity test every shell candidate must pass.
#[cfg(unix)]
fn validated(candidate: &str) -> bool {
    is_executable_file(Path::new(candidate))
}

/// The current user's login shell from the passwd database, unvalidated:
/// `resolve_shell` applies the shared [`validated`] check to it.
///
/// This is the step that matters for a Finder- or launchd-started app, which
/// can have no `$SHELL` at all: without it we would hand a zsh user `/bin/sh`
/// and call it their shell of choice.
#[cfg(unix)]
pub(crate) fn passwd_shell() -> Option<String> {
    // SAFETY: `getpwuid` returns a pointer to a static passwd struct owned by
    // libc, valid until the next passwd-database call. We copy the string out
    // before returning and make no other libc calls in between.
    let shell = unsafe {
        let ent = libc::getpwuid(libc::getuid());
        if ent.is_null() {
            return None;
        }
        let pw_shell = (*ent).pw_shell;
        if pw_shell.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(pw_shell).to_str().ok()?.to_owned()
    };

    Some(shell)
}

/// The login-shell `argv[0]` convention: the shell's basename prefixed with
/// `-`. This is what tells any shell — zsh, bash, fish, tcsh, nu — to run as
/// a login shell, without sniffing its name or guessing its flag grammar.
///
/// `portable-pty` applies this itself for a default program
/// (`cmdbuilder.rs:510-517`); we compute it here so the contract is asserted
/// against a value we own rather than against the dependency's behaviour.
pub fn login_argv0(shell: &str) -> String {
    let basename = shell.rsplit('/').next().unwrap_or(shell);
    format!("-{basename}")
}

/// Resolve the command shell on Windows from `ComSpec`, falling back to cmd.
#[cfg(windows)]
pub fn resolve_shell(shell_env: Option<&str>) -> String {
    shell_env.unwrap_or("cmd.exe").to_owned()
}
