//! Login-shell PATH discovery and nvm fallback.
//!
//! Extracted verbatim from `discovery.rs` to keep that file under the
//! file-size ratchet. Covers login-shell candidate selection, the cached
//! login-shell PATH probe, and nvm default-bin resolution.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::is_executable_file;

/// Test-only spawn counter lives beside `discovery.rs`; import it here so the
/// spawn-record call site stays byte-identical to the pre-extraction source.
#[cfg(test)]
use super::login_shell_spawn_probe;

/// Collect login shell candidates for the current platform.
///
/// On Unix: `/bin/zsh`, `/bin/bash` (the historical defaults).
/// On Windows: Git Bash via `resolve_bash_path` — skips `BUZZ_SHELL` because
/// login-shell callers use bash-only `-l -c` syntax.
pub(crate) fn login_shell_candidates() -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/bin/zsh"), PathBuf::from("/bin/bash")]
    }
    #[cfg(windows)]
    {
        super::super::git_bash::resolve_bash_path()
            .into_iter()
            .collect()
    }
}

/// Run a command in a login shell (tries zsh then bash on Unix, Git Bash on Windows).
/// Returns trimmed stdout if the command succeeds with non-empty output.
fn run_in_login_shell(args: &[&str]) -> Option<String> {
    #[cfg(test)]
    login_shell_spawn_probe::record();
    for shell in login_shell_candidates() {
        let mut cmd = Command::new(&shell);
        cmd.args(args);
        crate::util::configure_no_window(&mut cmd);
        let Ok(output) = cmd.output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            return Some(stdout);
        }
    }
    None
}

pub(crate) fn find_via_login_shell(command: &str) -> Option<PathBuf> {
    let stdout = run_in_login_shell(&["-l", "-c", r#"command -v -- "$1""#, "_", command])?;
    let resolved = stdout.lines().rfind(|line| !line.trim().is_empty())?;
    let path = PathBuf::from(resolved.trim());
    (path.is_absolute() && is_executable_file(&path)).then_some(path)
}

/// Three-state backing store for the login-shell PATH cache.
#[derive(Clone)]
enum LoginShellPath {
    /// Cache has never been populated; the next call will spawn a login shell.
    Uninit,
    /// A login shell was invoked; the inner value is the PATH it returned
    /// (`None` when the shell produced no output).
    Probed(Option<String>),
}

fn path_cache() -> &'static std::sync::Mutex<LoginShellPath> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<LoginShellPath>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LoginShellPath::Uninit))
}

fn fetch_login_shell_path_inner() -> Option<String> {
    // On Windows, Git Bash's `echo $PATH` returns POSIX colon-delimited paths
    // (`/mingw64/bin:/c/Users/...`) which poison native Windows children that
    // split on `;`. login_shell_path() feeds agent_models, runtime, and
    // cli_probe — all native processes. Return None so they inherit the real
    // Windows PATH instead.
    #[cfg(windows)]
    {
        return None;
    }

    #[cfg(not(windows))]
    {
        let stdout = run_in_login_shell(&["-l", "-c", "echo $PATH"])?;
        let last_line = stdout.lines().rfind(|l| !l.trim().is_empty())?;
        Some(last_line.trim().to_string())
    }
}

/// Return the user's full PATH from a login shell.
///
/// The result is cached after the first call. Call [`refresh_login_shell_path`]
/// to invalidate the cache so the next call re-fetches — e.g. after the user
/// installs Node.js mid-session and clicks Retry.
///
/// The lock is never held while the login shell spawns: we check for a cached
/// value, release the lock, run the shell, then re-lock to write. Two concurrent
/// callers may both run the shell (last-writer-wins is fine — both produce the
/// same result), but neither blocks a concurrent agent spawn on the Mutex.
pub fn login_shell_path() -> Option<String> {
    // Fast path: return cached result without spawning a shell.
    {
        let guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let LoginShellPath::Probed(ref result) = *guard {
            return result.clone();
        }
    }

    // Slow path: spawn shell outside any lock.
    let result = fetch_login_shell_path_inner();

    // Write back; last-writer-wins is safe here.
    {
        let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
        *guard = LoginShellPath::Probed(result.clone());
    }

    result
}

/// Invalidate the login-shell PATH cache so the next [`login_shell_path`] call
/// re-fetches from a fresh login shell.
///
/// Called before every install/retry operation and on Doctor Re-run so a
/// newly-installed tool becomes visible without restarting the app.
pub(crate) fn refresh_login_shell_path() {
    let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = LoginShellPath::Uninit;
}

#[cfg(test)]
pub(crate) fn is_login_shell_path_uninit() -> bool {
    matches!(
        *path_cache().lock().unwrap_or_else(|e| e.into_inner()),
        LoginShellPath::Uninit
    )
}

/// Return `true` when `tag` is a safe nvm alias/version tag that can be joined
/// onto a `PathBuf` without escaping the nvm root.
///
/// nvm uses tags like `v22.1.0` or `lts/hydrogen`. We allow ASCII alphanumeric
/// plus `. - / _` and require that no path component is `..` and that the tag
/// does not start with `/` (which would replace the base in `PathBuf::join`).
pub(crate) fn is_safe_nvm_tag(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    // An absolute path in the alias file would let PathBuf::join silently
    // replace the nvm root with an attacker-controlled path.
    if tag.starts_with('/') {
        return false;
    }
    // Reject any .. component to prevent upward traversal.
    for component in tag.split('/') {
        if component == ".." {
            return false;
        }
    }
    // Allow only the characters nvm uses in real tag names.
    tag.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/' | '_'))
}

/// Locate the `bin` directory for nvm's default Node.js version.
///
/// Reads `~/.nvm/alias/default`; resolves at most one alias hop to handle
/// nvm alias chains; falls back to the highest-semver directory under
/// `~/.nvm/versions/node/`. Returns the `bin` subdirectory only when it exists.
///
/// Cheap: at most two file reads or one `read_dir`. Never cached — computed
/// fresh per call so a mid-session `nvm install` is visible at the next spawn.
pub fn find_nvm_default_bin(home: &Path) -> Option<PathBuf> {
    let nvm_root = home.join(".nvm");
    let versions_root = nvm_root.join("versions").join("node");

    // 1. Try alias/default, with at most one hop.
    let default_alias = nvm_root.join("alias").join("default");
    if let Ok(content) = std::fs::read_to_string(&default_alias) {
        let tag = content.trim().to_string();
        if is_safe_nvm_tag(&tag) {
            let candidate = versions_root.join(&tag).join("bin");
            if candidate.is_dir() {
                return Some(candidate);
            }
            // One alias hop: ~/.nvm/alias/<tag>
            let hop_file = nvm_root.join("alias").join(&tag);
            if let Ok(hop_content) = std::fs::read_to_string(&hop_file) {
                let hop_tag = hop_content.trim().to_string();
                if is_safe_nvm_tag(&hop_tag) {
                    let hop_candidate = versions_root.join(&hop_tag).join("bin");
                    if hop_candidate.is_dir() {
                        return Some(hop_candidate);
                    }
                }
            }
        }
    }

    // 2. Fall back to highest-semver directory under ~/.nvm/versions/node/.
    let entries = std::fs::read_dir(&versions_root).ok()?;
    let best = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().into_owned();
            parse_semver_tag(&s).map(|v| (v, s))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b));

    let (_, tag) = best?;
    let bin = versions_root.join(&tag).join("bin");
    bin.is_dir().then_some(bin)
}

/// Parse a `vMAJ.MIN.PATCH` (or `vMAJ.MIN.PATCH-extra`) tag into a numeric
/// triple for semver comparison.
pub(crate) fn parse_semver_tag(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v')?;
    let mut parts = s.splitn(3, '.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_str = parts.next()?;
    let patch = patch_str.split('-').next()?.parse::<u64>().ok()?;
    Some((major, minor, patch))
}
