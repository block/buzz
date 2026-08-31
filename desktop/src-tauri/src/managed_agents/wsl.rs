//! WSL fallback discovery for ACP harness commands on Windows.
//!
//! Many agent CLIs (Hermes Agent, Oh My Pi, …) are commonly installed inside
//! a Windows Subsystem for Linux distribution rather than on the Windows host.
//! When a preset or custom harness command cannot be resolved on the Windows
//! PATH, discovery falls back to probing the *default* WSL distribution via
//! `wsl.exe -e <path>`. A positive result is cached so the spawn path
//! (`runtime.rs`) can hand the in-distro path to `buzz-acp` through
//! `BUZZ_ACP_AGENT_WSL_PATH`; `buzz-acp` then wraps the agent spawn through
//! `wsl.exe` and forwards injected environment via `WSLENV`.
//!
//! Design notes:
//! * Only bare command names are probed (`hermes-acp`, `omp`, …). Absolute or
//!   relative paths are never sent through the probe — a Windows path has no
//!   meaning inside the distro and would be a quoting/injection hazard.
//! * Probing always targets the *default* distribution (`wsl.exe -e` without
//!   `-d`). Multi-distro selection is future work; the `distro` field exists
//!   so the spawn contract does not need to change when it lands.
//! * All pure helpers (script building, stdout parsing, display paths) are
//!   platform-agnostic so the unit tests run on any host, mirroring the
//!   `git_bash.rs` pattern. Process-spawning probes are `cfg(windows)`-gated
//!   and return `None` everywhere else.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Hard wall-clock deadline for one WSL probe, on par with the other bounded
/// discovery spawns. WSL2 cold-boot (VM start + distro init) can exceed 10s,
/// so this is scaled up from the 10s used for CLI auth probes; a genuinely
/// wedged boot is still reaped rather than stalling discovery forever.
#[cfg(windows)]
const WSL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A harness command located inside the default WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WslCommandResolution {
    /// Distro name. Always `None` today (probing targets the default distro);
    /// carried so the spawn contract can grow multi-distro support without
    /// changing the `BUZZ_ACP_AGENT_WSL_*` env surface.
    pub distro: Option<String>,
    /// Absolute path of the command inside the distro
    /// (e.g. `/home/user/.local/bin/hermes-acp`).
    pub linux_path: String,
}

/// Return true when `command` is a bare executable name safe to embed in the
/// probe script: `[A-Za-z0-9._-]+` with no path separators. Paths (Windows or
/// POSIX shaped) are rejected — probing for them inside WSL is meaningless.
#[cfg(any(windows, test))]
fn is_probeable_command(command: &str) -> bool {
    !command.is_empty()
        && command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Build the POSIX sh probe executed inside the default distribution.
///
/// Well-known per-user install locations are checked first (`~/.local/bin`
/// is where the Hermes installer and `pip install --user` land, and it is
/// NOT on the non-login PATH WSL gives `wsl.exe -e sh`), then the distro's
/// own PATH via `command -v`. Prints the first match on stdout; prints
/// nothing when the command is absent.
///
/// Extracted as a pure function so tests can pin the probe contract.
#[cfg(any(windows, test))]
fn wsl_probe_script(command: &str) -> String {
    debug_assert!(is_probeable_command(command));
    format!(
        "for p in \"$HOME/.local/bin/{command}\" \"$HOME/bin/{command}\" \"/usr/local/bin/{command}\"; do \
            [ -x \"$p\" ] && {{ printf '%s\\n' \"$p\"; exit 0; }}; \
        done; \
        command -v {command} 2>/dev/null || true"
    )
}

/// Parse probe stdout into an in-distro absolute path.
///
/// Takes the first non-empty line that is an absolute POSIX path. Anything
/// else (blank output, relative `command -v` oddities, WSL interop noise)
/// means "not found".
#[cfg(any(windows, test))]
fn parse_probe_stdout(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('/'))
        .map(str::to_string)
}

/// Display path surfaced in the catalog (`binary_path`) for a WSL-resolved
/// command. Purely informational — the real resolution travels through the
/// cache into the spawn path.
pub(crate) fn wsl_display_path(resolution: &WslCommandResolution) -> PathBuf {
    match &resolution.distro {
        Some(distro) => PathBuf::from(format!("wsl://{distro}{}", resolution.linux_path)),
        None => PathBuf::from(format!("wsl://{}", resolution.linux_path)),
    }
}

fn wsl_cache() -> &'static Mutex<std::collections::HashMap<String, Option<WslCommandResolution>>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Option<WslCommandResolution>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Clear the WSL resolution cache so a command installed inside the default
/// distribution after the first miss is found on the next forced discovery.
/// Mirrors `resolve_command`'s `clear_resolve_cache`; both run on the
/// forced-discovery invalidation seam (see `commands/agent_discovery`).
pub(crate) fn clear_wsl_cache() {
    if let Ok(mut guard) = wsl_cache().lock() {
        guard.clear()
    }
}

/// Probe the default WSL distribution for `command`, caching the outcome
/// (positive or negative) for the app lifetime — mirroring `resolve_command`'s
/// cache contract so repeated discovery runs stay cheap.
///
/// Returns `None` on non-Windows hosts, when `wsl.exe` is unavailable, when
/// `command` is not a bare executable name, and when the probe finds nothing.
pub(crate) fn probe_wsl_command(command: &str) -> Option<WslCommandResolution> {
    if let Ok(guard) = wsl_cache().lock() {
        if let Some(cached) = guard.get(command) {
            return cached.clone();
        }
    }

    let result = probe_wsl_command_uncached(command);

    if let Ok(mut guard) = wsl_cache().lock() {
        guard.insert(command.to_string(), result.clone());
    }
    result
}

#[cfg(windows)]
fn probe_wsl_command_uncached(command: &str) -> Option<WslCommandResolution> {
    if !is_probeable_command(command) {
        return None;
    }
    let wsl = resolve_wsl_exe()?;
    let mut cmd = std::process::Command::new(wsl);
    cmd.args(["-e", "sh", "-c", &wsl_probe_script(command)]);
    crate::util::configure_no_window(&mut cmd);
    // Bound the probe on the same wall-clock deadline as other discovery
    // spawns: a hung distro boot (WSL2 cold start) or a wedged login shell must
    // not stall discovery forever. `output_with_timeout` reaps the child tree
    // on timeout and fails closed to `None`.
    let output = super::discovery::output_with_timeout(cmd, WSL_PROBE_TIMEOUT)?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let linux_path = parse_probe_stdout(&stdout)?;
    Some(WslCommandResolution {
        distro: None,
        linux_path,
    })
}

#[cfg(not(windows))]
fn probe_wsl_command_uncached(_command: &str) -> Option<WslCommandResolution> {
    None
}

/// Resolve `wsl.exe` itself: the System32 copy first, then a PATH scan that
/// skips the WindowsApps app-execution-alias stubs (see issue #2328 — those
/// stubs are launchers, not the real binary).
#[cfg(windows)]
fn resolve_wsl_exe() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("SystemRoot") {
        let candidate = PathBuf::from(root).join("System32").join("wsl.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("wsl.exe"))
            .find(|candidate| {
                candidate.is_file() && !super::git_bash::is_windows_apps_alias(candidate)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probeable_accepts_bare_command_names() {
        for command in [
            "hermes-acp",
            "omp",
            "grok",
            "claude_agent-acp",
            "amp-acp.cmd",
        ] {
            assert!(
                is_probeable_command(command),
                "expected probeable: {command}"
            );
        }
    }

    #[test]
    fn probeable_rejects_paths_and_shell_metacharacters() {
        for command in [
            "",
            "/usr/local/bin/hermes-acp",
            r"C:\tools\hermes-acp.exe",
            "./hermes-acp",
            "../escape",
            "hermes-acp; rm -rf /",
            "hermes-acp$(whoami)",
            "hermes acp",
            "hermes-acp\nevil",
        ] {
            assert!(
                !is_probeable_command(command),
                "expected rejected: {command:?}"
            );
        }
    }

    #[test]
    fn probe_script_checks_user_bins_before_path() {
        let script = wsl_probe_script("hermes-acp");
        // ~/.local/bin must be probed explicitly: it is not on the non-login
        // PATH that `wsl.exe -e sh` receives, and it is where the Hermes
        // installer drops hermes-acp.
        let local_bin = script
            .find("$HOME/.local/bin/hermes-acp")
            .expect("~/.local/bin probe");
        let command_v = script
            .find("command -v hermes-acp")
            .expect("command -v fallback");
        assert!(
            local_bin < command_v,
            "well-known bins must precede PATH probe"
        );
        // No format!/interpolation artifacts, no login shell (login shells can
        // print profile noise that would corrupt parse_probe_stdout).
        assert!(
            !script.contains("sh -l"),
            "probe must not use a login shell"
        );
    }

    #[test]
    fn parse_stdout_takes_first_absolute_path_line() {
        assert_eq!(
            parse_probe_stdout("/home/rat/.local/bin/hermes-acp\n"),
            Some("/home/rat/.local/bin/hermes-acp".to_string())
        );
        // Noise lines are skipped in favour of the first absolute path.
        assert_eq!(
            parse_probe_stdout("some warning\r\n/usr/local/bin/omp\n"),
            Some("/usr/local/bin/omp".to_string())
        );
        // Blank / relative-only output means not found.
        assert_eq!(parse_probe_stdout(""), None);
        assert_eq!(parse_probe_stdout("hermes-acp\n"), None);
    }

    #[test]
    fn display_path_carries_wsl_scheme() {
        let resolution = WslCommandResolution {
            distro: None,
            linux_path: "/home/rat/.local/bin/hermes-acp".to_string(),
        };
        assert_eq!(
            wsl_display_path(&resolution).display().to_string(),
            "wsl:///home/rat/.local/bin/hermes-acp"
        );
        let with_distro = WslCommandResolution {
            distro: Some("Ubuntu".to_string()),
            ..resolution
        };
        assert_eq!(
            wsl_display_path(&with_distro).display().to_string(),
            "wsl://Ubuntu/home/rat/.local/bin/hermes-acp"
        );
    }

    #[test]
    fn probe_caches_negative_results() {
        // A non-probeable command short-circuits to None and must be cached —
        // a second call must not re-run the (here: non-Windows, no-op) probe.
        assert_eq!(probe_wsl_command("definitely/not a command"), None);
        assert!(wsl_cache()
            .lock()
            .expect("cache lock")
            .contains_key("definitely/not a command"));
    }
}
