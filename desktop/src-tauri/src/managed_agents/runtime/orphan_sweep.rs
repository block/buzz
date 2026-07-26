use super::*;

/// Kill orphaned agent processes using PID file receipts. Reads all files from
/// `agent-pids/`, verifies each PID still belongs to a known agent binary,
/// then resolves each candidate's actual PGID and signals the process group.
/// Deletes the PID file after killing.
///
/// `skip_pids` are PIDs already handled by the tracked-agent path.
#[cfg(unix)]
pub(crate) fn sweep_orphaned_agent_processes(app: &AppHandle, skip_pids: &[u32]) {
    let legacy_entries = super::super::read_all_agent_pid_files(app);
    let instance_id = current_instance_id(app);
    let receipt_entries: Vec<_> = super::super::read_all_agent_runtime_receipts(app)
        .into_iter()
        .filter_map(|(path, receipt)| {
            if valid_agent_runtime_receipt(&path, &receipt, &instance_id) {
                Some((path, receipt))
            } else {
                super::super::remove_agent_runtime_receipt_path(&path);
                None
            }
        })
        .collect();
    // Collect live orphans AND dead-leader groups into a single kill batch.
    // Dead leaders: PGID may have been recycled, but the window is narrow
    // (PID files are from this session) and the cost of missing surviving
    // group members outweighs the recycling risk.
    let targets: Vec<i32> = legacy_entries
        .iter()
        .map(|(_, pid)| *pid)
        .chain(receipt_entries.iter().map(|(_, receipt)| receipt.pid))
        .filter(|pid| {
            if skip_pids.contains(pid) {
                return false;
            }
            (process_is_running(*pid) && process_belongs_to_us(*pid)) || !process_is_running(*pid)
        })
        .map(|pid| pid as i32)
        .collect();

    if !targets.is_empty() {
        resolve_pgids_and_kill(&targets);
    }

    // Clean up PID files for processes we just killed or that are already gone.
    for (pubkey, pid) in &legacy_entries {
        if skip_pids.contains(pid) {
            continue;
        }
        if !process_is_running(*pid) || !process_belongs_to_us(*pid) {
            super::super::remove_agent_pid_file(app, pubkey);
        }
    }
    for (_, receipt) in &receipt_entries {
        if skip_pids.contains(&receipt.pid) {
            continue;
        }
        if !process_is_running(receipt.pid) || !process_belongs_to_us(receipt.pid) {
            super::super::remove_agent_runtime_receipt(app, &receipt.key);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn sweep_orphaned_agent_processes(app: &AppHandle, _skip_pids: &[u32]) {
    let _ = app;
}

// ── macOS process-info FFI (shared by all sweep/reap functions) ──────────
//
// `proc_listallpids` lives in `sweep.rs` (which owns `collect_all_pids`).
// All callers in this file reach it through `sweep::collect_all_pids()`.
// `proc_pidinfo` and `BSDInfo` are declared here as `pub(super)` so that
// `sweep.rs` can call `super::proc_pidinfo` / use `super::BSDInfo` without
// redefining the struct layout in two places.

#[cfg(target_os = "macos")]
extern "C" {
    pub(super) fn proc_pidinfo(
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
pub(super) struct BSDInfo {
    _flags_status_xstatus: [u8; 12], // pbi_flags + pbi_status + pbi_xstatus
    pub(super) pbi_pid: u32,         // offset 12
    pub(super) pbi_ppid: u32,        // offset 16
    pub(super) pbi_uid: u32,         // offset 20
    _rest: [u8; 112],
}

#[cfg(target_os = "macos")]
const _: () = assert!(std::mem::size_of::<BSDInfo>() == 136);

#[cfg(target_os = "macos")]
pub(super) const PROC_PIDTBSDINFO: libc::c_int = 3;

/// Enumerate all processes on the system owned by the current user and kill any
/// agent binary stamped with *this* instance's `BUZZ_MANAGED_AGENT` marker
/// (`instance_id`) that isn't in `skip_pids`. This catches orphans that escaped
/// PID-file-based cleanup (e.g. agent workers spawned with their own process
/// group whose parent harness already exited and had its PID file removed),
/// while leaving another live Buzz instance's agents untouched.
#[cfg(target_os = "macos")]
pub(crate) fn sweep_system_agent_processes(instance_id: &str, skip_pids: &[u32]) {
    let my_uid = unsafe { libc::getuid() };
    let pids = sweep::collect_all_pids();
    if pids.is_empty() {
        return;
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
        if !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        // Live descendants of a tracked harness are exempt — see sweep::is_live_descendant_*.
        if sweep::is_live_descendant_macos(upid, info.pbi_ppid, skip_pids) {
            continue;
        }
        orphans.push(pid);
    }

    if !orphans.is_empty() {
        eprintln!(
            "buzz-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        resolve_pgids_and_kill(&orphans);
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
        if !process_belongs_to_us(upid) || !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        // Live descendants of a tracked harness are exempt — see sweep::is_live_descendant_*.
        if sweep::is_live_descendant_linux(upid, skip_pids) {
            continue;
        }
        orphans.push(pid);
    }

    if !orphans.is_empty() {
        eprintln!(
            "buzz-desktop: system sweep found {} orphaned agent process(es), cleaning up",
            orphans.len()
        );
        resolve_pgids_and_kill(&orphans);
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
        resolve_pgids_and_kill(&confirmed);
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

    let pids = sweep::collect_all_pids();
    if pids.is_empty() {
        return orphans;
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
        if !process_has_buzz_marker(upid, instance_id) {
            continue;
        }
        // Live descendants of a tracked harness are exempt — see sweep::is_live_descendant_*.
        if sweep::is_live_descendant_macos(upid, info.pbi_ppid, skip_pids) {
            continue;
        }
        orphans.insert(upid);
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
        // Live descendants of a tracked harness are exempt — see sweep::is_live_descendant_*.
        if sweep::is_live_descendant_linux(upid, skip_pids) {
            continue;
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
