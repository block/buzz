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
//! a claim has three independent parts. All three must say "gone" before
//! anything is removed:
//!
//! 1. **Owner marker** — a file recording the creating process's pid. The
//!    sweep deletes only when that pid is positively confirmed dead.
//! 2. **Owner lease** — a lock file the creating process holds open for
//!    exactly as long as it lives, released by the kernel when it dies,
//!    `SIGKILL` included. It answers the same question the pid does, minus
//!    the pid's weak spot: pid numbers get recycled, and a stranger's process
//!    wearing our dead owner's number reads as `Alive` forever. A held lease
//!    is proof the owner itself is still there.
//! 3. **Command registry** — a directory holding one file per command the
//!    owner spawned, created before the spawn and removed when the command is
//!    reaped. The sweep deletes only when every registered command is
//!    provably gone.
//!
//! The owner being dead is not enough, and the reason is the shell tool. On
//! Unix a spawned command gets its own process group
//! (`shell::set_process_group`), so a `SIGKILL` of the server leaves the
//! command running — the `Drop` guard that would have killed its group never
//! runs. That surviving command still has the shim directory first on `PATH`
//! and can invoke `buzz`/`rg`/`tree`/the git helpers out of it at any later
//! moment. "Owner is gone" says nothing about that command. The registry
//! does, and it is the reason removal is safe at all.
//!
//! ## Why the registry is on disk and not a descriptor
//!
//! The obvious cheaper trick is to have the lease descriptor inherited by
//! every spawned command, so the kernel tracks command lifetime for free.
//! That was the previous shape of this module and it is not sound. An
//! inherited descriptor lands in the command's own descriptor table, where
//! the command owns it: `bash` hands out descriptors 3 upward for its own
//! redirections, plenty of programs close everything above 2 on startup, and
//! any of that silently drops the lease while the command runs happily on.
//! The sweep then sees a free lease and deletes a live command's binaries.
//!
//! A registry entry is not reachable from the command at all. Nothing the
//! command does to its descriptors, its environment or its signal handlers
//! can retract it. Buzz writes it, Buzz removes it, and if Buzz is killed
//! before it can remove it the entry stays and the directory survives, which
//! is the direction this module errs in everywhere else too.
//!
//! Windows differs and is documented at [`acquire_lease`] and
//! [`command_liveness`].
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
//! file, lease held or unreadable, pid alive, pid liveness undeterminable, a
//! registry that cannot be read or holds an entry whose command cannot be
//! ruled out, or a removal failure — leaves the directory alone and logs. The
//! safe failure mode is a persisted leak, never a deleted live session.

use std::path::Path;

/// Every directory this crate creates in the system temp dir starts with
/// this prefix (`buzz-dev-mcp-` for the shim dir, `buzz-dev-mcp-session-`
/// for the session dir — both match, since the second extends the first).
pub(crate) const OWNER_PREFIX: &str = "buzz-dev-mcp-";

/// Ownership marker file name, dropped inside every temp dir this crate
/// creates.
const MARKER_FILE_NAME: &str = ".buzz-dev-mcp-owner";

/// Lease file name. Held open (and locked, on Unix) by the owning process for
/// as long as it lives. See [`acquire_lease`].
const LEASE_FILE_NAME: &str = ".buzz-dev-mcp-lease";

/// Command registry directory name, inside every claimed directory. Holds one
/// file per command the owner has spawned and not yet reaped. See
/// [`register_command`].
const CMDS_DIR_NAME: &str = ".buzz-dev-mcp-cmds";

/// Hard cap on registry entries read in one sweep of one directory. A real
/// registry holds one entry per concurrently running command, so single
/// digits. The cap only exists so a hostile directory in a world-writable
/// temp root cannot turn the startup sweep into an unbounded amount of work;
/// hitting it means the directory is not something to delete anyway.
const MAX_REGISTRY_ENTRIES: usize = 4096;

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

/// Claim `dir`: take the lease, create the command registry, then write the
/// owner marker.
///
/// The order matters and is load-bearing. The sweep treats "no marker" as
/// "never touch this directory", so writing the marker last means a marked
/// directory always has both a lease and a registry behind it. If either
/// cannot be created, the directory gets no marker at all and is permanently
/// off-limits to the sweep — a leak, which is the failure direction this
/// module chooses every time.
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

    // Before the marker, for the same reason the lease comes first: a marked
    // directory must never be missing a part of its claim. A registry that
    // cannot be created means commands cannot be registered, which means the
    // sweep must never be allowed to reason about this directory at all.
    if let Err(e) = std::fs::create_dir_all(dir.join(CMDS_DIR_NAME)) {
        tracing::warn!(
            error = %e,
            dir = %dir.display(),
            "buzz-dev-mcp: could not create the command registry; this directory will never be auto-reclaimed"
        );
        return None;
    }

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

/// Verdict on a directory's command registry, same reasoning as [`Liveness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandsState {
    /// The registry exists and every command in it is provably gone.
    Idle,
    /// At least one registered command is alive, or cannot be ruled out
    /// (an entry still being registered, an unparseable name, a liveness
    /// check that failed). All of these authorize exactly the same action,
    /// which is none.
    Busy,
    /// No registry directory at all: a directory this crate did not claim,
    /// or claimed with a version that had no registry. Not accountable, so
    /// not deletable.
    Missing,
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
/// Removal needs positive proof on *all three* axes: the pid that created the
/// directory is confirmed dead, the owner's lease is confirmed unheld, and
/// every command the owner registered is confirmed gone. Every other
/// combination — including each `Missing` case, since [`claim_dir`] writes
/// the lease and the registry before the marker and so a marked directory
/// missing either is a directory whose state we cannot account for — leaves
/// the directory alone.
fn may_remove(owner: Liveness, lease: LeaseState, commands: CommandsState) -> bool {
    matches!(owner, Liveness::Dead)
        && matches!(lease, LeaseState::Free)
        && matches!(commands, CommandsState::Idle)
}

/// Take the directory lease: proof that the process which created this
/// directory is still running.
///
/// The lease covers the owner and nothing else. Commands the owner spawns are
/// tracked by the registry ([`register_command`]), not by this descriptor —
/// see the module docs for why an inherited descriptor cannot do that job.
/// `FD_CLOEXEC` is therefore left at its default, so this descriptor never
/// reaches a spawned command in the first place.
///
/// **Unix.** The lock is `flock(LOCK_SH)`. It is released when the owner
/// exits however it exits, `SIGKILL` included, because the kernel closes the
/// descriptor. The guard's `Drop` also unlocks explicitly on the clean path
/// (`nix`'s `Flock` issues `LOCK_UN`), which reaches the same state by a
/// different route.
///
/// **Windows.** The handle is opened denying `FILE_SHARE_DELETE`, so the
/// directory cannot be removed while the server lives, and the probe below
/// detects the open handle. Spawned commands there are held in a Job Object
/// with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`shell::KillGroup`), so a hard
/// kill of the server takes the whole command tree with it. The one case
/// where the Job Object cannot be established is handled at the spawn site,
/// by surrendering the claim (see [`surrender_claim`]).
#[cfg(unix)]
fn acquire_lease(path: &Path) -> std::io::Result<Lease> {
    use nix::fcntl::{Flock, FlockArg};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
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

/// A registry slot reserved for a command that has not been spawned yet.
///
/// Reserved *before* the spawn on purpose. The dangerous window is a command
/// that is alive with no registry entry naming it, and that window is exactly
/// "after spawn, before the entry is written". Writing a placeholder first
/// closes it: the entry is already on disk when the command draws its first
/// breath, and the sweep treats a placeholder as an unfinished registration
/// it cannot reason about, so the directory is safe either way.
///
/// Dropping one without calling [`PendingCommand::confirm`] or
/// [`PendingCommand::abandon`] leaves the placeholder behind, which costs the
/// directory its reclaimability and nothing else.
pub(crate) struct PendingCommand {
    paths: Vec<std::path::PathBuf>,
}

/// A registered, running command. Dropping it deregisters, which is what
/// makes the directory reclaimable once every command has finished.
pub(crate) struct CommandLifetime {
    paths: Vec<std::path::PathBuf>,
}

/// Reserve a registry slot in each of `dirs` for a command about to be
/// spawned.
///
/// Best-effort per directory: a directory whose registry cannot be written
/// (never claimed, claimed by an older version, or unwritable) simply gets no
/// entry. That is not silently unsafe — a directory with no registry reads as
/// [`CommandsState::Missing`], which never authorizes a delete.
pub(crate) fn register_command(dirs: &[&Path]) -> PendingCommand {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Unique per reservation within this process; combined with the pid it is
    // unique across every process sharing the temp root.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "pending-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );

    let mut paths = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let path = dir.join(CMDS_DIR_NAME).join(&name);
        match std::fs::File::create(&path) {
            Ok(_) => paths.push(path),
            Err(e) => tracing::debug!(
                error = %e,
                path = %path.display(),
                "buzz-dev-mcp: could not reserve a command registry slot"
            ),
        }
    }
    PendingCommand { paths }
}

impl PendingCommand {
    /// Bind the reservation to the spawned command's lifetime handle: its
    /// process group on Unix, its pid on Windows. Renames rather than
    /// rewrites, so there is no instant where the slot is absent.
    pub(crate) fn confirm(self, handle: u32) -> CommandLifetime {
        let mut paths = Vec::with_capacity(self.paths.len());
        for pending in &self.paths {
            let Some(parent) = pending.parent() else {
                continue;
            };
            let final_path = parent.join(handle.to_string());
            match std::fs::rename(pending, &final_path) {
                Ok(()) => paths.push(final_path),
                Err(e) => {
                    // The placeholder is still there and still blocks
                    // reclamation, so nothing is at risk; the directory just
                    // stays unreclaimable until a human clears it.
                    tracing::debug!(
                        error = %e,
                        path = %pending.display(),
                        "buzz-dev-mcp: could not name the command registry slot; leaving the placeholder"
                    );
                }
            }
        }
        CommandLifetime { paths }
    }

    /// Give the reservation back, for a command that never started.
    pub(crate) fn abandon(self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for CommandLifetime {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Err(e) = std::fs::remove_file(path) {
                tracing::debug!(
                    error = %e,
                    path = %path.display(),
                    "buzz-dev-mcp: could not deregister a finished command"
                );
            }
        }
    }
}

/// Read a directory's command registry.
///
/// Like the marker read, this parses state owned by some other process in a
/// world-writable directory, so every unexpected shape resolves to
/// [`CommandsState::Busy`] rather than to a delete: an entry name that is not
/// a number, a registry that is a file or a symlink to somewhere strange, a
/// liveness check that fails, or more entries than any real session has.
fn probe_commands(dir: &Path) -> CommandsState {
    let registry = dir.join(CMDS_DIR_NAME);
    let entries = match std::fs::read_dir(&registry) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CommandsState::Missing,
        Err(e) => {
            tracing::debug!(
                error = %e,
                path = %registry.display(),
                "buzz-dev-mcp: startup sweep: unreadable command registry; treating the dir as in use"
            );
            return CommandsState::Busy;
        }
    };

    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_REGISTRY_ENTRIES {
            return CommandsState::Busy;
        }
        let Ok(entry) = entry else {
            return CommandsState::Busy;
        };
        let name = entry.file_name();
        let Some(handle) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            // A placeholder from `register_command`, or something that is not
            // ours at all. Either way it is not a command that can be ruled
            // out.
            return CommandsState::Busy;
        };
        if command_liveness(handle) != Liveness::Dead {
            return CommandsState::Busy;
        }
    }

    CommandsState::Idle
}

/// Is the command behind a registry entry still running?
///
/// **Unix.** The handle is a process group id, and the question is asked of
/// the whole group with `killpg`, not of one pid. A command is a `bash` that
/// forks freely; the group is what `shell::KillGroup` manages and what
/// survives a hard kill of the server, so the group is the unit whose life
/// the directory depends on. `killpg` succeeds while *any* member is alive,
/// including when the leader has already exited.
///
/// **Windows.** The handle is a pid, checked exactly like an owner pid.
///
/// A recycled id makes a finished command look alive, which costs a leaked
/// directory. There is no failure in the other direction: an id that is
/// provably gone cannot come back.
#[cfg(unix)]
fn command_liveness(pgid: u32) -> Liveness {
    use nix::errno::Errno;
    use nix::sys::signal::killpg;
    use nix::unistd::Pid;

    match killpg(Pid::from_raw(pgid as i32), None) {
        Ok(()) => Liveness::Alive,
        Err(Errno::ESRCH) => Liveness::Dead,
        Err(_) => Liveness::Unknown,
    }
}

#[cfg(not(unix))]
fn command_liveness(pid: u32) -> Liveness {
    pid_liveness(pid)
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
/// marker names a pid that is positively confirmed dead, its lease is
/// provably unheld, and every command it registered is provably gone. Every
/// other case is left alone and logged.
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
    let commands = probe_commands(dir);
    if !may_remove(owner, lease, commands) {
        if owner == Liveness::Dead {
            // The creating process is gone but the directory is still spoken
            // for, by a surviving command or by a lease that has not been
            // released: exactly the orphaned-command case this policy exists
            // for. Logged at info because it is the interesting one — the
            // directory will be reclaimed on a later startup, once that
            // command finishes.
            stats.skipped_in_use += 1;
            tracing::info!(
                dir = %dir.display(),
                pid = marker.pid,
                ?lease,
                ?commands,
                "buzz-dev-mcp: startup sweep: owner is gone but the temp dir is still in use; leaving it"
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

    /// Stand in for a directory this crate created and then lost its owner:
    /// an unheld lease file, an empty command registry, and a marker naming
    /// `pid`. Written by hand rather than via [`claim_dir`] because most
    /// cases need an owner pid other than this process's.
    fn claimed_dir(root: &Path, name: &str, pid: u32) -> std::path::PathBuf {
        let dir = root.join(format!("{OWNER_PREFIX}{name}"));
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::write(dir.join(LEASE_FILE_NAME), b"").expect("write lease");
        std::fs::create_dir(dir.join(CMDS_DIR_NAME)).expect("mkdir registry");
        std::fs::write(
            dir.join(MARKER_FILE_NAME),
            format!("pid={pid}\ncreated=1\n"),
        )
        .expect("write marker");
        dir
    }

    /// A registry handle that stays alive for as long as this test process
    /// does. On Unix that has to be a process *group* id, because that is
    /// what the registry names and what `killpg` can answer for: this
    /// process's own pid is not a group id unless it happens to be a group
    /// leader, and under a test runner it is not.
    fn live_handle() -> u32 {
        #[cfg(unix)]
        {
            nix::unistd::getpgrp().as_raw() as u32
        }
        #[cfg(not(unix))]
        {
            std::process::id()
        }
    }

    /// Write a registry entry naming `handle` into an already-claimed `dir`,
    /// the way [`register_command`] does for a running command.
    #[cfg(unix)]
    fn register_handle(dir: &Path, handle: u32) -> std::path::PathBuf {
        let path = dir.join(CMDS_DIR_NAME).join(handle.to_string());
        std::fs::write(&path, b"").expect("write registry entry");
        path
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

    /// The deletion policy in full. Only one of the twenty-seven states
    /// authorizes removal: the owner is provably dead, the lease is provably
    /// unheld, and the registry is provably idle. Everything else — every
    /// `Unknown`, every `InUse`, every `Busy`, every `Missing` — must not
    /// delete.
    #[test]
    fn removal_needs_a_dead_owner_a_free_lease_and_an_idle_registry() {
        let owners = [Liveness::Alive, Liveness::Dead, Liveness::Unknown];
        let leases = [LeaseState::Free, LeaseState::InUse, LeaseState::Missing];
        let registries = [
            CommandsState::Idle,
            CommandsState::Busy,
            CommandsState::Missing,
        ];
        let mut authorized = 0;
        for owner in owners {
            for lease in leases {
                for commands in registries {
                    let expected = owner == Liveness::Dead
                        && lease == LeaseState::Free
                        && commands == CommandsState::Idle;
                    authorized += usize::from(may_remove(owner, lease, commands));
                    assert_eq!(
                        may_remove(owner, lease, commands),
                        expected,
                        "may_remove({owner:?}, {lease:?}, {commands:?})"
                    );
                }
            }
        }
        assert_eq!(authorized, 1, "exactly one state may delete");
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

    /// A child that stays up for the whole test, in its own process group on
    /// Unix so its pid is also its process group id — the same shape
    /// `shell::run` spawns commands in.
    fn long_lived_child() -> std::process::Child {
        let mut cmd = if cfg!(windows) {
            // `ping` is the portable long sleep on Windows: no console
            // needed, one probe a second, and a single process rather than a
            // `cmd` wrapper that would leave a grandchild behind on kill.
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "61", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("60");
            c
        };
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            cmd.process_group(0);
        }
        cmd.spawn().expect("spawn long-lived child")
    }

    /// The regression for the orphaned-command case in the module docs: the
    /// process that created the directory is gone, a command it spawned is
    /// still running, and the directory has to survive until that command
    /// exits.
    ///
    /// The owner's death is not modelled by tidying up. The claim is dropped,
    /// the marker is rewritten to a pid that is genuinely dead, and the
    /// command's registry entry is deliberately leaked with `mem::forget`,
    /// because that is exactly what a `SIGKILL` leaves behind: an owner that
    /// never got to deregister anything.
    #[test]
    fn a_command_that_outlives_its_owner_keeps_the_directory() {
        let root = tempdir().expect("tempdir");
        let dir = root.path().join(format!("{OWNER_PREFIX}test-orphan-cmd"));
        std::fs::create_dir(&dir).expect("mkdir");

        let claim = claim_dir(&dir).expect("claim");
        // Registered before the spawn and confirmed after, the way
        // `shell::run` does it.
        let pending = register_command(&[dir.as_path()]);
        let mut child = long_lived_child();
        let handle = child.id();
        std::mem::forget(pending.confirm(handle));

        // The owner is gone: lease released, marker naming a dead pid. The
        // pid check alone would authorize a delete right here.
        drop(claim);
        std::fs::write(
            dir.join(MARKER_FILE_NAME),
            format!("pid={}\ncreated=1\n", dead_pid()),
        )
        .expect("rewrite marker with a dead owner");

        assert_eq!(
            probe_commands(&dir),
            CommandsState::Busy,
            "a registered, running command must hold the directory"
        );
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_in_use, 1, "{stats:?}");
        assert!(
            dir.exists(),
            "a directory still in use by a surviving command must not be removed"
        );

        // The command exits. Its entry is still on disk, because the process
        // that would have removed it is dead, so what the sweep is really
        // being asked is whether the thing the entry names is still running.
        child.kill().expect("kill child");
        child.wait().expect("reap child");
        assert!(
            dir.join(CMDS_DIR_NAME).join(handle.to_string()).exists(),
            "the stale entry must still be on disk, or this proves nothing"
        );
        assert_eq!(probe_commands(&dir), CommandsState::Idle);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 1, "{stats:?}");
        assert!(!dir.exists());
    }

    /// The window between "the command is alive" and "the command has a
    /// name" is covered by the placeholder, and the placeholder alone must
    /// hold the directory.
    #[test]
    fn an_unconfirmed_registration_blocks_removal() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-pending", dead_pid());
        let pending = register_command(&[target.as_path()]);

        assert_eq!(probe_commands(&target), CommandsState::Busy);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_in_use, 1, "{stats:?}");
        assert!(target.exists());

        // Handed back by a spawn that failed, the directory is reclaimable
        // again.
        pending.abandon();
        assert_eq!(probe_commands(&target), CommandsState::Idle);
        assert_eq!(sweep_stale_dirs(root.path()).removed, 1);
    }

    /// Deregistration on the normal path is a `Drop`, and without it every
    /// directory would stay pinned by commands that finished hours ago.
    #[test]
    fn dropping_a_command_lifetime_deregisters_it() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-deregister", dead_pid());
        let registered = register_command(&[target.as_path()]).confirm(live_handle());

        assert_eq!(
            probe_commands(&target),
            CommandsState::Busy,
            "the handle names this test process's group, which is alive"
        );
        drop(registered);
        assert_eq!(probe_commands(&target), CommandsState::Idle);
    }

    /// A command reaches the shim dir and the session dir both, so it is
    /// registered in both, and either entry is enough to save its directory.
    #[test]
    fn a_command_is_registered_in_every_directory_it_can_reach() {
        let root = tempdir().expect("tempdir");
        let shim = claimed_dir(root.path(), "test-two-dirs-shim", dead_pid());
        let session = claimed_dir(root.path(), "test-two-dirs-session", dead_pid());
        let registered =
            register_command(&[shim.as_path(), session.as_path()]).confirm(live_handle());

        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert_eq!(stats.skipped_in_use, 2, "{stats:?}");
        assert!(shim.exists() && session.exists());

        drop(registered);
        assert_eq!(sweep_stale_dirs(root.path()).removed, 2);
    }

    /// An entry whose name is not a number cannot be checked, and anything
    /// that cannot be checked holds the directory.
    #[test]
    fn an_unreadable_registry_entry_blocks_removal() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-garbage-entry", dead_pid());
        std::fs::write(target.join(CMDS_DIR_NAME).join("not-a-pid"), b"").expect("write entry");

        assert_eq!(probe_commands(&target), CommandsState::Busy);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert!(target.exists());
    }

    /// A directory with no registry at all — claimed by an older build, or
    /// never claimed by this crate — is not accountable and is never deleted.
    #[test]
    fn dead_owner_without_a_registry_is_not_swept() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-no-registry", dead_pid());
        std::fs::remove_dir(target.join(CMDS_DIR_NAME)).expect("remove registry");

        assert_eq!(probe_commands(&target), CommandsState::Missing);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert!(target.exists());
    }

    /// A registry that is a file rather than a directory is the shape a
    /// hostile world-writable temp root can plant. It reads as in use.
    #[test]
    fn a_registry_that_is_not_a_directory_blocks_removal() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-registry-is-a-file", dead_pid());
        std::fs::remove_dir(target.join(CMDS_DIR_NAME)).expect("remove registry");
        std::fs::write(target.join(CMDS_DIR_NAME), b"not a directory").expect("write file");

        assert_eq!(probe_commands(&target), CommandsState::Busy);
        assert_eq!(sweep_stale_dirs(root.path()).removed, 0);
        assert!(target.exists());
    }

    /// `claim_dir` has to leave a registry behind. Without one, every
    /// directory this crate creates reads as unaccountable forever and the
    /// sweep reclaims nothing.
    #[test]
    fn claiming_a_directory_creates_an_empty_registry() {
        let root = tempdir().expect("tempdir");
        let dir = root
            .path()
            .join(format!("{OWNER_PREFIX}test-claim-registry"));
        std::fs::create_dir(&dir).expect("mkdir");

        let _claim = claim_dir(&dir).expect("claim");

        assert!(dir.join(CMDS_DIR_NAME).is_dir(), "registry directory");
        assert_eq!(probe_commands(&dir), CommandsState::Idle);
    }

    /// The registry names a process *group*, and that is load-bearing rather
    /// than incidental. A command is a shell that forks, and the shell can
    /// exit while what it started keeps running with the shim dir on `PATH`.
    /// A pid check says the command is gone. The group check does not, and
    /// the group is the same unit `shell::KillGroup` manages.
    #[cfg(unix)]
    #[test]
    fn a_group_whose_leader_exited_is_still_alive() {
        let root = tempdir().expect("tempdir");
        let target = claimed_dir(root.path(), "test-group-outlives-leader", dead_pid());

        // A leader that backgrounds a long sleep and exits immediately. With
        // no job control the sleep stays in the leader's process group.
        let mut leader = {
            use std::os::unix::process::CommandExt as _;
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "sleep 60 & exit 0"]);
            c.process_group(0);
            c.spawn().expect("spawn group leader")
        };
        let pgid = leader.id();
        leader.wait().expect("reap leader");

        assert_eq!(
            pid_liveness(pgid),
            Liveness::Dead,
            "the shell that was the command is gone"
        );
        assert_eq!(
            command_liveness(pgid),
            Liveness::Alive,
            "what it left running is not"
        );

        register_handle(&target, pgid);
        let stats = sweep_stale_dirs(root.path());
        assert_eq!(stats.removed, 0, "{stats:?}");
        assert!(target.exists());

        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pgid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
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
