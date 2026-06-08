use std::collections::HashMap;

use tauri::AppHandle;

use crate::{
    managed_agents::{
        append_log_marker, known_acp_runtime, login_shell_path, managed_agent_log_path,
        missing_command_message, normalize_agent_args, open_log_file, resolve_command,
        ManagedAgentProcess, ManagedAgentRecord, ManagedAgentSummary,
    },
    util::now_iso,
};

type RespondToEnv = (Vec<(&'static str, String)>, Vec<&'static str>);

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

/// The value stamped into the `SPROUT_MANAGED_AGENT` env var of every agent we
/// spawn, identifying *which* desktop instance owns it. We use the app's bundle
/// identifier (`xyz.block.sprout.app` for release, `xyz.block.sprout.app.dev`
/// for `just dev`) because it is stable across restarts — a relaunched dev
/// instance still recognizes its own previously-spawned agents as reclaimable,
/// while never matching another instance's (e.g. a dev build never reaps a DMG
/// build's agents, and vice versa). This is what lets two Sprouts coexist on
/// one machine without one's cleanup nuking the other's agents.
pub(crate) fn current_instance_id(app: &AppHandle) -> String {
    app.config().identifier.clone()
}

/// Build the full `SPROUT_MANAGED_AGENT=<instance-id>` env entry we match
/// against when scanning processes. Kept here so the spawn stamp and the sweep
/// matcher can never drift apart.
fn sprout_marker_entry(instance_id: &str) -> Vec<u8> {
    format!("SPROUT_MANAGED_AGENT={instance_id}").into_bytes()
}

/// Check if a running process is one of *our* managed agents: it must carry
/// `SPROUT_MANAGED_AGENT=<instance_id>` in its environment, where `instance_id`
/// is this desktop instance's id. A process stamped with a *different* instance
/// id belongs to another live Sprout app and must never be reaped here.
#[cfg(target_os = "macos")]
fn process_has_sprout_marker(pid: u32, instance_id: &str) -> bool {
    let marker = sprout_marker_entry(instance_id);

    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut buf_size: libc::size_t = 0;

    // First call: get required buffer size.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut buf_size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return false;
    }

    let mut buf: Vec<u8> = vec![0; buf_size];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut buf_size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return false;
    }
    buf.truncate(buf_size);

    // Buffer layout: [i32 argc][exec_path\0][null padding][argv\0...][env\0...]
    if buf.len() < std::mem::size_of::<libc::c_int>() {
        return false;
    }
    let mut n_args: libc::c_int = 0;
    unsafe {
        std::ptr::copy_nonoverlapping(
            buf.as_ptr(),
            &mut n_args as *mut libc::c_int as *mut u8,
            std::mem::size_of::<libc::c_int>(),
        );
    }
    let mut pos = std::mem::size_of::<libc::c_int>();

    // Skip exec path (scan to first null).
    while pos < buf.len() && buf[pos] != 0 {
        pos += 1;
    }
    // Skip null padding between exec path and argv[0].
    while pos < buf.len() && buf[pos] == 0 {
        pos += 1;
    }
    // Skip argc argument strings.
    let mut args_remaining = n_args;
    while args_remaining > 0 && pos < buf.len() {
        while pos < buf.len() && buf[pos] != 0 {
            pos += 1;
        }
        while pos < buf.len() && buf[pos] == 0 {
            pos += 1;
        }
        args_remaining -= 1;
    }
    // Remaining bytes are null-delimited environment strings.
    buf[pos..].split(|&b| b == 0).any(|entry| entry == marker)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn process_has_sprout_marker(pid: u32, instance_id: &str) -> bool {
    let marker = sprout_marker_entry(instance_id);
    let Ok(data) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    data.split(|&b| b == 0).any(|entry| entry == marker)
}

#[cfg(not(unix))]
fn process_has_sprout_marker(_pid: u32, _instance_id: &str) -> bool {
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
        // Check if the group has any living members, not just the leader.
        // kill(-pid, 0) returns 0 if ANY member of the group is signalable.
        if unsafe { libc::kill(-pid, 0) } == 0 {
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
    // Collect live orphans AND dead-leader groups into a single kill batch.
    // Dead leaders: PGID may have been recycled, but the window is narrow
    // (PID files are from this session) and the cost of missing surviving
    // group members outweighs the recycling risk.
    let targets: Vec<i32> = entries
        .iter()
        .filter(|(_, pid)| {
            if skip_pids.contains(pid) {
                return false;
            }
            (process_is_running(*pid) && process_belongs_to_us(*pid)) || !process_is_running(*pid)
        })
        .map(|(_, pid)| *pid as i32)
        .collect();

    if !targets.is_empty() {
        sigterm_then_sigkill(&targets);
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

/// Enumerate all processes on the system owned by the current user and kill any
/// agent binary stamped with *this* instance's `SPROUT_MANAGED_AGENT` marker
/// (`instance_id`) that isn't in `skip_pids`. This catches orphans that escaped
/// PID-file-based cleanup (e.g. agent workers spawned with their own process
/// group whose parent harness already exited and had its PID file removed),
/// while leaving another live Sprout instance's agents untouched.
#[cfg(target_os = "macos")]
pub(crate) fn sweep_system_agent_processes(instance_id: &str, skip_pids: &[u32]) {
    extern "C" {
        fn proc_listallpids(buffer: *mut libc::c_int, buffersize: libc::c_int) -> libc::c_int;
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    #[repr(C)]
    struct BSDInfo {
        _pad: [u8; 20],
        pbi_uid: u32,
        _rest: [u8; 112],
    }
    const _: () = assert!(std::mem::size_of::<BSDInfo>() == 136);
    const PROC_PIDTBSDINFO: libc::c_int = 3;

    let my_uid = unsafe { libc::getuid() };

    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return;
    }

    let buf_len = (count as usize) * 2;
    let mut pids: Vec<libc::c_int> = vec![0; buf_len];
    let actual = unsafe {
        proc_listallpids(
            pids.as_mut_ptr(),
            (buf_len * std::mem::size_of::<libc::c_int>()) as libc::c_int,
        )
    };
    if actual <= 0 {
        return;
    }
    pids.truncate(actual as usize);

    let my_pid = std::process::id() as i32;
    let mut orphans: Vec<i32> = Vec::new();

    for &pid in &pids {
        if pid <= 0 {
            continue;
        }
        let upid = pid as u32;
        if skip_pids.contains(&upid) || pid == my_pid {
            continue;
        }
        // Check binary name first (cheap proc_name call) before UID lookup.
        if !process_belongs_to_us(upid) {
            continue;
        }
        // Verify UID to avoid killing another user's identically-named binary.
        let mut info = std::mem::MaybeUninit::<BSDInfo>::zeroed();
        let ret = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of::<BSDInfo>() as libc::c_int,
            )
        };
        if ret <= 0 {
            continue;
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_uid != my_uid {
            continue;
        }
        if !process_has_sprout_marker(upid, instance_id) {
            continue;
        }
        orphans.push(pid);
    }

    if !orphans.is_empty() {
        eprintln!(
            "sprout-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        sigterm_then_sigkill(&orphans);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn sweep_system_agent_processes(instance_id: &str, skip_pids: &[u32]) {
    let my_uid = unsafe { libc::getuid() };
    let mut orphans: Vec<i32> = Vec::new();
    let my_pid = std::process::id() as i32;

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<i32>() else {
            continue;
        };
        if pid <= 0 || pid == my_pid {
            continue;
        }
        let upid = pid as u32;
        if skip_pids.contains(&upid) {
            continue;
        }
        // Check ownership via /proc/<pid> metadata.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != my_uid {
            continue;
        }
        if process_belongs_to_us(upid) && process_has_sprout_marker(upid, instance_id) {
            orphans.push(pid);
        }
    }

    if !orphans.is_empty() {
        eprintln!(
            "sprout-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        sigterm_then_sigkill(&orphans);
    }
}

#[cfg(not(unix))]
pub(crate) fn sweep_system_agent_processes(_instance_id: &str, _skip_pids: &[u32]) {}

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
                super::meaningful_agent_error_from_log(&runtime.log_path)
                    .unwrap_or_else(|| format!("harness exited with status {status}"))
                    .into()
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
        env_vars: record.env_vars.clone(),
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
        respond_to: record.respond_to,
        respond_to_allowlist: record.respond_to_allowlist.clone(),
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

/// Pure decision function for the inbound author gate env vars.
///
/// Returns the env vars to **set** and the env vars to **remove**. Removal is
/// belt-and-suspenders: an inherited parent env var must not leak into a
/// child agent and silently change its security posture.
///
/// The `owner_hex` argument is the current workspace owner pubkey. It's used
/// as a fallback for legacy records (`auth_tag.is_none()`) — without it, the
/// harness's owner cache stays empty and `owner-only` / `allowlist` modes
/// drop everything.
///
/// Returns `Err(...)` if the record's allowlist fails validation. The harness
/// validates too, but doing it here means we never spawn a doomed process.
pub(crate) fn build_respond_to_env(
    record: &ManagedAgentRecord,
    owner_hex: Option<&str>,
) -> Result<RespondToEnv, String> {
    // Defensive re-validation: an on-disk record could have been hand-edited.
    let normalized = super::types::validate_respond_to_allowlist(&record.respond_to_allowlist)?;
    if record.respond_to == super::types::RespondTo::Allowlist && normalized.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }

    let mut set: Vec<(&'static str, String)> = Vec::new();
    let mut remove: Vec<&'static str> = Vec::new();

    set.push((
        "SPROUT_ACP_RESPOND_TO",
        record.respond_to.as_str().to_string(),
    ));

    if record.respond_to == super::types::RespondTo::Allowlist {
        set.push(("SPROUT_ACP_RESPOND_TO_ALLOWLIST", normalized.join(",")));
    } else {
        remove.push("SPROUT_ACP_RESPOND_TO_ALLOWLIST");
    }

    // Legacy fallback: agents created before NIP-OA lack `auth_tag`. Without
    // it the harness can't resolve the owner, and owner-dependent gate modes
    // would drop every event. Forwarding the workspace owner pubkey via
    // SPROUT_ACP_AGENT_OWNER keeps those records functional. Modern records
    // (`auth_tag = Some(...)`) use `SPROUT_AUTH_TAG` as before.
    if record.auth_tag.is_none() {
        if let Some(owner) = owner_hex {
            set.push(("SPROUT_ACP_AGENT_OWNER", owner.to_string()));
        } else {
            remove.push("SPROUT_ACP_AGENT_OWNER");
        }
    } else {
        remove.push("SPROUT_ACP_AGENT_OWNER");
    }

    Ok((set, remove))
}

/// Spawn an agent process without holding any locks on records or runtimes.
/// Returns the child process and log path on success. The caller is responsible
/// for updating `ManagedAgentRecord` fields and inserting into the runtimes map.
///
/// `owner_hex`: the workspace owner's pubkey, used as a fallback for legacy
/// records that have no NIP-OA `auth_tag`. See `build_respond_to_env`.
pub fn spawn_agent_child(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    owner_hex: Option<&str>,
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
    let resolved_acp_command = resolve_command(&record.acp_command)
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let resolved_mcp_command: Option<std::path::PathBuf> =
        if record.mcp_command.is_empty() {
            None
        } else {
            Some(resolve_command(&record.mcp_command).ok_or_else(|| {
                missing_command_message(&record.mcp_command, "MCP server command")
            })?)
        };
    // Resolve agent command to a full path (DMG launches have minimal PATH).
    let resolved_agent_command = resolve_command(&record.agent_command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| record.agent_command.clone());

    // Augment PATH for DMG launches so child processes can find:
    //   - sprout CLI via ~/.local/bin symlink
    //   - bundled sidecars (sprout, sprout-acp, etc.) via exe parent (Contents/MacOS/)
    //   - runtimes (node, python, etc.) via login shell PATH
    let augmented_path = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(home) = dirs::home_dir() {
            parts.push(home.join(".local").join("bin").display().to_string());
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                parts.push(parent.display().to_string());
            }
        }
        if let Some(shell_path) = login_shell_path() {
            parts.push(shell_path);
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(":"))
        }
    };

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
    match &resolved_mcp_command {
        Some(mcp_cmd) => {
            command.env("SPROUT_ACP_MCP_COMMAND", mcp_cmd);
        }
        None => {
            command.env("SPROUT_ACP_MCP_COMMAND", "");
        }
    }
    // Enable MCP hook tools (_Stop, _PostCompact) for agents that need them.
    let runtime_meta = known_acp_runtime(&record.agent_command);
    if runtime_meta.is_some_and(|r| r.mcp_hooks) {
        command.env("MCP_HOOK_SERVERS", "*");
    }
    // Only emit SPROUT_ACP_IDLE_TIMEOUT when the user has explicitly set an
    // override. When unset, the sprout-acp harness applies its own default
    // (see `DEFAULT_IDLE_TIMEOUT_SECS` in crates/sprout-acp/src/config.rs),
    // which is the single source of truth. The previously-emitted
    // `SPROUT_ACP_TURN_TIMEOUT` is deprecated upstream and was pinning every
    // agent to the desktop's stale default (320s), bypassing harness bumps.
    if let Some(idle) = record.idle_timeout_seconds {
        command.env("SPROUT_ACP_IDLE_TIMEOUT", idle.to_string());
    }

    let max_dur = record
        .max_turn_duration_seconds
        .unwrap_or(super::types::DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS);
    command.env("SPROUT_ACP_MAX_TURN_DURATION", max_dur.to_string());
    command.env("SPROUT_ACP_AGENTS", record.parallelism.to_string());
    command.env("SPROUT_ACP_MULTIPLE_EVENT_HANDLING", "owner-interrupt");
    command.env("SPROUT_ACP_DEDUP", "queue");
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                command.env(key, value);
            }
        }
    }
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
    if let Some(meta) = runtime_meta {
        if !meta.supports_acp_model_switching {
            if let (Some(env_key), Some(model)) = (meta.model_env_var, &effective_model) {
                command.env(env_key, model);
            }
        }
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

    // Inbound author gate: who is this agent allowed to respond to?
    // Validation is strict here — a malformed allowlist on disk fails before
    // we spawn anything (the harness would also reject it, but we'd rather
    // fail with a clear error than crash-loop the child).
    let (gate_set, gate_remove) = build_respond_to_env(record, owner_hex)?;
    for (key, value) in &gate_set {
        command.env(key, value);
    }
    for key in &gate_remove {
        command.env_remove(key);
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
    if let Some(cred_helper) = resolve_command("git-credential-nostr") {
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

    // ── User env vars: persona first, then per-agent (last wins) ────────
    //
    // Precedence: desktop parent env < persona env_vars < agent env_vars.
    // These writes go LAST so user-provided values win over every Sprout-set
    // env above — EXCEPT reserved keys (SPROUT_PRIVATE_KEY, NOSTR_PRIVATE_KEY,
    // SPROUT_AUTH_TAG, SPROUT_API_TOKEN, SPROUT_ACP_PRIVATE_KEY,
    // SPROUT_ACP_API_TOKEN), which `merged_user_env` strips. Those carry
    // Sprout's identity and must never be GUI-overridable.
    // Fail closed on persona-lookup errors: persona env_vars carry API
    // credentials, so silently substituting an empty map would spawn an
    // unauthenticated agent and surface as a confusing downstream auth error.
    let persona_env = super::env_vars::resolve_persona_env(app, record.persona_id.as_deref())?;
    for (key, value) in super::env_vars::merged_user_env(&persona_env, &record.env_vars) {
        command.env(key, value);
    }

    // Mark as Sprout-managed *and* which desktop instance owns us, so the
    // system-wide orphan sweep only reaps this instance's own agents and never
    // another live Sprout's (e.g. a `just dev` build won't kill a DMG build's
    // agents). Propagates automatically through the full tree (sprout-acp →
    // goose → MCP servers) because neither sprout-acp nor goose calls
    // env_clear().
    command.env("SPROUT_MANAGED_AGENT", current_instance_id(app));

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
    owner_hex: Option<&str>,
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

    let (child, log_path) = spawn_agent_child(app, record, owner_hex)?;

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
    use crate::managed_agents::known_acp_runtime;

    #[test]
    fn marker_entry_is_namespaced_by_instance_id() {
        // The spawn stamp and the sweep matcher must produce identical bytes;
        // both go through sprout_marker_entry, so this pins the on-the-wire
        // format and guards against a dev build (`...app.dev`) matching a
        // release build's (`...app`) agents.
        assert_eq!(
            super::sprout_marker_entry("xyz.block.sprout.app"),
            b"SPROUT_MANAGED_AGENT=xyz.block.sprout.app".to_vec()
        );
        assert_ne!(
            super::sprout_marker_entry("xyz.block.sprout.app"),
            super::sprout_marker_entry("xyz.block.sprout.app.dev")
        );
    }

    #[test]
    fn sprout_agent_has_mcp_hooks() {
        let p = known_acp_runtime("sprout-agent").expect("should resolve");
        assert!(p.mcp_hooks);
        assert_eq!(p.mcp_command, Some("sprout-dev-mcp"));
    }

    #[test]
    fn sprout_agent_resolved_via_path() {
        assert!(known_acp_runtime("/usr/local/bin/sprout-agent").is_some_and(|p| p.mcp_hooks));
    }

    #[test]
    fn goose_has_no_mcp_hooks() {
        let p = known_acp_runtime("goose").expect("should resolve");
        assert!(!p.mcp_hooks);
        assert_eq!(p.mcp_command, None);
    }

    #[test]
    fn unknown_command_returns_none() {
        assert!(known_acp_runtime("custom-agent").is_none());
    }

    // ── build_respond_to_env tests ───────────────────────────────────────

    use super::build_respond_to_env;
    use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

    /// Construct a minimal record fixture for env-building tests. Only the
    /// fields read by `build_respond_to_env` matter here.
    fn fixture(
        respond_to: RespondTo,
        allowlist: Vec<String>,
        auth_tag: Option<String>,
    ) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: "p".into(),
            name: "n".into(),
            persona_id: None,
            private_key_nsec: "nsec1fake".into(),
            auth_tag,
            relay_url: "ws://localhost:3000".into(),
            acp_command: "sprout-acp".into(),
            agent_command: "goose".into(),
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 320,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            mcp_toolsets: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            persona_pack_path: None,
            persona_name_in_pack: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to,
            respond_to_allowlist: allowlist,
        }
    }

    #[test]
    fn build_env_owner_only_sets_mode_and_removes_others() {
        let rec = fixture(RespondTo::OwnerOnly, vec![], Some("tag".into()));
        let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
        let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
        assert_eq!(
            set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
            Some("owner-only")
        );
        assert!(!set_map.contains_key("SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
        assert!(remove.contains(&"SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
        // auth_tag is present → no AGENT_OWNER fallback fires.
        assert!(remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
    }

    #[test]
    fn build_env_allowlist_sets_both_envs_and_joins() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let rec = fixture(
            RespondTo::Allowlist,
            vec![a.clone(), b.clone()],
            Some("tag".into()),
        );
        let (set, _remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
        let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
        assert_eq!(
            set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
            Some("allowlist")
        );
        assert_eq!(
            set_map
                .get("SPROUT_ACP_RESPOND_TO_ALLOWLIST")
                .map(String::as_str),
            Some(format!("{a},{b}").as_str()),
        );
    }

    #[test]
    fn build_env_anyone_omits_allowlist_var() {
        let rec = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
        let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
        let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
        assert_eq!(
            set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
            Some("anyone")
        );
        assert!(!set_map.contains_key("SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
        assert!(remove.contains(&"SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
    }

    #[test]
    fn build_env_legacy_record_without_auth_tag_emits_agent_owner() {
        let rec = fixture(RespondTo::OwnerOnly, vec![], None);
        let (set, remove) = build_respond_to_env(&rec, Some("ownerhex")).unwrap();
        let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
        assert_eq!(
            set_map.get("SPROUT_ACP_AGENT_OWNER").map(String::as_str),
            Some("ownerhex")
        );
        assert!(!remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
    }

    #[test]
    fn build_env_legacy_record_without_owner_hex_removes_agent_owner() {
        // No owner available to forward → make sure we don't inherit a leaked
        // env var from the parent.
        let rec = fixture(RespondTo::OwnerOnly, vec![], None);
        let (_set, remove) = build_respond_to_env(&rec, None).unwrap();
        assert!(remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
    }

    #[test]
    fn build_env_rejects_corrupted_allowlist() {
        let rec = fixture(
            RespondTo::Allowlist,
            vec!["not-hex".into()],
            Some("tag".into()),
        );
        assert!(build_respond_to_env(&rec, Some("owner")).is_err());
    }

    #[test]
    fn build_env_rejects_empty_allowlist_in_allowlist_mode() {
        let rec = fixture(RespondTo::Allowlist, vec![], Some("tag".into()));
        let err = build_respond_to_env(&rec, Some("owner")).unwrap_err();
        assert!(err.contains("at least one pubkey"));
    }
}
