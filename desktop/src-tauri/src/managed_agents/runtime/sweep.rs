//! Boot-time sweep for untracked same-bundle harness processes, plus low-level
//! process-tree helpers shared with the periodic orphan sweeps in `runtime.rs`.
//!
//! The env-var and PID-file sweeps cannot see a harness whose receipt is gone
//! or that predates `BUZZ_MANAGED_AGENT` injection. This sweep derives the
//! expected `buzz-acp` path from the running executable and kills any process
//! whose exe matches exactly, minus the tracked set. The PID enumeration,
//! procargs, parent/PGID lookups, and live-descendant classification helpers
//! collected here are also called directly by the periodic orphan sweeps.

use std::path::{Path, PathBuf};

// Re-declare the macOS process-info FFI so sweep.rs can call it independently.
// Multiple extern "C" declarations of the same symbol are legal in Rust; the
// linker sees one symbol regardless of how many translation units declare it.
#[cfg(target_os = "macos")]
extern "C" {
    fn proc_listallpids(buffer: *mut libc::c_int, buffersize: libc::c_int) -> libc::c_int;
    fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
}

// ── Shared low-level helpers ──────────────────────────────────────────────

/// Collect all PIDs currently on the system.
///
/// Loops until the buffer is large enough to hold all PIDs — under a fork
/// storm the count can grow between the probe and the fill call. Returns an
/// empty vec on any kernel error.
#[cfg(target_os = "macos")]
pub(super) fn collect_all_pids() -> Vec<libc::c_int> {
    let mut pids: Vec<libc::c_int>;
    loop {
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Vec::new();
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
            return Vec::new();
        }
        pids.truncate(actual as usize);
        if (actual as usize) < buf_len {
            return pids;
        }
    }
}

/// Read the raw `KERN_PROCARGS2` buffer for the given PID.
///
/// Two-phase sysctl: first probe for the required buffer size, then fill.
/// A process's arguments are immutable after `execve`, so the size reported
/// by the probe is stable — no grow-and-retry loop is needed.
///
/// Returns `None` if either sysctl call fails (e.g. the process has already
/// exited or we lack permission).
#[cfg(target_os = "macos")]
pub(super) fn procargs2_buffer(pid: u32) -> Option<Vec<u8>> {
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
    Some(buf)
}

// ── Ancestor walk ────────────────────────────────────────────────────────

/// True if walking `start`'s parent chain reaches any PID in `skip_pids`.
/// Bounded to 32 hops to guard against PPID cycles from PID reuse; a lookup
/// failure or reaching PID ≤ 1 ends the walk (process is not a descendant of
/// any tracked harness).
///
/// The candidate itself being in `skip_pids` is handled at the call site —
/// this function checks strict ancestors only.
#[cfg(unix)]
pub(super) fn walk_has_tracked_ancestor(
    start: u32,
    skip_pids: &[u32],
    parent_of: impl Fn(u32) -> Option<u32>,
) -> bool {
    const MAX_DEPTH: usize = 32;
    let mut cur = start;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = parent_of(cur) else {
            return false;
        };
        if parent <= 1 || parent == cur {
            return false;
        }
        if skip_pids.contains(&parent) {
            return true;
        }
        cur = parent;
    }
    false
}

/// OS-resolved parent-PID lookup for `walk_has_tracked_ancestor`.
/// Test-only: lets tests call a single platform-agnostic name without
/// cfg gates; production code calls `ppid_of_macos`/`ppid_of_linux` directly.
#[cfg(all(test, target_os = "macos"))]
pub(super) fn ppid_of(pid: u32) -> Option<u32> {
    ppid_of_macos(pid)
}

/// OS-resolved parent-PID lookup for `walk_has_tracked_ancestor`.
/// Test-only: lets tests call a single platform-agnostic name without
/// cfg gates; production code calls `ppid_of_macos`/`ppid_of_linux` directly.
#[cfg(all(test, unix, not(target_os = "macos")))]
pub(super) fn ppid_of(pid: u32) -> Option<u32> {
    ppid_of_linux(pid)
}

/// Return the parent PID of a process on macOS via `proc_pidinfo`.
/// Returns `None` if the syscall fails (process may have exited).
#[cfg(target_os = "macos")]
pub(super) fn ppid_of_macos(pid: u32) -> Option<u32> {
    let mut info = std::mem::MaybeUninit::<super::BSDInfo>::zeroed();
    let ret = unsafe {
        super::proc_pidinfo(
            pid as libc::c_int,
            super::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            std::mem::size_of::<super::BSDInfo>() as libc::c_int,
        )
    };
    if ret <= 0 {
        return None;
    }
    Some(unsafe { info.assume_init() }.pbi_ppid)
}

/// Parse the PPID and PGID fields from `/proc/<pid>/stat` in one read.
/// Fields after the last `)` (comm may contain spaces/parens): index 1 is
/// PPID, index 2 is PGID.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn proc_stat_ppid_pgid_linux(pid: u32) -> Option<(u32, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    // Fields after ')': " S ppid pgid ..."
    let mut fields = after_comm.split_whitespace();
    let _state = fields.next()?; // index 0: state
    let ppid = fields.next()?.parse::<u32>().ok()?; // index 1: PPID
    let pgid = fields.next()?.parse::<u32>().ok()?; // index 2: PGID
    Some((ppid, pgid))
}

/// Return the parent PID of a process from `/proc/<pid>/stat`.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn ppid_of_linux(pid: u32) -> Option<u32> {
    proc_stat_ppid_pgid_linux(pid).map(|(ppid, _)| ppid)
}

/// True if `pid` is a live descendant of any tracked harness in `skip_pids`.
///
/// Three complementary checks:
/// 1. Direct parent — `ppid` was already fetched by the caller's BSDInfo
///    UID gate, so this hop is free.
/// 2. Reparenting guard — if an intermediate in the ancestor chain died,
///    the process reparents to init (PPID 1) and the ancestor walk can no
///    longer reach the harness; a process that started inside the
///    harness's process group still has PGID == harness PID, so the PGID
///    check spares it. This is NOT a redundant fast-path — it covers a
///    case the walk cannot.
/// 3. Bounded ancestor walk from `ppid` — covers deeper live chains where
///    intermediates run in their own process groups (e.g. buzz-acp ->
///    node shim -> codex-acp).
#[cfg(target_os = "macos")]
pub(super) fn is_live_descendant_macos(pid: u32, ppid: u32, skip_pids: &[u32]) -> bool {
    if skip_pids.contains(&ppid) {
        return true;
    }
    let pgid = unsafe { libc::getpgid(pid as i32) };
    if pgid > 0 && skip_pids.contains(&(pgid as u32)) {
        return true;
    }
    walk_has_tracked_ancestor(ppid, skip_pids, ppid_of_macos)
}

/// Linux variant: reads PPID and PGID from `/proc/<pid>/stat` in a single
/// read, then applies the same three checks as the macOS variant. An
/// unreadable stat file (process exiting) yields `false` — the two-tick
/// grace in the periodic sweep absorbs transient failures.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn is_live_descendant_linux(pid: u32, skip_pids: &[u32]) -> bool {
    let Some((ppid, pgid)) = proc_stat_ppid_pgid_linux(pid) else {
        return false;
    };
    if skip_pids.contains(&ppid) || skip_pids.contains(&pgid) {
        return true;
    }
    walk_has_tracked_ancestor(ppid, skip_pids, ppid_of_linux)
}

// ── ProcessSnapshot and pure decision function ────────────────────────────

/// A snapshot of one process for the pure kill-decision function. Holds only
/// the fields needed to decide whether a process is an untracked same-bundle
/// harness — no live process handles, no system calls.
#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    /// PID of the process.
    pub pid: u32,
    /// Full executable path, as reported by the kernel.
    pub exe_path: PathBuf,
}

/// Strip the kernel-appended `" (deleted)"` suffix from an executable path.
///
/// On Linux, `read_link("/proc/<pid>/exe")` returns `…/buzz-acp (deleted)`
/// when the on-disk binary has been replaced since the process launched.
/// That is exactly the class of stale-install orphan this sweep targets, so
/// we must strip the suffix before comparing against the expected path.
/// Falls back to the original path unchanged when the suffix is absent.
// Available on Linux (where it is called from collect_process_snapshots) and
// in test builds on all platforms (including macOS). Dead on macOS outside
// tests — suppress the warning there.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
pub(super) fn strip_deleted_suffix(path: PathBuf) -> PathBuf {
    const SUFFIX: &str = " (deleted)";
    match path.to_str() {
        Some(s) if s.ends_with(SUFFIX) => PathBuf::from(&s[..s.len() - SUFFIX.len()]),
        _ => path,
    }
}

/// Pure kill-decision function: given a slice of process snapshots, the
/// expected harness executable path, and the set of tracked pids to spare,
/// returns the pids of processes that should be reaped.
///
/// Selection criteria:
/// - `exe_path` exactly matches `harness_exe` (same-bundle harness only).
/// - `pid` is not in `tracked_pids` (untracked — not owned by this session).
///
/// Children of tracked parents die when their parent's process group is
/// signalled — this function deliberately targets only harness-level processes
/// so we never directly kill a child of a live tracked parent.
pub fn select_untracked_bundle_harnesses(
    snapshots: &[ProcessSnapshot],
    harness_exe: &Path,
    tracked_pids: &[u32],
) -> Vec<u32> {
    snapshots
        .iter()
        .filter(|s| s.exe_path == harness_exe && !tracked_pids.contains(&s.pid))
        .map(|s| s.pid)
        .collect()
}

// ── Process-table enumeration ─────────────────────────────────────────────

/// Extract the executable path from a process's `KERN_PROCARGS2` buffer.
///
/// The buffer layout is: `[i32 argc][exec_path\0][null-pad][argv\0…][env\0…]`.
/// The exec path is therefore the first null-terminated string immediately
/// after the leading `i32` — no argv traversal is needed, unlike
/// `extract_buzz_marker_value` / `process_has_buzz_marker` which must skip
/// past both argv and the exec path to reach the environment entries.
///
/// Returns `None` if the buffer is unreadable or malformed.
#[cfg(target_os = "macos")]
fn proc_exe_path_from_procargs2(pid: u32) -> Option<PathBuf> {
    let buf = procargs2_buffer(pid)?;
    if buf.len() < std::mem::size_of::<libc::c_int>() {
        return None;
    }
    // Skip the argc i32 at the start of the buffer.
    let pos = std::mem::size_of::<libc::c_int>();
    // The exec path immediately follows — scan to the first null byte.
    let end = buf[pos..].iter().position(|&b| b == 0).map(|i| pos + i)?;
    let path_bytes = &buf[pos..end];
    if path_bytes.is_empty() {
        return None;
    }
    // KERN_PROCARGS2 exec paths are always absolute UTF-8 on macOS.
    let s = std::str::from_utf8(path_bytes).ok()?;
    Some(PathBuf::from(s))
}

/// Collect process snapshots for all user-owned processes on macOS.
///
/// Applies a cheap `proc_name` pre-filter before the expensive
/// `KERN_PROCARGS2` sysctl: only PIDs whose binary name matches the harness
/// binary name (`buzz-acp`) proceed to the full exe-path fetch.  This mirrors
/// the pattern `sweep_system_agent_processes` uses and cuts the expensive
/// two-sysctl call by ~99.9% (there are typically O(hundreds) of user
/// processes but at most a handful of `buzz-acp` instances).
#[cfg(target_os = "macos")]
fn collect_process_snapshots(harness_name: &str) -> Vec<ProcessSnapshot> {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;
    let mut snapshots = Vec::new();

    let pids = collect_all_pids();

    for &pid in &pids {
        if pid <= 0 || pid == my_pid {
            continue;
        }
        // Cheap name pre-filter: only proceed to the expensive KERN_PROCARGS2
        // sysctl for PIDs whose binary name matches the harness binary name.
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
        if name != harness_name {
            continue;
        }
        // Verify UID to avoid inspecting processes owned by other users.
        let upid = pid as u32;
        let mut info = std::mem::MaybeUninit::<super::BSDInfo>::zeroed();
        let ret = unsafe {
            super::proc_pidinfo(
                pid,
                super::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of::<super::BSDInfo>() as libc::c_int,
            )
        };
        if ret <= 0 {
            continue;
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_uid != my_uid {
            continue;
        }
        if let Some(exe_path) = proc_exe_path_from_procargs2(upid) {
            snapshots.push(ProcessSnapshot {
                pid: upid,
                exe_path,
            });
        }
    }
    snapshots
}

/// Collect process snapshots for all user-owned processes on Linux via /proc.
///
/// On Linux `/proc/<pid>/exe` is a symlink to the executable path. The kernel
/// appends `" (deleted)"` when the on-disk binary has been replaced since
/// launch — exactly the stale-install class this sweep targets. The suffix is
/// stripped so the comparison against `expected_harness_exe_path` succeeds.
#[cfg(all(unix, not(target_os = "macos")))]
fn collect_process_snapshots(harness_name: &str) -> Vec<ProcessSnapshot> {
    let my_uid = unsafe { libc::getuid() };
    let my_pid = std::process::id() as i32;
    let mut snapshots = Vec::new();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return snapshots;
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
        // Cheap name pre-filter via /proc/<pid>/comm (15-char truncated, but
        // "buzz-acp" is 8 chars so it's always preserved).
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{upid}/comm")) else {
            continue;
        };
        if comm.trim() != harness_name {
            continue;
        }
        // Check ownership.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != my_uid {
            continue;
        }
        // Resolve the executable path via the /proc symlink.
        // Strip " (deleted)" so orphans whose binary was replaced still match.
        if let Ok(exe_path) = std::fs::read_link(format!("/proc/{upid}/exe")) {
            snapshots.push(ProcessSnapshot {
                pid: upid,
                exe_path: strip_deleted_suffix(exe_path),
            });
        }
    }
    snapshots
}

// ── expected_harness_exe_path ─────────────────────────────────────────────

/// Derive the expected path of the `buzz-acp` harness binary next to the
/// current executable. Returns `None` if `current_exe()` fails or has no
/// parent directory.
///
/// In a `.app` bundle: `.../Contents/MacOS/buzz-acp`.
/// In a dev checkout: `<target-dir>/debug/buzz-acp` or similar.
/// Never hardcoded — always derived from the running process.
///
/// Attempts `std::fs::canonicalize` to resolve symlinks so the path
/// comparison in `select_untracked_bundle_harnesses` is stable even when the
/// bundle is accessed through a symlink. Falls back to the raw (unresolved)
/// path on `canonicalize` failure — canonicalization can fail for paths that
/// exist only as kernel metadata (e.g. a process launched from a path that
/// has since been moved), so failure must never prevent the sweep from running;
/// it only narrows the comparison to raw paths, which is still correct for
/// the common case.
///
/// Residual false-negative: if the bundle itself has been translocated or
/// moved since launch, `current_exe()` reflects the new path but a running
/// harness may report the old path. In that case the exe-path comparison
/// fails harmlessly — the orphan is not killed, but neither is anything
/// incorrectly killed. Similarly, an orphan spawned by an older install of
/// the same app (different bundle path, e.g. a prior DMG) will not match
/// this path — that class is handled by `sweep_system_agent_processes`, which
/// scopes by `BUZZ_MANAGED_AGENT` instance ID rather than exe path.
pub fn expected_harness_exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let raw = dir.join("buzz-acp");
    // Canonicalize if possible; fall back to the raw path on failure.
    Some(std::fs::canonicalize(&raw).unwrap_or(raw))
}

/// The basename of the harness binary — used for the cheap name pre-filter in
/// `collect_process_snapshots` before the expensive exe-path lookup.
const HARNESS_BINARY_NAME: &str = "buzz-acp";

// ── sweep_untracked_bundle_harnesses ─────────────────────────────────────

/// Sweep and kill harness processes that share this bundle's exact `buzz-acp`
/// executable path but are not in `skip_pids`.
///
/// Complements the env-var-based `sweep_system_agent_processes`: this sweep
/// catches orphans that predate the `BUZZ_MANAGED_AGENT` env var injection
/// and any that lost their PID-file receipt.
///
/// **Boot-time only.** This function is called once, under the store lock,
/// before Phase B spawns any new agents — there is no window for a legitimate
/// harness to have started between the tracked-pid snapshot and this scan, so
/// no grace mechanism is needed. A future periodic caller would need the
/// `sweep_system_agent_processes_with_grace`-style two-tick grace to avoid
/// killing a harness that is legitimately starting up between the skip-list
/// snapshot and the scan.
///
/// Scoping guarantee: only processes whose exe path (after symlink resolution
/// where possible) equals `<this bundle>/buzz-acp` are candidates. Dev builds
/// at a different path, other installs, and children of tracked parents are
/// never directly targeted. Children die with their parent's process group
/// when `resolve_pgids_and_kill` signals the PGID.
#[cfg(unix)]
pub(crate) fn sweep_untracked_bundle_harnesses(skip_pids: &[u32]) {
    let Some(harness_exe) = expected_harness_exe_path() else {
        return;
    };
    let snapshots = collect_process_snapshots(HARNESS_BINARY_NAME);
    let to_kill = select_untracked_bundle_harnesses(&snapshots, &harness_exe, skip_pids);
    if to_kill.is_empty() {
        return;
    }
    eprintln!(
        "buzz-desktop: sweep_untracked_bundle_harnesses: reaping {} stale harness process(es) {:?} (exe: {})",
        to_kill.len(),
        to_kill,
        harness_exe.display(),
    );
    // Small snapshot→kill PID-reuse window: a PID in `to_kill` could be
    // recycled between the snapshot and the kill call. This matches the
    // precedent set by the neighboring sweeps; `resolve_pgids_and_kill`'s
    // PGID-recycling retain guard (skip a resolved PGID that is alive but
    // not one of our orphan candidates) narrows the window further.
    let to_kill_i32: Vec<i32> = to_kill.iter().map(|&p| p as i32).collect();
    super::resolve_pgids_and_kill(&to_kill_i32);
}

#[cfg(not(unix))]
pub(crate) fn sweep_untracked_bundle_harnesses(_skip_pids: &[u32]) {}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_deleted_suffix ─────────────────────────────────────────────

    #[test]
    fn strip_deleted_suffix_removes_kernel_suffix() {
        // Linux appends " (deleted)" when the binary has been replaced since
        // launch — this is exactly the stale orphan class we want to reap.
        let p = PathBuf::from("/Applications/Buzz.app/Contents/MacOS/buzz-acp (deleted)");
        assert_eq!(
            strip_deleted_suffix(p),
            PathBuf::from("/Applications/Buzz.app/Contents/MacOS/buzz-acp")
        );
    }

    #[test]
    fn strip_deleted_suffix_leaves_normal_path_unchanged() {
        let p = PathBuf::from("/Applications/Buzz.app/Contents/MacOS/buzz-acp");
        assert_eq!(strip_deleted_suffix(p.clone()), p,);
    }

    #[test]
    fn strip_deleted_suffix_does_not_strip_partial_match() {
        // "(deleted)" without the leading space must not be stripped.
        let p = PathBuf::from("/some/path/buzz-acp(deleted)");
        assert_eq!(strip_deleted_suffix(p.clone()), p,);
    }

    // ── select_untracked_bundle_harnesses ────────────────────────────────

    const BUNDLE_HARNESS: &str = "/Applications/Buzz.app/Contents/MacOS/buzz-acp";
    const DEV_HARNESS: &str = "/Users/dev/buzz/.worktrees/main/target/debug/buzz-acp";

    fn snap(pid: u32, path: &str) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            exe_path: PathBuf::from(path),
        }
    }

    #[test]
    fn untracked_same_bundle_harness_is_killed() {
        let snapshots = vec![snap(1001, BUNDLE_HARNESS)];
        let result =
            select_untracked_bundle_harnesses(&snapshots, &PathBuf::from(BUNDLE_HARNESS), &[]);
        assert_eq!(result, vec![1001]);
    }

    #[test]
    fn tracked_harness_is_spared() {
        let snapshots = vec![snap(1002, BUNDLE_HARNESS)];
        let result =
            select_untracked_bundle_harnesses(&snapshots, &PathBuf::from(BUNDLE_HARNESS), &[1002]);
        assert!(result.is_empty());
    }

    #[test]
    fn different_bundle_path_is_spared() {
        let snapshots = vec![snap(1003, DEV_HARNESS)];
        let result =
            select_untracked_bundle_harnesses(&snapshots, &PathBuf::from(BUNDLE_HARNESS), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn child_of_tracked_parent_not_directly_targeted() {
        // A non-harness binary is never selected regardless of tracked state.
        let snapshots = vec![snap(1004, "/Applications/Buzz.app/Contents/MacOS/goose")];
        let result =
            select_untracked_bundle_harnesses(&snapshots, &PathBuf::from(BUNDLE_HARNESS), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_process_list_returns_empty() {
        let result = select_untracked_bundle_harnesses(&[], &PathBuf::from(BUNDLE_HARNESS), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mixed_snapshot_kills_only_untracked_same_bundle() {
        let snapshots = vec![
            snap(2001, BUNDLE_HARNESS),   // tracked → spared
            snap(2002, BUNDLE_HARNESS),   // untracked → killed
            snap(2003, DEV_HARNESS),      // different path → spared
            snap(2004, "/usr/bin/goose"), // unrelated → spared
        ];
        let mut result =
            select_untracked_bundle_harnesses(&snapshots, &PathBuf::from(BUNDLE_HARNESS), &[2001]);
        result.sort();
        assert_eq!(result, vec![2002]);
    }

    #[test]
    fn deleted_suffix_stripped_path_matches_expected() {
        // Snapshot with " (deleted)" suffix stripped → should match the clean expected path.
        let raw = PathBuf::from("/Applications/Buzz.app/Contents/MacOS/buzz-acp (deleted)");
        let snaps = vec![ProcessSnapshot {
            pid: 3001,
            exe_path: strip_deleted_suffix(raw),
        }];
        let result = select_untracked_bundle_harnesses(&snaps, &PathBuf::from(BUNDLE_HARNESS), &[]);
        assert_eq!(result, vec![3001]);
    }

    // ── walk_has_tracked_ancestor ────────────────────────────────────────

    #[cfg(unix)]
    fn map_parent(tree: &std::collections::HashMap<u32, u32>, pid: u32) -> Option<u32> {
        tree.get(&pid).copied()
    }

    #[cfg(unix)]
    #[test]
    fn walk_direct_child_of_tracked_harness_is_exempted() {
        // PID 101's parent is 100 (tracked) → live descendant, not an orphan.
        let tree: std::collections::HashMap<u32, u32> = [(101, 100)].into_iter().collect();
        assert!(walk_has_tracked_ancestor(101, &[100], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_grandchild_via_own_group_wrapper_is_exempted() {
        // Production tree: harness(100) → node-wrapper(101, own group) → codex-acp(102).
        // One-level PPID check misses 102; the walk catches it.
        let tree: std::collections::HashMap<u32, u32> =
            [(101, 100), (102, 101)].into_iter().collect();
        assert!(walk_has_tracked_ancestor(102, &[100], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_real_orphan_ending_at_pid1_returns_false() {
        // Genuine orphan: chain ends at init (PID 1), no tracked ancestor.
        let tree: std::collections::HashMap<u32, u32> = [(201, 1)].into_iter().collect();
        assert!(!walk_has_tracked_ancestor(201, &[100], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_ppid_cycle_terminates_and_returns_false() {
        // PPID cycle (a → b → a) from PID reuse must terminate, not loop.
        let tree: std::collections::HashMap<u32, u32> =
            [(300, 301), (301, 300)].into_iter().collect();
        assert!(!walk_has_tracked_ancestor(300, &[999], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_missing_parent_entry_returns_false() {
        // proc_pidinfo / /proc stat failure (process exited) → not a descendant.
        let tree: std::collections::HashMap<u32, u32> = [].into_iter().collect();
        assert!(!walk_has_tracked_ancestor(400, &[100], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_finds_ancestor_at_exact_depth_cap() {
        // Chain with exactly 32 edges: 1000 → 1001 → … → 1032.
        // The ancestor at hop 32 is within MAX_DEPTH and must be found.
        let mut tree = std::collections::HashMap::new();
        for i in 0..32u32 {
            tree.insert(1000 + i, 1000 + i + 1);
        }
        assert!(walk_has_tracked_ancestor(1000, &[1032], |p| map_parent(
            &tree, p
        )));
    }

    #[cfg(unix)]
    #[test]
    fn walk_misses_ancestor_beyond_depth_cap() {
        // Chain with 33 edges: 1000 → 1001 → … → 1033.
        // Hop 33 exceeds MAX_DEPTH (32) — the ancestor must not be found.
        let mut tree = std::collections::HashMap::new();
        for i in 0..33u32 {
            tree.insert(1000 + i, 1000 + i + 1);
        }
        assert!(!walk_has_tracked_ancestor(1000, &[1033], |p| map_parent(
            &tree, p
        )));
    }
}
