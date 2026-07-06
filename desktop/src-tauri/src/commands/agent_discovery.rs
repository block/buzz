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

#[tauri::command]
pub fn discover_acp_providers() -> Vec<AcpRuntimeCatalogEntry> {
    crate::managed_agents::clear_resolve_cache();
    crate::managed_agents::discover_acp_runtimes()
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

    // Phase 2: Install adapter if missing and commands are available.
    let adapter_found = runtime
        .commands
        .iter()
        .any(|cmd| crate::managed_agents::resolve_command(cmd).is_some());
    if !adapter_found {
        for cmd in runtime.adapter_install_commands {
            let result = run_install_command("adapter", cmd);
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

fn run_install_command(step: &str, command: &str) -> InstallStepResult {
    let shell_path = crate::managed_agents::login_shell_path();
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

    if let Some(ref path) = shell_path {
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

#[tauri::command]
pub fn discover_managed_agent_prereqs(
    input: DiscoverManagedAgentPrereqsRequest,
) -> ManagedAgentPrereqsInfo {
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

    // On Windows, check whether a bash-compatible shell is available. The probe
    // uses the same algorithm as the runtime resolver in buzz-dev-mcp/src/shell.rs:
    // BUZZ_SHELL > GIT_BASH > Program Files\Git > LocalAppData\Programs\Git > PATH
    // (excluding %SystemRoot% to avoid WSL's bash.exe).
    #[cfg(windows)]
    let bash = Some(detect_windows_bash());
    #[cfg(not(windows))]
    let bash = None;

    ManagedAgentPrereqsInfo {
        acp: command_availability(acp_command),
        mcp: command_availability(mcp_command),
        bash,
    }
}

/// Probe for a bash-compatible shell on Windows, mirroring the resolver in
/// `buzz-dev-mcp/src/shell.rs`. Returns a `CommandAvailabilityInfo` so the
/// frontend can surface a prereq hint in the same shape as acp/mcp.
///
/// Probe order (first hit wins):
///   1. `BUZZ_SHELL` env override — absolute path or bare command name.
///      Bare names are resolved through PATH WITHOUT System32 exclusion
///      (same policy as the runtime resolver: explicit overrides can choose
///      cmd/pwsh which legitimately live in System32).
///   2. `GIT_BASH` legacy override
///   3. Installed Git for Windows (`Program Files\Git` / `LocalAppData\Programs\Git`)
///   4. PATH scan for `bash.exe`, excluding `%SystemRoot%` (WSL guard)
///
/// This must stay behavior-identical to the Windows arm of `resolve_bash`
/// in `crates/buzz-dev-mcp/src/shell.rs`. Keep them in sync.
#[cfg(windows)]
fn detect_windows_bash() -> crate::managed_agents::CommandAvailabilityInfo {
    fn found(path: std::path::PathBuf) -> crate::managed_agents::CommandAvailabilityInfo {
        crate::managed_agents::CommandAvailabilityInfo {
            command: "bash".to_string(),
            available: true,
            resolved_path: Some(path.display().to_string()),
        }
    }

    let path_env = std::env::var("PATH").unwrap_or_default();

    // 1. BUZZ_SHELL override — absolute path or bare command name.
    if let Some(raw) = std::env::var_os("BUZZ_SHELL") {
        let p = std::path::PathBuf::from(&raw);
        if p.components().count() > 1 || p.has_root() {
            // Absolute path: must exist as a file.
            if p.is_file() {
                return found(p);
            }
        } else {
            // Bare command name: scan PATH without System32 exclusion —
            // the operator explicitly chose this shell.
            if let Some(resolved) = scan_path_for_command_ui(&p, &path_env, None) {
                return found(resolved);
            }
        }
    }

    // 2. GIT_BASH legacy override.
    if let Some(p) = std::env::var_os("GIT_BASH").map(std::path::PathBuf::from) {
        if p.is_file() {
            return found(p);
        }
    }

    // 3. Installed Git for Windows.
    for root in ["ProgramFiles", "LocalAppData"] {
        if let Some(base) = std::env::var_os(root) {
            let candidate = match root {
                "LocalAppData" => std::path::PathBuf::from(&base).join("Programs").join("Git"),
                _ => std::path::PathBuf::from(&base).join("Git"),
            }
            .join("bin")
            .join("bash.exe");
            if candidate.is_file() {
                return found(candidate);
            }
        }
    }

    // 4. PATH scan for bash.exe, excluding %SystemRoot% to avoid WSL's bash.exe.
    let system_root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from);
    if let Some(p) = scan_path_for_bash_ui(&path_env, system_root.as_deref()) {
        return found(p);
    }

    // Not found.
    crate::managed_agents::CommandAvailabilityInfo {
        command: "bash".to_string(),
        available: false,
        resolved_path: None,
    }
}

/// Scan `path_env` for `bash.exe`, skipping directories under `system_root`.
/// Duplicated from `buzz-dev-mcp/src/shell.rs::scan_path_for_bash` — must be
/// kept in sync. Delegates to `scan_path_for_command_ui`.
#[cfg(windows)]
fn scan_path_for_bash_ui(
    path_env: &str,
    system_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    scan_path_for_command_ui(std::path::Path::new("bash.exe"), path_env, system_root)
}

/// Scan `path_env` for `name` (or `name.exe` if no extension), optionally
/// skipping directories under `system_root`. Duplicated from
/// `buzz-dev-mcp/src/shell.rs::scan_path_for_command` — must be kept in sync.
/// Pass `system_root = None` for explicit BUZZ_SHELL overrides (no exclusion).
#[cfg(windows)]
fn scan_path_for_command_ui(
    name: &std::path::Path,
    path_env: &str,
    system_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let needs_exe = name.extension().is_none();
    for dir in std::env::split_paths(path_env) {
        if let Some(root) = system_root {
            if is_under_dir_ui(&dir, root) {
                continue;
            }
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if needs_exe {
            let mut with_exe = dir.join(name);
            with_exe.set_extension("exe");
            if with_exe.is_file() {
                return Some(with_exe);
            }
        }
    }
    None
}

/// Case-insensitive component-wise prefix check. Duplicated from
/// `buzz-dev-mcp/src/shell.rs::is_under_dir` — must be kept in sync.
#[cfg(windows)]
fn is_under_dir_ui(dir: &std::path::Path, root: &std::path::Path) -> bool {
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

#[cfg(all(test, windows))]
mod windows_bash_detect_tests {
    use super::*;
    use std::env;
    use tempfile::tempdir;

    fn touch(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, b"").expect("touch");
    }

    fn available(info: &crate::managed_agents::CommandAvailabilityInfo) -> bool {
        info.available
    }

    /// BUZZ_SHELL bare name (pwsh) resolves through PATH with no System32 exclusion.
    /// Tests the scan_path_for_command_ui helper used by detect_windows_bash().
    #[test]
    fn detect_buzz_shell_bare_name_resolves_from_path() {
        let dir = tempdir().expect("tempdir");
        let fake_pwsh = dir.path().join("pwsh.exe");
        touch(&fake_pwsh);

        let path_env = env::join_paths([dir.path().to_path_buf()]).expect("join");
        env::set_var("BUZZ_SHELL", "pwsh");

        let result = scan_path_for_command_ui(
            std::path::Path::new("pwsh"),
            path_env.to_str().expect("utf8"),
            None, // no System32 exclusion for explicit overrides
        );

        env::remove_var("BUZZ_SHELL");

        let resolved = result.expect("pwsh must be found via PATH");
        assert_eq!(resolved, fake_pwsh);
    }

    /// detect_windows_bash() end-to-end: BUZZ_SHELL=pwsh (bare name) resolves
    /// to a real executable on PATH and is reported as available.
    #[test]
    fn detect_windows_bash_buzz_shell_bare_name_end_to_end() {
        let dir = tempdir().expect("tempdir");
        let fake_pwsh = dir.path().join("pwsh.exe");
        touch(&fake_pwsh);

        let old_path = env::var_os("PATH");
        let old_buzz_shell = env::var_os("BUZZ_SHELL");

        // Put fake pwsh.exe at the front of PATH and set BUZZ_SHELL to the bare name.
        let new_path = env::join_paths(
            std::iter::once(dir.path().to_path_buf()).chain(
                old_path
                    .as_ref()
                    .map(|p| env::split_paths(p).collect::<Vec<_>>().into_iter())
                    .into_iter()
                    .flatten(),
            ),
        )
        .expect("join");
        env::set_var("PATH", &new_path);
        env::set_var("BUZZ_SHELL", "pwsh");

        let info = detect_windows_bash();

        // Restore env.
        match old_path {
            Some(p) => env::set_var("PATH", p),
            None => env::remove_var("PATH"),
        }
        match old_buzz_shell {
            Some(v) => env::set_var("BUZZ_SHELL", v),
            None => env::remove_var("BUZZ_SHELL"),
        }

        assert!(info.available, "detect_windows_bash must report available");
        assert_eq!(
            info.resolved_path.as_deref(),
            Some(fake_pwsh.display().to_string()).as_deref(),
            "resolved path must point to the fake pwsh.exe"
        );
    }

    /// detect_windows_bash() end-to-end: PATH-only bash.exe (no Git for Windows)
    /// is found and reported as available.
    #[test]
    fn detect_windows_bash_path_only_bash_end_to_end() {
        let dir = tempdir().expect("tempdir");
        let fake_bash = dir.path().join("bash.exe");
        touch(&fake_bash);

        // Use a separate dir as the "SystemRoot" so the scan-exclusion logic
        // doesn't interfere. detect_windows_bash() reads SystemRoot from env.
        let fake_sysroot = tempdir().expect("sysroot");

        let old_path = env::var_os("PATH");
        let old_system_root = env::var_os("SystemRoot");
        let old_buzz_shell = env::var_os("BUZZ_SHELL");

        let new_path = env::join_paths([dir.path().to_path_buf()]).expect("join");
        env::set_var("PATH", &new_path);
        env::set_var("SystemRoot", fake_sysroot.path());
        env::remove_var("BUZZ_SHELL");

        let info = detect_windows_bash();

        match old_path {
            Some(p) => env::set_var("PATH", p),
            None => env::remove_var("PATH"),
        }
        match old_system_root {
            Some(v) => env::set_var("SystemRoot", v),
            None => env::remove_var("SystemRoot"),
        }
        match old_buzz_shell {
            Some(v) => env::set_var("BUZZ_SHELL", v),
            None => env::remove_var("BUZZ_SHELL"),
        }

        assert!(info.available, "PATH-only bash.exe must be found");
        assert_eq!(
            info.resolved_path.as_deref(),
            Some(fake_bash.display().to_string()).as_deref(),
            "resolved path must point to the fake bash.exe"
        );
    }

    /// PATH-only bash.exe is found by the scan_path_for_bash_ui helper.
    #[test]
    fn detect_path_only_bash_found() {
        let real = tempdir().expect("real");
        let real_bash = real.path().join("bash.exe");
        touch(&real_bash);
        let sys_root = tempdir().expect("sysroot"); // empty

        let found = scan_path_for_bash_ui(
            env::join_paths([real.path().to_path_buf()])
                .expect("join")
                .to_str()
                .expect("utf8"),
            Some(sys_root.path()),
        )
        .expect("bash found");
        assert_eq!(found, real_bash);
    }

    /// System32 bash.exe is skipped by the implicit scan.
    #[test]
    fn detect_system32_bash_skipped() {
        let sys32 = tempdir().expect("sys32");
        touch(&sys32.path().join("bash.exe"));
        let parent = sys32.path().parent().unwrap().to_path_buf();

        let found = scan_path_for_bash_ui(
            env::join_paths([sys32.path().to_path_buf()])
                .expect("join")
                .to_str()
                .expect("utf8"),
            Some(&parent),
        );
        assert!(found.is_none(), "System32 bash must be skipped");
    }
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
