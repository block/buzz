//! Run a child process to completion under a hard wall-clock deadline.
//!
//! Every spawn on the discovery path — the CLI auth probes and the login-shell
//! PATH lookups — must return in bounded time no matter how the child behaves.
//! A login shell that blocks on an interactive prompt, a child that traps
//! `SIGTERM`, or a forked descendant that keeps a pipe open must not be able to
//! stall discovery; that stall is what left "Check again" spinning forever.

use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// Poll interval while waiting for the child to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Grace period between the initial `SIGTERM` and the escalating `SIGKILL` for a
/// timed-out process group. Long enough for a well-behaved child to flush and
/// exit cleanly, short enough that a signal-ignoring one is reaped promptly.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_millis(500);

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
/// - **Hard termination.** On timeout the child's whole process group is torn
///   down (see [`terminate`]) — `SIGTERM` then an escalating `SIGKILL` on Unix,
///   `Child::kill` on Windows — so a `SIGTERM`-trapping child or a forked
///   descendant cannot outlive the deadline.
pub(crate) fn output_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    let mut stdout_file = tempfile::tempfile().ok()?;
    let mut stderr_file = tempfile::tempfile().ok()?;

    command
        .stdin(std::process::Stdio::null())
        .stdout(stdout_file.try_clone().ok()?)
        .stderr(stderr_file.try_clone().ok()?);

    // Run the child in its own process group so a timeout can terminate the
    // whole tree, not just a direct child that may have forked workers.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }

    let mut child = command.spawn().ok()?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    terminate(&mut child);
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                terminate(&mut child);
                return None;
            }
        }
    };

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

/// Tear down a timed-out child and its process group with a bounded return.
///
/// The child is a group leader (`process_group(0)`), so its PID is also the
/// group ID. `SIGTERM` the whole group for a clean shutdown, wait the fixed
/// [`KILL_GRACE`], then unconditionally `SIGKILL` the group so a signal-ignoring
/// child or descendant cannot survive. `wait` then reaps the (now-killed) direct
/// child. Every step is time-bounded, so this returns in at most `KILL_GRACE`.
#[cfg(unix)]
fn terminate(child: &mut std::process::Child) {
    let pgid = child.id() as i32;
    // SAFETY: `killpg` with a valid PID/PGID; unused result is intentional —
    // the group may already be gone.
    unsafe {
        libc::killpg(pgid, libc::SIGTERM);
    }
    std::thread::sleep(KILL_GRACE);
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Windows teardown: `Child::kill` maps to `TerminateProcess`, a guaranteed kill
/// of the child (not mere PID consumption). `wait` reaps it so no handle leaks.
#[cfg(not(unix))]
fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn returns_output_for_fast_command() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "printf hi; printf oops 1>&2"]);
        let out = output_with_timeout(cmd, Duration::from_secs(5))
            .expect("a fast command must complete within the timeout");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"hi");
        assert_eq!(out.stderr, b"oops");
    }

    // Adversarial: a child that traps and ignores SIGTERM. The old
    // wait-thread + lone-SIGTERM helper never returned for this input; the
    // process-group SIGKILL escalation must reap it inside the grace period.
    #[cfg(unix)]
    #[test]
    fn kills_sigterm_ignoring_child_within_bound() {
        let start = Instant::now();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "trap '' TERM; while :; do sleep 1; done"]);
        let result = output_with_timeout(cmd, Duration::from_millis(200));
        assert!(result.is_none(), "a timed-out child must yield None");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "must return promptly after the deadline even when SIGTERM is ignored, \
             took {:?}",
            start.elapsed()
        );
    }

    // Adversarial: the direct child exits immediately but leaves a background
    // descendant holding the inherited stdout/stderr descriptors open. A pipe
    // read would block on that descendant; the temp-file capture must return at
    // EOF regardless, so the call cannot hang.
    #[cfg(unix)]
    #[test]
    fn returns_when_descendant_retains_pipes() {
        let start = Instant::now();
        let mut cmd = Command::new("/bin/sh");
        // Fork a grandchild that survives the parent and keeps fd 1/2 open.
        cmd.args(["-c", "printf done; (sleep 30) & exit 0"]);
        let out = output_with_timeout(cmd, Duration::from_secs(5))
            .expect("the direct child exits, so this must return its output");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"done");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "must not block on a descendant holding the output descriptors, took {:?}",
            start.elapsed()
        );
    }
}
