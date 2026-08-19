//! Ownership claim (marker + lease) and startup sweep for buzz-dev-mcp's
//! per-process temp directories (issue #6025).
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
//! ## What authorizes a delete
//!
//! An age-based sweep ("delete anything older than N hours") is deliberately
//! NOT the mechanism here: a session can legitimately run for days, and
//! deleting its shim/session dir out from under it would break a live agent
//! mid-command. Instead every directory this crate creates is *claimed*, and
//! a claim has two independent parts. Both must say "gone" before anything is
//! removed:
//!
//! 1. **Owner marker** — a file recording the creating process's pid. The
//!    sweep deletes only when that pid is positively confirmed dead.
//! 2. **Directory lease** — a lock file the creating process holds open, and
//!    that every command it spawns inherits. The sweep deletes only when the
//!    lease is provably unheld.
//!
//! The pid alone is not enough, and the reason is the shell tool. On Unix a
//! spawned command gets its own process group (`shell::set_process_group`), so
//! a `SIGKILL` of the server leaves the command running — the `Drop` guard
//! that would have killed its group never runs. That surviving command still
//! has the shim directory first on `PATH` and can invoke `buzz`/`rg`/`tree`/the
//! git helpers out of it at any later moment. "Owner pid is dead" says nothing
//! about that command. The lease does: it is held on the open file
//! description, so an inherited descriptor keeps it held for exactly as long
//! as any process that came from this server is alive, and it is released by
//! the kernel the moment the last of them exits — including on `SIGKILL`,
//! where no user-space cleanup runs at all.
//!
//! Windows differs on both halves and is documented at [`acquire_lease`].
//!
//! ## Untrusted input
//!
//! Everything the sweep reads belongs to some *other* process, in a directory
//! (`std::env::temp_dir()`) that is world-writable on a typical Unix box. The
//! marker is therefore parsed as hostile input: opened without following
//! symlinks and without blocking, rejected unless it is a regular file, and
//! read under a hard byte cap. A marker that is a FIFO, a device, a symlink,
//! a directory, or simply too large is skipped, and the sweep moves on.
//!
//! ## Failure direction
//!
//! Every uncertain case — marker missing, marker corrupt or not a regular
//! file, lease held or unreadable, pid alive, pid liveness undeterminable, or
//! a removal failure — leaves the directory alone and logs. The safe failure
//! mode is a persisted leak, never a deleted live session.

use std::path::Path;

/// Every directory this crate creates in the system temp dir starts with
/// this prefix (`buzz-dev-mcp-` for the shim dir, `buzz-dev-mcp-session-`
/// for the session dir — both match, since the second extends the first).
pub(crate) const OWNER_PREFIX: &str = "buzz-dev-mcp-";

/// Ownership marker file name, dropped inside every temp dir this crate
/// creates.
const MARKER_FILE_NAME: &str = ".buzz-dev-mcp-owner";

/// Lease file name. Held open (and locked, on Unix) for as long as the
/// directory is in use. See [`acquire_lease`].
const LEASE_FILE_NAME: &str = ".buzz-dev-mcp-lease";

/// Hard cap on the marker read. A real marker is two short `key=value` lines,
/// around 40 bytes. Anything bigger is not one of ours and is not read.
const MAX_MARKER_BYTES: u64 = 256;

/// Live claim on a temp directory: hold this for as long as the directory is
/// in use, and drop it (or die) to release it.
///
/// Dropping releases the lease, which is what makes the directory reclaimable
/// by a later startup sweep. Callers keep it in the same struct as the
/// `TempDir` it belongs to, **declared before** that `TempDir`: fields drop in
/// declaration order, and on Windows the lease file must be closed before
/// `TempDir` can remove the directory containing it.
pub(crate) struct DirClaim {
    _lease: Lease,
}

/// Claim `dir`: take the lease first, then write the owner marker.
///
/// The order matters and is load-bearing. The sweep treats "no marker" as
/// "never touch this directory", so writing the marker last means a marked
/// directory always has a lease behind it. If the lease cannot be taken, the
/// directory gets no marker at all and is permanently off-limits to the
/// sweep — a leak, which is the failure direction this module chooses every
/// time.
///
/// Best-effort: a failure here only affects a *future* startup sweep, never
/// this session, so it must not fail directory creation.
pub(crate) fn claim_dir(dir: &Path) -> Option<DirClaim> {
    let lease_path = dir.join(LEASE_FILE_NAME);
    let lease = match acquire_lease(&lease_path) {
        Ok(lease) => lease,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %lease_path.display(),
                "buzz-dev-mcp: could not take the temp dir lease; this directory will never be auto-reclaimed"
            );
            return None;
        }
    };

    if let Err(e) = write_owner_marker(dir) {
        tracing::warn!(
            error = %e,
            dir = %dir.display(),
            "buzz-dev-mcp: failed to write the temp dir ownership marker; a future startup sweep will leave this directory alone"
        );
    }

    Some(DirClaim { _lease: lease })
}

/// Give up the right to ever reclaim `dir`, by removing its owner marker.
///
/// Called when something makes a spawned command's use of the directory
/// unobservable — see the Windows job-object path in `shell::run`. Removing
/// the marker downgrades the directory to the same "leave it alone forever"
/// state as a pre-claim directory. That trades a bounded disk leak for the
/// guarantee that nothing is deleted under a live command, which is the trade
/// this module makes everywhere else too.
pub(crate) fn surrender_claim(dir: &Path) {
    let path = dir.join(MARKER_FILE_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::warn!(
            dir = %dir.display(),
            "buzz-dev-mcp: surrendered the temp dir ownership claim; this directory will never be auto-reclaimed"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(
            error = %e,
            path = %path.display(),
            "buzz-dev-mcp: could not remove the ownership marker while surrendering the claim"
        ),
    }
}

/// Write the ownership marker recording this process's pid and creation
/// time (unix seconds). Trivial `key=value` lines, one per line: a future
/// field can be appended without breaking older readers (unknown keys are
/// ignored), and a half-written file simply fails to yield a `pid` rather
/// than panicking anything downstream.
fn write_owner_marker(dir: &Path) -> std::io::Result<()> {
    let pid = std::process::id();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::fs::write(
        dir.join(MARKER_FILE_NAME),
        format!("pid={pid}\ncreated={created}\n"),
    )
}

#[derive(Debug, Clone, Copy)]
struct OwnerMarker {
    pid: u32,
}

/// Parse the marker in `dir`, treating it as untrusted input (see module
/// docs). Returns `None` if the marker file is missing, is not a plain
/// regular file, is larger than [`MAX_MARKER_BYTES`], is not UTF-8, or does
/// not contain a usable `pid` line. Every one of those is handled identically
/// by [`sweep_one`] (leave the directory alone), so a hostile marker fails
/// exactly as safe as one that was never written.
fn read_owner_marker(dir: &Path) -> Option<OwnerMarker> {
    use std::io::Read as _;

    let file = open_untrusted_regular_file(&dir.join(MARKER_FILE_NAME))?;
    // Cheap pre-check on the handle we already hold, so an oversized marker
    // is rejected without reading it at all.
    if file.metadata().ok()?.len() > MAX_MARKER_BYTES {
        return None;
    }
    // Bounded read anyway: the file can grow between the fstat and the read.
    // One byte over the cap is enough to detect the overrun.
    let mut buf = Vec::new();
    (&file)
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_MARKER_BYTES {
        return None;
    }

    let mut pid = None;
    for line in std::str::from_utf8(&buf).ok()?.lines() {
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

/// Open a file that some other process owns, for reading, refusing anything
/// that could block or redirect us.
///
/// Unix: `O_NOFOLLOW` rejects a symlinked path outright, `O_NONBLOCK` means
/// opening a FIFO or a device returns immediately instead of waiting for a
/// peer, and the `fstat` on the resulting handle (not the path, so nothing
/// can be swapped underneath it) rejects everything that is not a regular
/// file. Between them, none of FIFO/symlink-to-FIFO/device/directory can
/// stall or divert the sweep.
fn open_untrusted_regular_file(path: &Path) -> Option<std::fs::File> {
    let file = open_no_follow_no_block(path).ok()?;
    // fstat on the open handle: a check-then-replace race cannot change what
    // this handle refers to.
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    Some(file)
}

#[cfg(unix)]
fn open_no_follow_no_block(path: &Path) -> std::io::Result<std::fs::File> {
    use nix::fcntl::OFlag;
    use std::os::unix::fs::OpenOptionsExt as _;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow_no_block(path: &Path) -> std::io::Result<std::fs::File> {
    // Windows has no `O_NOFOLLOW` and no filesystem FIFOs — a named pipe
    // lives in the `\\.\pipe\` namespace, not under the temp dir — so a read
    // here cannot block on a peer that never arrives. What does exist is
    // reparse points, so reject those before opening. That check is
    // path-based and therefore racy in principle; unlike Unix's shared
    // `/tmp`, the Windows temp root is per-user, so winning that race already
    // requires an account that can write the directory outright.
    if path.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to read a temp dir marker through a reparse point",
        ));
    }
    std::fs::File::open(path)
}

/// Tri-state result of a pid liveness check, kept distinct from a plain
/// `bool` so every call site has to name the "we don't actually know"
/// branch instead of silently defaulting one way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Alive,
    Dead,
    /// Could not determine (e.g. a permissions error probing the pid).
    Unknown,
}

/// Tri-state result of a lease probe, same reasoning as [`Liveness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseState {
    /// Provably unheld: nothing that came from the owning server is alive.
    Free,
    /// Held by a live process, or unreadable — the two are not distinguished
    /// because they authorize exactly the same action, which is none.
    InUse,
    /// No lease file at all.
    Missing,
}

/// The whole deletion policy, as one pure function.
///
/// Removal needs positive proof on *both* axes: the process that created the
/// directory is gone, AND nothing that inherited its lease is still running.
/// Every other combination — including the three `Missing` cases, since
/// [`claim_dir`] writes the lease before the marker and so a marked directory
/// without a lease file is a directory whose state we cannot account for —
/// leaves the directory alone.
fn may_remove(owner: Liveness, lease: LeaseState) -> bool {
    matches!(owner, Liveness::Dead) && matches!(lease, LeaseState::Free)
}

/// Take the directory lease.
///
/// **Unix.** The lock is `flock(LOCK_SH)`, and `FD_CLOEXEC` is cleared on the
/// descriptor so that every command this process spawns inherits it. `flock`
/// locks belong to the open file description rather than to a process, so an
/// inherited descriptor holds the same lock: the lease stays held while the
/// server or *any* command descended from it is alive, and the kernel drops
/// it when the last of them exits, `SIGKILL` included. That is what makes the
/// sweep safe against the orphaned-command case in the module docs.
///
/// **Windows.** The handle is opened denying `FILE_SHARE_DELETE`, so the
/// directory cannot be removed while the server lives, and the probe below
/// detects the open handle. It is deliberately *not* marked inheritable:
/// spawned commands there are held in a Job Object with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`shell::KillGroup`), so a hard kill
/// of the server takes the whole command tree with it and no orphan can
/// outlive the pid check. Inheriting the handle instead would keep a file
/// open inside the directory during the normal shutdown path, where a
/// just-terminated child that has not finished exiting would block
/// `TempDir`'s own cleanup and turn every clean exit into a leak. The one
/// case where the Job Object cannot be established is handled at the spawn
/// site, by surrendering the claim (see [`surrender_claim`]).
#[cfg(unix)]
fn acquire_lease(path: &Path) -> std::io::Result<Lease> {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag, Flock, FlockArg};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    // Must happen before any command is spawned, which is why the claim is
    // taken at directory-creation time.
    fcntl(&file, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|errno| std::io::Error::from_raw_os_error(errno as i32))?;
    Flock::lock(file, FlockArg::LockSharedNonblock)
        .map(|lock| Lease { _lock: lock })
        .map_err(|(_file, errno)| std::io::Error::from_raw_os_error(errno as i32))
}

#[cfg(unix)]
struct Lease {
    _lock: nix::fcntl::Flock<std::fs::File>,
}

#[cfg(unix)]
fn probe_lease(path: &Path) -> LeaseState {
    use nix::fcntl::{Flock, FlockArg};

    let Some(file) = open_untrusted_regular_file(path) else {
        return match path.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LeaseState::Missing,
            // Present but not a plain regular file we can open: not ours, and
            // not something to delete a directory over.
            _ => LeaseState::InUse,
        };
    };
    // An exclusive lock can only be taken when no shared holder is left, so
    // success means every descendant of the owning server has exited. The
    // guard is dropped immediately, releasing it again.
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(_guard) => LeaseState::Free,
        // EWOULDBLOCK (someone holds it) and anything else are the same
        // answer here: not provably free.
        Err(_) => LeaseState::InUse,
    }
}

#[cfg(windows)]
fn acquire_lease(path: &Path) -> std::io::Result<Lease> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    // Readers and writers are fine; deleters are not. While this handle is
    // open, the lease file — and so the directory holding it — cannot be
    // removed by anyone else.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)?;
    Ok(Lease { _file: file })
}

#[cfg(windows)]
struct Lease {
    _file: std::fs::File,
}

#[cfg(windows)]
fn probe_lease(path: &Path) -> LeaseState {
    use std::os::windows::fs::OpenOptionsExt as _;

    // Ask for the file with no sharing at all: this succeeds only if nobody
    // else holds a handle on it.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
    {
        Ok(_) => LeaseState::Free,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LeaseState::Missing,
        Err(_) => LeaseState::InUse,
    }
}

#[cfg(not(any(unix, windows)))]
fn acquire_lease(path: &Path) -> std::io::Result<Lease> {
    // No lease primitive wired up for this target. The file is still created
    // so the on-disk shape matches every other platform, but the probe below
    // never reports it free, so nothing here is ever deleted — the same
    // stance `pid_liveness` takes on an unknown target.
    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map(|file| Lease { _file: file })
}

#[cfg(not(any(unix, windows)))]
struct Lease {
    _file: std::fs::File,
}

#[cfg(not(any(unix, windows)))]
fn probe_lease(path: &Path) -> LeaseState {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LeaseState::Missing,
        _ => LeaseState::InUse,
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

/// Classify an `OpenProcess` failure into [`Liveness`].
///
/// Only `ERROR_INVALID_PARAMETER` — what Windows returns for a pid no
/// process object exists for — establishes that the owner is gone.
/// `ERROR_ACCESS_DENIED` means the process is there but not queryable (a
/// protected process, or one owned by another user). Everything else
/// (resource exhaustion, transient kernel failures) says nothing either way
/// about whether the pid exists, and under this module's fail-safe contract
/// "says nothing" must never authorize a delete, so every code but the one
/// that positively proves absence maps to `Unknown`.
///
/// Split out of `pid_liveness` so the classification is unit-testable
/// directly, without having to provoke real system errors.
#[cfg(windows)]
fn classify_open_process_error(err: u32) -> Liveness {
    use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;

    if err == ERROR_INVALID_PARAMETER {
        Liveness::Dead
    } else {
        Liveness::Unknown
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn pid_liveness(pid: u32) -> Liveness {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, STILL_ACTIVE};
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
            return classify_open_process_error(GetLastError());
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
    pub(crate) skipped_in_use: usize,
    pub(crate) skipped_no_marker: usize,
    pub(crate) errors: usize,
}

/// Startup sweep: scan `temp_root` for entries left behind by a killed
/// buzz-dev-mcp process (see module docs). Removes an entry ONLY when its
/// marker names a pid that is positively confirmed dead AND its lease is
/// provably unheld. Every other case is left alone and logged.
///
/// Best-effort end to end: nothing here returns an error or panics, since a
/// stuck or hostile temp directory must never prevent the MCP server from
/// starting.
pub(crate) fn sweep_stale_dirs(temp_root: &Path) -> SweepStats {
    sweep_stale_dirs_with(temp_root, &|dir| std::fs::remove_dir_all(dir))
}

/// [`sweep_stale_dirs`] with the removal step injected, so tests can drive
/// the removal-failure branch deterministically on every platform instead of
/// trying to provoke a real filesystem error.
fn sweep_stale_dirs_with(
    temp_root: &Path,
    remove: &dyn Fn(&Path) -> std::io::Result<()>,
) -> SweepStats {
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

        // file_type() does not follow symlinks, so a symlink pointing at a
        // directory elsewhere is not a directory here and is skipped below.
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {}
            Ok(_) => continue, // a same-prefixed file or link is never one of ours
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

        sweep_one(&entry.path(), &mut stats, remove);
    }

    stats
}

fn sweep_one(dir: &Path, stats: &mut SweepStats, remove: &dyn Fn(&Path) -> std::io::Result<()>) {
    let Some(marker) = read_owner_marker(dir) else {
        // Legacy rule: a directory with no confirmable owner — either it
        // predates this change, or its marker is corrupt, hostile, or
        // half-written — is never auto-deleted. A dead process cannot be
        // told apart from a session that has legitimately run for days
        // without a marker to check, and getting that wrong deletes a live
        // session's binaries out from under it. So these are left alone for
        // a human (or a future one-time cleanup tool) rather than guessed at
        // with, say, an age cutoff. Worst case the pre-existing leak
        // persists — that's a pre-existing condition, not a regression
        // introduced here.
        stats.skipped_no_marker += 1;
        return;
    };

    let owner = pid_liveness(marker.pid);
    let lease = probe_lease(&dir.join(LEASE_FILE_NAME));
    if !may_remove(owner, lease) {
        if owner == Liveness::Dead {
            // The creating process is gone but something it spawned still
            // holds the lease: exactly the orphaned-command case the lease
            // exists for. Logged at info because it is the interesting one —
            // the directory will be reclaimed on a later startup, once that
            // command finishes.
            stats.skipped_in_use += 1;
            tracing::info!(
                dir = %dir.display(),
                pid = marker.pid,
                ?lease,
                "buzz-dev-mcp: startup sweep: owner is gone but the temp dir is still leased; leaving it"
            );
        } else {
            stats.skipped_alive_or_unknown += 1;
        }
        return;
    }

    match remove(dir) {
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

    /// Stand in for a directory this crate created: an unheld lease file plus
    /// a marker naming `pid`. Written by hand rather than via [`claim_dir`]
    /// because most cases need an owner pid other than this process's.
    fn claimed_dir(root: &Path, name: &str, pid: u32) -> std::path::PathBuf {
        let dir = root.join(format!("{OWNER_PREFIX}{name}"));
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::write(dir.join(LEASE_FILE_NAME), b"").expect("write lease");
        std::fs::write(
            dir.join(MARKER_FILE_NAME),
            format!("pid={pid}\ncreated=1\n"),
        )
        .expect("write marker");
        dir
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

    /// The deletion policy in full. Only one of the nine states authorizes
    /// removal: the owner is provably dead and the lease is provably unheld.
    /// Everything else — every `Unknown`, every `InUse`, every `Missing` —
    /// must not delete.
    #[test]
    fn only_a_dead_owner_with_a_free_lease_authorizes_removal() {
        let owners = [Liveness::Alive, Liveness::Dead, Liveness::Unknown];
        let leases = [LeaseState::Free, LeaseState::InUse, LeaseState::Missing];
        for owner in owners {
            for lease in leases {
                let expected = owner == Liveness::Dead && lease == LeaseState::Free;
                assert_eq!(
                    may_remove(owner, lease),
                    expected,
                    "may_remove({owner:?}, {lease:?})"
                );
            }
        }
    }

    #[test]
    fn dead_owner_with_a_free_lease_is_swept() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-dead", dead_pid());

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 1, "{stats:?}");
        assert!(
            !target.exists(),
            "orphaned dir with a dead-pid marker and a free lease must be removed"
        );
    }

    #[test]
    fn live_pid_marker_is_not_swept() {
        let root = tempdir().expect("tempdir");
        // This test process: definitely alive.
        let target = claimed_dir(root.path(), "test-live", std::process::id());

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_alive_or_unknown, 1, "{stats:?}");
        assert!(
            target.exists(),
            "a dir owned by a live pid must survive the sweep"
        );
    }

    /// [`claim_dir`] writes the lease before the marker, so a marked
    /// directory with no lease file is a directory whose state we cannot
    /// account for. It is not evidence of an idle directory, so it must not
    /// be deleted.
    #[test]
    fn dead_owner_without_a_lease_file_is_not_swept() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-no-lease", dead_pid());
        std::fs::remove_file(target.join(LEASE_FILE_NAME)).expect("remove lease");

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_in_use, 1, "{stats:?}");
        assert!(target.exists());
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
        let target = claimed_dir(root.path(), "test-corrupt", dead_pid());
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

    /// A marker bigger than the cap is not read and not trusted, so its
    /// directory is skipped. Guards the bounded-read rule against a marker
    /// that has been padded out to something unbounded.
    #[test]
    fn oversized_marker_is_skipped() {
        let root = tempdir().expect("tempdir");
        let pid = dead_pid();
        let target = claimed_dir(root.path(), "test-oversized", pid);
        let mut padded = format!("pid={pid}\n");
        padded.push_str(&"x".repeat(MAX_MARKER_BYTES as usize * 4));
        std::fs::write(target.join(MARKER_FILE_NAME), padded).expect("write big marker");

        let stats = sweep_stale_dirs(root.path());

        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(target.exists());
    }

    /// A marker that is a directory rather than a file must be rejected by
    /// the regular-file check on the opened handle.
    #[test]
    fn marker_that_is_a_directory_is_skipped() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-marker-is-dir", dead_pid());
        std::fs::remove_file(target.join(MARKER_FILE_NAME)).expect("remove marker");
        std::fs::create_dir(target.join(MARKER_FILE_NAME)).expect("mkdir marker");

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

    /// A directory whose removal fails must be counted as an error and left
    /// in place, and one failure must not abandon the rest of the sweep.
    ///
    /// The remover is injected rather than provoked with real filesystem
    /// permissions. Stripping permissions from the directory makes its
    /// *marker* unreadable first, so the sweep skips it as unowned and never
    /// reaches removal at all; on top of that, chmod has no Windows analogue
    /// and root bypasses it entirely, so a permission-based version of this
    /// test would silently cover nothing on three different setups.
    #[test]
    fn removal_failures_are_counted_and_do_not_abort_the_sweep() {
        let root = tempdir().expect("tempdir");
        let first = claimed_dir(root.path(), "test-remove-fails-a", dead_pid());
        let second = claimed_dir(root.path(), "test-remove-fails-b", dead_pid());

        let stats = sweep_stale_dirs_with(root.path(), &|_dir| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected removal failure",
            ))
        });

        // Two errors, not one: whatever order read_dir hands the entries
        // back in, the sweep kept going after the first failure.
        assert_eq!(stats.errors, 2, "{stats:?}");
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert!(
            first.exists() && second.exists(),
            "a dir whose removal failed must be left in place"
        );
    }

    /// A live claim keeps its own directory out of the sweep even though the
    /// marker and lease are the real ones, not hand-written.
    #[test]
    fn a_live_claim_protects_its_own_directory() {
        let root = tempdir().expect("tempdir");
        let dir = root.path().join(format!("{OWNER_PREFIX}test-live-claim"));
        std::fs::create_dir(&dir).expect("mkdir");

        let claim = claim_dir(&dir).expect("claim");
        assert_eq!(probe_lease(&dir.join(LEASE_FILE_NAME)), LeaseState::InUse);

        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert!(dir.exists());

        drop(claim);
        assert_eq!(probe_lease(&dir.join(LEASE_FILE_NAME)), LeaseState::Free);
    }

    /// Surrendering the claim removes the marker, which puts the directory
    /// permanently out of the sweep's reach.
    #[test]
    fn surrendering_a_claim_makes_a_directory_unreclaimable() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-surrender", dead_pid());

        surrender_claim(&target);

        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(target.exists());
    }

    /// The regression for the orphaned-command case in the module docs: the
    /// process that created the directory is gone, but a command it spawned
    /// is still running with the inherited lease. The directory must survive
    /// — and become reclaimable the moment that command exits.
    ///
    /// The owner "dying" is modelled by dropping the lease in this process
    /// after the child has inherited it. That is the same thing the kernel
    /// does on `SIGKILL`: the owner's descriptor closes, and the lock stays
    /// held by the inherited one, because an `flock` belongs to the open file
    /// description rather than to a process.
    #[cfg(unix)]
    #[test]
    fn a_command_that_outlives_its_owner_keeps_the_directory() {
        let root = tempdir().expect("tempdir");
        let dir = root.path().join(format!("{OWNER_PREFIX}test-orphan-cmd"));
        std::fs::create_dir(&dir).expect("mkdir");
        let lease_path = dir.join(LEASE_FILE_NAME);

        let claim = claim_dir(&dir).expect("claim");
        // Spawned after the claim, so it inherits the lease descriptor.
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn long-lived child");

        // The owner is gone; only the spawned command is left.
        drop(claim);
        // Its marker still names a dead pid, so the pid check alone would
        // authorize a delete here.
        std::fs::write(
            dir.join(MARKER_FILE_NAME),
            format!("pid={}\ncreated=1\n", dead_pid()),
        )
        .expect("rewrite marker with a dead owner");

        assert_eq!(
            probe_lease(&lease_path),
            LeaseState::InUse,
            "the spawned command must still hold the inherited lease"
        );
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_in_use, 1, "{stats:?}");
        assert!(
            dir.exists(),
            "a directory still in use by a surviving command must not be removed"
        );

        // Once that command exits, the kernel releases the last reference to
        // the lease and the directory becomes reclaimable.
        child.kill().expect("kill child");
        child.wait().expect("reap child");
        assert_eq!(probe_lease(&lease_path), LeaseState::Free);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 1, "{stats:?}");
        assert!(!dir.exists());
    }

    /// A marker that is a FIFO must be skipped, and — the part that matters —
    /// must not block the sweep waiting for a writer that never comes. The
    /// sweep runs on its own thread so a regression fails on the timeout
    /// instead of hanging the whole test binary.
    #[cfg(unix)]
    #[test]
    fn marker_that_is_a_fifo_is_skipped_without_blocking() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-fifo", dead_pid());
        std::fs::remove_file(target.join(MARKER_FILE_NAME)).expect("remove marker");
        nix::unistd::mkfifo(
            &target.join(MARKER_FILE_NAME),
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .expect("mkfifo");

        let scan_root = root.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(sweep_stale_dirs(&scan_root));
        });

        let stats = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("startup sweep blocked on a FIFO marker");
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(target.exists());
    }

    /// Same for a symlinked marker, including one pointing at a FIFO: the
    /// open refuses to follow it at all.
    #[cfg(unix)]
    #[test]
    fn marker_that_is_a_symlink_to_a_fifo_is_skipped_without_blocking() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-symlink", dead_pid());
        let fifo = root.path().join("elsewhere.fifo");
        nix::unistd::mkfifo(
            &fifo,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .expect("mkfifo");
        std::fs::remove_file(target.join(MARKER_FILE_NAME)).expect("remove marker");
        std::os::unix::fs::symlink(&fifo, target.join(MARKER_FILE_NAME)).expect("symlink");

        let scan_root = root.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(sweep_stale_dirs(&scan_root));
        });

        let stats = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("startup sweep blocked on a symlinked FIFO marker");
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_no_marker, 1, "{stats:?}");
        assert!(target.exists());
    }

    /// A lease file that is a FIFO must not block the probe either, and must
    /// never read as free.
    #[cfg(unix)]
    #[test]
    fn lease_that_is_a_fifo_is_never_free() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-fifo-lease", dead_pid());
        let lease_path = target.join(LEASE_FILE_NAME);
        std::fs::remove_file(&lease_path).expect("remove lease");
        nix::unistd::mkfifo(
            &lease_path,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .expect("mkfifo");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(probe_lease(&lease_path));
        });
        let state = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("lease probe blocked on a FIFO");
        assert_eq!(state, LeaseState::InUse);
    }

    /// Only the error code that positively proves a pid has no process
    /// object may authorize a delete. Anything else is a probe that failed,
    /// which is not evidence the owner is gone.
    #[cfg(windows)]
    #[test]
    fn only_an_invalid_pid_error_proves_the_owner_is_gone() {
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY,
            ERROR_NO_SYSTEM_RESOURCES,
        };

        assert_eq!(
            classify_open_process_error(ERROR_INVALID_PARAMETER),
            Liveness::Dead
        );
        // Exists, just not queryable by us.
        assert_eq!(
            classify_open_process_error(ERROR_ACCESS_DENIED),
            Liveness::Unknown
        );
        // Resource pressure and transient kernel failures say nothing about
        // whether the pid exists — deleting on these would take out a live
        // session's binaries.
        assert_eq!(
            classify_open_process_error(ERROR_NOT_ENOUGH_MEMORY),
            Liveness::Unknown
        );
        assert_eq!(
            classify_open_process_error(ERROR_NO_SYSTEM_RESOURCES),
            Liveness::Unknown
        );
        assert_eq!(classify_open_process_error(0), Liveness::Unknown);
    }
}
