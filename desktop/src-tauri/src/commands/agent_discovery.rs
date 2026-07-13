use std::io::Read;
use tauri::State;

use crate::{
    app_state::AppState,
    managed_agents::{
        command_availability, is_npm_global_install, AcpRuntimeCatalogEntry,
        DiscoverManagedAgentPrereqsRequest, InstallRuntimeResult, InstallStepResult,
        ManagedAgentPrereqsInfo, RelayAgentInfo, DEFAULT_ACP_COMMAND,
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
/// For the codex **outdated** case the returned sequence is a two-step
/// reinstall: first uninstall the old `@zed-industries/codex-acp` package
/// (idempotent — exit 0 when absent), then install the new
/// `@agentclientprotocol/codex-acp`.  This is required because both packages
/// install a global binary named `codex-acp`, and npm ≥7 refuses to overwrite
/// a bin file owned by a different package with `EEXIST`.
///
/// For the **missing** case the catalog's `adapter_install_commands` are used
/// as-is (no prior package to remove).
///
/// This is a pure planning function: it never spawns a process.  Tests use it to
/// assert the correct install command is selected without touching real npm.
pub(crate) fn plan_adapter_install<'c>(
    runtime_id: &str,
    adapter_path: Option<&std::path::Path>,
    adapter_install_commands: &'c [&'c str],
) -> Option<Vec<&'c str>> {
    match adapter_path {
        // Adapter present and current — no install needed.
        Some(_) if runtime_id != "codex" => None,
        Some(path) if !crate::managed_agents::codex_adapter_is_outdated(path) => None,
        // Codex adapter is outdated: uninstall the old package first so npm
        // doesn't hit EEXIST on the shared `codex-acp` bin-link, then install.
        Some(_) => Some(vec![
            "npm uninstall -g @zed-industries/codex-acp",
            "npm install -g @agentclientprotocol/codex-acp",
        ]),
        // Adapter missing: use the catalog's install commands directly.
        None => Some(adapter_install_commands.to_vec()),
    }
}

#[tauri::command]
pub async fn discover_acp_providers() -> Result<Vec<AcpRuntimeCatalogEntry>, String> {
    tokio::task::spawn_blocking(|| {
        crate::managed_agents::clear_resolve_cache();
        crate::managed_agents::refresh_login_shell_path();
        crate::managed_agents::discover_acp_runtimes()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn install_acp_runtime(
    runtime_id: String,
    app: tauri::AppHandle,
) -> Result<InstallRuntimeResult, String> {
    // ── Phase 1: blocking install ────────────────────────────────────────────
    //
    // Run the npm install steps synchronously in spawn_blocking.  The
    // active_installs guard is dropped when install_acp_runtime_blocking
    // returns (Guard impl Drop) — so Phase 2's restart path runs outside
    // the guard and cannot re-enter the mutex.
    let runtime_id_clone = runtime_id.clone();
    let install_result =
        tokio::task::spawn_blocking(move || install_acp_runtime_blocking(&runtime_id_clone))
            .await
            .map_err(|e| format!("install task panicked: {e}"))??;

    if !install_result.success {
        return Ok(install_result);
    }

    // ── Phase 2: async restart of stuck agents ───────────────────────────────
    //
    // Mirror set_global_agent_config: after a successful install, restart any
    // local agents that were spawned in setup-listener mode for this runtime
    // and whose readiness now computes Ready.  Best-effort — errors are logged
    // and returned as failed_restart_count without failing the command.
    let (restarted_count, failed_restart_count) =
        restart_setup_mode_agents_after_install(&app, &runtime_id).await;

    Ok(InstallRuntimeResult {
        success: true,
        steps: install_result.steps,
        restarted_count,
        failed_restart_count,
    })
}

/// Err(_) = infrastructure failure (panic, concurrency guard).
/// Ok({success: false}) = an install step failed (stderr captured in steps).
fn install_acp_runtime_blocking(runtime_id: &str) -> Result<InstallRuntimeResult, String> {
    // Re-fetch the login-shell PATH so a Node.js installation that happened
    // after app launch (or after a previous failed install) is visible to this
    // run and to the subsequent discover_acp_providers call.
    crate::managed_agents::refresh_login_shell_path();
    // Clear the resolve cache so newly-installed binaries are found.
    crate::managed_agents::clear_resolve_cache();

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
                        restarted_count: 0,
                        failed_restart_count: 0,
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
                        restarted_count: 0,
                        failed_restart_count: 0,
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
                    restarted_count: 0,
                    failed_restart_count: 0,
                });
            }
        }
    }

    // Clear the resolve cache so the next discovery picks up new binaries.
    crate::managed_agents::clear_resolve_cache();

    Ok(InstallRuntimeResult {
        success: true,
        steps,
        restarted_count: 0,
        failed_restart_count: 0,
    })
}

// ── Post-install auto-restart (Phase 2 of install_acp_runtime) ───────────────
//
// After a successful adapter install, restart any local agents that:
//   1. are local backend + have a live PID,
//   2. their effective command maps to the just-installed runtime,
//   3. were spawned in setup-listener mode (setup_mode stamp), AND
//   4. their readiness now computes Ready.
//
// Mirrors the two-phase shape of set_global_agent_config.

/// Outcome of a single per-agent restart attempt during post-install restart.
#[derive(Debug)]
enum InstallRestartOutcome {
    Restarted,
    FailedAfterStop,
    Skipped,
}

/// Pure predicate: should this agent be restarted after an adapter install?
///
/// Extracted for unit testing — callers must still re-verify under the lock.
/// The caller is responsible for computing `pid_alive` (via `process_is_running`)
/// before invoking this function, keeping the predicate OS-agnostic and testable
/// on all platforms.
///
/// An agent qualifies iff:
/// - it is a local backend with a live PID (`pid_alive`),
/// - its effective command maps to `runtime_id`,
/// - it was **spawned in setup-listener mode** (`setup_mode`), AND
/// - its readiness **now computes `Ready`** (install fixed the blocker).
fn should_restart_after_install(
    is_local: bool,
    pid_alive: bool,
    runtime_matches: bool,
    setup_mode: bool,
    now_ready: bool,
) -> bool {
    is_local && pid_alive && runtime_matches && setup_mode && now_ready
}

/// Restart all setup-mode agents whose runtime matches `runtime_id` and whose
/// readiness now computes Ready.  Returns `(restarted_count, failed_restart_count)`.
async fn restart_setup_mode_agents_after_install(
    app: &tauri::AppHandle,
    runtime_id: &str,
) -> (u32, u32) {
    use crate::{
        app_state::AppState,
        managed_agents::{
            agent_readiness, known_acp_runtime, load_global_agent_config, load_managed_agents,
            load_personas, record_agent_command, resolve_effective_agent_env, AgentReadiness,
            BackendKind,
        },
    };
    use tauri::Manager;

    // ── Pre-scan: collect candidate pubkeys without holding locks ────────────
    let state = app.state::<AppState>();
    let owner_hex = match super::agents::workspace_owner_hex(&state) {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "buzz-desktop: install_acp_runtime: failed to compute owner_hex for restart: {e}"
            );
            return (0, 0);
        }
    };

    let app_for_scan = app.clone();
    let runtime_id_owned = runtime_id.to_string();
    let candidates = tokio::task::spawn_blocking(move || {
        let records = load_managed_agents(&app_for_scan).unwrap_or_default();
        let personas = load_personas(&app_for_scan).unwrap_or_default();
        let global = load_global_agent_config(&app_for_scan).unwrap_or_default();

        // Read the runtimes map to check setup_mode stamps.
        let state_inner = app_for_scan.state::<AppState>();
        let runtimes = state_inner
            .managed_agent_processes
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        records
            .iter()
            .filter(|record| {
                let is_local = record.backend == BackendKind::Local;
                let pid = record.runtime_pid;
                let effective_cmd = record_agent_command(record, &personas);
                let runtime_matches =
                    known_acp_runtime(&effective_cmd).is_some_and(|r| r.id == runtime_id_owned);
                let setup_mode = runtimes
                    .get(&record.pubkey)
                    .map(|p| p.setup_mode)
                    .unwrap_or(false);
                let effective = resolve_effective_agent_env(
                    record,
                    &personas,
                    known_acp_runtime(&effective_cmd),
                    &global,
                );
                let now_ready = matches!(agent_readiness(&effective), AgentReadiness::Ready);
                let pid_alive = pid.is_some_and(|p| crate::managed_agents::process_is_running(p));
                should_restart_after_install(
                    is_local,
                    pid_alive,
                    runtime_matches,
                    setup_mode,
                    now_ready,
                )
            })
            .map(|r| r.pubkey.clone())
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();

    if candidates.is_empty() {
        return (0, 0);
    }

    let mut restarted_count: u32 = 0;
    let mut failed_restart_count: u32 = 0;

    for pubkey in &candidates {
        let outcome = restart_single_agent_after_install(app, pubkey, &owner_hex, runtime_id).await;
        match outcome {
            InstallRestartOutcome::Restarted => restarted_count += 1,
            InstallRestartOutcome::FailedAfterStop => failed_restart_count += 1,
            InstallRestartOutcome::Skipped => {}
        }
    }

    (restarted_count, failed_restart_count)
}

/// Stop-then-start a single setup-mode agent after a successful adapter install.
///
/// Mirrors `restart_local_agent_on_config_change` from `global_agent_config.rs`:
/// eligibility is re-verified under the store lock before the stop, then the
/// agent is restarted via `start_local_agent_with_preflight`.
async fn restart_single_agent_after_install(
    app: &tauri::AppHandle,
    pubkey: &str,
    owner_hex: &str,
    runtime_id: &str,
) -> InstallRestartOutcome {
    use crate::{
        app_state::AppState,
        managed_agents::{
            agent_readiness, current_instance_id, find_managed_agent_mut, known_acp_runtime,
            load_global_agent_config, load_managed_agents, load_personas, process_is_running,
            record_agent_command, resolve_effective_agent_env, save_managed_agents,
            stop_managed_agent_process, sync_managed_agent_processes, AgentReadiness, BackendKind,
        },
    };
    use tauri::Manager;

    let app_for_stop = app.clone();
    let pubkey_owned = pubkey.to_string();
    let runtime_id_owned = runtime_id.to_string();

    let stop_result = tokio::task::spawn_blocking(move || {
        let state = app_for_stop.state::<AppState>();

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| format!("failed to acquire store lock: {e}"))?;

        let mut records = load_managed_agents(&app_for_stop)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| format!("failed to acquire runtimes lock: {e}"))?;

        // Sync process state so PID liveness reflects current reality.
        let (sync_changed, _) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &current_instance_id(&app_for_stop),
        );
        if sync_changed {
            save_managed_agents(&app_for_stop, &records)?;
        }

        // Re-verify eligibility under lock.
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey_owned)
            .ok_or_else(|| format!("agent {pubkey_owned} not found"))?;

        if record.backend != BackendKind::Local {
            return Err(format!("agent {pubkey_owned} is no longer a local agent"));
        }
        let Some(pid) = record.runtime_pid else {
            return Err(format!(
                "agent {pubkey_owned} no longer has a recorded PID after sync"
            ));
        };
        if !process_is_running(pid) {
            return Err(format!(
                "agent {pubkey_owned} process {pid} is no longer running"
            ));
        }

        let personas = load_personas(&app_for_stop).unwrap_or_default();
        let global = load_global_agent_config(&app_for_stop).unwrap_or_default();

        let effective_cmd = record_agent_command(record, &personas);
        let runtime_matches =
            known_acp_runtime(&effective_cmd).is_some_and(|r| r.id == runtime_id_owned);
        if !runtime_matches {
            return Err(format!(
                "agent {pubkey_owned} runtime no longer matches {runtime_id_owned} under lock"
            ));
        }

        let setup_mode = runtimes
            .get(&pubkey_owned)
            .map(|p| p.setup_mode)
            .unwrap_or(false);
        if !setup_mode {
            return Err(format!(
                "agent {pubkey_owned} is not in setup mode under lock — skipping"
            ));
        }

        let runtime_meta = known_acp_runtime(&effective_cmd);
        let effective = resolve_effective_agent_env(record, &personas, runtime_meta, &global);
        if !matches!(agent_readiness(&effective), AgentReadiness::Ready) {
            return Err(format!(
                "agent {pubkey_owned} readiness is still NotReady after install — not bouncing"
            ));
        }

        // Stop the process.
        let record_mut = find_managed_agent_mut(&mut records, &pubkey_owned)?;
        stop_managed_agent_process(&app_for_stop, record_mut, &mut runtimes)?;
        save_managed_agents(&app_for_stop, &records)?;

        Ok(())
    })
    .await;

    let stopped = match stop_result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            eprintln!("buzz-desktop: install_acp_runtime: skipping restart of {pubkey}: {e}");
            false
        }
        Err(e) => {
            eprintln!(
                "buzz-desktop: install_acp_runtime: spawn_blocking failed for stop of {pubkey}: {e}"
            );
            false
        }
    };

    if !stopped {
        return InstallRestartOutcome::Skipped;
    }

    // Start via the normal preflight path — same as config-change restart.
    {
        use tauri::Manager;
        let state = app.state::<AppState>();
        match super::agents::start_local_agent_with_preflight(app, &state, pubkey, owner_hex, false)
            .await
        {
            Ok(_) => {
                eprintln!(
                    "buzz-desktop: install_acp_runtime: restarted setup-mode agent {pubkey} after install"
                );
                InstallRestartOutcome::Restarted
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: install_acp_runtime: failed to start {pubkey} after install: {e}"
                );
                // Persist last_error so the UI surfaces a diagnosable stopped state.
                if let Err(save_err) = persist_last_error_on_install(app, pubkey, &e) {
                    eprintln!(
                        "buzz-desktop: install_acp_runtime: failed to persist last_error for {pubkey}: {save_err}"
                    );
                }
                InstallRestartOutcome::FailedAfterStop
            }
        }
    }
}

/// Persist a `last_error` on the agent record under the store lock.
/// Best-effort: called only after a failed restart.
fn persist_last_error_on_install(
    app: &tauri::AppHandle,
    pubkey: &str,
    error: &str,
) -> Result<(), String> {
    use crate::{
        app_state::AppState,
        managed_agents::{find_managed_agent_mut, load_managed_agents, save_managed_agents},
    };
    use tauri::Manager;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire store lock: {e}"))?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = crate::util::now_iso();
    save_managed_agents(app, &records)
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
            "npm install -g @agentclientprotocol/codex-acp"
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
    ///   - A 0.x binary (AdapterOutdated) → uninstall-then-install sequence returned
    ///   - A 1.x binary (Available) → None (no reinstall)
    ///   - Missing binary (None path) → catalog install commands returned
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
        // Outdated arm: must uninstall the old package first, then install new.
        assert_eq!(
            cmds,
            vec![
                "npm uninstall -g @zed-industries/codex-acp",
                "npm install -g @agentclientprotocol/codex-acp",
            ],
            "outdated codex adapter must produce uninstall-then-install sequence; got {cmds:?}"
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
    fn test_plan_adapter_install_returns_catalog_cmds_when_no_adapter_path() {
        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", None, install_cmds);
        assert!(plan.is_some(), "missing adapter must trigger install plan");
        // Missing arm: use the catalog's install commands directly (no prior
        // package to uninstall — fresh install, not a reinstall).
        assert_eq!(
            plan.unwrap(),
            vec!["npm install -g @agentclientprotocol/codex-acp"],
            "missing codex adapter must use catalog install commands only"
        );
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

    // ── should_restart_after_install ─────────────────────────────────────────

    /// Setup-mode agent on matching runtime that is now Ready → restart.
    #[test]
    fn test_should_restart_after_install_setup_mode_now_ready_is_candidate() {
        assert!(
            should_restart_after_install(true, true, true, true, true),
            "setup-mode codex agent that became Ready must be restarted after install"
        );
    }

    /// Setup-mode agent still NotReady after install (e.g. logged out) → no restart.
    #[test]
    fn test_should_restart_after_install_still_not_ready_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, true, true, false),
            "setup-mode agent still NotReady must NOT be restarted (would re-enter setup mode)"
        );
    }

    /// Healthy in-pool agent (setup_mode=false) → no restart, even if now Ready.
    #[test]
    fn test_should_restart_after_install_healthy_agent_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, true, false, true),
            "healthy in-pool agent (setup_mode=false) must NOT be bounced on install"
        );
    }

    /// Agent on a different runtime_id → no restart.
    #[test]
    fn test_should_restart_after_install_different_runtime_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, false, true, true),
            "agent on a different runtime must NOT be restarted by this install"
        );
    }

    /// Remote/provider-backend agent → no restart (not local).
    #[test]
    fn test_should_restart_after_install_non_local_is_not_candidate() {
        assert!(
            !should_restart_after_install(false, true, true, true, true),
            "non-local (provider-backend) agent must NOT be restarted"
        );
    }

    /// Dead process (pid_alive=false) → no restart.
    #[test]
    fn test_should_restart_after_install_dead_pid_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, false, true, true, true),
            "agent whose process is no longer running must NOT be restarted"
        );
    }

    // ── badge availability-drift (Phase 2) ───────────────────────────────────
    //
    // `availability_drift` is a pure predicate over two `Option` values —
    // no global state, no parallelism hazard.

    /// Both sides known and different → drift detected.
    #[test]
    fn test_availability_drift_detected_when_stamped_differs_from_current() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            availability_drift(
                Some(&AcpAvailabilityStatus::Available),
                Some(AcpAvailabilityStatus::AdapterOutdated),
            ),
            "Available stamped vs AdapterOutdated current must be detected as drift"
        );
    }

    /// Both sides known and equal → no drift.
    #[test]
    fn test_availability_drift_no_drift_when_stamped_equals_current() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(
                Some(&AcpAvailabilityStatus::Available),
                Some(AcpAvailabilityStatus::Available),
            ),
            "matching stamped and current must not show drift"
        );
    }

    /// Stamped is None (cold cache at spawn) → no drift regardless of current.
    #[test]
    fn test_availability_drift_none_stamp_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(None, Some(AcpAvailabilityStatus::Available)),
            "None stamp (cold cache at spawn) must never signal drift"
        );
    }

    /// Current is None (cache cold now) → no drift regardless of stamp.
    #[test]
    fn test_availability_drift_none_current_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(Some(&AcpAvailabilityStatus::Available), None),
            "None current (cache cold) must never signal drift"
        );
    }

    /// Non-codex agent (stamp is None) → no drift (None case).
    #[test]
    fn test_availability_drift_non_codex_none_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        // Non-codex agents have `adapter_availability = None` — must never flip.
        assert!(
            !availability_drift(None, Some(AcpAvailabilityStatus::AdapterMissing)),
            "non-codex agent (None stamp) must never trigger drift badge"
        );
    }
}
