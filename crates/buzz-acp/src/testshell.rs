//! Interpreter resolution for tests that drive a fake ACP agent through a
//! POSIX shell script.
//!
//! Dozens of tests in this crate stand up a fake agent by spawning
//! `bash -c '<script>'` and speaking JSON-RPC to it over the child's pipes.
//! Spawning the *bare name* `bash` is the bug this module exists to prevent.
//!
//! `Command::new("bash")` resolves through the Win32 search order, which places
//! `%SystemRoot%\System32` ahead of `PATH`. On any Windows machine with the WSL
//! optional feature enabled — the default on a developer box —
//! `System32\bash.exe` is **WSL's launcher**, so the fake agent runs as a Linux
//! process instead of an MSYS one. Two consequences, both silent:
//!
//! 1. **`read` returns nothing.** Over the native Win32 anonymous pipe that
//!    `Stdio::piped()` creates, WSL bash's `read` builtin completes
//!    successfully with an empty value even though the bytes arrived (`cat`
//!    sees them). A fixture that echoes `"$REQ"` back into a JSON literal then
//!    emits malformed JSON, the response is never matched, and the test fails
//!    as [`AcpError::AgentExited`](crate::acp::AcpError::AgentExited) — a
//!    symptom that looks nothing like its cause. Git Bash reads the same pipe
//!    correctly.
//! 2. **Win32 paths stop resolving.** WSL has no drive-letter namespace, so a
//!    `C:\Users\...\Temp\x.json` capture path handed to the script is treated
//!    as a *relative* filename and created in the process cwd — the crate
//!    source directory — with `:` and `\` stored as the private-use code points
//!    U+F03A / U+F05C that DrvFs substitutes for characters Windows forbids.
//!
//! The probe order below mirrors the already-reviewed production resolver in
//! `buzz-dev-mcp` (`shell.rs::resolve_bash`), which exists for the same reason
//! on the agent shell-tool path and whose `is_under_dir` comparison this
//! borrows. It is reimplemented here rather than shared because that function
//! is private to a crate `buzz-acp` does not depend on, and making it `pub`
//! solely for tests would widen a production API for no production caller.
//!
//! On Unix this is simply `bash`; there is no ambiguity to resolve.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The POSIX shell test fixtures spawn, resolved once per test binary.
///
/// Prefer this over the string `"bash"` anywhere a test spawns a shell.
/// Panics with an actionable message if no suitable shell exists — a missing
/// prerequisite must be loud, since the alternative is a suite that appears to
/// pass while asserting nothing.
pub(crate) fn posix_shell() -> &'static Path {
    static SHELL: OnceLock<PathBuf> = OnceLock::new();
    SHELL.get_or_init(resolve_posix_shell)
}

/// [`posix_shell`] as a `String`, for the `&str` command parameter of
/// [`AcpClient::spawn`](crate::acp::AcpClient::spawn).
pub(crate) fn posix_shell_command() -> String {
    posix_shell().to_string_lossy().into_owned()
}

/// Render `path` for safe interpolation *inside single quotes* in a shell
/// script, i.e. the caller writes `> '{}'`.
///
/// A path must never be interpolated bare. Absolute Windows paths contain
/// backslashes, which bash consumes as escapes outside quotes: a redirect to
/// `C:\Users\me\AppData\Local\Temp\x.json` silently becomes a redirect to the
/// relative file `C:UsersmeAppDataLocalTempx.json`, created in the process cwd
/// (the crate source directory) instead of the intended target. Single quotes
/// preserve the backslashes, and MSYS accepts a Win32 path in a filesystem
/// call.
pub(crate) fn quote_for_shell(path: &Path) -> String {
    // Close the quote, emit an escaped literal quote, reopen — the standard
    // POSIX idiom, since a single-quoted string cannot contain a single quote.
    path.to_string_lossy().replace('\'', r"'\''")
}

#[cfg(not(windows))]
fn resolve_posix_shell() -> PathBuf {
    PathBuf::from("bash")
}

/// Windows probe order (first hit wins), mirroring `buzz-dev-mcp`:
///   1. `BUZZ_TEST_SHELL` — explicit override, for hosts where the heuristics
///      below cannot find a usable shell.
///   2. `bash.exe` on `PATH`, skipping `%SystemRoot%` and the app-execution
///      alias directory so WSL's launcher and the Store stub are never chosen.
///   3. `git.exe` on `PATH` → its sibling `..\bin\bash.exe`. Git for Windows's
///      recommended PATH option adds `Git\cmd`, not `Git\bin`, so this is the
///      usual route on a stock install.
///   4. The standard Git for Windows install locations.
#[cfg(windows)]
fn resolve_posix_shell() -> PathBuf {
    if let Some(raw) = std::env::var_os("BUZZ_TEST_SHELL") {
        let p = PathBuf::from(raw);
        if p.is_file() {
            return p;
        }
    }

    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);

    if let Some(found) = scan_path(&path_env, "bash.exe", system_root.as_deref()) {
        return found;
    }

    if let Some(git) = scan_path(&path_env, "git.exe", system_root.as_deref()) {
        // <install>\cmd\git.exe -> <install>\bin\bash.exe
        if let Some(install) = git.parent().and_then(Path::parent) {
            let candidate = install.join("bin").join("bash.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    for base in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(dir) = std::env::var_os(base) {
            let candidate = PathBuf::from(dir).join("Git").join("bin").join("bash.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    panic!(
        "buzz-acp tests need Git for Windows (Git Bash) to run their fake-agent \
         fixtures, but none was found. Checked BUZZ_TEST_SHELL, bash.exe and \
         git.exe on PATH (excluding %SystemRoot%), and the standard Git install \
         locations. Install it from https://git-scm.com/download/win, or set \
         BUZZ_TEST_SHELL to a Git Bash executable. Note that WSL's \
         System32\\bash.exe is deliberately NOT accepted: its `read` builtin \
         returns empty over a Windows pipe, which makes these fixtures fail in \
         ways that look like product bugs."
    );
}

/// Scan `PATH` for `name`, skipping `%SystemRoot%` and the Windows
/// app-execution-alias directory.
///
/// Skipping happens per entry so the scan continues past an alias rather than
/// giving up, and `split_paths` is used instead of a hand-split on `;` so this
/// sees exactly what a spawned child would.
#[cfg(windows)]
fn scan_path(
    path_env: &std::ffi::OsStr,
    name: &str,
    system_root: Option<&Path>,
) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_env) {
        if let Some(root) = system_root {
            if is_under_dir(&dir, root) {
                continue;
            }
        }
        if is_windows_apps_alias(&dir) {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// True if `dir` is `root` or lives under it, comparing components
/// case-insensitively.
///
/// `Path::starts_with` compares components case-sensitively on every platform,
/// so a `PATH` entry spelled `C:\WINDOWS\System32` would slip past a
/// `%SystemRoot%` = `C:\Windows` prefix test and let WSL's bash through.
/// Comparing components (rather than lowercased substrings) also avoids a false
/// hit on a sibling directory such as `C:\Windows2`.
#[cfg(windows)]
fn is_under_dir(dir: &Path, root: &Path) -> bool {
    let mut dir_components = dir.components();
    for root_component in root.components() {
        match dir_components.next() {
            Some(d)
                if d.as_os_str()
                    .eq_ignore_ascii_case(root_component.as_os_str()) => {}
            _ => return false,
        }
    }
    true
}

/// `%LOCALAPPDATA%\Microsoft\WindowsApps` holds zero-byte app-execution alias
/// stubs, including a `bash.exe` that launches the Store build of WSL.
#[cfg(windows)]
fn is_windows_apps_alias(dir: &Path) -> bool {
    let mut components = dir.components().rev();
    matches!(
        (components.next(), components.next()),
        (Some(last), Some(parent))
            if last.as_os_str().eq_ignore_ascii_case("WindowsApps")
                && parent.as_os_str().eq_ignore_ascii_case("Microsoft")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolved shell must actually exist and be runnable — a resolver that
    /// returns a plausible-looking path nothing can spawn would reintroduce the
    /// failure mode in a new disguise.
    #[test]
    fn resolved_shell_runs_a_script() {
        let out = std::process::Command::new(posix_shell())
            .args(["-c", "printf ok"])
            .output()
            .expect("resolved POSIX shell must be spawnable");
        assert!(out.status.success(), "shell exited non-zero: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ok");
    }

    /// The whole point: never WSL. If this regresses, the fixtures that echo a
    /// request back through `read` start failing as `AgentExited`.
    #[cfg(windows)]
    #[test]
    fn resolved_shell_is_not_wsl() {
        let out = std::process::Command::new(posix_shell())
            .args(["-c", "uname -s"])
            .output()
            .expect("resolved POSIX shell must be spawnable");
        let uname = String::from_utf8_lossy(&out.stdout);
        assert!(
            !uname.contains("Linux"),
            "resolved shell is WSL (uname -s = {}), whose `read` builtin returns \
             empty over a Windows pipe; expected an MSYS shell",
            uname.trim()
        );
    }

    /// `read` over the child's stdin pipe is the capability every echo-back
    /// fixture depends on, and the one WSL silently lacks. Assert it directly
    /// so a bad resolution is reported here rather than as a dozen confusing
    /// `AgentExited` failures elsewhere.
    #[test]
    fn resolved_shell_reads_stdin_pipe() {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = std::process::Command::new(posix_shell())
            .args(["-c", "read -r line; printf '%s' \"$line\""])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn resolved POSIX shell");
        child
            .stdin
            .take()
            .expect("child stdin")
            .write_all(b"round-trip\n")
            .expect("write to child stdin");
        let out = child.wait_with_output().expect("collect child output");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "round-trip",
            "the resolved shell's `read` builtin lost data over a pipe"
        );
    }

    #[cfg(windows)]
    #[test]
    fn system32_is_excluded_from_path_scan() {
        let system_root = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let system32 = system_root.join("System32");
        let path = std::env::join_paths([&system32]).expect("join_paths");
        assert!(
            scan_path(&path, "bash.exe", Some(&system_root)).is_none(),
            "System32 must never yield a shell — that is WSL's launcher"
        );
    }

    /// Case-insensitivity is the subtle half: `Path::starts_with` is
    /// case-sensitive, so a differently-cased PATH entry would slip through.
    #[cfg(windows)]
    #[test]
    fn is_under_dir_ignores_case_but_not_sibling_dirs() {
        assert!(is_under_dir(
            Path::new(r"C:\WINDOWS\System32"),
            Path::new(r"C:\Windows")
        ));
        assert!(!is_under_dir(
            Path::new(r"C:\Windows2\bin"),
            Path::new(r"C:\Windows")
        ));
    }

    /// The quoting contract, asserted on the shape that actually broke: a
    /// Windows absolute path must survive with every backslash intact.
    #[test]
    fn quote_for_shell_preserves_backslashes_and_escapes_quotes() {
        assert_eq!(
            quote_for_shell(Path::new(r"C:\Users\me\AppData\Local\Temp\x.json")),
            r"C:\Users\me\AppData\Local\Temp\x.json"
        );
        assert_eq!(
            quote_for_shell(Path::new("/tmp/it's here.json")),
            r"/tmp/it'\''s here.json"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_apps_alias_dir_is_recognized() {
        assert!(is_windows_apps_alias(Path::new(
            r"C:\Users\x\AppData\Local\Microsoft\WindowsApps"
        )));
        assert!(!is_windows_apps_alias(Path::new(
            r"C:\Program Files\Git\bin"
        )));
    }
}
