use std::collections::HashMap;

use tauri::AppHandle;

use crate::{
    managed_agents::{
        append_log_marker, known_acp_provider, login_shell_path, managed_agent_log_path,
        missing_command_message, normalize_agent_args, open_log_file, resolve_command,
        ManagedAgentProcess, ManagedAgentRecord, ManagedAgentSummary,
    },
    util::now_iso,
};

/// Binary name fragments for all known agent/harness processes that Sprout
/// may spawn. Used by `process_belongs_to_us()` and the orphan sweep to
/// identify processes we should clean up. Both hyphenated and underscored
/// variants are listed because macOS `proc_name()` and Linux `/proc/comm`
/// may report either form depending on how the binary was built.
pub(crate) const KNOWN_AGENT_BINARIES: &[&str] = &[
    "sprout-acp",
    "sprout_acp",
    "sprout-agent",
    "sprout_agent",
    "claude-agent-acp",
    "claude_agent_acp",
    "claude-code-acp",
    "claude_code_acp",
    "codex-acp",
    "codex_acp",
    "goose",
    "sprout-mcp",
    "sprout_mcp",
    // sprout-dev-mcp's multicall personalities (rg, tree, sprout,
    // git-credential-nostr, git-sign-nostr) are short-lived per-tool-call
    // invocations — not listed here.
    "sprout-dev-mcp",
    "sprout_dev_mcp",
];

/// Check if a process name matches any of our known agent binaries.
/// Uses exact match or prefix-with-separator to avoid false positives
/// (e.g. `"goose"` must not match `"mongoose"`).
fn name_matches_known_binary(name: &str) -> bool {
    KNOWN_AGENT_BINARIES.iter().any(|&binary| {
        name == binary || {
            name.starts_with(binary) && {
                let rest = &name[binary.len()..];
                rest.starts_with('-') || rest.starts_with('_') || rest.starts_with('.')
            }
        }
    })
}

#[cfg(unix)]
pub(crate) fn process_is_running(pid: u32) -> bool {
    // Use libc::kill with signal 0 instead of forking a subprocess.
    // Returns true only if the process exists AND we can signal it.
    // Returns false for non-existent PIDs (ESRCH) and PIDs owned by
    // other users (EPERM) — callers should not interact with those.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub(crate) fn process_is_running(_pid: u32) -> bool {
    false
}

/// Check if a PID belongs to a known agent process we spawned.
/// Returns false for recycled PIDs that now belong to other processes.
#[cfg(target_os = "macos")]
pub(crate) fn process_belongs_to_us(pid: u32) -> bool {
    // Use proc_name() from libproc to get the process name without spawning
    // a subprocess.
    extern "C" {
        fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
    }
    let mut buf = [0u8; 1024];
    let len = unsafe {
        proc_name(
            pid as i32,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if len <= 0 {
        return false;
    }
    let name = String::from_utf8_lossy(&buf[..len as usize]);
    name_matches_known_binary(&name)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn process_belongs_to_us(pid: u32) -> bool {
    // First try /proc/<pid>/comm. Note: comm is truncated to 15 bytes on Linux,
    // so binaries with names longer than 15 chars (e.g. "claude-agent-acp")
    // will never match here.
    if let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        if name_matches_known_binary(name.trim()) {
            return true;
        }
    }

    // Fallback: read /proc/<pid>/exe which is a symlink to the full binary path.
    // This is not subject to the 15-byte truncation limit.
    if let Ok(exe_path) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        if let Some(basename) = exe_path.file_name().and_then(|n| n.to_str()) {
            return name_matches_known_binary(basename);
        }
    }

    false
}

#[cfg(not(unix))]
pub(crate) fn process_belongs_to_us(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn signal_process_group_or_leader(pid: u32, signal: i32, action: &str) -> Result<(), String> {
    let pgid = -(pid as i32);

    if unsafe { libc::kill(pgid, signal) } == 0 {
        return Ok(());
    }

    let group_err = std::io::Error::last_os_error();
    if !process_is_running(pid) {
        return Ok(());
    }

    // Some local agent trees can no longer be signalled as a process group
    // (for example if the leader changed groups, or macOS returns EPERM for one
    // descendant). Fall back to the leader PID so stop/delete can still recover.
    if matches!(
        group_err.raw_os_error(),
        Some(libc::EPERM) | Some(libc::ESRCH)
    ) {
        if unsafe { libc::kill(pid as i32, signal) } == 0 {
            return Ok(());
        }

        let leader_err = std::io::Error::last_os_error();
        if leader_err.raw_os_error() == Some(libc::ESRCH) || !process_is_running(pid) {
            return Ok(());
        }

        return Err(format!("failed to {action} process {pid}: {leader_err}"));
    }

    Err(format!(
        "failed to {action} process group {pid}: {group_err}"
    ))
}

#[cfg(unix)]
pub(crate) fn terminate_process(pid: u32) -> Result<(), String> {
    // Try graceful shutdown first (SIGTERM to the group).
    signal_process_group_or_leader(pid, libc::SIGTERM, "terminate")?;

    // Wait up to 1s for graceful exit.
    for _ in 0..10 {
        if !process_is_running(pid) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Escalate to SIGKILL on the entire group.
    signal_process_group_or_leader(pid, libc::SIGKILL, "kill")?;

    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn terminate_process(_pid: u32) -> Result<(), String> {
    Err("managed agent shutdown after app restart is only supported on Unix".to_string())
}

/// Send SIGTERM to all given PIDs (as process groups), wait, then SIGKILL
/// any survivors. Uses `-pid` to kill the entire process group — if an
/// orphaned agent called `setsid()`, it IS the group leader, so this
/// reaches its children too.
#[cfg(unix)]
fn sigterm_then_sigkill(pids: &[i32]) {
    // Send SIGTERM to each process group. Track whether any signal was
    // actually delivered so we can skip the sleep when everything is
    // already gone.
    let mut any_signalled = false;
    for &pid in pids {
        if unsafe { libc::kill(-pid, libc::SIGTERM) } == 0 {
            any_signalled = true;
        }
    }

    if !any_signalled {
        return;
    }

    std::thread::sleep(std::time::Duration::from_millis(200));

    for &pid in pids {
        if process_is_running(pid as u32) {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

/// Kill orphaned agent processes using PID file receipts. Reads all files from
/// `agent-pids/`, verifies each PID still belongs to a known agent binary,
/// then kills the process group. Deletes the PID file after killing.
///
/// `skip_pids` are PIDs already handled by the tracked-agent path.
#[cfg(unix)]
pub(crate) fn sweep_orphaned_agent_processes(app: &AppHandle, skip_pids: &[u32]) {
    let entries = super::read_all_agent_pid_files(app);
    let orphans: Vec<i32> = entries
        .iter()
        .filter(|(_, pid)| {
            !skip_pids.contains(pid) && process_is_running(*pid) && process_belongs_to_us(*pid)
        })
        .map(|(_, pid)| *pid as i32)
        .collect();

    if !orphans.is_empty() {
        sigterm_then_sigkill(&orphans);
    }

    // Clean up PID files for processes we just killed or that are already gone.
    for (pubkey, pid) in &entries {
        if skip_pids.contains(pid) {
            continue;
        }
        if !process_is_running(*pid) || !process_belongs_to_us(*pid) {
            super::remove_agent_pid_file(app, pubkey);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn sweep_orphaned_agent_processes(app: &AppHandle, _skip_pids: &[u32]) {
    let _ = app;
}

/// Kill stale agent processes from a previous session whose PID is still alive
/// but not tracked in the current `runtimes` map. Updates the record fields and
/// returns `true` if any records were modified.
pub fn kill_stale_tracked_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<String, ManagedAgentProcess>,
) -> bool {
    use crate::managed_agents::BackendKind;

    let mut changed = false;
    for record in records.iter_mut() {
        if record.backend != BackendKind::Local {
            continue;
        }
        let Some(pid) = record.runtime_pid else {
            continue;
        };
        if !runtimes.contains_key(&record.pubkey) {
            if process_belongs_to_us(pid) {
                let _ = terminate_process(pid);
            }
            record.runtime_pid = None;
            record.last_stopped_at = Some(crate::util::now_iso());
            record.updated_at = crate::util::now_iso();
            changed = true;
        }
    }
    changed
}

pub fn sync_managed_agent_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<String, ManagedAgentProcess>,
) -> bool {
    let mut changed = false;
    let mut exited = Vec::new();

    for (pubkey, runtime) in runtimes.iter_mut() {
        let status = match runtime.child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                if let Some(record) = records.iter_mut().find(|record| record.pubkey == *pubkey) {
                    record.updated_at = now_iso();
                    record.last_error = Some(format!("failed to inspect process state: {error}"));
                }
                changed = true;
                exited.push(pubkey.clone());
                continue;
            }
        };

        let Some(status) = status else {
            continue;
        };

        if let Some(record) = records.iter_mut().find(|record| record.pubkey == *pubkey) {
            record.updated_at = now_iso();
            record.runtime_pid = None;
            record.last_stopped_at = Some(now_iso());
            record.last_exit_code = status.code();
            record.last_error = if status.success() {
                None
            } else {
                Some(format!("harness exited with status {status}"))
            };
        }

        changed = true;
        exited.push(pubkey.clone());
    }

    for pubkey in exited {
        runtimes.remove(&pubkey);
    }

    for record in records.iter_mut() {
        if runtimes.contains_key(&record.pubkey) {
            continue;
        }

        let Some(pid) = record.runtime_pid else {
            continue;
        };

        if process_is_running(pid) && process_belongs_to_us(pid) {
            continue;
        }

        record.runtime_pid = None;
        record.updated_at = now_iso();
        if record.last_stopped_at.is_none() {
            record.last_stopped_at = Some(now_iso());
        }
        changed = true;
    }

    changed
}

pub fn build_managed_agent_summary(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    runtimes: &HashMap<String, ManagedAgentProcess>,
) -> Result<ManagedAgentSummary, String> {
    use crate::managed_agents::BackendKind;

    let (status, pid, log_path) = if record.backend != BackendKind::Local {
        // Two-axis status model for remote agents:
        //
        //   Control-plane (this field): "deployed" = provider has been invoked and
        //   returned a backend_agent_id. "not_deployed" = no deploy call yet (or it
        //   failed). This axis tracks whether infrastructure *exists*, not whether
        //   the process is currently running.
        //
        //   Live axis (relay presence, polled by frontend): online/away/offline.
        //   Shown as a PresenceDot next to the agent name. This is the real-time
        //   signal for whether the harness is connected.
        //
        // After !shutdown the agent goes offline (presence) but stays "deployed"
        // (infrastructure still exists). This is intentional — the provider may
        // have allocated a VM/container that persists across process restarts.
        // A future provider `undeploy` operation (v2) will handle teardown.
        let status = if record.backend_agent_id.is_some() {
            "deployed".to_string()
        } else {
            "not_deployed".to_string()
        };
        (status, None, String::new())
    } else {
        let persisted_pid = record.runtime_pid.filter(|pid| process_is_running(*pid));
        if let Some(runtime) = runtimes.get(&record.pubkey) {
            (
                "running".to_string(),
                Some(runtime.child.id()),
                runtime.log_path.display().to_string(),
            )
        } else if let Some(pid) = persisted_pid {
            (
                "running".to_string(),
                Some(pid),
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
            )
        } else {
            (
                "stopped".to_string(),
                None,
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
            )
        }
    };

    Ok(ManagedAgentSummary {
        pubkey: record.pubkey.clone(),
        name: record.name.clone(),
        persona_id: record.persona_id.clone(),
        relay_url: record.relay_url.clone(),
        acp_command: record.acp_command.clone(),
        agent_command: record.agent_command.clone(),
        agent_args: record.agent_args.clone(),
        mcp_command: record.mcp_command.clone(),
        turn_timeout_seconds: record.turn_timeout_seconds,
        idle_timeout_seconds: record.idle_timeout_seconds,
        max_turn_duration_seconds: record.max_turn_duration_seconds,
        parallelism: record.parallelism,
        system_prompt: record.system_prompt.clone(),
        model: record.model.clone(),
        mcp_toolsets: record.mcp_toolsets.clone(),
        backend: record.backend.clone(),
        backend_agent_id: record.backend_agent_id.clone(),
        status,
        pid,
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        last_started_at: record.last_started_at.clone(),
        last_stopped_at: record.last_stopped_at.clone(),
        last_exit_code: record.last_exit_code,
        last_error: record.last_error.clone(),
        start_on_app_launch: record.start_on_app_launch,
        log_path,
    })
}

pub fn find_managed_agent_mut<'a>(
    records: &'a mut [ManagedAgentRecord],
    pubkey: &str,
) -> Result<&'a mut ManagedAgentRecord, String> {
    records
        .iter_mut()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))
}

/// Spawn an agent process without holding any locks on records or runtimes.
/// Returns the child process and log path on success. The caller is responsible
/// for updating `ManagedAgentRecord` fields and inserting into the runtimes map.
pub fn spawn_agent_child(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Result<(std::process::Child, std::path::PathBuf), String> {
    let log_path = managed_agent_log_path(app, &record.pubkey)?;
    append_log_marker(
        &log_path,
        &format!(
            "\n=== starting {} ({}) at {} ===",
            record.name,
            record.pubkey,
            now_iso()
        ),
    )?;

    let stdout = open_log_file(&log_path)?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone log handle: {error}"))?;
    let agent_args = normalize_agent_args(&record.agent_command, record.agent_args.clone());
    let resolved_acp_command = resolve_command(&record.acp_command, Some(app))
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let resolved_mcp_command = resolve_command(&record.mcp_command, Some(app))
        .ok_or_else(|| missing_command_message(&record.mcp_command, "MCP server command"))?;
    // Resolve agent command to a full path (DMG launches have minimal PATH).
    let resolved_agent_command = resolve_command(&record.agent_command, Some(app))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| record.agent_command.clone());

    // Augment PATH for DMG launches so child processes (e.g. #!/usr/bin/env node) can find their runtimes.
    let augmented_path = login_shell_path();

    let mut command = std::process::Command::new(&resolved_acp_command);
    if let Some(home) = super::default_agent_workdir() {
        command.current_dir(home);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::from(stdout));
    command.stderr(std::process::Stdio::from(stderr));
    if let Some(ref path) = augmented_path {
        command.env("PATH", path);
    }
    command.env("RUST_LOG", child_rust_log_filter());
    command.env("SPROUT_PRIVATE_KEY", &record.private_key_nsec);
    command.env("SPROUT_RELAY_URL", &record.relay_url);
    command.env("SPROUT_ACP_AGENT_COMMAND", &resolved_agent_command);
    command.env("SPROUT_ACP_AGENT_ARGS", agent_args.join(","));
    command.env("SPROUT_ACP_MCP_COMMAND", &resolved_mcp_command);
    // Enable MCP hook tools (_Stop, _PostCompact) for agents that need them.
    // Uses "*" because build_mcp_servers() hard-codes the server name to "sprout-mcp".
    if known_acp_provider(&record.agent_command).is_some_and(|p| p.mcp_hooks) {
        command.env("MCP_HOOK_SERVERS", "*");
    }
    if let Some(idle) = record.idle_timeout_seconds {
        command.env("SPROUT_ACP_IDLE_TIMEOUT", idle.to_string());
        command.env("SPROUT_ACP_TURN_TIMEOUT", idle.to_string());
    } else {
        command.env(
            "SPROUT_ACP_TURN_TIMEOUT",
            record.turn_timeout_seconds.to_string(),
        );
    }

    let max_dur = record
        .max_turn_duration_seconds
        .unwrap_or(super::types::DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS);
    command.env("SPROUT_ACP_MAX_TURN_DURATION", max_dur.to_string());
    command.env("SPROUT_ACP_AGENTS", record.parallelism.to_string());
    command.env("SPROUT_ACP_MULTIPLE_EVENT_HANDLING", "owner-interrupt");
    command.env("SPROUT_ACP_DEDUP", "queue");
    command.env(
        "GOOSE_MODE",
        std::env::var("GOOSE_MODE").unwrap_or_else(|_| "auto".to_string()),
    );
    if let (Some(pack_path), Some(persona_name)) =
        (&record.persona_pack_path, &record.persona_name_in_pack)
    {
        command.env("SPROUT_ACP_PERSONA_PACK", pack_path);
        command.env("SPROUT_ACP_PERSONA_NAME", persona_name);
    }

    // Resolve system prompt and model: prefer the persona definition (if a
    // persona pack is configured and the persona matched), otherwise fall back
    // to the record-level overrides.
    let has_persona_pack =
        record.persona_pack_path.is_some() && record.persona_name_in_pack.is_some();
    let persona_prompt_and_model: Option<(String, Option<String>)> = has_persona_pack
        .then(|| {
            record
                .persona_id
                .as_deref()
                .and_then(|pid| {
                    super::load_personas(app)
                        .ok()?
                        .into_iter()
                        .find(|p| p.id == pid)
                })
                .map(|p| (p.system_prompt, p.model))
        })
        .flatten();

    let (effective_prompt, effective_model) = match persona_prompt_and_model {
        Some((prompt, model)) => (Some(prompt), model),
        None => (record.system_prompt.clone(), record.model.clone()),
    };

    if let Some(prompt) = &effective_prompt {
        command.env("SPROUT_ACP_SYSTEM_PROMPT", prompt);
    } else {
        command.env_remove("SPROUT_ACP_SYSTEM_PROMPT");
    }
    if let Some(model) = &effective_model {
        command.env("SPROUT_ACP_MODEL", model);
    } else {
        command.env_remove("SPROUT_ACP_MODEL");
    }
    if let Some(toolsets) = &record.mcp_toolsets {
        command.env("SPROUT_TOOLSETS", toolsets);
    } else {
        command.env("SPROUT_TOOLSETS", "default,canvas,forums,dms,media");
    }
    command.env_remove("SPROUT_ACP_PRIVATE_KEY");
    command.env_remove("SPROUT_ACP_API_TOKEN");
    command.env_remove("SPROUT_API_TOKEN");

    if let Some(ref auth_tag) = record.auth_tag {
        command.env("SPROUT_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("SPROUT_AUTH_TAG");
    }

    command.env("SPROUT_ACP_RELAY_OBSERVER", "true");

    // ── Git credential helper for Sprout relay ──────────────────────────
    //
    // Agents need to clone/push repos hosted on the Sprout relay's git
    // server, which authenticates via NIP-98. The `git-credential-nostr`
    // binary signs auth events using the agent's nostr key.
    //
    // We configure git via GIT_CONFIG_COUNT env vars (ephemeral, no
    // filesystem writes) scoped to the relay's git URL so we don't
    // interfere with other remotes (e.g. GitHub).
    //
    // NOSTR_PRIVATE_KEY mirrors SPROUT_PRIVATE_KEY — keep in sync.
    if let Some(cred_helper) = resolve_command("git-credential-nostr", Some(app)) {
        let relay_http_url = crate::relay::relay_http_base_url(&record.relay_url);

        command.env("NOSTR_PRIVATE_KEY", &record.private_key_nsec);
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.env("GIT_CONFIG_COUNT", "2");
        command.env(
            "GIT_CONFIG_KEY_0",
            format!("credential.{relay_http_url}/git.helper"),
        );
        command.env("GIT_CONFIG_VALUE_0", cred_helper.display().to_string());
        command.env(
            "GIT_CONFIG_KEY_1",
            format!("credential.{relay_http_url}/git.useHttpPath"),
        );
        command.env("GIT_CONFIG_VALUE_1", "true");
    } else {
        eprintln!(
            "sprout-desktop: git-credential-nostr not found — agent {} will not have automatic Sprout git auth",
            record.name,
        );
    }

    // Spawn the harness in its own process group so we can kill the entire
    // tree (harness + MCP servers + agent subprocesses) on shutdown.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command.spawn().map_err(|error| {
        format!(
            "failed to spawn `{}` for agent {}: {error}",
            resolved_acp_command.display(),
            record.name
        )
    })?;

    let _ = super::write_agent_pid_file(app, &record.pubkey, child.id());

    Ok((child, log_path))
}

fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("sprout_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},sprout_acp=info"),
        _ => "sprout_acp=info".to_string(),
    }
}

pub fn start_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<String, ManagedAgentProcess>,
) -> Result<(), String> {
    if let Some(runtime) = runtimes.get_mut(&record.pubkey) {
        if runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect running process: {error}"))?
            .is_none()
        {
            return Ok(());
        }

        runtimes.remove(&record.pubkey);
    }

    if let Some(pid) = record.runtime_pid {
        if process_is_running(pid) && process_belongs_to_us(pid) {
            record.updated_at = now_iso();
            record.last_error = None;
            return Ok(());
        }

        record.runtime_pid = None;
    }

    let (child, log_path) = spawn_agent_child(app, record)?;

    let now = now_iso();
    record.updated_at = now.clone();
    record.runtime_pid = Some(child.id());
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;

    runtimes.insert(
        record.pubkey.clone(),
        ManagedAgentProcess { child, log_path },
    );
    Ok(())
}

pub fn stop_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<String, ManagedAgentProcess>,
) -> Result<(), String> {
    let Some(mut runtime) = runtimes.remove(&record.pubkey) else {
        if let Some(pid) = record.runtime_pid {
            if process_is_running(pid) {
                terminate_process(pid)?;
            }

            let now = now_iso();
            record.runtime_pid = None;
            record.updated_at = now.clone();
            record.last_stopped_at = Some(now);
            record.last_exit_code = None;
            record.last_error = None;
        }
        super::remove_agent_pid_file(app, &record.pubkey);
        return Ok(());
    };

    // On Unix, kill the entire process group via terminate_process.
    // On non-Unix, fall back to Child::kill() since terminate_process
    // is not implemented there.
    #[cfg(unix)]
    terminate_process(runtime.child.id())?;
    #[cfg(not(unix))]
    runtime
        .child
        .kill()
        .map_err(|error| format!("failed to kill agent process: {error}"))?;
    let status = runtime
        .child
        .wait()
        .map_err(|error| format!("failed to wait for agent shutdown: {error}"))?;
    let now = now_iso();
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_stopped_at = Some(now);
    record.last_exit_code = status.code();
    record.last_error = None;

    super::remove_agent_pid_file(app, &record.pubkey);

    append_log_marker(
        &runtime.log_path,
        &format!(
            "=== stopped {} ({}) at {} ===",
            record.name,
            record.pubkey,
            now_iso()
        ),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::managed_agents::known_acp_provider;

    #[test]
    fn sprout_agent_has_mcp_hooks() {
        let p = known_acp_provider("sprout-agent").expect("should resolve");
        assert!(p.mcp_hooks);
        assert_eq!(p.mcp_command, Some("sprout-dev-mcp"));
    }

    #[test]
    fn sprout_agent_resolved_via_path() {
        assert!(known_acp_provider("/usr/local/bin/sprout-agent").is_some_and(|p| p.mcp_hooks));
    }

    #[test]
    fn goose_has_no_mcp_hooks() {
        let p = known_acp_provider("goose").expect("should resolve");
        assert!(!p.mcp_hooks);
        assert_eq!(p.mcp_command, None);
    }

    #[test]
    fn unknown_command_returns_none() {
        assert!(known_acp_provider("custom-agent").is_none());
    }
}
