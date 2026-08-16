//! Ownership marker + startup sweep for buzz-dev-mcp's per-process temp
//! directories (issue #6025).
//!
//! ## Why this exists
//!
//! `shim::Shim::install` and `shell::SharedState::new` each create a
//! `tempfile::TempDir` (prefixed `buzz-dev-mcp-` and
//! `buzz-dev-mcp-session-` respectively) whose cleanup runs in `Drop`. When
//! the MCP server process is killed outright (SIGKILL, a Windows hard
//! terminate, an ACP harness reaping a stuck agent) `Drop` never runs and the
//! directory — holding full copies of buzz/rg/tree/git helpers, tens of MB
//! each — is orphaned. Both prefixes start with `buzz-dev-mcp-`, so a single
//! sweep over that one prefix catches both.
//!
//! ## Design
//!
//! An age-based sweep ("delete anything older than N hours") is
//! deliberately NOT the mechanism here: a session can legitimately run for
//! days, and deleting its shim/session dir out from under it would break a
//! live agent mid-command. Instead, every directory this crate creates gets
//! an ownership marker recording the creating process's pid. At startup we
//! remove a directory only when we can positively confirm that pid is no
//! longer running. Every uncertain case — marker missing, marker corrupt,
//! pid alive, pid liveness undeterminable, or a removal failure — is left
//! alone and logged. The safe failure mode is a persisted leak, never a
//! deleted live session.

use std::path::Path;

/// Every directory this crate creates in the system temp dir starts with
/// this prefix (`buzz-dev-mcp-` for the shim dir, `buzz-dev-mcp-session-`
/// for the session dir — both match, since the second extends the first).
pub(crate) const OWNER_PREFIX: &str = "buzz-dev-mcp-";

/// Ownership marker file name, dropped inside every temp dir this crate
/// creates.
const MARKER_FILE_NAME: &str = ".buzz-dev-mcp-owner";

/// Write the ownership marker recording this process's pid and creation
/// time (unix seconds). Trivial `key=value` lines, one per line: a future
/// field can be appended without breaking older readers (unknown keys are
/// ignored), and a half-written file simply fails to yield a `pid` rather
/// than panicking anything downstream.
///
/// Best-effort: a write failure only affects a *future* startup sweep, never
/// this session, so it must not fail directory creation. Logged at `warn`
/// since it silently degrades the sweep to "never touch this dir".
pub(crate) fn write_owner_marker(dir: &Path) {
    let pid = std::process::id();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(MARKER_FILE_NAME);
    if let Err(e) = std::fs::write(&path, format!("pid={pid}\ncreated={created}\n")) {
        tracing::warn!(
            error = %e,
            path = %path.display(),
            "buzz-dev-mcp: failed to write temp dir ownership marker; a future startup sweep will leave this directory alone"
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct OwnerMarker {
    pid: u32,
}

/// Parse the marker in `dir`. Returns `None` if the marker file is missing,
/// unreadable, or does not contain a usable `pid` line — all three are
/// handled identically by [`sweep_one`] (leave the directory alone), so a
/// half-written or corrupt marker fails exactly as safe as one that was
/// never written.
fn read_owner_marker(dir: &Path) -> Option<OwnerMarker> {
    let text = std::fs::read_to_string(dir.join(MARKER_FILE_NAME)).ok()?;
    let mut pid = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("pid=") {
            // Reject 0 and anything past a POSIX pid_t: a corrupt or
            // adversarial `pid=0` would otherwise reach the Unix liveness
            // check, where `kill(0, ...)` means "every process in my
            // process group", not "no such process".
            pid = value
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|&p| p != 0 && p <= i32::MAX as u32);
        }
    }
    pid.map(|pid| OwnerMarker { pid })
}

/// Tri-state result of a pid liveness check, kept distinct from a plain
/// `bool` so every call site has to name the "we don't actually know"
/// branch instead of silently defaulting one way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Alive,
    Dead,
    /// Could not determine (e.g. a permissions error probing the pid).
    /// Treated identically to `Alive` — see module docs.
    Unknown,
}

impl Liveness {
    fn may_delete(self) -> bool {
        self == Liveness::Dead
    }
}

#[cfg(unix)]
fn pid_liveness(pid: u32) -> Liveness {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Signal 0 sends nothing; the kernel still performs the existence and
    // permission checks, which is exactly the question being asked here.
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => Liveness::Alive,
        Err(Errno::ESRCH) => Liveness::Dead,
        Err(_) => Liveness::Unknown, // e.g. EPERM: exists, owned by someone else
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn pid_liveness(pid: u32) -> Liveness {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess/GetExitCodeProcess/CloseHandle are plain,
    // documented Win32 calls used per their contract. `pid` is
    // attacker-controlled (it comes from a marker file on disk), but
    // OpenProcess validates it itself and returns NULL rather than
    // requiring the caller to pre-validate. The handle from a successful
    // OpenProcess is closed exactly once, immediately after the single
    // GetExitCodeProcess call that uses it, on every return path.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return if GetLastError() == ERROR_ACCESS_DENIED {
                // The process exists but this query is denied (e.g. a
                // protected process) — the safe direction is to assume it's
                // alive rather than guess it's gone.
                Liveness::Unknown
            } else {
                // Typically ERROR_INVALID_PARAMETER: no such pid.
                Liveness::Dead
            };
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if ok == 0 {
            return Liveness::Unknown;
        }
        if exit_code == STILL_ACTIVE as u32 {
            Liveness::Alive
        } else {
            Liveness::Dead
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_liveness(_pid: u32) -> Liveness {
    // No liveness primitive wired up for this target: fail safe by never
    // confirming "dead", so the sweep never deletes anything here. Keeps
    // the crate compiling everywhere without pretending to a guarantee it
    // can't back up.
    Liveness::Unknown
}

/// Outcome of one [`sweep_stale_dirs`] run — surfaced so callers can log a
/// single summary line and tests can assert on behavior without depending
/// on tracing output.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SweepStats {
    pub(crate) removed: usize,
    pub(crate) skipped_alive_or_unknown: usize,
    pub(crate) skipped_no_marker: usize,
    pub(crate) errors: usize,
}

/// Startup sweep: scan `temp_root` for entries left behind by a killed
/// buzz-dev-mcp process (see module docs). Removes an entry ONLY when its
/// marker names a pid that is positively confirmed dead. Every other case —
/// no marker, corrupt marker, pid alive, pid liveness undeterminable, or a
/// removal failure — is left alone and logged; see module docs for why that
/// is the safe default.
///
/// Best-effort end to end: nothing here returns an error or panics, since a
/// stuck or misconfigured temp directory must never prevent the MCP server
/// from starting.
pub(crate) fn sweep_stale_dirs(temp_root: &Path) -> SweepStats {
    let mut stats = SweepStats::default();

    let entries = match std::fs::read_dir(temp_root) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                error = %e,
                dir = %temp_root.display(),
                "buzz-dev-mcp: startup sweep could not read the temp directory; skipping"
            );
            stats.errors += 1;
            return stats;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "buzz-dev-mcp: startup sweep: unreadable directory entry; skipping"
                );
                stats.errors += 1;
                continue;
            }
        };

        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue; // never one of ours: tempfile prefixes are plain ASCII
        };
        if !name.starts_with(OWNER_PREFIX) {
            continue;
        }

        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {}
            Ok(_) => continue, // a same-prefixed file is never one of ours
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    entry = %entry.path().display(),
                    "buzz-dev-mcp: startup sweep: could not stat entry; skipping"
                );
                stats.errors += 1;
                continue;
            }
        }

        sweep_one(&entry.path(), &mut stats);
    }

    stats
}

fn sweep_one(dir: &Path, stats: &mut SweepStats) {
    let Some(marker) = read_owner_marker(dir) else {
        // Legacy rule: a directory with no confirmable owner — either it
        // predates this change, or its marker is corrupt/half-written — is
        // never auto-deleted. A dead process cannot be told apart from a
        // session that has legitimately run for days without a marker to
        // check, and getting that wrong deletes a live session's binaries
        // out from under it. So these are left alone for a human (or a
        // future one-time cleanup tool) rather than guessed at with, say,
        // an age cutoff. Worst case the pre-existing leak persists — that's
        // a pre-existing condition, not a regression introduced here.
        stats.skipped_no_marker += 1;
        return;
    };

    if !pid_liveness(marker.pid).may_delete() {
        stats.skipped_alive_or_unknown += 1;
        return;
    }

    match std::fs::remove_dir_all(dir) {
        Ok(()) => {
            stats.removed += 1;
            tracing::info!(
                dir = %dir.display(),
                pid = marker.pid,
                "buzz-dev-mcp: removed orphaned temp dir left by a killed process (#6025)"
            );
        }
        Err(e) => {
            stats.errors += 1;
            tracing::warn!(
                error = %e,
                dir = %dir.display(),
                pid = marker.pid,
                "buzz-dev-mcp: startup sweep: failed to remove orphaned temp dir; leaving it"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_marker(dir: &Path, pid: u32) {
        std::fs::write(
            dir.join(MARKER_FILE_NAME),
            format!("pid={pid}\ncreated=1\n"),
        )
        .expect("write marker");
    }

    /// Spawn and immediately reap a trivial child process so its pid is
    /// guaranteed dead (not a zombie, not merely "probably unused") by the
    /// time a test uses it — the only reliable way to get a "known dead"
    /// pid without guessing at an unused number.
    fn dead_pid() -> u32 {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "exit", "0"]);
            c
        } else {
            std::process::Command::new("true")
        };
        let mut child = cmd.spawn().expect("spawn short-lived helper process");
        let pid = child.id();
        let _ = child.wait().expect("reap helper process");
        pid
    }

    #[test]
    fn dead_pid_marker_is_swept() {
        let root = tempdir().expect("tempdir");
        let target = root.path().join(format!("{OWNER_PREFIX}test-dead"));
        std::fs::create_dir(&target).expect("mkdir");
        write_marker(&target, dead_pid());

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 1, "{stats:?}");
        assert!(
            !target.exists(),
            "orphaned dir with a dead-pid marker must be removed"
        );
    }

    #[test]
    fn live_pid_marker_is_not_swept() {
        let root = tempdir().expect("tempdir");
        let target = root.path().join(format!("{OWNER_PREFIX}test-live"));
        std::fs::create_dir(&target).expect("mkdir");
        write_marker(&target, std::process::id()); // this test process: definitely alive

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_alive_or_unknown, 1, "{stats:?}");
        assert!(
            target.exists(),
            "a dir owned by a live pid must survive the sweep"
        );
    }

    #[test]
    fn missing_marker_is_left_alone_per_the_legacy_rule() {
        // Documents the chosen legacy rule (see sweep_one's doc comment): a
        // directory with no marker at all — e.g. one created before this
        // change shipped — is never auto-deleted. There is no way to tell a
        // pre-fix directory whose owner is long gone apart from one whose
        // session has legitimately run for days, so the sweep declines to
        // guess via e.g. an age cutoff.
        let root = tempdir().expect("tempdir");
        let target = root.path().join(format!("{OWNER_PREFIX}test-legacy"));
        std::fs::create_dir(&target).expect("mkdir");
        // No marker file written.

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(
            target.exists(),
            "a directory with no marker must survive the sweep"
        );
    }

    #[test]
    fn corrupt_marker_is_treated_like_a_missing_one() {
        let root = tempdir().expect("tempdir");
        let target = root.path().join(format!("{OWNER_PREFIX}test-corrupt"));
        std::fs::create_dir(&target).expect("mkdir");
        std::fs::write(
            target.join(MARKER_FILE_NAME),
            b"not a valid marker\n\x00\xff",
        )
        .expect("write corrupt marker");

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(target.exists());
    }

    #[test]
    fn unrelated_entries_are_ignored() {
        let root = tempdir().expect("tempdir");
        let other_dir = root.path().join("some-other-apps-tmp-dir");
        std::fs::create_dir(&other_dir).expect("mkdir");
        let other_file = root.path().join(format!("{OWNER_PREFIX}not-a-dir"));
        std::fs::write(&other_file, b"file, not a directory").expect("write file");

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats, SweepStats::default());
        assert!(other_dir.exists());
        assert!(other_file.exists());
    }

    #[test]
    fn sweep_tolerates_a_temp_root_that_cannot_be_read() {
        // Point the sweep at a path that doesn't exist rather than the real
        // system temp dir. This must degrade to a no-op + error count, not
        // panic or propagate an error that could abort startup.
        let missing = std::env::temp_dir().join("buzz-dev-mcp-test-does-not-exist-xyz");
        let stats = sweep_stale_dirs(&missing);
        assert_eq!(stats.errors, 1, "{stats:?}");
        assert_eq!(stats.removed, 0, "{stats:?}");
    }

    #[cfg(unix)]
    #[test]
    fn sweep_tolerates_a_permission_denied_entry() {
        use std::os::unix::fs::PermissionsExt;

        // root bypasses permission checks entirely, so chmod 000 would not
        // reproduce a permission error under root (e.g. some CI containers)
        // — skip rather than assert something that can't happen there.
        if nix::unistd::Uid::effective().is_root() {
            eprintln!("skipping: running as root, permission checks are bypassed");
            return;
        }

        let root = tempdir().expect("tempdir");
        let target = root.path().join(format!("{OWNER_PREFIX}test-noperm"));
        std::fs::create_dir(&target).expect("mkdir");
        write_marker(&target, dead_pid());
        // Strip all permissions from the child dir so remove_dir_all on it
        // fails with a permission error instead of succeeding — the sweep
        // must record the error and move on rather than propagating it.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000))
            .expect("chmod 000");

        let stats = sweep_stale_dirs(root.path());

        // Restore permissions so the tempdir guard can clean up afterwards.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
            .expect("restore perms for cleanup");

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.errors, 1, "{stats:?}");
    }
}
