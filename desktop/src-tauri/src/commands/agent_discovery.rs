use std::io::Read;
use tauri::State;

use crate::{
    app_state::AppState,
    managed_agents::{
        command_availability, AcpRuntimeCatalogEntry, DiscoverManagedAgentPrereqsRequest,
        InstallRuntimeResult, InstallStepResult, ManagedAgentPrereqsInfo, RelayAgentInfo,
        DEFAULT_ACP_COMMAND,
    },
    nostr_convert,
    relay::query_relay,
};

fn active_installs() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Returns the adapter install commands that `install_acp_runtime_blocking` would
/// run for `runtime_id` given a resolved adapter binary at `adapter_path` (or
/// `None` if none was found).
///
/// Returns `None` when no install is needed (adapter is present and current).
/// Returns `Some(cmds)` when the adapter is missing or (for codex) outdated.
///
/// This is a pure planning function: it never spawns a process.  Tests use it to
/// assert the correct install command is selected without touching real npm.
pub(crate) fn plan_adapter_install<'c>(
    runtime_id: &str,
    adapter_path: Option<&std::path::Path>,
    adapter_install_commands: &'c [&'c str],
) -> Option<&'c [&'c str]> {
    let needs_install = match adapter_path {
        None => true,
        Some(path) if runtime_id == "codex" => {
            crate::managed_agents::codex_adapter_is_outdated(path)
        }
        Some(_) => false,
    };
    if needs_install {
        Some(adapter_install_commands)
    } else {
        None
    }
}

#[tauri::command]
pub async fn discover_acp_providers() -> Result<Vec<AcpRuntimeCatalogEntry>, String> {
    tokio::task::spawn_blocking(|| {
        crate::managed_agents::clear_resolve_cache();
        crate::managed_agents::discover_acp_runtimes()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn install_acp_runtime(runtime_id: String) -> Result<InstallRuntimeResult, String> {
    tokio::task::spawn_blocking(move || install_acp_runtime_blocking(&runtime_id))
        .await
        .map_err(|e| format!("install task panicked: {e}"))?
}

/// Err(_) = infrastructure failure (panic, concurrency guard).
/// Ok({success: false}) = an install step failed (stderr captured in steps).
fn install_acp_runtime_blocking(runtime_id: &str) -> Result<InstallRuntimeResult, String> {
    // Prevent concurrent installs for the same runtime.
    {
        let mut set = active_installs()
            .lock()
            .map_err(|_| "install lock poisoned".to_string())?;
        if !set.insert(runtime_id.to_string()) {
            return Err(format!(
                "an install is already in progress for {runtime_id}"
            ));
        }
    }

    struct Guard(String);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(mut set) = active_installs().lock() {
                set.remove(&self.0);
            }
        }
    }
    let _guard = Guard(runtime_id.to_string());

    let runtime = crate::managed_agents::known_acp_runtime_exact(runtime_id)
        .ok_or_else(|| format!("unknown runtime: {runtime_id}"))?;

    let mut steps = Vec::new();

    // Phase 1: Install CLI if missing and commands are available.
    // NOTE: the npm EACCES preflight and `npm_eacces_hint` classifier only run
    // in Phase 2 below. Today every entry in `cli_install_commands` is a
    // curl-pipe; all `npm install -g` commands live in `adapter_install_commands`.
    // If a future runtime adds an npm-global CLI install it must also add the
    // preflight and classifier to this loop.
    if let Some(cli) = runtime.underlying_cli {
        if crate::managed_agents::resolve_command(cli).is_none() {
            for cmd in runtime.cli_install_commands {
                let result = run_install_command("cli", cmd);
                let success = result.success;
                steps.push(result);
                if !success {
                    return Ok(InstallRuntimeResult {
                        success: false,
                        steps,
                    });
                }
            }
        }
    }

    // Phase 2: Install adapter if missing (or outdated) and commands are available.
    // For the codex runtime, "found" is not enough — the resolved binary must also
    // pass the 1.x version gate. An outdated 0.16.x adapter must be overwritten by
    // the new npm install so the CODEX_CONFIG spawn contract works correctly.
    let adapter_path = runtime
        .commands
        .iter()
        .find_map(|cmd| crate::managed_agents::resolve_command(cmd));
    if let Some(cmds) = plan_adapter_install(
        runtime_id,
        adapter_path.as_deref(),
        runtime.adapter_install_commands,
    ) {
        for cmd in cmds {
            if is_npm_global_install(cmd) {
                if let Some(step) = npm_preflight_check("adapter", cmd) {
                    steps.push(step);
                    return Ok(InstallRuntimeResult {
                        success: false,
                        steps,
                    });
                }
            }
            let mut result = run_install_command("adapter", cmd);
            if !result.success && result.hint.is_none() && is_npm_global_install(cmd) {
                result.hint = npm_eacces_hint(&result.stderr, cmd);
            }
            let success = result.success;
            steps.push(result);
            if !success {
                return Ok(InstallRuntimeResult {
                    success: false,
                    steps,
                });
            }
        }
    }

    // Clear the resolve cache so the next discovery picks up new binaries.
    crate::managed_agents::clear_resolve_cache();

    Ok(InstallRuntimeResult {
        success: true,
        steps,
    })
}

/// Build a login-shell `Command` for `command` with the hermit env vars
/// stripped and the user's PATH set. This is the single source of truth for
/// the shell selection and environment cleanup shared by `run_install_command`
/// and `resolve_npm_prefix` — keeping them in sync so the hermit-strip list
/// can't drift between the two paths.
fn install_shell_command(command: &str) -> std::process::Command {
    let shell = if std::path::Path::new("/bin/zsh").exists() {
        "/bin/zsh"
    } else {
        "/bin/bash"
    };

    let mut cmd = std::process::Command::new(shell);
    cmd.args(["-l", "-c", command]);

    // Strip hermit env vars so npm/node use the user's normal registry and
    // global prefix rather than the project-local hermit-managed paths.
    cmd.env_remove("NPM_CONFIG_PREFIX");
    cmd.env_remove("NPM_CONFIG_CACHE");
    cmd.env_remove("COREPACK_HOME");

    if let Some(ref path) = crate::managed_agents::login_shell_path() {
        cmd.env("PATH", path);
    }

    // Detach from the controlling terminal so install scripts that read from
    // /dev/tty (e.g. Codex's "Start Codex now? [y/N]") fall back to stdin
    // (which is /dev/null) instead of blocking forever.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    cmd
}

fn run_install_command(step: &str, command: &str) -> InstallStepResult {
    let mut cmd = install_shell_command(command);

    let mut child = match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("failed to spawn shell: {e}"),
                exit_code: None,
                hint: None,
            };
        }
    };

    // Drain stdout/stderr on background threads to prevent pipe buffer deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });

    // Save the PID before moving `child` into the wait thread so we can
    // kill the process on timeout.
    let child_pid = child.id();

    let (tx, rx) = std::sync::mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    // 5-minute timeout for install commands.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            // Timeout: kill the child process via its PID, then join all
            // threads so nothing leaks.
            #[cfg(unix)]
            unsafe {
                libc::kill(child_pid as i32, libc::SIGTERM);
            }
            drop(rx);
            let _ = wait_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: "install command timed out after 5 minutes".to_string(),
                exit_code: None,
                hint: None,
            };
        }

        match rx.recv_timeout(std::time::Duration::from_millis(200).min(remaining)) {
            Ok(Ok(status)) => {
                let _ = wait_thread.join();
                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr_raw = stderr_thread.join().unwrap_or_default();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: status.success(),
                    stdout: truncate_output(stdout),
                    stderr: truncate_output(stderr_raw),
                    exit_code: status.code(),
                    hint: None,
                };
            }
            Ok(Err(e)) => {
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: format!("failed to check process status: {e}"),
                    exit_code: None,
                    hint: None,
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Still running; loop and check deadline again.
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // wait_thread dropped sender without sending — shouldn't happen.
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: "internal error: wait thread disconnected".to_string(),
                    exit_code: None,
                    hint: None,
                };
            }
        }
    }
}

/// Cap output to head + tail to avoid flooding the UI with large error dumps,
/// while preserving the most useful parts of the output.
fn truncate_output(s: String) -> String {
    const HEAD: usize = 512;
    const TAIL: usize = 1024;
    const LIMIT: usize = HEAD + TAIL;
    if s.len() <= LIMIT {
        return s;
    }
    let head_end = floor_char_boundary(&s, HEAD);
    let tail_start = floor_char_boundary(&s, s.len().saturating_sub(TAIL));
    let omitted = tail_start - head_end;
    format!(
        "{}\n... ({omitted} bytes omitted) ...\n{}",
        &s[..head_end],
        &s[tail_start..]
    )
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    index = index.min(s.len());
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

// ── npm EACCES preflight ──────────────────────────────────────────────────────

/// Returns true when `command` is an npm global install invocation.
fn is_npm_global_install(command: &str) -> bool {
    let t = command.trim_start();
    t.starts_with("npm install -g ") || t.starts_with("npm i -g ")
}

/// Guidance text for the EACCES / unwritable-prefix case.
fn npm_eacces_guidance(command: &str) -> String {
    format!(
        "npm's global install directory isn't writable by your user.\n\
\n\
Fix (no sudo):\n\
  1. Run:  npm config set prefix ~/.npm-global\n\
  2. Add to ~/.zprofile:  export PATH=\"$HOME/.npm-global/bin:$PATH\"\n\
  3. Restart Buzz, then click Install again.\n\
\n\
Or install manually, then click Refresh:\n\
  sudo {command}"
    )
}

/// Guidance text shown when npm / Node.js is not found in the login-shell PATH.
const NPM_MISSING_HINT: &str = "Node.js / npm was not found. Install Node.js \
(https://nodejs.org or your version manager), restart Buzz, then click Install again.\n\
If npm works in your terminal, make sure your Node version manager is initialized in \
~/.zprofile (not only ~/.zshrc) — Buzz resolves tools via non-interactive login shells.";

/// Result of probing `npm prefix -g` in the hermit-stripped login shell.
#[cfg(unix)]
enum NpmPrefix {
    /// npm responded with a parseable prefix path.
    Found(std::path::PathBuf),
    /// npm was not found, the spawn failed, the command returned a non-zero
    /// exit, or the output could not be parsed.
    Unavailable,
    /// The probe exceeded the 30-second deadline (e.g. a version-manager init
    /// that blocks on `/dev/tty`). The install should proceed so the stderr
    /// classifier remains the backstop.
    TimedOut,
}

/// Spawn the same login shell used by `run_install_command` and run
/// `npm prefix -g` to discover where npm would install global packages.
#[cfg(unix)]
fn resolve_npm_prefix() -> NpmPrefix {
    let mut cmd = install_shell_command("npm prefix -g");
    let mut child = match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return NpmPrefix::Unavailable,
    };

    // Drain stdout/stderr on background threads to prevent pipe-buffer deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        // Drain stderr so the child doesn't block on a full pipe.
        if let Some(mut pipe) = stderr_pipe {
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        }
    });

    let child_pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    // 30-second timeout — plenty for `npm prefix -g`; intentionally shorter
    // than the 5-minute install budget in `run_install_command`.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let raw_bytes: Option<Vec<u8>> = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            // Timed out: send SIGTERM, clean up threads, signal the caller to
            // fall through to the install path rather than abort.
            unsafe { libc::kill(child_pid as i32, libc::SIGTERM) };
            drop(rx);
            let _ = wait_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            eprintln!(
                "buzz: npm prefix probe timed out after 30s; \
                 proceeding to install (stderr classifier is the backstop)"
            );
            return NpmPrefix::TimedOut;
        }
        match rx.recv_timeout(std::time::Duration::from_millis(200).min(remaining)) {
            Ok(Ok(status)) => {
                let _ = wait_thread.join();
                let stdout = stdout_thread.join().unwrap_or_default();
                let _ = stderr_thread.join();
                break if status.success() { Some(stdout) } else { None };
            }
            Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                break None;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        }
    };

    let bytes = match raw_bytes {
        Some(b) => b,
        None => return NpmPrefix::Unavailable,
    };
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    // Version managers can print banner lines before the real prefix — take the
    // last non-empty line to skip any preamble.
    let prefix = match raw.lines().rfind(|l| !l.trim().is_empty()) {
        Some(l) => l.trim().to_string(),
        None => return NpmPrefix::Unavailable,
    };
    if prefix.is_empty() {
        return NpmPrefix::Unavailable;
    }
    NpmPrefix::Found(std::path::PathBuf::from(prefix))
}

/// Check write access to a file-system path using the POSIX `access(2)` syscall.
#[cfg(unix)]
fn unix_is_writable(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let Ok(c_path) = std::ffi::CString::new(bytes) else {
        return false;
    };
    // SAFETY: `access` is a pure read-only syscall; we pass a valid NUL-terminated
    // path and a standard flag constant.  This mirrors the existing `setsid`/`kill`
    // usage in this file.
    unsafe { libc::access(c_path.as_ptr(), libc::W_OK) == 0 }
}

/// Returns true when the directory where npm would write global packages is
/// writable by the current process user.
///
/// On non-unix platforms always returns `true` — the EACCES preflight is a
/// no-op there; the stderr classifier still applies.
fn npm_install_target_is_writable(prefix: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        // Probe the most specific candidate that exists; fall back up the tree.
        for candidate in &[
            prefix.join("lib/node_modules"),
            prefix.join("lib"),
            prefix.to_path_buf(),
        ] {
            if candidate.exists() {
                return unix_is_writable(candidate);
            }
        }
        // Nothing exists — npm couldn't create it either.
        unix_is_writable(prefix)
    }
    #[cfg(not(unix))]
    {
        let _ = prefix;
        true
    }
}

/// Inspect `stderr` for known npm EACCES patterns and return actionable
/// guidance if matched, or `None` when the error is unrelated.
fn npm_eacces_hint(stderr: &str, command: &str) -> Option<String> {
    if stderr.contains("EACCES: permission denied") || stderr.contains("npm error EACCES") {
        Some(npm_eacces_guidance(command))
    } else {
        None
    }
}

/// Run the npm preflight before executing an npm global install command.
/// Returns `Some(failed InstallStepResult)` to abort, or `None` to proceed.
fn npm_preflight_check(step: &str, command: &str) -> Option<InstallStepResult> {
    #[cfg(unix)]
    {
        match resolve_npm_prefix() {
            NpmPrefix::Unavailable => Some(InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                hint: Some(NPM_MISSING_HINT.to_string()),
            }),
            NpmPrefix::Found(prefix) if !npm_install_target_is_writable(&prefix) => {
                Some(InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: format!(
                        "npm global prefix '{}' is not writable by the current user.",
                        prefix.display()
                    ),
                    exit_code: None,
                    hint: Some(npm_eacces_guidance(command)),
                })
            }
            // `Found` + writable, or `TimedOut` — proceed; let the install run and
            // the stderr classifier serve as the backstop.
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (step, command);
        None
    }
}

// ── end npm preflight ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn discover_managed_agent_prereqs(
    input: DiscoverManagedAgentPrereqsRequest,
) -> Result<ManagedAgentPrereqsInfo, String> {
    tokio::task::spawn_blocking(move || {
        let acp_command = input
            .acp_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_ACP_COMMAND);
        let mcp_command = input
            .mcp_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");

        ManagedAgentPrereqsInfo {
            acp: command_availability(acp_command),
            mcp: command_availability(mcp_command),
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn list_relay_agents(state: State<'_, AppState>) -> Result<Vec<RelayAgentInfo>, String> {
    // Query kind:10100 agent profile events from the relay.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [10100],
        })],
    )
    .await?;

    // The convert helper returns `{"agents": [...]}`. Extract and re-deserialize
    // into the strongly-typed `Vec<RelayAgentInfo>` the frontend expects.
    let value = nostr_convert::agents_from_events(&events);
    let agents = value
        .get("agents")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    serde_json::from_value(agents).map_err(|e| format!("agent parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_npm_global_install ─────────────────────────────────────────────────

    #[test]
    fn test_is_npm_global_install_accepts_catalog_claude_command() {
        assert!(is_npm_global_install(
            "npm install -g @agentclientprotocol/claude-agent-acp"
        ));
    }

    #[test]
    fn test_is_npm_global_install_accepts_catalog_codex_command() {
        assert!(is_npm_global_install(
            "npm install -g @zed-industries/codex-acp"
        ));
    }

    #[test]
    fn test_is_npm_global_install_accepts_short_flag() {
        assert!(is_npm_global_install("npm i -g some-package"));
    }

    #[test]
    fn test_is_npm_global_install_accepts_leading_whitespace() {
        assert!(is_npm_global_install("  npm install -g foo"));
    }

    #[test]
    fn test_is_npm_global_install_rejects_curl_pipe() {
        assert!(!is_npm_global_install(
            "curl -fsSL https://example.com/install.sh | bash"
        ));
    }

    #[test]
    fn test_is_npm_global_install_rejects_non_global_install() {
        assert!(!is_npm_global_install("npm install foo"));
    }

    #[test]
    fn test_is_npm_global_install_rejects_unrelated_command() {
        assert!(!is_npm_global_install("cargo install some-tool"));
    }

    // ── npm_eacces_hint ───────────────────────────────────────────────────────

    #[test]
    fn test_npm_eacces_hint_detects_old_format() {
        let stderr = "npm ERR! code EACCES\nnpm ERR! syscall mkdir\nnpm ERR! path /usr/local/lib/node_modules\nnpm ERR! errno -13\nnpm ERR! Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
        assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
    }

    #[test]
    fn test_npm_eacces_hint_detects_new_format() {
        let stderr = "npm error EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
        assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
    }

    #[test]
    fn test_npm_eacces_hint_returns_none_for_404_stderr() {
        let stderr = "npm error 404 Not Found - GET https://registry.npmjs.org/no-such-pkg";
        assert!(npm_eacces_hint(stderr, "npm install -g no-such-pkg").is_none());
    }

    #[test]
    fn test_npm_eacces_hint_guidance_contains_npm_global_path() {
        let hint = npm_eacces_hint("EACCES: permission denied", "npm install -g foo").unwrap();
        assert!(hint.contains("~/.npm-global"), "hint: {hint}");
    }

    #[test]
    fn test_npm_eacces_hint_guidance_contains_zprofile() {
        let hint = npm_eacces_hint("EACCES: permission denied", "npm install -g foo").unwrap();
        assert!(hint.contains("~/.zprofile"), "hint: {hint}");
    }

    #[test]
    fn test_npm_eacces_hint_guidance_contains_sudo_command() {
        let hint = npm_eacces_hint("EACCES: permission denied", "npm install -g foo").unwrap();
        assert!(hint.contains("sudo npm install -g foo"), "hint: {hint}");
    }

    // ── npm_install_target_is_writable ────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_npm_install_target_is_writable_true_on_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(npm_install_target_is_writable(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn test_npm_install_target_is_writable_false_when_lib_node_modules_unwritable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        let node_modules = lib.join("node_modules");
        std::fs::create_dir_all(&node_modules).unwrap();
        // Make node_modules read-only.
        std::fs::set_permissions(&node_modules, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = npm_install_target_is_writable(dir.path());
        // Restore before the dir is dropped so cleanup can delete it.
        std::fs::set_permissions(&node_modules, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Skip this assertion when running as root (root can write to 0o555).
        if unsafe { libc::getuid() } != 0 {
            assert!(!result);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_npm_install_target_is_writable_walks_up_to_lib() {
        let dir = tempfile::tempdir().unwrap();
        // Create only `lib/` — no `lib/node_modules`.
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        assert!(npm_install_target_is_writable(dir.path()));
    }

    // ── adapter_needs_install (codex version gate) ────────────────────────────

    /// plan_adapter_install is the pure install-plan seam used by
    /// install_acp_runtime_blocking. These tests verify:
    ///   - A 0.x binary (AdapterOutdated) → install commands returned
    ///   - A 1.x binary (Available) → None (no reinstall)
    ///   - Missing binary (None path) → install commands returned
    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_selects_npm_command_for_outdated_0x_codex_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex-acp");
        // Simulate old 0.16.x: --version exits non-zero (unrecognised flag)
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", Some(&bin), install_cmds);

        assert!(
            plan.is_some(),
            "0.x codex adapter must trigger install plan"
        );
        let cmds = plan.unwrap();
        assert!(
            cmds.iter()
                .any(|c| c.contains("@agentclientprotocol/codex-acp")),
            "install plan must select @agentclientprotocol/codex-acp; got {cmds:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_returns_none_for_current_1x_codex_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex-acp");
        // Simulate 1.x adapter: outputs version and exits 0
        std::fs::write(
            &bin,
            "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\nexit 0\n",
        )
        .expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", Some(&bin), install_cmds);

        assert!(
            plan.is_none(),
            "1.x codex adapter must not trigger install plan (no reinstall needed)"
        );
    }

    #[test]
    fn test_plan_adapter_install_returns_cmds_when_no_adapter_path() {
        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", None, install_cmds);
        assert!(plan.is_some(), "missing adapter must trigger install plan");
    }

    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_non_codex_runtime_never_reinstalls() {
        use std::os::unix::fs::PermissionsExt;

        // For non-codex runtimes, any resolved binary means no install needed.
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("goose-acp");
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @block/goose-acp"];
        let plan = plan_adapter_install("goose", Some(&bin), install_cmds);
        assert!(
            plan.is_none(),
            "non-codex runtime with resolved binary must not trigger reinstall"
        );
    }
}
