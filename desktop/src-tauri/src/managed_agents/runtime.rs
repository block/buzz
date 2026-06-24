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

mod path;
use path::build_augmented_path;

type RespondToEnv = (Vec<(&'static str, String)>, Vec<&'static str>);

/// Binary name fragments for all known agent/harness processes that Buzz
/// may spawn. Used by `process_belongs_to_us()` and the orphan sweep to
/// identify processes we should clean up. Both hyphenated and underscored
/// variants are listed because macOS `proc_name()` and Linux `/proc/comm`
/// may report either form depending on how the binary was built.
pub(crate) const KNOWN_AGENT_BINARIES: &[&str] = &[
    "buzz-acp",
    "buzz_acp",
    "buzz-agent",
    "buzz_agent",
    "claude-agent-acp",
    "claude_agent_acp",
    "claude-code-acp",
    "claude_code_acp",
    "codex-acp",
    "codex_acp",
    "goose",
    // buzz-dev-mcp's multicall personalities (rg, tree, buzz,
    // git-credential-nostr, git-sign-nostr) are short-lived per-tool-call
    // invocations — not listed here.
    "buzz-dev-mcp",
    "buzz_dev_mcp",
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

/// The value stamped into the `BUZZ_MANAGED_AGENT` env var of every agent we
/// spawn, identifying *which* desktop instance owns it. We use the app's bundle
/// identifier (`xyz.block.buzz.app` for release, `xyz.block.buzz.app.dev`
/// for `just dev`) because it is stable across restarts — a relaunched dev
/// instance still recognizes its own previously-spawned agents as reclaimable,
/// while never matching another instance's (e.g. a dev build never reaps a DMG
/// build's agents, and vice versa). This is what lets two Buzzs coexist on
/// one machine without one's cleanup nuking the other's agents.
pub(crate) fn current_instance_id(app: &AppHandle) -> String {
    app.config().identifier.clone()
}

/// Build the full `BUZZ_MANAGED_AGENT=<instance-id>` env entry we match
/// against when scanning processes. Kept here so the spawn stamp and the sweep
/// matcher can never drift apart.
fn buzz_marker_entry(instance_id: &str) -> Vec<u8> {
    format!("BUZZ_MANAGED_AGENT={instance_id}").into_bytes()
}

/// Check if a running process is one of *our* managed agents: it must carry
/// `BUZZ_MANAGED_AGENT=<instance_id>` in its environment, where `instance_id`
/// is this desktop instance's id. A process stamped with a *different* instance
/// id belongs to another live Buzz app and must never be reaped here.
#[cfg(target_os = "macos")]
fn process_has_buzz_marker(pid: u32, instance_id: &str) -> bool {
    let marker = buzz_marker_entry(instance_id);

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
fn process_has_buzz_marker(pid: u32, instance_id: &str) -> bool {
    let marker = buzz_marker_entry(instance_id);
    let Ok(data) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    data.split(|&b| b == 0).any(|entry| entry == marker)
}

#[cfg(not(unix))]
fn process_has_buzz_marker(_pid: u32, _instance_id: &str) -> bool {
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

#[cfg(windows)]
pub(crate) fn terminate_process(pid: u32) -> Result<(), String> {
    // No job handle is available on this path (e.g. after an app restart, when
    // we only recovered the PID from the record), so fall back to taskkill on
    // the whole tree.
    super::process_lifecycle::taskkill_tree(pid)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn terminate_process(_pid: u32) -> Result<(), String> {
    Err("managed agent shutdown after app restart is not supported on this platform".to_string())
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

// ── macOS process-info FFI (shared by all sweep/reap functions) ──────────

#[cfg(target_os = "macos")]
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

/// Subset of `struct proc_bsdinfo` from `<sys/proc_info.h>`. Layout verified
/// against the macOS SDK — total size 136 bytes.
#[cfg(target_os = "macos")]
#[repr(C)]
struct BSDInfo {
    _flags_status_xstatus: [u8; 12], // pbi_flags + pbi_status + pbi_xstatus
    pbi_pid: u32,                    // offset 12
    pbi_ppid: u32,                   // offset 16
    pbi_uid: u32,                    // offset 20
    _rest: [u8; 112],
}

#[cfg(target_os = "macos")]
const _: () = assert!(std::mem::size_of::<BSDInfo>() == 136);

#[cfg(target_os = "macos")]
const PROC_PIDTBSDINFO: libc::c_int = 3;

/// Enumerate all processes on the system owned by the current user and kill any
/// agent binary stamped with *this* instance's `BUZZ_MANAGED_AGENT` marker
/// (`instance_id`) that isn't in `skip_pids`. This catches orphans that escaped
/// PID-file-based cleanup (e.g. agent workers spawned with their own process
/// group whose parent harness already exited and had its PID file removed),
/// while leaving another live Buzz instance's agents untouched.
#[cfg(target_os = "macos")]
pub(crate) fn sweep_system_agent_processes(instance_id: &str, skip_pids: &[u32]) {
    let my_uid = unsafe { libc::getuid() };

    // Loop until the buffer is large enough to hold all PIDs. Under a fork
    // storm the process table can outgrow the initial estimate between the
    // probe and the fill call.
    let mut pids: Vec<libc::c_int>;
    loop {
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return;
        }
        let buf_len = (count as usize) * 2;
        pids = vec![0; buf_len];
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
        if (actual as usize) < buf_len {
            break;
        }
    }

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
        // Verify UID and PPID via proc_pidinfo.
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
        // Live child of a tracked harness — not an orphan.
        if skip_pids.contains(&info.pbi_ppid) {
            continue;
        }
        if !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        orphans.push(pid);
    }

    if !orphans.is_empty() {
        eprintln!(
            "buzz-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        sigterm_then_sigkill(&orphans);
    }
}

/// Read the parent PID of a process from /proc/<pid>/stat.
/// The comm field (field 2) may contain spaces and parens, so we find the last
/// ')' and parse fields after it. Field 1 after ')' is state, field 2 is PPID.
#[cfg(all(unix, not(target_os = "macos")))]
fn read_ppid_linux(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    // Fields after ')': " S ppid pgid ..."
    let ppid_str = after_comm.split_whitespace().nth(1)?;
    ppid_str.parse::<u32>().ok()
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
        if !process_belongs_to_us(upid) || !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        // Live child of a tracked harness — not an orphan. If /proc/<pid>/stat
        // is unreadable (process exiting, transient I/O error), we treat the
        // process as orphaned — safe because an exiting process will disappear
        // shortly, and the two-tick grace in the periodic path prevents acting
        // on transient failures.
        if let Some(ppid) = read_ppid_linux(upid) {
            if skip_pids.contains(&ppid) {
                continue;
            }
        }
        orphans.push(pid);
    }

    if !orphans.is_empty() {
        eprintln!(
            "buzz-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        sigterm_then_sigkill(&orphans);
    }
}

#[cfg(not(unix))]
pub(crate) fn sweep_system_agent_processes(_instance_id: &str, _skip_pids: &[u32]) {}

/// Periodic-sweep variant with two-tick grace: only reaps same-instance orphans
/// that were also seen orphaned on the previous tick. This prevents killing a
/// legitimately-starting agent that spawned between the skip-list snapshot and
/// the process scan. Returns the current orphan set for use as `prev_orphans`
/// on the next tick.
#[cfg(unix)]
pub(crate) fn sweep_system_agent_processes_with_grace(
    instance_id: &str,
    skip_pids: &[u32],
    prev_orphans: &std::collections::HashSet<u32>,
) -> std::collections::HashSet<u32> {
    let current = collect_same_instance_orphans(instance_id, skip_pids);
    // Only reap PIDs seen orphaned on two consecutive ticks.
    let confirmed: Vec<i32> = current
        .iter()
        .filter(|pid| prev_orphans.contains(pid))
        .map(|&pid| pid as i32)
        .collect();
    if !confirmed.is_empty() {
        eprintln!(
            "buzz-desktop: periodic sweep confirmed {} orphaned agent process(es), cleaning up",
            confirmed.len()
        );
        sigterm_then_sigkill(&confirmed);
    }
    current
}

#[cfg(not(unix))]
pub(crate) fn sweep_system_agent_processes_with_grace(
    _instance_id: &str,
    _skip_pids: &[u32],
    _prev_orphans: &std::collections::HashSet<u32>,
) -> std::collections::HashSet<u32> {
    std::collections::HashSet::new()
}

/// Collect PIDs of same-instance agent processes that appear orphaned (not in
/// `skip_pids`). Returns the set for use in two-tick grace logic — does NOT
/// kill anything.
#[cfg(target_os = "macos")]
pub(crate) fn collect_same_instance_orphans(
    instance_id: &str,
    skip_pids: &[u32],
) -> std::collections::HashSet<u32> {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;
    let mut orphans = std::collections::HashSet::new();

    let mut pids: Vec<libc::c_int>;
    loop {
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return orphans;
        }
        let buf_len = (count as usize) * 2;
        pids = vec![0; buf_len];
        let actual = unsafe {
            proc_listallpids(
                pids.as_mut_ptr(),
                (buf_len * std::mem::size_of::<libc::c_int>()) as libc::c_int,
            )
        };
        if actual <= 0 {
            return orphans;
        }
        pids.truncate(actual as usize);
        if (actual as usize) < buf_len {
            break;
        }
    }

    for &pid in &pids {
        if pid <= 0 || pid == my_pid {
            continue;
        }
        let upid = pid as u32;
        if skip_pids.contains(&upid) {
            continue;
        }
        if !process_belongs_to_us(upid) {
            continue;
        }
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
        // Live child of a tracked harness — not an orphan.
        if skip_pids.contains(&info.pbi_ppid) {
            continue;
        }
        if process_has_buzz_marker(upid, instance_id) {
            orphans.insert(upid);
        }
    }
    orphans
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn collect_same_instance_orphans(
    instance_id: &str,
    skip_pids: &[u32],
) -> std::collections::HashSet<u32> {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;
    let mut orphans = std::collections::HashSet::new();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return orphans;
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
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != my_uid {
            continue;
        }
        if !process_belongs_to_us(upid) || !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        // Live child of a tracked harness — not an orphan. If /proc/<pid>/stat
        // is unreadable (process exiting, transient I/O error), we treat the
        // process as orphaned — safe because an exiting process will disappear
        // shortly, and the two-tick grace prevents acting on transient failures.
        if let Some(ppid) = read_ppid_linux(upid) {
            if skip_pids.contains(&ppid) {
                continue;
            }
        }
        orphans.insert(upid);
    }
    orphans
}

#[cfg(not(unix))]
pub(crate) fn collect_same_instance_orphans(
    _instance_id: &str,
    _skip_pids: &[u32],
) -> std::collections::HashSet<u32> {
    std::collections::HashSet::new()
}

/// Binary names for the Buzz desktop/Tauri process. Used by dead-instance
/// detection to confirm the owning desktop is still alive.
const DESKTOP_BINARY_NAMES: &[&str] = &["Buzz", "buzz-desktop", "buzz_desktop"];

/// Check if a process name matches a known Buzz desktop binary.
fn is_desktop_binary(name: &str) -> bool {
    DESKTOP_BINARY_NAMES.contains(&name)
}

/// Check whether `buf` contains `id` as a complete identifier — not as a
/// prefix of a longer dotted name. The identifier appears in the Tauri config
/// JSON as `"identifier":"xyz.block.buzz.app.dev"` and in environment entries
/// as `KEY=...app.dev\0`, so a valid match is followed by a non-identifier byte
/// (not `[A-Za-z0-9._-]`) or sits at the end of the buffer. This prevents
/// `xyz.block.buzz.app` from matching inside `xyz.block.buzz.app.dev`.
fn buffer_contains_identifier(buf: &[u8], id: &[u8]) -> bool {
    if id.is_empty() {
        return false;
    }
    buf.windows(id.len()).enumerate().any(|(i, w)| {
        if w != id {
            return false;
        }
        // Boundary check on the byte immediately after the match: end-of-buffer
        // or any byte that can't continue a dotted reverse-DNS identifier.
        match buf.get(i + id.len()) {
            None => true,
            Some(&next) => {
                !next.is_ascii_alphanumeric() && next != b'.' && next != b'_' && next != b'-'
            }
        }
    })
}

/// Extract the `BUZZ_MANAGED_AGENT` value from a process's environment.
/// Returns `None` if the process doesn't have the marker or can't be read.
#[cfg(target_os = "macos")]
fn extract_buzz_marker_value(pid: u32) -> Option<String> {
    let prefix = b"BUZZ_MANAGED_AGENT=";

    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut buf_size: libc::size_t = 0;

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
        return None;
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
        return None;
    }
    buf.truncate(buf_size);

    if buf.len() < std::mem::size_of::<libc::c_int>() {
        return None;
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

    // Skip exec path.
    while pos < buf.len() && buf[pos] != 0 {
        pos += 1;
    }
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
    // Search environment entries for our marker.
    for entry in buf[pos..].split(|&b| b == 0) {
        if entry.starts_with(prefix) {
            return String::from_utf8(entry[prefix.len()..].to_vec()).ok();
        }
    }
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
fn extract_buzz_marker_value(pid: u32) -> Option<String> {
    let prefix = b"BUZZ_MANAGED_AGENT=";
    let data = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    for entry in data.split(|&b| b == 0) {
        if entry.starts_with(prefix) {
            return String::from_utf8(entry[prefix.len()..].to_vec()).ok();
        }
    }
    None
}

#[cfg(not(unix))]
fn extract_buzz_marker_value(_pid: u32) -> Option<String> {
    None
}

/// Check if a Buzz desktop process is still alive for the given instance ID.
/// Scans all user-owned processes named "Buzz" or "buzz-desktop" and checks
/// whether any has the identifier in its command-line args (KERN_PROCARGS2 buffer
/// includes both argv and environ — the `--config` JSON from `tauri dev` contains
/// the identifier string).
#[cfg(target_os = "macos")]
fn desktop_is_alive_for_instance(instance_id: &str) -> bool {
    extern "C" {
        fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
    }

    let my_uid = unsafe { libc::getuid() };
    let identifier_bytes = instance_id.as_bytes();

    let mut pids: Vec<libc::c_int>;
    loop {
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return false;
        }
        let buf_len = (count as usize) * 2;
        pids = vec![0; buf_len];
        let actual = unsafe {
            proc_listallpids(
                pids.as_mut_ptr(),
                (buf_len * std::mem::size_of::<libc::c_int>()) as libc::c_int,
            )
        };
        if actual <= 0 {
            return false;
        }
        pids.truncate(actual as usize);
        if (actual as usize) < buf_len {
            break;
        }
    }

    for &pid in &pids {
        if pid <= 0 {
            continue;
        }
        // Check binary name — only look at desktop binaries.
        let mut name_buf = [0u8; 1024];
        let len = unsafe {
            proc_name(
                pid,
                name_buf.as_mut_ptr() as *mut libc::c_void,
                name_buf.len() as u32,
            )
        };
        if len <= 0 {
            continue;
        }
        let name = String::from_utf8_lossy(&name_buf[..len as usize]);
        if !is_desktop_binary(&name) {
            continue;
        }
        // Verify UID.
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
        // Check if this desktop process's args/env contain the identifier.
        // The KERN_PROCARGS2 buffer holds argv + environ as null-delimited strings.
        let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut buf_size: libc::size_t = 0;
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
            continue;
        }
        let mut args_buf: Vec<u8> = vec![0; buf_size];
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                args_buf.as_mut_ptr() as *mut libc::c_void,
                &mut buf_size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            continue;
        }
        args_buf.truncate(buf_size);
        // Boundary-anchored search: the identifier in the config JSON is
        // followed by a non-identifier char (typically `"`). A raw substring
        // match would let `...app` match inside `...app.dev`.
        if buffer_contains_identifier(&args_buf, identifier_bytes) {
            return true;
        }
    }
    false
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_is_alive_for_instance(instance_id: &str) -> bool {
    let my_uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };
        // Check ownership.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != my_uid {
            continue;
        }
        // Check binary name via /proc/<pid>/comm.
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
            continue;
        };
        if !is_desktop_binary(comm.trim()) {
            continue;
        }
        // Check cmdline for the identifier with boundary anchoring.
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        if buffer_contains_identifier(&cmdline, instance_id.as_bytes()) {
            return true;
        }
    }
    false
}

#[cfg(not(unix))]
fn desktop_is_alive_for_instance(_instance_id: &str) -> bool {
    false
}

/// Reap agent processes belonging to dead Buzz desktop instances.
///
/// Scans all user processes for `BUZZ_MANAGED_AGENT=*`, groups them by
/// instance ID, and for each foreign instance (≠ `our_instance_id`) checks
/// whether a Buzz desktop binary is still alive for that instance. If not,
/// all agents from that dead instance are reaped.
#[cfg(target_os = "macos")]
pub(crate) fn reap_dead_instance_agents(our_instance_id: &str, skip_pids: &[u32]) {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;

    let mut pids: Vec<libc::c_int>;
    loop {
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return;
        }
        let buf_len = (count as usize) * 2;
        pids = vec![0; buf_len];
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
        if (actual as usize) < buf_len {
            break;
        }
    }

    // Collect (pid, instance_id) for all foreign agent processes.
    let mut foreign_agents: HashMap<String, Vec<i32>> = HashMap::new();

    for &pid in &pids {
        if pid <= 0 || pid == my_pid {
            continue;
        }
        let upid = pid as u32;
        if skip_pids.contains(&upid) {
            continue;
        }
        if !process_belongs_to_us(upid) {
            continue;
        }
        // Verify UID.
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
        // Extract the instance ID from this agent's env.
        let Some(agent_instance_id) = extract_buzz_marker_value(upid) else {
            continue;
        };
        // Skip agents belonging to our own instance (handled by sweep_system_agent_processes).
        if agent_instance_id == our_instance_id {
            continue;
        }
        foreign_agents
            .entry(agent_instance_id)
            .or_default()
            .push(pid);
    }

    // For each foreign instance, check if its desktop is still alive.
    for (instance_id, agent_pids) in &foreign_agents {
        if desktop_is_alive_for_instance(instance_id) {
            continue;
        }
        eprintln!(
            "buzz-desktop: reaping {} orphaned agent(s) from dead instance '{instance_id}'",
            agent_pids.len()
        );
        sigterm_then_sigkill(agent_pids);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn reap_dead_instance_agents(our_instance_id: &str, skip_pids: &[u32]) {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;
    let mut foreign_agents: HashMap<String, Vec<i32>> = HashMap::new();

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
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != my_uid {
            continue;
        }
        if !process_belongs_to_us(upid) {
            continue;
        }
        let Some(agent_instance_id) = extract_buzz_marker_value(upid) else {
            continue;
        };
        if agent_instance_id == our_instance_id {
            continue;
        }
        foreign_agents
            .entry(agent_instance_id)
            .or_default()
            .push(pid);
    }

    for (instance_id, agent_pids) in &foreign_agents {
        if desktop_is_alive_for_instance(instance_id) {
            continue;
        }
        eprintln!(
            "buzz-desktop: reaping {} orphaned agent(s) from dead instance '{instance_id}'",
            agent_pids.len()
        );
        sigterm_then_sigkill(agent_pids);
    }
}

#[cfg(not(unix))]
pub(crate) fn reap_dead_instance_agents(_our_instance_id: &str, _skip_pids: &[u32]) {}

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
    personas: &[crate::managed_agents::types::PersonaRecord],
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

    // Resolve the effective model and system_prompt from the linked persona
    // (mirrors spawn-time logic) so the UI displays the current persona values,
    // not the stale record snapshot.
    let (effective_prompt, effective_model, _effective_provider) =
        resolve_effective_prompt_model_provider(
            record.persona_id.as_deref(),
            personas,
            record.system_prompt.clone(),
            record.model.clone(),
        );

    // Resolve the effective harness the same way, then derive args/mcp from it,
    // so the UI reflects the persona's current harness (or an explicit pin).
    let effective_command = crate::managed_agents::effective_agent_command(
        record.persona_id.as_deref(),
        personas,
        record.agent_command_override.as_deref(),
    );
    let effective_args = normalize_agent_args(&effective_command, record.agent_args.clone());
    let effective_mcp_command = known_acp_runtime(&effective_command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("")
        .to_string();

    Ok(ManagedAgentSummary {
        pubkey: record.pubkey.clone(),
        name: record.name.clone(),
        persona_id: record.persona_id.clone(),
        relay_url: record.relay_url.clone(),
        acp_command: record.acp_command.clone(),
        agent_command: effective_command,
        agent_command_override: record.agent_command_override.clone(),
        agent_args: effective_args,
        mcp_command: effective_mcp_command,
        turn_timeout_seconds: record.turn_timeout_seconds,
        idle_timeout_seconds: record.idle_timeout_seconds,
        max_turn_duration_seconds: record.max_turn_duration_seconds,
        parallelism: record.parallelism,
        system_prompt: effective_prompt,
        model: effective_model,
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
        "BUZZ_ACP_RESPOND_TO",
        record.respond_to.as_str().to_string(),
    ));

    if record.respond_to == super::types::RespondTo::Allowlist {
        set.push(("BUZZ_ACP_RESPOND_TO_ALLOWLIST", normalized.join(",")));
    } else {
        remove.push("BUZZ_ACP_RESPOND_TO_ALLOWLIST");
    }

    // Legacy fallback: agents created before NIP-OA lack `auth_tag`. Without
    // it the harness can't resolve the owner, and owner-dependent gate modes
    // would drop every event. Forwarding the workspace owner pubkey via
    // BUZZ_ACP_AGENT_OWNER keeps those records functional. Modern records
    // (`auth_tag = Some(...)`) use `BUZZ_AUTH_TAG` as before.
    if record.auth_tag.is_none() {
        if let Some(owner) = owner_hex {
            set.push(("BUZZ_ACP_AGENT_OWNER", owner.to_string()));
        } else {
            remove.push("BUZZ_ACP_AGENT_OWNER");
        }
    } else {
        remove.push("BUZZ_ACP_AGENT_OWNER");
    }

    Ok((set, remove))
}

/// Resolve the effective system prompt, model, and provider for a spawn. The
/// linked persona always wins so persona edits propagate on the next spawn; the
/// record snapshot is the fallback only when no persona is linked or it was
/// deleted. Provider comes from the persona (the record has no provider field).
pub(crate) fn resolve_effective_prompt_model_provider(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::PersonaRecord],
    record_prompt: Option<String>,
    record_model: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    match persona_id.and_then(|pid| personas.iter().find(|p| p.id == pid)) {
        Some(p) => (
            Some(p.system_prompt.clone()),
            p.model.clone(),
            p.provider.clone(),
        ),
        None => (record_prompt, record_model, None),
    }
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
) -> Result<crate::managed_agents::ManagedAgentProcess, String> {
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
    // Resolve the effective harness (agent command) from the linked persona, so
    // persona harness edits propagate on the next spawn; an explicit per-agent
    // override wins. `agent_args` and `mcp_command` are pure derivations of the
    // command, so we recompute them from the effective value rather than the
    // frozen record snapshot. Mirrors the model resolution below.
    let personas = super::load_personas(app).unwrap_or_default();
    let effective_command = super::effective_agent_command(
        record.persona_id.as_deref(),
        &personas,
        record.agent_command_override.as_deref(),
    );
    let agent_args = normalize_agent_args(&effective_command, record.agent_args.clone());
    let resolved_acp_command = resolve_command(&record.acp_command)
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let effective_mcp_command = known_acp_runtime(&effective_command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("");
    let resolved_mcp_command: Option<std::path::PathBuf> = if effective_mcp_command.is_empty() {
        None
    } else {
        match resolve_command(effective_mcp_command) {
            Some(path) => Some(path),
            None => {
                eprintln!(
                    "buzz-desktop: mcp_command {effective_mcp_command:?} not found, skipping"
                );
                None
            }
        }
    };
    // Resolve agent command to a full path (DMG launches have minimal PATH).
    let resolved_agent_command = resolve_command(&effective_command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| effective_command.clone());

    // The agent's effective relay drives both the child's relay connection
    // (BUZZ_RELAY_URL) and git credential-helper URL: an explicit per-agent
    // relay wins; an empty one falls back to the active workspace relay.
    let effective_relay_url = {
        use tauri::Manager;
        let state = app.state::<crate::app_state::AppState>();
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &crate::relay::relay_ws_url_with_override(&state),
        )
    };

    // Augment PATH for DMG launches so child processes can find:
    //   - bundled CLI via ~/.local/bin symlink
    //   - bundled sidecars (buzz, buzz-acp, etc.) via exe parent (Contents/MacOS/)
    //   - runtimes (node, python, etc.) via login shell PATH
    let augmented_path = build_augmented_path(
        dirs::home_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        login_shell_path(),
    );

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
    command.env("BUZZ_PRIVATE_KEY", &record.private_key_nsec);
    command.env("BUZZ_RELAY_URL", &effective_relay_url);
    command.env("BUZZ_ACP_AGENT_COMMAND", &resolved_agent_command);
    command.env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
    match &resolved_mcp_command {
        Some(mcp_cmd) => {
            command.env("BUZZ_ACP_MCP_COMMAND", mcp_cmd);
        }
        None => {
            command.env("BUZZ_ACP_MCP_COMMAND", "");
        }
    }
    // Enable MCP hook tools (_Stop, _PostCompact) for agents that need them.
    // Uses "*" because build_mcp_servers() hard-codes the server name to "buzz-mcp".
    let runtime_meta = known_acp_runtime(&effective_command);
    if runtime_meta.is_some_and(|r| r.mcp_hooks) {
        command.env("MCP_HOOK_SERVERS", "*");
    }
    // Only emit BUZZ_ACP_IDLE_TIMEOUT when the user has explicitly set an
    // override. When unset, the buzz-acp harness applies its own default
    // (see `DEFAULT_IDLE_TIMEOUT_SECS` in crates/buzz-acp/src/config.rs),
    // which is the single source of truth. The previously-emitted
    // `BUZZ_ACP_TURN_TIMEOUT` is deprecated upstream and was pinning every
    // agent to the desktop's stale default (320s), bypassing harness bumps.
    if let Some(idle) = record.idle_timeout_seconds {
        command.env("BUZZ_ACP_IDLE_TIMEOUT", idle.to_string());
    }

    let max_dur = record
        .max_turn_duration_seconds
        .unwrap_or(super::types::DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS);
    command.env("BUZZ_ACP_MAX_TURN_DURATION", max_dur.to_string());
    command.env("BUZZ_ACP_AGENTS", record.parallelism.to_string());
    command.env("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "owner-interrupt");
    command.env("BUZZ_ACP_DEDUP", "queue");
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                command.env(key, value);
            }
        }
    }
    if let (Some(team_dir), Some(persona_name)) =
        (&record.persona_team_dir, &record.persona_name_in_team)
    {
        command.env("BUZZ_ACP_PERSONA_PACK", team_dir);
        command.env("BUZZ_ACP_PERSONA_NAME", persona_name);
    }

    // Resolve system prompt, model, and provider: the linked persona is the
    // source of truth, so persona edits reach the agent on the next spawn. Fall
    // back to the record snapshot only when no persona is linked or it was
    // deleted. Provider flows from the persona (the record has no provider).
    // `personas` was loaded above for the harness resolution.
    let (effective_prompt, effective_model, effective_provider) =
        resolve_effective_prompt_model_provider(
            record.persona_id.as_deref(),
            &personas,
            record.system_prompt.clone(),
            record.model.clone(),
        );

    if let Some(prompt) = &effective_prompt {
        command.env("BUZZ_ACP_SYSTEM_PROMPT", prompt);
    } else {
        command.env_remove("BUZZ_ACP_SYSTEM_PROMPT");
    }
    if let Some(model) = &effective_model {
        command.env("BUZZ_ACP_MODEL", model);
    } else {
        command.env_remove("BUZZ_ACP_MODEL");
    }
    if let Some(meta) = runtime_meta {
        for (key, value) in runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            effective_model.as_deref(),
            effective_provider.as_deref(),
        ) {
            command.env(key, value);
        }
    }
    if let Some(toolsets) = &record.mcp_toolsets {
        command.env("BUZZ_TOOLSETS", toolsets);
    } else {
        command.env("BUZZ_TOOLSETS", "default,canvas,forums,dms,media");
    }
    command.env_remove("BUZZ_ACP_PRIVATE_KEY");
    command.env_remove("BUZZ_ACP_API_TOKEN");
    command.env_remove("BUZZ_API_TOKEN");

    if let Some(ref auth_tag) = record.auth_tag {
        command.env("BUZZ_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("BUZZ_AUTH_TAG");
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

    command.env("BUZZ_ACP_RELAY_OBSERVER", "true");

    // ── Git credential helper for Buzz relay ──────────────────────────
    //
    // Agents need to clone/push repos hosted on the Buzz relay's git
    // server, which authenticates via NIP-98. The `git-credential-nostr`
    // binary signs auth events using the agent's nostr key.
    //
    // We configure git via GIT_CONFIG_COUNT env vars (ephemeral, no
    // filesystem writes) scoped to the relay's git URL so we don't
    // interfere with other remotes (e.g. GitHub).
    //
    // NOSTR_PRIVATE_KEY mirrors BUZZ_PRIVATE_KEY — keep in sync.
    if let Some(cred_helper) = resolve_command("git-credential-nostr") {
        let relay_http_url = crate::relay::relay_http_base_url(&effective_relay_url);

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
            "buzz-desktop: git-credential-nostr not found — agent {} will not have automatic Buzz git auth",
            record.name,
        );
    }

    // Baked-in Databricks defaults for internal builds (buzz-releases sets
    // BUZZ_BUILD_DATABRICKS_* at compile time; OSS builds bake nothing).
    // Written BEFORE user env_vars so a GUI/persona override still wins.
    for (key, value) in build_databricks_defaults() {
        command.env(key, value);
    }

    // ── User env vars: persona first, then per-agent (last wins) ────────
    //
    // Precedence: desktop parent env < persona env_vars < agent env_vars.
    // These writes go LAST so user-provided values win over every Buzz-set
    // env above — EXCEPT reserved keys (BUZZ_PRIVATE_KEY, NOSTR_PRIVATE_KEY,
    // BUZZ_AUTH_TAG, BUZZ_API_TOKEN, BUZZ_ACP_PRIVATE_KEY,
    // BUZZ_ACP_API_TOKEN), which `merged_user_env` strips. Those carry
    // Buzz's identity and must never be GUI-overridable.
    // Fail closed on persona-lookup errors: persona env_vars carry API
    // credentials, so silently substituting an empty map would spawn an
    // unauthenticated agent and surface as a confusing downstream auth error.
    let persona_env = super::env_vars::resolve_persona_env(app, record.persona_id.as_deref())?;
    for (key, value) in super::env_vars::merged_user_env(&persona_env, &record.env_vars) {
        command.env(key, value);
    }

    // Mark as Buzz-managed *and* which desktop instance owns us, so the
    // system-wide orphan sweep only reaps this instance's own agents and never
    // another live Buzz's (e.g. a `just dev` build won't kill a DMG build's
    // agents). Propagates automatically through the full tree (buzz-acp →
    // goose → MCP servers) because neither buzz-acp nor goose calls
    // env_clear().
    command.env("BUZZ_MANAGED_AGENT", current_instance_id(app));

    // Spawn the harness in its own process group so we can kill the entire
    // tree (harness + MCP servers + agent subprocesses) on shutdown.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // Windows: suppress the harness console window. Without this a bare
    // terminal pops for buzz-acp.exe and lingers (the app itself sets
    // windows_subsystem="windows", but the spawned child does not inherit it).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command.spawn().map_err(|error| {
        format!(
            "failed to spawn `{}` for agent {}: {error}",
            resolved_acp_command.display(),
            record.name
        )
    })?;

    let _ = super::write_agent_pid_file(app, &record.pubkey, child.id());

    // Windows: assign the harness to a Job Object so its whole tree dies with
    // the handle. The Unix process-group equivalent is set above.
    #[cfg(windows)]
    return Ok(super::process_lifecycle::finish_spawn(
        child,
        log_path,
        &record.name,
    ));
    #[cfg(not(windows))]
    Ok(crate::managed_agents::ManagedAgentProcess { child, log_path })
}

fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

/// Databricks host/model baked in at compile time for internal builds. Empty
/// in OSS builds, where the `BUZZ_BUILD_DATABRICKS_*` env is unset.
fn build_databricks_defaults() -> Vec<(&'static str, &'static str)> {
    let mut defaults = Vec::new();
    if let Some(host) = option_env!("BUZZ_DESKTOP_BUILD_DATABRICKS_HOST") {
        if !host.is_empty() {
            defaults.push(("DATABRICKS_HOST", host));
        }
    }
    if let Some(model) = option_env!("BUZZ_DESKTOP_BUILD_DATABRICKS_MODEL") {
        if !model.is_empty() {
            defaults.push(("DATABRICKS_MODEL", model));
        }
    }
    defaults
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

    let process = spawn_agent_child(app, record, owner_hex)?;

    let now = now_iso();
    record.updated_at = now.clone();
    record.runtime_pid = Some(process.child.id());
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;

    runtimes.insert(record.pubkey.clone(), process);
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
    // On Windows, drop the Job Object handle (KILL_ON_JOB_CLOSE) so the whole
    // harness tree dies — Child::kill() would orphan the agent workers + MCP
    // servers. If job assignment failed at spawn, fall back to Child::kill().
    #[cfg(unix)]
    terminate_process(runtime.child.id())?;
    #[cfg(windows)]
    match runtime.job.take() {
        Some(job) => drop(job),
        None => runtime
            .child
            .kill()
            .map_err(|error| format!("failed to kill agent process: {error}"))?,
    }
    #[cfg(not(any(unix, windows)))]
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

/// Returns the (key, value) env var pairs that should be forwarded to the
/// agent process for model and provider selection.
///
/// Model injection is unconditional — even agents that support ACP model
/// switching need the initial bootstrap value. Provider injection is skipped
/// when `provider_locked` is true (e.g. Claude runtimes that only work with
/// Anthropic).
fn runtime_metadata_env_vars<'a>(
    model_env_var: Option<&'a str>,
    provider_env_var: Option<&'a str>,
    provider_locked: bool,
    effective_model: Option<&'a str>,
    effective_provider: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut vars = Vec::new();
    if let (Some(env_key), Some(model)) = (model_env_var, effective_model) {
        vars.push((env_key, model));
    }
    if !provider_locked {
        if let (Some(env_key), Some(provider)) = (provider_env_var, effective_provider) {
            vars.push((env_key, provider));
        }
    }
    vars
}

#[cfg(test)]
mod tests;
