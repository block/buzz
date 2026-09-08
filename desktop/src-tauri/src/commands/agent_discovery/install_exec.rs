//! Execution of runtime install commands: spawning the built command,
//! draining its output under a timeout, and retrying transient failures.
//!
//! Command *construction* stays in the parent module (`install_shell_command`,
//! `install_powershell_command`, `build_install_command`); this module owns
//! only what happens once a `Command` exists.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::install_capture::{drain_into, Capture, LineObserver};
use super::install_report::{InstallOutcome, InstallReporter};
use crate::managed_agents::InstallStepResult;

/// Maximum number of attempts for a transient-looking install command.
const INSTALL_MAX_ATTEMPTS: u32 = 3;

/// Absolute wall-clock ceiling for a single install command.
///
/// This is a ceiling, not an inactivity timeout: nothing observable
/// distinguishes a hung installer from one silently transferring a large
/// artifact (the Goose step downloads a ~79MB release asset with no progress
/// output, and npm at its default log level prints only at the end), so silence
/// alone never kills an install. The previous 300s wall killed
/// slow-but-working installs — Windows Defender scanning every file npm
/// extracts pushes past it routinely (#2401).
///
/// The cost of a larger ceiling: skipping onboarding does not cancel a running
/// install and a per-runtime guard rejects a second one, so this is also the
/// longest a user who skipped a genuinely *hung* install waits before Install
/// works again in Settings. User-facing cancellation is the product-level fix.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

/// How long the group gets to exit on SIGTERM before the ceiling escalates to
/// SIGKILL.
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(1);

/// How long the ceiling waits after killing the install's process group —
/// applied separately to reaping the killed child and to the output drains
/// finishing. The kill closes the pipe write ends, so both normally complete
/// within microseconds; the bound covers the cases where they don't (a process
/// that escaped the group and still holds a pipe, or a termination that failed
/// outright). Neither may hold the install — nor the per-runtime concurrency
/// guard behind it — open past the ceiling.
const POST_KILL_GRACE: Duration = Duration::from_secs(2);

/// Run an install command, retrying transient failures with backoff.
///
/// Runtime installs pull artifacts over the network — Goose's `curl … | bash`
/// fetches a native release-asset tarball from GitHub's CDN with no retry of
/// its own, and the npm adapter installs hit the registry. A single blip there
/// currently fails onboarding outright. This retries a command that ran to
/// completion but exited nonzero (the transient-download signature) up to
/// `INSTALL_MAX_ATTEMPTS` times. Failures with no exit code — a timeout or a
/// shell that never spawned — are not retried, since re-running them just costs
/// the user more time without a plausible path to success.
///
/// Every attempt is recorded through `reporter`, so the install log holds the
/// full retry history even though the UI only ever sees the last attempt.
pub(super) fn run_install_command_with_retry(
    step: &str,
    command: &str,
    reporter: &InstallReporter,
) -> InstallStepResult {
    run_install_with_retry(
        INSTALL_MAX_ATTEMPTS,
        |attempt| {
            // Before the command spawns, so the previous attempt's last line
            // stops being displayed for the whole backoff rather than until the
            // new attempt happens to print something.
            reporter.start_attempt();
            let outcome = run_install_command(step, command, reporter.line_observer());
            reporter.record_attempt(attempt, outcome)
        },
        std::thread::sleep,
    )
}

/// Core retry loop, decoupled from the real command runner and clock so it can
/// be unit-tested without spawning shells or sleeping. `run` receives the
/// 1-based attempt number.
fn run_install_with_retry(
    max_attempts: u32,
    mut run: impl FnMut(u32) -> InstallStepResult,
    mut sleep: impl FnMut(std::time::Duration),
) -> InstallStepResult {
    let mut attempt = 1;
    loop {
        let result = run(attempt);
        if result.success || !install_failure_is_retryable(&result) || attempt >= max_attempts {
            return if attempt > 1 && !result.success {
                annotate_retry_attempts(result, attempt)
            } else {
                result
            };
        }
        sleep(install_retry_backoff(attempt));
        attempt += 1;
    }
}

/// Only retry commands that actually ran and exited nonzero — the signature of
/// a transient download failure. A missing exit code means the command timed
/// out or the shell failed to spawn, neither of which a retry is likely to fix.
fn install_failure_is_retryable(result: &InstallStepResult) -> bool {
    !result.success && result.exit_code.is_some()
}

/// Linear backoff: 3s before attempt 2, 6s before attempt 3.
fn install_retry_backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_secs(3 * attempt as u64)
}

/// Prefix the surfaced error so the UI shows the install was retried rather than
/// failed on a single unlucky attempt.
fn annotate_retry_attempts(mut result: InstallStepResult, attempts: u32) -> InstallStepResult {
    result.stderr = format!(
        "install failed after {attempts} attempts (retried with backoff)\n{}",
        result.stderr
    );
    result
}

/// Build the install command and point it at a writable working directory.
///
/// A packaged desktop launch inherits `/` as its working directory, and
/// installers that write relative to the CWD then fail on a read-only root, so
/// they run from Buzz's own default workdir instead (#2245).
///
/// This is the only command builder [`run_install_command`] calls, so anything
/// it spawns is guaranteed to carry the workdir — which is what makes the
/// working directory assertable without spawning a real login shell.
fn prepare_install_command(command: &str) -> Result<std::process::Command, String> {
    let mut cmd = super::build_install_command(command)?;
    if let Some(workdir) = crate::managed_agents::default_agent_workdir() {
        cmd.current_dir(workdir);
    }
    Ok(cmd)
}

fn run_install_command(
    step: &str,
    command: &str,
    observer: Option<LineObserver>,
) -> InstallOutcome {
    let mut cmd = match prepare_install_command(command) {
        Ok(cmd) => cmd,
        Err(hint) => {
            return InstallOutcome::synthesized(InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: "no suitable shell found for install commands".to_string(),
                exit_code: None,
                hint: Some(hint),
            });
        }
    };

    let child = match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallOutcome::synthesized(InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("failed to spawn shell: {e}"),
                exit_code: None,
                hint: None,
            });
        }
    };

    await_install_child(step, command, child, INSTALL_TIMEOUT, observer)
}

/// Drain a spawned install child's output into bounded buffers and wait for it
/// to exit, killing it at `timeout`.
///
/// Split from the spawn so the timing-sensitive half is testable without a real
/// login shell: shell startup alone can outlast a short test ceiling on a
/// loaded machine. Production always passes [`INSTALL_TIMEOUT`].
fn await_install_child(
    step: &str,
    command: &str,
    mut child: std::process::Child,
    timeout: Duration,
    observer: Option<LineObserver>,
) -> InstallOutcome {
    // Drain stdout/stderr on background threads to prevent pipe buffer
    // deadlock. Each drain feeds a bounded capture the main thread can read at
    // any time, so a timeout can still surface whatever the install printed
    // before it stalled.
    let stdout_capture = Arc::new(Capture::new());
    let stderr_capture = Arc::new(Capture::new());
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // One event stream carries every input the ceiling waits on, so the exit
    // and the drains are governed by the same deadline instead of the exit
    // releasing the drains from it.
    let (events_tx, events) = std::sync::mpsc::channel();

    std::thread::spawn({
        let (capture, done, observer) = (
            Arc::clone(&stdout_capture),
            events_tx.clone(),
            observer.clone(),
        );
        move || {
            if let Some(pipe) = stdout_pipe {
                drain_into(pipe, &capture, observer.as_ref());
            }
            let _ = done.send(Settled::Drained);
        }
    });
    std::thread::spawn({
        let (capture, done) = (Arc::clone(&stderr_capture), events_tx.clone());
        move || {
            if let Some(pipe) = stderr_pipe {
                drain_into(pipe, &capture, observer.as_ref());
            }
            let _ = done.send(Settled::Drained);
        }
    });

    // Save the PID before moving `child` into the wait thread so we can
    // kill the process on timeout.
    let child_pid = child.id();

    std::thread::spawn(move || {
        let _ = events_tx.send(Settled::Exited(child.wait()));
    });

    // No thread is ever joined. Each sends its one event before exiting, so a
    // join after a complete settle would add nothing — and a join before one
    // would reintroduce the unbounded wait this loop exists to prevent.
    let mut settle = Settle::default();
    let ended = settle.collect(&events, Instant::now() + timeout);
    if ended == Collected::Deadline {
        // Ceiling reached: kill the install's whole process group — the install
        // shell is a session leader (`setsid` in its `pre_exec`), so signalling
        // only the leader would leave descendants running and holding the
        // output pipes open.
        //
        // Whether the leader had already exited decides the verdict. If it had,
        // only a descendant was holding a drain open: the install genuinely
        // finished and its real status stands. If it had not, the install itself
        // was still running and this is a timeout — the status the kill produces
        // moments later describes the kill, not the install, so it is discarded.
        let install_finished = settle.status.is_some();
        terminate_install_group(child_pid);
        // Reaping the child and finishing the drains share one bound. Both
        // normally complete within microseconds of the kill, which closes the
        // pipes; when they don't — a process that escaped the group still
        // holding a pipe, or a termination that failed outright — waiting would
        // defeat the very ceiling that fired and keep the per-runtime install
        // guard behind it closed. Stragglers are detached instead; the captures
        // are read under the lock either way.
        settle.collect(&events, Instant::now() + POST_KILL_GRACE);
        if !install_finished {
            return failed_with_capture(
                step,
                command,
                timeout_message(timeout),
                &stdout_capture,
                &stderr_capture,
            );
        }
    }

    match settle.status {
        Some(Ok(status)) => InstallOutcome {
            step: InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: status.success(),
                stdout: stdout_capture.ui(),
                stderr: stderr_capture.ui(),
                exit_code: status.code(),
                hint: None,
            },
            log_stdout: stdout_capture.log(),
            log_stderr: stderr_capture.log(),
        },
        Some(Err(e)) => failed_with_capture(
            step,
            command,
            format!("failed to check process status: {e}"),
            &stdout_capture,
            &stderr_capture,
        ),
        // Every sender is gone without an exit ever arriving.
        None => failed_with_capture(
            step,
            command,
            "internal error: install wait ended without a status".to_string(),
            &stdout_capture,
            &stderr_capture,
        ),
    }
}

/// One input the ceiling waits on.
enum Settled {
    Exited(std::io::Result<std::process::ExitStatus>),
    Drained,
}

/// How a bounded [`Settle::collect`] ended.
#[derive(PartialEq, Debug)]
enum Collected {
    /// The child exited and both drains reached EOF.
    Complete,
    /// The deadline passed first.
    Deadline,
    /// Every sender is gone — a thread died without reporting.
    Disconnected,
}

/// What the install has settled so far: the child's exit status once it is
/// known, and how many of the two drains have reached EOF.
///
/// Collecting is resumable, so the ceiling can fold more events into the same
/// state under a second, post-kill deadline.
#[derive(Default)]
struct Settle {
    status: Option<std::io::Result<std::process::ExitStatus>>,
    drained: usize,
}

impl Settle {
    const DRAINS: usize = 2;

    fn is_complete(&self) -> bool {
        self.status.is_some() && self.drained >= Self::DRAINS
    }

    /// Fold events until the install has fully settled or `deadline` passes.
    ///
    /// The exit and the drains share one deadline deliberately: a shell can exit
    /// while a descendant it left behind still holds the inherited output pipes,
    /// and waiting on those drains outside the deadline would let such a
    /// descendant outlast the ceiling — holding the per-runtime install guard,
    /// which is the very failure the ceiling exists to prevent.
    fn collect(
        &mut self,
        events: &std::sync::mpsc::Receiver<Settled>,
        deadline: Instant,
    ) -> Collected {
        while !self.is_complete() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(remaining) {
                Ok(Settled::Exited(status)) => self.status = Some(status),
                Ok(Settled::Drained) => self.drained += 1,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Collected::Deadline,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Collected::Disconnected
                }
            }
        }
        Collected::Complete
    }
}

/// Kill the install's process group, escalating on the *tree's* liveness.
///
/// The install ceiling owns this rather than reusing
/// `managed_agents::terminate_process`, which escalates to SIGKILL only while
/// the group *leader* is still running: a descendant that ignores SIGTERM
/// outlives the leader, keeps the output pipes open, and never receives the
/// group SIGKILL. The ceiling's contract is that nothing survives it, and the
/// shared helper's escalation is load-bearing for the agent stop/restore paths,
/// so the stricter rule lives here instead of changing it for them.
///
/// Nothing is returned: every outcome — including a signal that could not be
/// delivered at all — has the same handling, the bounded waits at the call
/// site.
#[cfg(unix)]
fn terminate_install_group(pid: u32) {
    signal_install_tree(pid, libc::SIGTERM);
    let deadline = Instant::now() + TERM_GRACE;
    while install_tree_is_alive(pid) {
        if Instant::now() >= deadline {
            signal_install_tree(pid, libc::SIGKILL);
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Signal every process in `pid`'s group, falling back to the leader alone when
/// the group cannot be signalled — the leader may have changed groups, or macOS
/// may refuse one member — since killing the install shell beats killing
/// nothing.
#[cfg(unix)]
fn signal_install_tree(pid: u32, signal: i32) {
    if unsafe { libc::kill(-(pid as i32), signal) } != 0 {
        unsafe { libc::kill(pid as i32, signal) };
    }
}

/// Whether anything the ceiling aimed at is still running: a member of the
/// process group, or the leader itself.
#[cfg(unix)]
fn install_tree_is_alive(pid: u32) -> bool {
    signal_reaches(-(pid as i32)) || signal_reaches(pid as i32)
}

/// `kill(target, 0)` distinguishes "nothing there" (`ESRCH`) from every other
/// outcome. Anything ambiguous — notably `EPERM` for a member we may not
/// signal — counts as alive, so an unclear answer escalates rather than
/// declaring the tree dead.
#[cfg(unix)]
fn signal_reaches(target: i32) -> bool {
    if unsafe { libc::kill(target, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Windows has no process groups on this path: `terminate_process` runs
/// `taskkill /T /F`, which is already tree-wide and unconditional, so there is
/// no escalation to get wrong.
#[cfg(not(unix))]
fn terminate_install_group(pid: u32) {
    let _ = crate::managed_agents::terminate_process(pid);
}

/// A failure carrying whatever the drains captured, with `reason` leading
/// stderr so the surfaced message names the failure before the install's own
/// output.
fn failed_with_capture(
    step: &str,
    command: &str,
    reason: String,
    stdout: &Capture,
    stderr: &Capture,
) -> InstallOutcome {
    InstallOutcome {
        step: InstallStepResult {
            step: step.to_string(),
            command: command.to_string(),
            success: false,
            stdout: stdout.ui(),
            stderr: lead_with_reason(&reason, stderr.ui()),
            exit_code: None,
            hint: None,
        },
        log_stdout: stdout.log(),
        log_stderr: lead_with_reason(&reason, stderr.log()),
    }
}

/// Put `reason` ahead of the install's own stderr, so the surfaced message names
/// the failure before the output. An empty capture leaves the reason alone,
/// without a dangling separator.
fn lead_with_reason(reason: &str, captured: String) -> String {
    if captured.is_empty() {
        reason.to_string()
    } else {
        format!("{reason}\n{captured}")
    }
}

/// Name the limit that fired and its value, so a ceiling kill is
/// distinguishable from the installer's own failure.
fn timeout_message(timeout: Duration) -> String {
    let secs = timeout.as_secs();
    let limit = if secs >= 60 {
        format!("{}-minute", secs / 60)
    } else {
        format!("{secs}-second")
    };
    format!("install command exceeded the {limit} ceiling and was terminated")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── install retry ─────────────────────────────────────────────────────────

    /// Build an `InstallStepResult` with just the fields the retry loop reads.
    fn step_result(success: bool, exit_code: Option<i32>, stderr: &str) -> InstallStepResult {
        InstallStepResult {
            step: "cli".to_string(),
            command: "curl … | bash".to_string(),
            success,
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code,
            hint: None,
        }
    }

    #[test]
    fn test_retryable_only_for_nonzero_exit() {
        // Ran to completion but exited nonzero — the transient-download signature.
        assert!(install_failure_is_retryable(&step_result(
            false,
            Some(1),
            ""
        )));
        // No exit code — timeout or shell-never-spawned; retry won't help.
        assert!(!install_failure_is_retryable(&step_result(false, None, "")));
        // Success is never retryable.
        assert!(!install_failure_is_retryable(&step_result(
            true,
            Some(0),
            ""
        )));
    }

    #[test]
    fn test_retry_backoff_is_linear() {
        assert_eq!(install_retry_backoff(1), std::time::Duration::from_secs(3));
        assert_eq!(install_retry_backoff(2), std::time::Duration::from_secs(6));
    }

    #[test]
    fn test_retry_stops_on_first_success() {
        let mut calls = 0;
        let mut sleeps = 0;
        let result = run_install_with_retry(
            3,
            |_| {
                calls += 1;
                step_result(true, Some(0), "")
            },
            |_| sleeps += 1,
        );
        assert!(result.success);
        assert_eq!(calls, 1, "a first-attempt success must not re-run");
        assert_eq!(sleeps, 0, "no backoff sleep when nothing is retried");
    }

    #[test]
    fn test_retry_recovers_after_transient_failure() {
        let mut calls = 0;
        let result = run_install_with_retry(
            3,
            |attempt| {
                calls += 1;
                // Fail the first attempt with a nonzero exit, then succeed.
                step_result(attempt >= 2, Some(if attempt >= 2 { 0 } else { 1 }), "blip")
            },
            |_| {},
        );
        assert!(result.success);
        assert_eq!(calls, 2, "should retry once then succeed");
        // A recovered install must not carry the retry-failure annotation.
        assert!(!result.stderr.contains("attempts"));
    }

    #[test]
    fn test_retry_does_not_retry_unretryable_failure() {
        let mut calls = 0;
        let result = run_install_with_retry(
            3,
            |_| {
                calls += 1;
                step_result(false, None, "timed out")
            },
            |_| {},
        );
        assert!(!result.success);
        assert_eq!(calls, 1, "a failure with no exit code must not be retried");
        assert_eq!(
            result.stderr, "timed out",
            "unretried failure is unannotated"
        );
    }

    #[test]
    fn test_retry_exhausts_attempts_and_annotates() {
        let mut calls = 0;
        let mut sleeps = 0;
        let result = run_install_with_retry(
            3,
            |_| {
                calls += 1;
                step_result(false, Some(1), "download failed")
            },
            |_| sleeps += 1,
        );
        assert!(!result.success);
        assert_eq!(calls, 3, "must try exactly max_attempts times");
        assert_eq!(
            sleeps, 2,
            "backoff sleeps between attempts, not after the last"
        );
        assert!(
            result.stderr.contains("after 3 attempts"),
            "exhausted retries must surface the attempt count, got: {}",
            result.stderr
        );
        assert!(
            result.stderr.contains("download failed"),
            "original stderr must be preserved"
        );
    }

    // ── install working directory ─────────────────────────────────────────────

    /// Every install child must run from Buzz's writable default workdir. A
    /// packaged launch inherits `/`, where installers that write relative to
    /// the CWD fail on a read-only root (#2245).
    ///
    /// Asserts the prepared `Command` rather than spawning one: `run_install_command`
    /// would start a real login shell, which is neither hermetic nor fast.
    #[test]
    fn test_prepared_install_command_uses_default_workdir() {
        let expected = crate::managed_agents::default_agent_workdir()
            .expect("a default workdir must resolve on any test host");

        let cmd = prepare_install_command("echo test").expect("install shell must resolve");

        assert_eq!(cmd.get_current_dir(), Some(expected.as_path()));
    }

    // ── install ceiling ───────────────────────────────────────────────────────

    /// The ceiling is Will's ruling: 15 minutes, and the error names the limit
    /// that fired so a ceiling kill is not mistaken for the installer's own
    /// failure.
    #[test]
    fn test_ceiling_is_fifteen_minutes_and_error_names_it() {
        assert_eq!(INSTALL_TIMEOUT, Duration::from_secs(900));
        assert!(
            timeout_message(INSTALL_TIMEOUT).contains("15-minute"),
            "got: {}",
            timeout_message(INSTALL_TIMEOUT)
        );
    }

    /// Spawn `script` under `sh` as a process-group leader with piped output —
    /// the same shape [`run_install_command`] hands to
    /// [`await_install_child`], minus the login shell whose own startup can
    /// outlast a short test ceiling.
    #[cfg(unix)]
    fn spawn_group_leader(script: &str) -> std::process::Child {
        use std::os::unix::process::CommandExt;

        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(script);
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("sh must spawn")
    }

    /// A command killed by the ceiling must surface what it printed before
    /// stalling — that partial output is the only evidence of where the install
    /// got stuck — and must stay unretryable, since re-running a hang just
    /// costs the user another ceiling.
    #[cfg(unix)]
    #[test]
    fn test_ceiling_returns_captured_output_and_stays_unretryable() {
        let child = spawn_group_leader("echo out-before-hang; echo err-before-hang >&2; sleep 60");

        let started = Instant::now();
        let outcome = await_install_child("cli", "install", child, Duration::from_secs(5), None);
        let result = &outcome.step;

        assert!(!result.success);
        assert_eq!(result.exit_code, None, "a killed command has no exit code");
        assert!(
            !install_failure_is_retryable(result),
            "a ceiling kill must not be retried"
        );
        assert!(
            result.stdout.contains("out-before-hang"),
            "stdout captured before the stall must survive, got: {:?}",
            result.stdout
        );
        assert!(
            result.stderr.contains("5-second ceiling"),
            "stderr must name the ceiling that actually fired, got: {:?}",
            result.stderr
        );
        assert!(
            result.stderr.contains("err-before-hang"),
            "stderr captured before the stall must survive, got: {:?}",
            result.stderr
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the ceiling must not wait on the hung command's own exit"
        );
        assert!(
            outcome.log_stderr.contains("err-before-hang"),
            "the log record of a ceiling kill must carry the output too, got: {:?}",
            outcome.log_stderr
        );
    }

    /// A failure whose stream captured nothing surfaces the reason alone — no
    /// dangling separator from an empty capture.
    #[test]
    fn test_failure_with_no_captured_output_reports_only_the_reason() {
        let result = failed_with_capture(
            "cli",
            "curl … | bash",
            "boom".to_string(),
            &Capture::new(),
            &Capture::new(),
        )
        .step;

        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "boom");
    }

    // ── post-kill settle bound ────────────────────────────────────────────────

    /// A sender that never arrives — the shape of a failed termination, whose
    /// child is never reaped — must not extend the wait past its deadline.
    #[test]
    fn test_settling_on_a_message_that_never_arrives_stops_at_the_deadline() {
        let (_tx, events) = std::sync::mpsc::channel::<Settled>();

        let started = Instant::now();
        let ended = Settle::default().collect(&events, started + Duration::from_millis(200));

        assert_eq!(ended, Collected::Deadline);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the wait must end at its deadline, took {:?}",
            started.elapsed()
        );
    }

    /// An exit alone is not a settle: the drains are inputs to the same wait, so
    /// a shell that exited while a descendant holds a pipe still hits the
    /// deadline instead of being released from it.
    #[test]
    fn test_exit_without_drains_still_hits_the_deadline() {
        let (tx, events) = std::sync::mpsc::channel();
        tx.send(Settled::Exited(Ok(exit_status_zero()))).unwrap();

        let started = Instant::now();
        let mut settle = Settle::default();
        let ended = settle.collect(&events, started + Duration::from_millis(200));

        assert_eq!(
            ended,
            Collected::Deadline,
            "a leader exit must not complete the settle while a drain is outstanding"
        );
        assert!(settle.status.is_some(), "the exit status must be retained");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// The settle completes only when the exit and both drains have arrived, and
    /// it is resumable: state folded under the first deadline carries into the
    /// post-kill one.
    #[test]
    fn test_settle_completes_on_exit_plus_both_drains_and_resumes() {
        let (tx, events) = std::sync::mpsc::channel();
        tx.send(Settled::Drained).unwrap();

        let mut settle = Settle::default();
        assert_eq!(
            settle.collect(&events, Instant::now() + Duration::from_millis(50)),
            Collected::Deadline
        );

        tx.send(Settled::Exited(Ok(exit_status_zero()))).unwrap();
        tx.send(Settled::Drained).unwrap();

        assert_eq!(
            settle.collect(&events, Instant::now() + Duration::from_secs(5)),
            Collected::Complete,
            "the second collect must build on the first's state, not restart it"
        );
    }

    /// Exit status of a trivially successful command, for driving `Settle`
    /// without a real install.
    fn exit_status_zero() -> std::process::ExitStatus {
        std::process::Command::new("true")
            .status()
            .expect("run `true`")
    }

    /// A shell can exit while a descendant it left behind still holds the
    /// inherited output pipes. If the exit released the drains from the
    /// deadline, that descendant would hold the install — and the per-runtime
    /// concurrency guard behind it — open indefinitely, which is exactly the
    /// failure the ceiling exists to prevent. The leader here exits in
    /// milliseconds; only the descendant outlives the ceiling.
    #[cfg(unix)]
    #[test]
    fn test_promptly_exited_leader_with_a_pipe_holding_descendant_still_obeys_the_ceiling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("lingering.pid");
        let child = spawn_group_leader(&format!(
            "sh -c 'echo $$ > {pid}; sleep 120' & exit 3",
            pid = pidfile.display()
        ));

        let started = Instant::now();
        let outcome = await_install_child("cli", "install", child, Duration::from_secs(2), None);

        assert!(
            started.elapsed() < Duration::from_secs(30),
            "a descendant holding the pipe must not outlast the ceiling, took {:?}",
            started.elapsed()
        );
        assert_eq!(
            outcome.step.exit_code,
            Some(3),
            "the leader's real status outranks the ceiling's verdict once it is known"
        );
        // The deadline must still reach the kill on this path: a leader exit that
        // skipped termination would leave the descendant running with the pipes
        // open, which is the defect itself rather than a detail of it.
        let pid = recorded_pid(&pidfile);
        assert!(
            await_death(pid),
            "descendant {pid} survived — a leader exit must not skip the ceiling's kill"
        );
    }

    /// Wait up to 3s for `pid` to disappear.
    #[cfg(unix)]
    fn await_death(pid: u32) -> bool {
        for _ in 0..30 {
            if !crate::managed_agents::process_is_running(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Read the pid a test descendant recorded for itself.
    #[cfg(unix)]
    fn recorded_pid(pidfile: &std::path::Path) -> u32 {
        for _ in 0..50 {
            if let Ok(text) = std::fs::read_to_string(pidfile) {
                if let Ok(pid) = text.trim().parse() {
                    return pid;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the descendant never recorded its pid at {pidfile:?}");
    }

    /// The install shell is a process-group leader, and its descendants inherit
    /// the output pipes. Killing only the leader leaves them running and the
    /// drains blocked on a pipe nobody will close, so the ceiling kills the
    /// whole group.
    #[cfg(unix)]
    #[test]
    fn test_ceiling_kills_descendants_holding_the_output_pipe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("descendant.pid");
        let child = spawn_group_leader(&format!(
            "sh -c 'echo $$ > {pid}; sleep 60' & echo leader-up; sleep 60",
            pid = pidfile.display()
        ));

        let started = Instant::now();
        let outcome = await_install_child("cli", "install", child, Duration::from_secs(5), None);
        let result = &outcome.step;

        assert!(!result.success);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the drains must not block on a descendant's inherited pipe"
        );

        let pid = recorded_pid(&pidfile);
        assert!(
            await_death(pid),
            "descendant {pid} survived the ceiling kill — the group was not signalled"
        );
    }

    /// Escalation must key off the group, not the leader: a descendant that
    /// ignores SIGTERM outlives the leader, and if SIGKILL is skipped because
    /// the leader is gone it keeps running with the output pipes open — past the
    /// ceiling, and past the concurrency guard that blocks the next install.
    #[cfg(unix)]
    #[test]
    fn test_ceiling_kills_sigterm_ignoring_descendant() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("stubborn.pid");
        // An ignored disposition survives exec, so the descendant's own `sleep`
        // ignores SIGTERM too — nothing in that subtree dies without SIGKILL.
        let child = spawn_group_leader(&format!(
            "sh -c 'trap \"\" TERM; echo $$ > {pid}; sleep 60' & echo leader-up; sleep 60",
            pid = pidfile.display()
        ));

        let started = Instant::now();
        let outcome = await_install_child("cli", "install", child, Duration::from_secs(5), None);
        let result = &outcome.step;

        assert!(!result.success);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "a SIGTERM-ignoring descendant must not hold the ceiling open"
        );

        let pid = recorded_pid(&pidfile);
        assert!(
            await_death(pid),
            "SIGTERM-ignoring descendant {pid} survived — escalation followed the leader, not the group"
        );
    }
}
