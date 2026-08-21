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
//! So on Unix `PATH` is *constructed*, not filtered. The child gets the
//! platform's standard user path, which is what a login shell would have
//! produced had Buzz never been in the picture.
//!
//! **Windows constructs nothing, and the difference is not a shortcut.** The
//! Unix design leans on a mechanism Windows does not have: the login shell
//! reads rc files that *rebuild* `PATH`, so a minimal starting point is a
//! starting point rather than a ceiling. `cmd.exe` has no rc file. A Windows
//! user's `PATH` lives in the registry (`HKCU\Environment` +
//! `HKLM\...\Session Manager\Environment`) and is composed into the process
//! environment at launch; nothing inside the session ever puts back what we
//! leave out. Handing the child `System32;Windows;Wbem` therefore does not
//! give it a clean path — it deletes `ssh`, `git`, `node`, `python`, and every
//! other tool the user installed, permanently, for that session. Field-
//! verified on 0.5.9-fork.3: `ssh` reported "not recognized as an internal or
//! external command" in the Buzz terminal on a machine where it resolves
//! everywhere else.
//!
//! Nor does the reason for constructing apply there: Hermit activation is a
//! POSIX shell script (`. ./bin/activate-hermit`) and is never in effect in a
//! Windows process, so there is no build toolchain to fence out of the
//! inherited value.

#[cfg(any(windows, test))]
use crate::shell::validated_windows_system_root;

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

/// The user `PATH` for a spawned shell on Windows: the inherited value, plus
/// the system directories it must contain.
///
/// See the module header for why this inherits where Unix constructs.
#[cfg(windows)]
pub fn user_shell_path() -> String {
    windows_shell_path(
        std::env::var("PATH").ok().as_deref(),
        std::env::var("SystemRoot").ok().as_deref(),
    )
}

/// System directories the child must be able to reach whatever the inherited
/// `PATH` says. They are appended, not prepended: an entry the user put ahead
/// of `System32` is a deliberate override, and reordering it would make Buzz's
/// terminal disagree with every other terminal on the machine.
///
/// `Wbem` carries `wmic` and is on the stock path; `WindowsPowerShell\v1.0` is
/// how `powershell` resolves at all. `System32\OpenSSH` is deliberately *not*
/// listed: it is present only when the OpenSSH client feature is installed,
/// and when it is installed the installer puts it on the machine `PATH`, so
/// inheritance already covers it. Guessing at optional components is how a
/// "safe default" becomes a list of dead entries.
#[cfg(any(windows, test))]
fn windows_system_directories(system_root: &str) -> [String; 4] {
    [
        format!(r"{system_root}\System32"),
        system_root.to_string(),
        format!(r"{system_root}\System32\Wbem"),
        format!(r"{system_root}\System32\WindowsPowerShell\v1.0"),
    ]
}

/// The pure half of the Windows [`user_shell_path`], compiled on every
/// platform so the Unix CI this repo actually runs can exercise it.
///
/// Order is preserved and entries are de-duplicated case-insensitively —
/// Windows path comparison is case-insensitive, and a `PATH` that names the
/// same directory twice is how a "we appended the system dirs" fix announces
/// itself in every `echo %PATH%` from then on.
#[cfg(any(windows, test))]
pub fn windows_shell_path(inherited: Option<&str>, system_root: Option<&str>) -> String {
    // Use the shell resolver's validation so a relative process-environment
    // value cannot add current-directory-relative entries to the child PATH.
    let root = validated_windows_system_root(system_root);
    let mut entries: Vec<String> = Vec::new();
    for entry in inherited
        .unwrap_or_default()
        .split(';')
        .map(str::to_owned)
        .chain(windows_system_directories(root))
    {
        push_unique(&mut entries, entry);
    }
    entries.join(";")
}

/// Appends `entry` unless it is empty or already present.
///
/// Comparison ignores case and a trailing separator, because `C:\Windows\`
/// and `c:\windows` are the same directory to Windows and differ only in how
/// they were typed.
#[cfg(any(windows, test))]
fn push_unique(entries: &mut Vec<String>, entry: String) {
    let key = |value: &str| {
        value
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
            .to_owned()
    };
    if entry.trim().is_empty() {
        return;
    }
    if entries.iter().any(|existing| key(existing) == key(&entry)) {
        return;
    }
    entries.push(entry);
}
