//! Run a child process to completion under a hard wall-clock deadline.
//!
//! Every spawn on the discovery path — the CLI auth probes and the login-shell
//! PATH lookups — must return in bounded time no matter how the child behaves.
//! A login shell that blocks on an interactive prompt, a child that traps
//! `SIGTERM`, or a forked descendant that keeps a pipe open must not be able to
//! stall discovery; that stall is what left "Check again" spinning forever.

use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// Poll interval while waiting for the child to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Grace period between the initial `SIGTERM` and the escalating `SIGKILL` for a
/// timed-out process group. Long enough for a well-behaved child to flush and
/// exit cleanly, short enough that a signal-ignoring one is reaped promptly.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_millis(500);

/// A spawned child plus ownership of its entire descendant tree, so the tree
/// can be torn down on *every* exit path — timeout, error, or successful exit.
///
/// The two platforms establish ownership differently, but both own the tree for
/// the guard's whole lifetime rather than looking it up after the fact:
///
/// - **Unix:** the child leads its own process group (`process_group(0)`), so
///   `killpg` reaches every descendant that has not left the group.
/// - **Windows:** the child is assigned to a kill-on-close Job Object at spawn.
///   Closing that job reaps the whole tree *even after the root has exited* —
///   the distinction that makes `taskkill /T <pid>` (a live-root lookup) unfit
///   for the success path, where the root is already gone by the time we tear
///   down. This mirrors the Job Object discipline the harness already uses to
///   reap its 24 agent workers (`process_lifecycle.rs`).
struct BoundedChild {
    child: std::process::Child,
    /// `Some` while the job owns the tree; taken and dropped by `kill_tree` so
    /// the reap happens exactly once. `None` means job assignment failed at
    /// spawn — teardown then falls back to killing the direct child only.
    #[cfg(windows)]
    job: Option<crate::managed_agents::JobHandle>,
}

impl BoundedChild {
    /// Spawn `command`, establishing tree ownership before returning. Returns
    /// `None` only if the spawn itself fails; a Windows job-assignment failure
    /// is retained as degraded (`job: None`) rather than aborting the spawn.
    fn spawn(mut command: Command) -> Option<Self> {
        // Run the child in its own process group so the whole tree can be torn
        // down as a unit, not just a direct child that may have forked workers.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }

        let child = command.spawn().ok()?;

        #[cfg(windows)]
        let job = {
            let job = crate::managed_agents::create_job_for_child(child.id());
            if job.is_none() {
                eprintln!(
                    "buzz-desktop: bounded command could not take a Job Object; \
                     a backgrounded descendant may outlive teardown"
                );
            }
            job
        };

        Some(Self {
            child,
            #[cfg(windows)]
            job,
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
        match self.job.take() {
            // Closing the kill-on-close job reaps every descendant, even once
            // the root has exited — which `taskkill /T <root>` cannot.
            Some(job) => drop(job),
            // Job assignment failed at spawn (already logged): no tree
            // guarantee. Fail closed by killing the direct child; a descendant
            // it backgrounded may still leak.
            None => {
                let _ = self.child.kill();
            }
        }
    }

    /// Reap the direct child so no zombie lingers after the tree is killed.
    fn reap(&mut self) {
        let _ = self.child.wait();
    }
}

/// Run `command` to completion, bounded by `timeout`.
///
/// Returns `Some(output)` when the child exits within the deadline, `None` when
/// it fails to spawn or is killed for exceeding it. Guarantees a bounded return
/// regardless of child cooperation:
///
/// - **No pipe-drain hang.** Stdout and stderr are captured to regular temp
///   files rather than pipes. A pipe's read blocks until every writer — the
///   child *and* any descendant that inherited the write end — closes it; a
///   forked worker outliving its parent could hold it open forever. A regular
///   file returns EOF at its current write position no matter who inherits the
///   descriptor, so the post-exit read is bounded on every platform. There are
///   no background drain threads to join, so no drain-thread hang exists.
/// - **No wait hang.** The child is polled with [`Child::try_wait`] against the
///   deadline rather than blocked on with `wait()`.
/// - **Hard tree termination on every exit path.** [`BoundedChild`] owns the
///   child's whole descendant tree (Unix process group / Windows Job Object)
///   and tears it down whether the child times out, errors, *or exits
///   successfully*, before the captured output is read. Success is not an
///   exemption: a login-shell rc file or an auth CLI can legitimately background
///   a descendant (`worker &`) that would otherwise outlive discovery. The
///   timeout path additionally sends a graceful `SIGTERM` and a bounded grace
///   period before the final kill.
pub(crate) fn output_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    let mut stdout_file = tempfile::tempfile().ok()?;
    let mut stderr_file = tempfile::tempfile().ok()?;

    command
        .stdin(Stdio::null())
        .stdout(stdout_file.try_clone().ok()?)
        .stderr(stderr_file.try_clone().ok()?);

    let mut child = BoundedChild::spawn(command)?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    child.terminate_timed_out();
                    child.reap();
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                child.kill_tree();
                child.reap();
                return None;
            }
        }
    };

    // Leader exited within the deadline. Tear the tree down before reading, so
    // no descendant it backgrounded keeps the captured-output descriptors (or a
    // busy loop) alive after discovery reports success.
    child.kill_tree();
    child.reap();

    Some(Output {
        status,
        stdout: read_captured(&mut stdout_file),
        stderr: read_captured(&mut stderr_file),
    })
}

/// Rewind a captured-output temp file and read it in full. Bounded because the
/// file is regular: the read reaches EOF at the current write position.
fn read_captured(file: &mut std::fs::File) -> Vec<u8> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    if file.seek(SeekFrom::Start(0)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    let _ = file.read_to_end(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Run `output_with_timeout` under an independent wall-clock watchdog. The
    /// helper is driven on its own thread; if it fails to return within `bound`
    /// the test fails instead of hanging forever. This is the real outer bound —
    /// an inline `elapsed() < bound` assertion is never reached if the helper
    /// itself hangs.
    #[cfg(unix)]
    fn run_watchdogged(cmd: Command, timeout: Duration, bound: Duration) -> Option<Output> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(output_with_timeout(cmd, timeout));
        });
        match rx.recv_timeout(bound) {
            Ok(result) => result,
            Err(_) => panic!("output_with_timeout did not return within {bound:?}"),
        }
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
    // Two guarantees under test: (1) the temp-file capture returns at EOF
    // rather than blocking on the descendant, and (2) `kill_tree` reaps that
    // descendant before returning, so no survivor keeps consuming disk/CPU
    // after discovery reports success. This is the pass-2 leak Thufir proved
    // with `(yes) & exit 0`.
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
}
