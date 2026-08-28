//! Run a child process to completion under a hard wall-clock deadline.
//!
//! Every spawn on the discovery path — the CLI auth probes and the login-shell
//! PATH lookups — must return in bounded time no matter how the child behaves.
//! A login shell that blocks on an interactive prompt, a child that traps
//! `SIGTERM`, or a forked descendant that keeps a pipe open must not be able to
//! stall discovery; that stall is what left "Check again" spinning forever.

use std::io::{ErrorKind, Read};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Poll interval while waiting for the child to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Idle backoff for a nonblocking Unix drain that has no bytes available and
/// has not yet been told to stop. Short so a running child's output is pulled
/// promptly and the post-teardown join returns quickly.
#[cfg(unix)]
const DRAIN_IDLE_POLL: Duration = Duration::from_millis(5);

/// Maximum bytes retained across stdout + stderr for one bounded probe.
///
/// Discovery output is tiny — a version string, an auth-status word, a PATH
/// lookup. A probe that emits more than this is noisy or hostile. The ceiling
/// is enforced *in the drain sink* (see [`spawn_drain`]): each stream is pulled
/// on its own thread into a capped buffer, the limit is checked the moment a
/// bounded read crosses it, and the probe is failed closed — so an over-cap
/// payload is never retained in memory (and, since output goes to pipes not
/// temp files, never written to disk). The ceiling is *aggregate*, not
/// per-stream, so a probe cannot double it by splitting output across stdout
/// and stderr.
const CAPTURE_LIMIT: u64 = 1 << 20; // 1 MiB

/// Grace period between the initial `SIGTERM` and the escalating `SIGKILL` for a
/// timed-out process group. Long enough for a well-behaved child to flush and
/// exit cleanly, short enough that a signal-ignoring one is reaped promptly.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_millis(500);

/// Freeze the child so the Job Object can take ownership before any child code
/// runs (see [`BoundedChild::spawn`]).
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Suppress the console window a GUI-spawned console child would otherwise
/// flash — the same suppression [`crate::util::configure_no_window`] applies to
/// non-bounded spawns.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The exact creation flags every bounded child is spawned with.
///
/// `Command::creation_flags` *replaces* rather than accumulates (std ORs only
/// `CREATE_UNICODE_ENVIRONMENT` afterward), and [`BoundedChild::spawn`] is the
/// last writer before spawn, so a caller's earlier `configure_no_window` is
/// wiped. This constant therefore has to carry every flag a bounded child
/// needs, and owning both here keeps the window-suppression contract in one
/// place instead of split between the caller and the helper.
#[cfg(windows)]
const BOUNDED_CREATION_FLAGS: u32 = CREATE_SUSPENDED | CREATE_NO_WINDOW;

/// Compile-time guard: the bounded flags must always carry *both* bits. A
/// future edit that drops `CREATE_NO_WINDOW` (reintroducing the console-flash
/// regression) or `CREATE_SUSPENDED` (reopening the spawn-to-assign race) fails
/// the build on Windows rather than shipping silently.
#[cfg(windows)]
const _: () = {
    assert!(BOUNDED_CREATION_FLAGS & CREATE_SUSPENDED == CREATE_SUSPENDED);
    assert!(BOUNDED_CREATION_FLAGS & CREATE_NO_WINDOW == CREATE_NO_WINDOW);
};

/// A spawned child plus ownership of its descendant tree, torn down on *every*
/// exit path — timeout, error, or successful exit. The two platforms establish
/// ownership differently, and the guarantee is deliberately asymmetric — the
/// adjudicated design, not an oversight:
///
/// - **Unix:** the child leads its own process group (`process_group(0)`), so
///   `killpg` reaches every descendant that has not left the group. A
///   `setsid`/`setpgid` escapee holding a pipe is *not* owned and may survive
///   one probe, yet never hangs the helper (see [`output_with_timeout`]).
/// - **Windows:** the child is spawned `CREATE_SUSPENDED`, assigned to a
///   kill-on-close Job Object while frozen, then resumed. The job owns the root
///   before any descendant can exist and is created without breakaway, so no
///   writer can escape it — a hard whole-tree guarantee. Closing that job reaps
///   the whole tree *even after the root has exited* — the distinction that
///   makes `taskkill /T <pid>` (a live-root lookup) unfit for the success path.
///   This mirrors the Job Object discipline the harness uses to reap its 24
///   agent workers (`process_lifecycle.rs`).
struct BoundedChild {
    child: std::process::Child,
    /// The kill-on-close job that owns the whole tree. Taken and dropped by
    /// `kill_tree` so the reap happens exactly once. Spawn is fail-closed: if
    /// the job cannot be created, assigned, or the child resumed, the child is
    /// terminated and `spawn` returns `None` rather than running unowned.
    #[cfg(windows)]
    job: Option<crate::managed_agents::JobHandle>,
}

impl BoundedChild {
    /// Spawn `command`, establishing tree ownership before the child can run.
    /// Returns `None` if the spawn fails or — on Windows — if the job cannot be
    /// created, assigned, or the frozen child resumed; in every such case the
    /// child is terminated and reaped before returning, so no unowned process
    /// survives.
    fn spawn(mut command: Command) -> Option<Self> {
        // Run the child in its own process group so the whole tree can be torn
        // down as a unit, not just a direct child that may have forked workers.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }

        // Spawn frozen so the Job Object can take ownership before any child
        // code runs and forks a descendant that would escape the job. The flags
        // are set here as the last writer before spawn; `Command::creation_flags`
        // replaces rather than ORs, so `BOUNDED_CREATION_FLAGS` must itself carry
        // `CREATE_NO_WINDOW` — a caller's earlier `configure_no_window` would be
        // clobbered otherwise, flashing a console window on GUI discovery.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(BOUNDED_CREATION_FLAGS);
        }

        // `mut` is used only on the Windows fail-closed path (kill/wait on the
        // frozen child); Unix moves the child unmodified into `Self`.
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut child = command.spawn().ok()?;

        #[cfg(windows)]
        let job = {
            // Assign the frozen child to a kill-on-close job, then resume it.
            // Any failure is fail-closed: terminate + reap the still-owned
            // child and abort the spawn, never run it unowned to the deadline.
            let Some(job) = crate::managed_agents::create_job_for_child(child.id()) else {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            };
            if !crate::managed_agents::resume_process(child.id()) {
                // Dropping the job kills the still-suspended child via
                // kill-on-close; reap it so no zombie lingers.
                drop(job);
                let _ = child.wait();
                return None;
            }
            job
        };

        Some(Self {
            child,
            #[cfg(windows)]
            job: Some(job),
        })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Timeout teardown: a graceful `SIGTERM` to the group and a bounded grace
    /// period for a clean flush on Unix, then the unconditional forced kill.
    /// Windows has no group signal, so it goes straight to the forced kill.
    fn terminate_timed_out(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: `killpg` on the group led by the child; an ignored result
            // is intentional — the group may already be gone (ESRCH).
            unsafe {
                libc::killpg(self.child.id() as i32, libc::SIGTERM);
            }
            std::thread::sleep(KILL_GRACE);
        }
        self.kill_tree();
    }

    /// Forcibly reap the whole tree. Idempotent and safe on an already-exited
    /// tree. Runs on every exit path — including success, because a login shell
    /// or auth CLI can background a descendant that outlives the leader while
    /// still holding the captured-output descriptors.
    fn kill_tree(&mut self) {
        #[cfg(unix)]
        // SAFETY: `killpg` on the group led by the child; ignored result is
        // intentional — `ESRCH` on a dead group is the success case.
        unsafe {
            libc::killpg(self.child.id() as i32, libc::SIGKILL);
        }
        #[cfg(windows)]
        // Closing the kill-on-close job reaps every descendant, even once the
        // root has exited — which `taskkill /T <root>` cannot. `spawn` is
        // fail-closed, so the job is always present until this first take;
        // a later take is a no-op (the tree is already reaped).
        if let Some(job) = self.job.take() {
            drop(job);
        }
    }

    /// Reap the direct child so no zombie lingers after the tree is killed.
    fn reap(&mut self) {
        let _ = self.child.wait();
    }

    /// Take the captured stdout pipe. `Some` because [`output_with_timeout`]
    /// configures `Stdio::piped()` before spawn.
    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// Take the captured stderr pipe.
    fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
}

/// Set a file descriptor nonblocking so a read on it returns `WouldBlock`
/// instead of parking when no bytes are available. Returns `false` on any
/// `fcntl` failure, which the caller treats as fail-closed.
#[cfg(unix)]
fn set_nonblocking<F: std::os::unix::io::AsRawFd>(f: &F) -> bool {
    let fd = f.as_raw_fd();
    // SAFETY: `fd` is owned by `f` for the duration of this call; `F_GETFL` /
    // `F_SETFL` read and set the descriptor's flags without transferring
    // ownership or touching any other resource.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return false;
        }
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == 0
    }
}

/// Drain one child stream on its own thread into a buffer capped by the shared
/// aggregate budget, so the sink itself — not a post-hoc size sample — enforces
/// [`CAPTURE_LIMIT`].
///
/// Continuous draining keeps the pipe buffer from filling, so the child can
/// never block on a full pipe while we poll it. Retention is bounded: `total`
/// reserves a disjoint byte range per chunk across both streams, so the sum of
/// both buffers never exceeds the aggregate cap. The moment a read crosses the
/// cap, `overflow` is set and the drain returns immediately — it does not keep
/// reading, so a writer that keeps the pipe continuously readable cannot spin
/// this loop forever (it must cross the finite cap). A read error other than
/// `Interrupted`/`WouldBlock` returns `Err`, which the caller treats as
/// fail-closed.
///
/// **Bounded completion differs by platform, because tree ownership does.**
/// - **Unix:** the read end is nonblocking (see [`set_nonblocking`]). A killed
///   in-group writer's descriptors close, so the read reaches EOF (`Ok(0)`) and
///   the thread returns normally. But `kill_tree` is a `killpg` on the child's
///   group, which does *not* reach a descendant that left the group via
///   `setsid`/`setpgid` while retaining the pipe; that writer keeps the write
///   end open and EOF never comes. So once teardown has set `stop`, a
///   `WouldBlock` (nothing more buffered) ends the drain rather than waiting on
///   that escaped writer forever. This is what makes bounded return hold
///   *without* depending on every inherited writer exiting — the correction to
///   the round-8 blocking-EOF design.
/// - **Windows:** the read blocks to EOF. That is sound because the whole tree
///   is owned by a kill-on-close Job Object created without
///   `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so no descendant can escape the job; job
///   close reaps every writer and the read reaches EOF. `stop` is unused there.
fn spawn_drain<R: Read + Send + 'static>(
    mut reader: R,
    total: Arc<AtomicU64>,
    overflow: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
    // `stop` gates only the nonblocking Unix drain; the Windows path blocks to
    // the job-close EOF and never consults it.
    #[cfg(windows)]
    let _ = &stop;
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return Ok(buf),
                Ok(n) => {
                    // Atomically reserve [prev, prev + n) of the shared budget;
                    // `prev` is unique per call, so the two streams keep
                    // disjoint ranges and their retained bytes sum to <= cap.
                    let prev = total.fetch_add(n as u64, Ordering::Relaxed);
                    if prev.saturating_add(n as u64) > CAPTURE_LIMIT {
                        overflow.store(true, Ordering::Relaxed);
                        let keep = CAPTURE_LIMIT.saturating_sub(prev).min(n as u64) as usize;
                        buf.extend_from_slice(&chunk[..keep]);
                        // Overflow: the result is already fail-closed, so nothing
                        // still in the pipe is worth preserving. Return NOW rather
                        // than draining to EOF — this is what bounds the `Ok(n)`
                        // path against a writer that keeps the pipe continuously
                        // readable, which would otherwise never reach the
                        // `WouldBlock`/`stop` check below and hang the join. It is
                        // safe to stop draining: the poll loop sees `overflow` and
                        // kills the tree, and a writer that then blocks on a full
                        // pipe dies to `killpg`/job-close. Do NOT "fix" that
                        // blocked-writer case by resuming an unbounded drain here.
                        return Ok(buf);
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                // Nonblocking read (Unix only): no bytes available right now.
                // After teardown, an escaped out-of-group writer is the only
                // thing that could still hold the pipe open, so stop draining it
                // rather than block the join forever; otherwise back off and
                // retry so a running child's later output is still captured.
                #[cfg(unix)]
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(buf);
                    }
                    std::thread::sleep(DRAIN_IDLE_POLL);
                }
                Err(e) => return Err(e),
            }
        }
    })
}

/// Run `command` to completion, bounded by `timeout`.
///
/// Returns `Some(output)` when the child exits within the deadline, `None` when
/// it fails to spawn, exceeds the deadline, or breaches the capture ceiling.
/// Guarantees a bounded return regardless of child cooperation:
///
/// - **Sink-enforced capture bound.** Stdout and stderr are piped to two drain
///   threads that read into buffers capped by a shared aggregate budget
///   ([`spawn_drain`]); nothing over [`CAPTURE_LIMIT`] is ever retained. On a
///   breach the poll loop fails closed — kill the tree, return `None` — so a
///   noisy or hostile probe cannot force unbounded memory (and, with pipes
///   rather than temp files, cannot fill the disk either). Continuous draining
///   also keeps the pipe buffer from filling, so the child can never block on a
///   full pipe while we poll.
/// - **Bounded drain completion without depending on writer death.** Tree
///   teardown runs on *every* exit path before the drains are joined —
///   [`BoundedChild::kill_tree`] on timeout, error, cap breach, *and* success.
///   But teardown alone does not guarantee EOF on Unix: `kill_tree` is a
///   `killpg` on the child's process group, and a descendant that left the
///   group (`setsid`/`setpgid`) while retaining the pipe survives it and keeps
///   the write end open. So the drains do not rely on EOF from every writer:
///   the Unix reads are nonblocking, and after teardown sets the shared `stop`
///   flag a `WouldBlock` (no more buffered bytes) ends each drain. An escaped
///   writer is allowed to survive; the join still returns promptly. On Windows
///   the reads block to EOF, which is sound because the kill-on-close Job Object
///   is created without breakaway, so no writer can escape the job. This is the
///   correction to the round-8 design, whose blocking Unix reads could hang the
///   join forever on a group-escaping writer.
/// - **No wait hang.** The child is polled with [`Child::try_wait`] against the
///   deadline rather than blocked on with `wait()`.
/// - **Tree termination on every exit path.** [`BoundedChild`] tears the tree
///   down whether the child times out, errors, breaches the cap, *or exits
///   successfully* — a login-shell rc file or auth CLI can legitimately
///   background a descendant (`worker &`) that would outlive discovery.
///   Ownership is a hard whole-tree guarantee on Windows but only the child's
///   process group on Unix (the group-escapee case bounded by the drain rule
///   above) — the adjudicated asymmetry. The timeout path additionally sends a
///   graceful `SIGTERM` and a grace period before the kill.
pub(crate) fn output_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = BoundedChild::spawn(command)?;

    let stdout_pipe = child.take_stdout();
    let stderr_pipe = child.take_stderr();

    // Unix: make the parent read ends nonblocking so a drain can be told to stop
    // (post-teardown) instead of parking forever on a group-escaping writer that
    // still holds the pipe. Fail closed if the fd cannot be reconfigured — the
    // child is still fully owned here, so cleanup is just kill + reap.
    #[cfg(unix)]
    {
        let stdout_ok = match stdout_pipe.as_ref() {
            Some(p) => set_nonblocking(p),
            None => true,
        };
        let stderr_ok = match stderr_pipe.as_ref() {
            Some(p) => set_nonblocking(p),
            None => true,
        };
        if !(stdout_ok && stderr_ok) {
            child.kill_tree();
            child.reap();
            return None;
        }
    }

    // Shared drain state: one aggregate byte budget across both streams, an
    // overflow flag the poll loop watches so a streaming producer that never
    // exits is failed closed the moment it crosses the cap, and a stop flag that
    // teardown raises to end the nonblocking Unix drains.
    let total = Arc::new(AtomicU64::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let stdout_drain =
        stdout_pipe.map(|s| spawn_drain(s, total.clone(), overflow.clone(), stop.clone()));
    let stderr_drain =
        stderr_pipe.map(|s| spawn_drain(s, total.clone(), overflow.clone(), stop.clone()));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    child.terminate_timed_out();
                    break None;
                }
                // Fail closed on a capture breach *while the child runs*: the
                // drain kept nothing over the cap; teardown below ends the
                // drains so the join cannot hang.
                if overflow.load(Ordering::Relaxed) {
                    child.kill_tree();
                    break None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                child.kill_tree();
                break None;
            }
        }
    };

    // Tree down on every path (timeout/error/overflow killed it above; a clean
    // exit may still have backgrounded a descendant holding the pipe). Kill is
    // idempotent, so calling it here on the success path is safe. Then raise
    // `stop`: a killed in-group writer's pipe reaches EOF and ends its drain on
    // its own, but a group-escaping writer never will — `stop` ends that drain
    // on the next `WouldBlock` so the joins below return promptly.
    child.kill_tree();
    child.reap();
    stop.store(true, Ordering::Relaxed);

    let stdout = join_drain(stdout_drain);
    let stderr = join_drain(stderr_drain);

    // Fail closed if the child exited within the deadline but overran the cap in
    // a final burst, or if either drain hit a read error (join_drain -> None).
    let (status, stdout, stderr) = (status?, stdout?, stderr?);
    if overflow.load(Ordering::Relaxed) {
        return None;
    }

    Some(Output {
        status,
        stdout,
        stderr,
    })
}

/// Join a drain thread, returning its captured bytes. `None` (fail closed) if
/// the stream was absent, the thread panicked, or the read errored.
fn join_drain(drain: Option<JoinHandle<std::io::Result<Vec<u8>>>>) -> Option<Vec<u8>> {
    match drain {
        Some(handle) => handle.join().ok()?.ok(),
        None => Some(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Drive `output_with_timeout` on its own thread under an independent
    /// wall-clock `bound` — the real outer bound, unreachable by an inline `elapsed()` assertion if the helper hangs.
    /// The raw result lets the Windows sites fold transcripts into the expiry panic.
    #[cfg(any(unix, windows))]
    fn run_watchdogged_raw(
        cmd: Command,
        timeout: Duration,
        bound: Duration,
    ) -> Result<Option<Output>, mpsc::RecvTimeoutError> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(output_with_timeout(cmd, timeout));
        });
        rx.recv_timeout(bound)
    }
    #[cfg(unix)]
    fn run_watchdogged(cmd: Command, timeout: Duration, bound: Duration) -> Option<Output> {
        run_watchdogged_raw(cmd, timeout, bound)
            .unwrap_or_else(|_| panic!("output_with_timeout did not return within {bound:?}"))
    }

    /// True while a Unix process (or a reaped-but-not-waited zombie under this
    /// test process) still exists. `kill(pid, 0)` probes existence without
    /// signalling. Descendants reparent to init on exit, so a survivor stays
    /// probeable; once `kill_tree` reaps it, the pid is gone (ESRCH).
    #[cfg(unix)]
    fn pid_alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[cfg(unix)]
    #[test]
    fn returns_output_for_fast_command() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "printf hi; printf oops 1>&2"]);
        let out = run_watchdogged(cmd, Duration::from_secs(5), Duration::from_secs(10))
            .expect("a fast command must complete within the timeout");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"hi");
        assert_eq!(out.stderr, b"oops");
    }

    // Adversarial: a child that traps and ignores SIGTERM. The old
    // wait-thread + lone-SIGTERM helper never returned for this input; the
    // process-group SIGKILL escalation must reap it inside the grace period.
    // The watchdog thread is the real bound — the helper hanging fails the
    // test rather than hanging it.
    #[cfg(unix)]
    #[test]
    fn kills_sigterm_ignoring_child_within_bound() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "trap '' TERM; while :; do sleep 1; done"]);
        let result = run_watchdogged(cmd, Duration::from_millis(200), Duration::from_secs(5));
        assert!(result.is_none(), "a timed-out child must yield None");
    }

    // Adversarial (success path): the direct child exits 0 but backgrounds a
    // descendant that keeps writing to the inherited stdout/stderr forever.
    // Two guarantees under test: (1) the drain returns rather than blocking on
    // the descendant, and (2) `kill_tree` reaps that descendant before
    // returning, so no survivor keeps consuming CPU after discovery reports
    // success. This is the pass-2 leak Thufir proved with `(yes) & exit 0`.
    #[cfg(unix)]
    #[test]
    fn reaps_backgrounded_descendant_on_success() {
        let pid_file = tempfile::NamedTempFile::new().expect("temp file for descendant pid");
        let pid_path = pid_file
            .path()
            .to_str()
            .expect("utf-8 temp path")
            .to_string();
        // Background a real child process (`sleep`), record ITS pid via `$!`
        // (not `$$`, which in a subshell is the invoking shell), then exit 0.
        // The leader waits until the pid is recorded so the test can read it
        // deterministically even though the success path kills the group at
        // once. `$!` is the pass-2 `(yes) & exit 0` survivor, made observable.
        let script = format!(
            "sleep 30 & echo $! > '{pid_path}'; \
             until [ -s '{pid_path}' ]; do :; done; printf done; exit 0"
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", &script]);
        let out = run_watchdogged(cmd, Duration::from_secs(5), Duration::from_secs(10))
            .expect("the direct child exits, so this must return its output");
        assert!(out.status.success());

        let descendant_pid: i32 = std::fs::read_to_string(&pid_path)
            .expect("descendant must have recorded its PID")
            .trim()
            .parse()
            .expect("descendant PID must be numeric");
        // Give the reaped group a moment to fully disappear, then assert dead.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !pid_alive(descendant_pid),
            "backgrounded descendant {descendant_pid} must be reaped on success, but it survived"
        );
    }

    // Adversarial (timeout path): a SIGTERM-ignoring leader that backgrounds a
    // descendant, both looping forever. The leader's process group is killed on
    // timeout, so the descendant (same group) must die too. The descendant is a
    // real child process whose PID is recorded via `$!`, so the test proves the
    // actual descendant — not the already-reaped leader — reaches ESRCH.
    #[cfg(unix)]
    #[test]
    fn reaps_descendant_on_timeout() {
        let pid_file = tempfile::NamedTempFile::new().expect("temp file for descendant pid");
        let pid_path = pid_file
            .path()
            .to_str()
            .expect("utf-8 temp path")
            .to_string();
        let script = format!(
            "trap '' TERM; sleep 300 & echo $! > '{pid_path}'; \
             while :; do sleep 1; done"
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", &script]);
        let result = run_watchdogged(cmd, Duration::from_millis(300), Duration::from_secs(5));
        assert!(result.is_none(), "a timed-out tree must yield None");

        let descendant_pid: i32 = std::fs::read_to_string(&pid_path)
            .expect("descendant must have written its PID")
            .trim()
            .parse()
            .expect("descendant PID must be numeric");
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !pid_alive(descendant_pid),
            "backgrounded descendant {descendant_pid} must be group-killed on timeout, but it survived"
        );
    }

    // Deterministic seam regression (Thufir's finding): a drain fed a reader
    // that stays continuously readable — every `read` returns `Ok(8192)`, never
    // `WouldBlock` — must still complete, because the `stop`/`WouldBlock` check
    // alone never fires on such a reader. The bound comes from the `Ok(n)` path
    // returning the instant the aggregate cap is crossed. No real process and no
    // scheduler timing: the reader is a pure in-test `Read` impl, so this pins
    // the control flow rather than relying on a descendant eventually blocking.
    // With the round-9-initial code (which kept reading after overflow) the
    // drain never returns and the join below hangs past the watchdog.
    #[test]
    fn overflow_bounds_a_continuously_readable_drain() {
        /// A reader that is always ready with a full 8192-byte chunk. It never
        /// returns 0 (EOF) or `WouldBlock`, so only the overflow return can end
        /// a drain reading it.
        struct AlwaysReady;
        impl Read for AlwaysReady {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                for b in buf.iter_mut() {
                    *b = b'x';
                }
                Ok(buf.len())
            }
        }

        let total = Arc::new(AtomicU64::new(0));
        let overflow = Arc::new(AtomicBool::new(false));
        // `stop` set from the start: a correct drain must NOT depend on it here,
        // since a continuously-ready reader never hits the `WouldBlock` arm that
        // consults it. The overflow return is the only thing that can bound it.
        let stop = Arc::new(AtomicBool::new(true));
        let drain = spawn_drain(AlwaysReady, total.clone(), overflow.clone(), stop);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(drain.join());
        });
        let joined = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("a continuously-readable drain must be bounded by the capture cap");
        let buf = joined
            .expect("drain thread must not panic")
            .expect("drain read must not error");
        assert!(
            overflow.load(Ordering::Relaxed),
            "the drain must have tripped overflow"
        );
        assert!(
            buf.len() as u64 <= CAPTURE_LIMIT,
            "retained bytes {} must not exceed the cap {CAPTURE_LIMIT}",
            buf.len()
        );
    }

    // Adversarial (group escape): the leader backgrounds a descendant that
    // calls `setsid()` — leaving the leader's process group while retaining the
    // inherited stdout — then sleeps 300s; the leader itself loops forever, so
    // the helper times out. `kill_tree` is a `killpg` on the leader's group and
    // cannot reach the escaped descendant, so its pipe write end stays open and
    // never reaches EOF. The helper must still return within the outer watchdog
    // and fail closed: the nonblocking drains stop on `WouldBlock` after
    // teardown rather than blocking on that surviving writer. This is the exact
    // primitive Thufir reproduced against the round-8 blocking-read design; with
    // blocking reads the drain join hangs forever and `run_watchdogged` panics.
    //
    // Non-vacuous: the descendant is asserted *alive* after the helper returns,
    // proving it genuinely escaped the `killpg` (so it was still holding the
    // pipe at join time) — the return therefore came from the stop path, not
    // from an EOF the kill happened to produce. The test then reaps it.
    #[cfg(unix)]
    #[test]
    fn returns_when_escaped_descendant_retains_pipe() {
        let pid_file = tempfile::NamedTempFile::new().expect("temp file for descendant pid");
        let pid_path = pid_file
            .path()
            .to_str()
            .expect("utf-8 temp path")
            .to_string();
        // The perl descendant `setsid()`s out of the leader's group, records its
        // PID, writes a few bytes to the retained stdout, then sleeps. The
        // leader waits until the PID is recorded (so the test can read it) and
        // then loops forever, forcing the timeout path.
        let script = format!(
            "perl -MPOSIX -e 'POSIX::setsid() or die; open(my $f,\">\",$ARGV[0]) or die; \
             print $f $$; close $f; print \"x\" x 4096; sleep 300;' '{pid_path}' & \
             until [ -s '{pid_path}' ]; do :; done; while :; do sleep 1; done"
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", &script]);
        let result = run_watchdogged(cmd, Duration::from_millis(300), Duration::from_secs(5));
        assert!(
            result.is_none(),
            "a timed-out probe must fail closed even when an escaped writer holds the pipe"
        );

        let descendant_pid: i32 = std::fs::read_to_string(&pid_path)
            .expect("escaped descendant must have recorded its PID")
            .trim()
            .parse()
            .expect("descendant PID must be numeric");
        assert!(
            pid_alive(descendant_pid),
            "descendant {descendant_pid} was expected to survive the group kill (proving it escaped)"
        );
        // Reap the escaped writer so the test leaves nothing behind.
        unsafe {
            libc::kill(descendant_pid, libc::SIGKILL);
        }
    }

    // Adversarial (capture bound): a producer that streams zero bytes
    // *indefinitely* — it never exits and never stops writing on its own, so
    // the only thing that can end the probe is the in-flight ceiling check
    // tripping `overflow`, killing the tree, and failing closed (None).
    //
    // The discriminator is `timeout >> bound`: the deadline is 60s but the
    // watchdog fails the test at 10s, so a return within the bound proves the
    // *cap* ended the probe, not the timeout. Neuter the overflow check and the
    // helper runs until the 60s deadline, blowing the 10s watchdog. Pipe
    // backpressure cannot end it either: the drains pull continuously, so `cat`
    // would keep writing forever. Retention stays bounded by construction —
    // `spawn_drain` reserves a disjoint byte range per chunk against the shared
    // budget and discards everything past `CAPTURE_LIMIT` — so no over-cap
    // payload is ever materialized even though the producer is infinite.
    #[cfg(unix)]
    #[test]
    fn fails_closed_when_capture_exceeds_limit() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "exec cat /dev/zero"]);
        let result = run_watchdogged(cmd, Duration::from_secs(60), Duration::from_secs(10));
        assert!(
            result.is_none(),
            "an unbounded producer must fail closed on the capture cap, well before the deadline"
        );
    }

    // The complement of the bound: output at or under the ceiling still returns
    // in full, so the limit rejects only genuine overruns.
    #[cfg(unix)]
    #[test]
    fn returns_full_output_at_capture_limit() {
        let mut cmd = Command::new("/bin/sh");
        // Comfortably under 1 MiB, emitted in one burst then a clean exit.
        cmd.args(["-c", "head -c 4096 /dev/zero"]);
        let out = run_watchdogged(cmd, Duration::from_secs(5), Duration::from_secs(10))
            .expect("output under the limit must be returned");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 4096);
    }

    // ---- Windows tree-ownership verification (Will's box) ----------------
    //
    // No CI lane executes Windows tests for this helper, so these are
    // `#[ignore]`-gated for a sanctioned local run on a real Windows machine:
    //
    //   cargo test -p buzz-desktop --lib bounded_command -- --ignored --nocapture
    //
    // Both assert on the actual PowerShell-recorded descendant PID (not the
    // already-exited root), so neutering the Job Object ownership leaves that
    // PID alive and fails the test — the mutation is observable.

    /// True while a Windows process still exists. Opens with the minimal
    /// query right and reads its exit code: `STILL_ACTIVE` (259) means running,
    /// any other code means exited. A failed open means the PID is gone.
    #[cfg(windows)]
    fn pid_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }

    /// Read a PID that a probe wrote to `path`, retrying briefly since the
    /// descendant records it asynchronously. Dumps `logs` on failure so a remote
    /// run diagnoses itself instead of panicking blind.
    #[cfg(windows)]
    fn read_recorded_pid(path: &str, logs: &[&str]) -> u32 {
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    return pid;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "descendant never recorded its PID at {path}\n{}",
            dump_logs(logs)
        );
    }

    /// Write a PowerShell payload to `path` as a `.ps1` file. Invoking these via
    /// `powershell -File` avoids the Rust-std → cmd.exe → powershell quoting
    /// gauntlet that silently mangled the inline `-Command` fixtures (the root
    /// exited without its payload ever running), so the payload reaches
    /// PowerShell verbatim.
    #[cfg(windows)]
    fn write_ps1(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).expect("write .ps1 payload");
    }

    /// Collect the named transcript files (each written by the fixture's
    /// PowerShell) into one string for a self-diagnosing assert message. Missing
    /// files are reported as such rather than skipped.
    #[cfg(windows)]
    fn dump_logs(paths: &[&str]) -> String {
        let mut out = String::from("---- fixture transcripts ----\n");
        for p in paths {
            out.push_str(&format!("[{p}]\n"));
            match std::fs::read_to_string(p) {
                Ok(text) if text.is_empty() => out.push_str("(empty)\n"),
                Ok(text) => {
                    out.push_str(&text);
                    if !text.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Err(e) => out.push_str(&format!("(unreadable: {e})\n")),
            }
        }
        out
    }

    // Success path, run in a loop to hammer the spawn/assign race. A PowerShell
    // root (no cmd.exe anywhere) launches a hidden, detached PowerShell
    // descendant via `Start-Process -WindowStyle Hidden`; the descendant records
    // its own PID and sleeps. The root then waits synchronously until the PID
    // file is non-empty before exiting 0 — without that wait the root would exit
    // in the same tick, the success path would close the kill-on-close job
    // immediately, and the descendant would be reaped mid-cold-start before it
    // could record its PID, starving the test of its evidence. The descendant is
    // still born inside the job (suspend → assign → resume, no breakaway), so the
    // reaping guarantee under test is unchanged; only the delivery mechanism (a
    // `.ps1` via `-File`, not a mangled inline `-Command`) is fixed. Every assert
    // dumps the PowerShell transcripts so a remote failure is self-diagnosing.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows host; run manually with --ignored"]
    fn reaps_backgrounded_descendant_on_success_windows() {
        for iteration in 0..25 {
            let dir = tempfile::tempdir().expect("temp dir for fixture scripts");
            let pid_path = dir.path().join("descendant.pid");
            let child_ps1 = dir.path().join("child.ps1");
            let root_ps1 = dir.path().join("root.ps1");
            let root_log = dir.path().join("root.log");
            let child_log = dir.path().join("child.log");
            let pid_s = pid_path.to_str().expect("utf-8 pid path");
            let root_log_s = root_log.to_str().expect("utf-8 root log");
            let child_log_s = child_log.to_str().expect("utf-8 child log");

            write_ps1(
                &child_ps1,
                &format!(
                    "$PID | Set-Content -Encoding ascii -Path '{pid_s}'\n\
                     Add-Content -Path '{child_log_s}' -Value \"descendant $PID started\"\n\
                     Start-Sleep -Seconds 30\n"
                ),
            );
            write_ps1(
                &root_ps1,
                &format!(
                    "Add-Content -Path '{root_log_s}' -Value \"root $PID launching descendant\"\n\
                     Start-Process -FilePath 'powershell' -WindowStyle Hidden -ArgumentList \
                     '-NoProfile','-ExecutionPolicy','Bypass','-File','{child}'\n\
                     $deadline = (Get-Date).AddSeconds(15)\n\
                     while (((-not (Test-Path '{pid_s}')) -or ((Get-Item '{pid_s}').Length -eq 0)) \
                     -and (Get-Date) -lt $deadline) {{ Start-Sleep -Milliseconds 50 }}\n\
                     Add-Content -Path '{root_log_s}' -Value \"root observed pid file, exiting\"\n\
                     exit 0\n",
                    child = child_ps1.to_str().expect("utf-8 child path"),
                ),
            );

            let mut cmd = Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                root_ps1.to_str().expect("utf-8 root path"),
            ]);
            let out = run_watchdogged_raw(cmd, Duration::from_secs(20), Duration::from_secs(40))
                .ok()
                .flatten()
                .unwrap_or_else(|| {
                    panic!(
                        "iteration {iteration}: root exits, so this must return output\n{}",
                        dump_logs(&[root_log_s, child_log_s])
                    )
                });
            assert!(
                out.status.success(),
                "iteration {iteration}: root must exit 0\nstdout={}\nstderr={}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
                dump_logs(&[root_log_s, child_log_s])
            );

            let descendant_pid = read_recorded_pid(pid_s, &[root_log_s, child_log_s]);
            std::thread::sleep(Duration::from_millis(300));
            assert!(
                !pid_alive(descendant_pid),
                "iteration {iteration}: descendant {descendant_pid} must be reaped on success, but it survived\n{}",
                dump_logs(&[root_log_s, child_log_s])
            );
        }
    }

    // Timeout path: a PowerShell root launches a hidden, detached PowerShell
    // descendant (records its PID, sleeps 300s), waits synchronously until the
    // PID file is non-empty, then enters its own 300s block so the helper's
    // deadline fires inside it. The helper must time out and close the job,
    // reaping both. The synchronous wait is the evidence — the descendant's PID
    // is recorded before the root reaches the block the deadline fires in, so the
    // reap cannot kill it mid-cold-start and starve the assert. Same `.ps1`
    // delivery as the success fixture (no cmd tokenizer), and every assert dumps
    // the transcripts.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows host; run manually with --ignored"]
    fn reaps_descendant_on_timeout_windows() {
        let dir = tempfile::tempdir().expect("temp dir for fixture scripts");
        let pid_path = dir.path().join("descendant.pid");
        let child_ps1 = dir.path().join("child.ps1");
        let root_ps1 = dir.path().join("root.ps1");
        let root_log = dir.path().join("root.log");
        let child_log = dir.path().join("child.log");
        let pid_s = pid_path.to_str().expect("utf-8 pid path");
        let root_log_s = root_log.to_str().expect("utf-8 root log");
        let child_log_s = child_log.to_str().expect("utf-8 child log");

        write_ps1(
            &child_ps1,
            &format!(
                "$PID | Set-Content -Encoding ascii -Path '{pid_s}'\n\
                 Add-Content -Path '{child_log_s}' -Value \"descendant $PID started\"\n\
                 Start-Sleep -Seconds 300\n"
            ),
        );
        write_ps1(
            &root_ps1,
            &format!(
                "Add-Content -Path '{root_log_s}' -Value \"root $PID launching descendant\"\n\
                 Start-Process -FilePath 'powershell' -WindowStyle Hidden -ArgumentList \
                 '-NoProfile','-ExecutionPolicy','Bypass','-File','{child}'\n\
                 $deadline = (Get-Date).AddSeconds(15)\n\
                 while (((-not (Test-Path '{pid_s}')) -or ((Get-Item '{pid_s}').Length -eq 0)) \
                 -and (Get-Date) -lt $deadline) {{ Start-Sleep -Milliseconds 50 }}\n\
                 Add-Content -Path '{root_log_s}' -Value \"root observed pid file, blocking\"\n\
                 Start-Sleep -Seconds 300\n",
                child = child_ps1.to_str().expect("utf-8 child path"),
            ),
        );

        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            root_ps1.to_str().expect("utf-8 root path"),
        ]);
        let result = run_watchdogged_raw(cmd, Duration::from_secs(20), Duration::from_secs(40))
            .unwrap_or_else(|_| {
                panic!(
                    "watchdog expired — output_with_timeout hung on the timeout path\n{}",
                    dump_logs(&[root_log_s, child_log_s])
                )
            });
        assert!(
            result.is_none(),
            "a timed-out tree must yield None\n{}",
            dump_logs(&[root_log_s, child_log_s])
        );

        let descendant_pid = read_recorded_pid(pid_s, &[root_log_s, child_log_s]);
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !pid_alive(descendant_pid),
            "descendant {descendant_pid} must be job-killed on timeout, but it survived\n{}",
            dump_logs(&[root_log_s, child_log_s])
        );
    }
}
