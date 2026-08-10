use super::*;

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

/// Script interpreters that may host managed agent wrappers (e.g. npm shims).
/// A process whose name matches here is NOT immediately claimed — it must also
/// carry `BUZZ_MANAGED_AGENT` in its environment (checked by the caller via
/// `process_has_buzz_marker()`). This avoids sweeping unrelated node processes.
pub(crate) const KNOWN_SCRIPT_INTERPRETERS: &[&str] = &["node"];

/// Expand a process snapshot into descendant generations rooted at `root`.
/// Kept platform-independent so the Windows recovery teardown topology can be
/// unit-tested on every CI host.
#[cfg(any(windows, test))]
pub(crate) fn descendant_process_waves(entries: &[(u32, u32)], root: u32) -> Vec<Vec<u32>> {
    let mut known = std::collections::HashSet::from([root]);
    let mut waves = Vec::new();
    loop {
        let mut wave_seen = std::collections::HashSet::new();
        let wave = entries
            .iter()
            .filter_map(|(pid, parent)| {
                (*pid != root
                    && !known.contains(pid)
                    && known.contains(parent)
                    && wave_seen.insert(*pid))
                .then_some(*pid)
            })
            .collect::<Vec<_>>();
        if wave.is_empty() {
            break;
        }
        known.extend(wave.iter().copied());
        waves.push(wave);
    }
    waves
}

/// Stable identity for one Windows process instance. Windows may recycle a
/// numeric PID as soon as the prior process object is released, while the
/// creation time remains unique for the lifetime of that process instance.
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WindowsProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) creation_time: u64,
}

/// Identity observation made immediately before a destructive Windows action.
/// `Unverified` is deliberately distinct from `Exited`: callers must fail
/// closed when access is denied or metadata cannot be read.
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsIdentityObservation {
    Verified(WindowsProcessIdentity),
    Exited,
    Unverified,
}

/// Run `terminate` only when the currently opened Windows process object still
/// has the expected creation identity. This small platform-independent seam is
/// used by the native implementation and makes the no-kill-on-reuse rule
/// directly testable on non-Windows CI.
#[cfg(any(windows, test))]
pub(crate) fn terminate_if_windows_identity_matches(
    expected: WindowsProcessIdentity,
    observed: WindowsIdentityObservation,
    terminate: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    match observed {
        WindowsIdentityObservation::Verified(actual) if actual == expected => {
            terminate()?;
            Ok(true)
        }
        WindowsIdentityObservation::Exited => Ok(false),
        WindowsIdentityObservation::Verified(actual) => Err(format!(
            "refusing to terminate recycled Windows PID {}: expected creation {}, observed {}",
            expected.pid, expected.creation_time, actual.creation_time
        )),
        WindowsIdentityObservation::Unverified => Err(format!(
            "refusing to terminate Windows PID {} because process identity could not be verified",
            expected.pid
        )),
    }
}

/// One identity-bearing ToolHelp row. `identity: None` means the PID was
/// visible but its stable identity could not be queried; such a row can be
/// reported as a bounded cleanup failure but must never become a kill target.
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsProcessSnapshotEntry {
    pub(crate) pid: u32,
    pub(crate) parent_pid: u32,
    pub(crate) identity: Option<WindowsProcessIdentity>,
}

/// Find the next direct descendant generation whose parent is already bound
/// to a verified, open process object. Existing identities are never replaced
/// by a same-PID/different-creation-time row.
#[cfg(any(windows, test))]
pub(crate) fn next_verified_windows_descendants(
    entries: &[WindowsProcessSnapshotEntry],
    known: &std::collections::HashMap<u32, WindowsProcessIdentity>,
) -> (Vec<WindowsProcessIdentity>, Vec<u32>) {
    let mut verified = Vec::new();
    let mut unverified = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        if entry.pid == 0
            || known.contains_key(&entry.pid)
            || !known.contains_key(&entry.parent_pid)
            || !seen.insert(entry.pid)
        {
            continue;
        }
        match entry.identity {
            Some(identity) if identity.pid == entry.pid => verified.push(identity),
            _ => unverified.push(entry.pid),
        }
    }
    (verified, unverified)
}

/// Check if a process name matches any of our known agent binaries.
/// Uses exact match or prefix-with-separator to avoid false positives
/// (e.g. `"goose"` must not match `"mongoose"`).
pub(super) fn name_matches_known_binary(name: &str) -> bool {
    KNOWN_AGENT_BINARIES.iter().any(|&binary| {
        name == binary || {
            name.starts_with(binary) && {
                let rest = &name[binary.len()..];
                rest.starts_with('-') || rest.starts_with('_') || rest.starts_with('.')
            }
        }
    })
}

/// Check if a process name is a known script interpreter that may be hosting
/// a managed agent wrapper (e.g. `node` running an npm shim for `codex-acp`).
/// Callers must additionally verify `BUZZ_MANAGED_AGENT` ownership.
pub(super) fn name_matches_interpreter(name: &str) -> bool {
    KNOWN_SCRIPT_INTERPRETERS.contains(&name)
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
    // Fall through for script interpreters (e.g. `node` hosting an npm shim):
    // the caller's `process_has_buzz_marker()` check decides true ownership.
    name_matches_known_binary(&name) || name_matches_interpreter(&name)
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
        // Interpreter check: `node` is 4 bytes, never truncated.
        if name_matches_interpreter(name.trim()) {
            return true;
        }
    }

    // Fallback: read /proc/<pid>/exe which is a symlink to the full binary path.
    // This is not subject to the 15-byte truncation limit.
    if let Ok(exe_path) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        if let Some(basename) = exe_path.file_name().and_then(|n| n.to_str()) {
            // Fall through for script interpreters — caller checks the marker.
            return name_matches_known_binary(basename) || name_matches_interpreter(basename);
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
pub(super) fn buzz_marker_entry(instance_id: &str) -> Vec<u8> {
    format!("BUZZ_MANAGED_AGENT={instance_id}").into_bytes()
}

/// Check if a running process is one of *our* managed agents: it must carry
/// `BUZZ_MANAGED_AGENT=<instance_id>` in its environment, where `instance_id`
/// is this desktop instance's id. A process stamped with a *different* instance
/// id belongs to another live Buzz app and must never be reaped here.
#[cfg(target_os = "macos")]
pub(crate) fn process_has_buzz_marker(pid: u32, instance_id: &str) -> bool {
    let marker = buzz_marker_entry(instance_id);
    let Some(buf) = sweep::procargs2_buffer(pid) else {
        return false;
    };

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
pub(crate) fn process_has_buzz_marker(pid: u32, instance_id: &str) -> bool {
    let marker = buzz_marker_entry(instance_id);
    let Ok(data) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    data.split(|&b| b == 0).any(|entry| entry == marker)
}

#[cfg(not(unix))]
pub(crate) fn process_has_buzz_marker(_pid: u32, _instance_id: &str) -> bool {
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
    Err(format!(
        "refusing to terminate Windows process tree {pid} without a stable process identity"
    ))
}

#[cfg(windows)]
pub(crate) fn terminate_process_with_identity(
    pid: u32,
    process_identity: Option<u64>,
) -> Result<(), String> {
    super::super::process_lifecycle::terminate_process_tree(pid, process_identity)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn terminate_process(_pid: u32) -> Result<(), String> {
    Err("managed agent shutdown after app restart is not supported on this platform".to_string())
}

pub(crate) fn managed_process_identity(process: &super::super::ManagedAgentProcess) -> Option<u64> {
    #[cfg(windows)]
    {
        return Some(process.process_identity);
    }
    #[cfg(not(windows))]
    {
        let _ = process;
        None
    }
}

/// Terminate a process that is still represented by its stable `Child`
/// generation. Windows prefers the existing Job Object; if assignment failed,
/// native fallback remains bound to the creation identity captured at spawn.
pub(crate) fn terminate_managed_process(
    process: &mut super::super::ManagedAgentProcess,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        terminate_process(process.child.id())
    }
    #[cfg(windows)]
    {
        if let Some(job) = process.job.take() {
            job.terminate_and_wait(std::time::Duration::from_secs(1))
        } else {
            terminate_process_with_identity(process.child.id(), Some(process.process_identity))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        process
            .child
            .kill()
            .map_err(|error| format!("failed to kill managed process: {error}"))
    }
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

/// Resolve orphan candidate PIDs to their actual process group IDs, dedupe,
/// and signal the groups. An orphaned grandchild (e.g. `goose` or `buzz-dev-mcp`)
/// whose harness has exited retains the harness's PGID — signaling that PGID
/// kills the entire orphaned subtree. Falls back to the candidate PID itself
/// when PGID resolution fails (process may have exited between detection and
/// kill).
#[cfg(target_os = "macos")]
pub(super) fn resolve_pgids_and_kill(candidate_pids: &[i32]) {
    let candidate_set: std::collections::HashSet<i32> = candidate_pids.iter().copied().collect();
    let mut pgids = std::collections::HashSet::new();
    for &pid in candidate_pids {
        let pgid = unsafe { libc::getpgid(pid) };
        if pgid > 0 {
            pgids.insert(pgid);
        } else {
            // Process may have exited; try signaling it directly as a group.
            pgids.insert(pid);
        }
    }
    // PID-recycling guard: if a resolved PGID is alive but isn't one of our
    // orphan candidates, the old harness PID was recycled by a new process
    // that called setsid() — skip it to avoid killing an unrelated group.
    let candidate_groups = pgids.len();
    pgids.retain(|&pgid| {
        if candidate_set.contains(&pgid) {
            return true;
        }
        let alive = unsafe { libc::kill(pgid, 0) } == 0;
        !alive
    });
    if pgids.is_empty() && candidate_groups > 0 {
        eprintln!(
            "buzz-desktop: orphan sweep: skipped all {candidate_groups} candidate group(s) (live foreign group leader or candidate already exited); nothing signalled"
        );
    }
    let unique: Vec<i32> = pgids.into_iter().collect();
    sigterm_then_sigkill(&unique);
}

/// Resolve orphan candidate PIDs to their actual process group IDs, dedupe,
/// and signal the groups. Linux variant reads PGID from /proc/<pid>/stat.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn resolve_pgids_and_kill(candidate_pids: &[i32]) {
    let candidate_set: std::collections::HashSet<i32> = candidate_pids.iter().copied().collect();
    let mut pgids = std::collections::HashSet::new();
    for &pid in candidate_pids {
        if let Some((_, pgid)) = sweep::proc_stat_ppid_pgid_linux(pid as u32) {
            pgids.insert(pgid as i32);
        } else {
            // Process may have exited; try signaling it directly as a group.
            pgids.insert(pid);
        }
    }
    // PID-recycling guard: if a resolved PGID is alive but isn't one of our
    // orphan candidates, the old harness PID was recycled by a new process
    // that called setsid() — skip it to avoid killing an unrelated group.
    let candidate_groups = pgids.len();
    pgids.retain(|&pgid| {
        if candidate_set.contains(&pgid) {
            return true;
        }
        let alive = unsafe { libc::kill(pgid, 0) } == 0;
        !alive
    });
    if pgids.is_empty() && candidate_groups > 0 {
        eprintln!(
            "buzz-desktop: orphan sweep: skipped all {candidate_groups} candidate group(s) (live foreign group leader or candidate already exited); nothing signalled"
        );
    }
    let unique: Vec<i32> = pgids.into_iter().collect();
    sigterm_then_sigkill(&unique);
}

pub(crate) fn valid_agent_runtime_receipt(
    path: &std::path::Path,
    receipt: &super::super::ManagedAgentRuntimeReceipt,
    instance_id: &str,
) -> bool {
    #[cfg(windows)]
    {
        let Ok(canonical) =
            ManagedAgentRuntimeKey::new(receipt.key.pubkey.clone(), &receipt.key.relay_url)
        else {
            return false;
        };
        return canonical == receipt.key
            && path.file_name().and_then(|name| name.to_str())
                == Some(&format!("{}.json", receipt.key.runtime_id()))
            && receipt.desktop_instance_id == instance_id
            && receipt.process_identity.is_some_and(|identity| {
                super::super::process_lifecycle::process_identity_matches(receipt.pid, identity)
            });
    }
    #[cfg(not(windows))]
    valid_agent_runtime_receipt_with(
        path,
        receipt,
        instance_id,
        process_is_running,
        process_has_buzz_marker,
    )
}

/// Injectable version of `valid_agent_runtime_receipt` for testing.
/// `is_running(pid)` and `has_marker(pid, instance_id)` can be substituted by
/// test doubles without spawning real processes.
pub(crate) fn valid_agent_runtime_receipt_with(
    path: &std::path::Path,
    receipt: &super::super::ManagedAgentRuntimeReceipt,
    instance_id: &str,
    is_running: impl Fn(u32) -> bool,
    has_marker: impl Fn(u32, &str) -> bool,
) -> bool {
    let Ok(canonical) =
        ManagedAgentRuntimeKey::new(receipt.key.pubkey.clone(), &receipt.key.relay_url)
    else {
        return false;
    };
    canonical == receipt.key
        && path.file_name().and_then(|name| name.to_str())
            == Some(&format!("{}.json", receipt.key.runtime_id()))
        && receipt.desktop_instance_id == instance_id
        && is_running(receipt.pid)
        // Receipts are written by THIS instance at spawn time, so they are
        // Buzz-owned by construction. Marker-only ownership: custom-harness
        // binaries (not in KNOWN_AGENT_BINARIES) must not be rejected by a
        // name gate — see the sweep ownership rule in runtime/orphan_sweep.rs.
        && has_marker(receipt.pid, &receipt.desktop_instance_id)
}

pub(super) fn terminate_runtime_receipt_with(
    path: &std::path::Path,
    receipt: &super::super::ManagedAgentRuntimeReceipt,
    terminate: impl FnOnce(u32) -> Result<(), String>,
    mut is_running: impl FnMut(u32) -> bool,
    remove: impl FnOnce(&std::path::Path),
) -> Result<(), String> {
    terminate(receipt.pid)?;
    for _ in 0..20 {
        if !is_running(receipt.pid) {
            remove(path);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!(
        "prior runtime {} for pair {} on {} did not exit",
        receipt.pid, receipt.key.pubkey, receipt.key.relay_url
    ))
}

/// Replace a valid prior-session process before registering a new child for
/// the same pair. The caller must hold the runtime transition lock so receipt
/// inspection, termination, spawn, and registration cannot race shutdown or
/// another start.
pub(crate) fn terminate_untracked_pair_runtime(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    let instance_id = current_instance_id(app);
    let Some((path, receipt)) = super::super::read_all_agent_runtime_receipts(app)
        .into_iter()
        .find(|(path, receipt)| {
            receipt.key == *key && valid_agent_runtime_receipt(path, receipt, &instance_id)
        })
    else {
        return Ok(());
    };

    terminate_runtime_receipt_with(
        &path,
        &receipt,
        |pid| {
            #[cfg(windows)]
            {
                terminate_process_with_identity(pid, receipt.process_identity)
            }
            #[cfg(not(windows))]
            {
                terminate_process(pid)
            }
        },
        |pid| {
            #[cfg(windows)]
            {
                receipt.process_identity.is_some_and(|identity| {
                    super::super::process_lifecycle::process_identity_matches(pid, identity)
                })
            }
            #[cfg(not(windows))]
            {
                process_is_running(pid)
            }
        },
        super::super::remove_agent_runtime_receipt_path,
    )
}

#[cfg(test)]
mod windows_identity_tests {
    use super::{
        next_verified_windows_descendants, terminate_if_windows_identity_matches,
        WindowsIdentityObservation, WindowsProcessIdentity, WindowsProcessSnapshotEntry,
    };
    use std::{cell::Cell, collections::HashMap};

    fn identity(pid: u32, creation_time: u64) -> WindowsProcessIdentity {
        WindowsProcessIdentity { pid, creation_time }
    }

    #[test]
    fn root_pid_reuse_never_invokes_termination() {
        let calls = Cell::new(0);
        let result = terminate_if_windows_identity_matches(
            identity(10, 100),
            WindowsIdentityObservation::Verified(identity(10, 200)),
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn descendant_pid_reuse_is_not_rediscovered_or_terminated() {
        let root = identity(10, 100);
        let child = identity(11, 110);
        let mut known = HashMap::from([(root.pid, root)]);
        let first = [WindowsProcessSnapshotEntry {
            pid: child.pid,
            parent_pid: root.pid,
            identity: Some(child),
        }];
        let (discovered, unverified) = next_verified_windows_descendants(&first, &known);
        assert_eq!(discovered, vec![child]);
        assert!(unverified.is_empty());
        known.insert(child.pid, child);

        let replacement = identity(child.pid, 999);
        let second = [WindowsProcessSnapshotEntry {
            pid: replacement.pid,
            parent_pid: root.pid,
            identity: Some(replacement),
        }];
        let (discovered, unverified) = next_verified_windows_descendants(&second, &known);
        assert!(discovered.is_empty());
        assert!(unverified.is_empty());

        let calls = Cell::new(0);
        assert!(terminate_if_windows_identity_matches(
            child,
            WindowsIdentityObservation::Verified(replacement),
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
        )
        .is_err());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn descendants_created_during_teardown_are_discovered_by_generation() {
        let root = identity(10, 100);
        let child = identity(11, 110);
        let grandchild = identity(12, 120);
        let mut known = HashMap::from([(root.pid, root)]);
        let first = [WindowsProcessSnapshotEntry {
            pid: child.pid,
            parent_pid: root.pid,
            identity: Some(child),
        }];
        let (first_wave, _) = next_verified_windows_descendants(&first, &known);
        assert_eq!(first_wave, vec![child]);
        known.insert(child.pid, child);

        let later = [WindowsProcessSnapshotEntry {
            pid: grandchild.pid,
            parent_pid: child.pid,
            identity: Some(grandchild),
        }];
        let (second_wave, _) = next_verified_windows_descendants(&later, &known);
        assert_eq!(second_wave, vec![grandchild]);
    }

    #[test]
    fn natural_exit_race_is_harmless_and_idempotent() {
        let calls = Cell::new(0);
        for _ in 0..2 {
            assert_eq!(
                terminate_if_windows_identity_matches(
                    identity(10, 100),
                    WindowsIdentityObservation::Exited,
                    || {
                        calls.set(calls.get() + 1);
                        Ok(())
                    },
                ),
                Ok(false)
            );
        }
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn unverifiable_identity_never_invokes_termination() {
        let calls = Cell::new(0);
        let result = terminate_if_windows_identity_matches(
            identity(10, 100),
            WindowsIdentityObservation::Unverified,
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 0);

        let known = HashMap::from([(10, identity(10, 100))]);
        let entries = [WindowsProcessSnapshotEntry {
            pid: 11,
            parent_pid: 10,
            identity: None,
        }];
        let (verified, unverified) = next_verified_windows_descendants(&entries, &known);
        assert!(verified.is_empty());
        assert_eq!(unverified, vec![11]);
    }
}
