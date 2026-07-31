//! Execution of runtime install commands: spawning the built command,
//! draining its output under a timeout, and retrying transient failures.
//!
//! Command *construction* stays in the parent module (`install_shell_command`,
//! `install_powershell_command`, `build_install_command`); this module owns
//! only what happens once a `Command` exists.

use std::io::Read;

use crate::managed_agents::InstallStepResult;

/// Maximum number of attempts for a transient-looking install command.
const INSTALL_MAX_ATTEMPTS: u32 = 3;

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
pub(super) fn run_install_command_with_retry(step: &str, command: &str) -> InstallStepResult {
    run_install_with_retry(
        INSTALL_MAX_ATTEMPTS,
        |_attempt| run_install_command(step, command),
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

fn run_install_command(step: &str, command: &str) -> InstallStepResult {
    let mut cmd = match prepare_install_command(command) {
        Ok(cmd) => cmd,
        Err(hint) => {
            return InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: "no suitable shell found for install commands".to_string(),
                exit_code: None,
                hint: Some(hint),
            };
        }
    };

    let mut child = match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("failed to spawn shell: {e}"),
                exit_code: None,
                hint: None,
            };
        }
    };

    // Drain stdout/stderr on background threads to prevent pipe buffer deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });

    // Save the PID before moving `child` into the wait thread so we can
    // kill the process on timeout.
    let child_pid = child.id();

    let (tx, rx) = std::sync::mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    // 5-minute timeout for install commands.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            // Timeout: kill the child process via its PID, then join all
            // threads so nothing leaks.
            #[cfg(unix)]
            unsafe {
                libc::kill(child_pid as i32, libc::SIGTERM);
            }
            #[cfg(windows)]
            {
                let _ = crate::managed_agents::taskkill_tree(child_pid);
            }
            drop(rx);
            let _ = wait_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return InstallStepResult {
                step: step.to_string(),
                command: command.to_string(),
                success: false,
                stdout: String::new(),
                stderr: "install command timed out after 5 minutes".to_string(),
                exit_code: None,
                hint: None,
            };
        }

        match rx.recv_timeout(std::time::Duration::from_millis(200).min(remaining)) {
            Ok(Ok(status)) => {
                let _ = wait_thread.join();
                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr_raw = stderr_thread.join().unwrap_or_default();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: status.success(),
                    stdout: truncate_output(stdout),
                    stderr: truncate_output(stderr_raw),
                    exit_code: status.code(),
                    hint: None,
                };
            }
            Ok(Err(e)) => {
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: format!("failed to check process status: {e}"),
                    exit_code: None,
                    hint: None,
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Still running; loop and check deadline again.
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // wait_thread dropped sender without sending — shouldn't happen.
                let _ = wait_thread.join();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return InstallStepResult {
                    step: step.to_string(),
                    command: command.to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: "internal error: wait thread disconnected".to_string(),
                    exit_code: None,
                    hint: None,
                };
            }
        }
    }
}

/// Cap output to head + tail to avoid flooding the UI with large error dumps,
/// while preserving the most useful parts of the output.
fn truncate_output(s: String) -> String {
    const HEAD: usize = 512;
    const TAIL: usize = 1024;
    const LIMIT: usize = HEAD + TAIL;
    if s.len() <= LIMIT {
        return s;
    }
    let head_end = floor_char_boundary(&s, HEAD);
    let tail_start = floor_char_boundary(&s, s.len().saturating_sub(TAIL));
    let omitted = tail_start - head_end;
    format!(
        "{}\n... ({omitted} bytes omitted) ...\n{}",
        &s[..head_end],
        &s[tail_start..]
    )
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    index = index.min(s.len());
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
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

    // ── output truncation ─────────────────────────────────────────────────────

    /// Output within the cap is passed through byte-for-byte — no marker, no loss.
    #[test]
    fn test_truncate_output_leaves_short_output_untouched() {
        let short = "a".repeat(1536);

        assert_eq!(truncate_output(short.clone()), short);
    }

    /// Over the cap, both ends survive and the middle is replaced by a marker
    /// naming the omitted byte count — the head keeps the command's opening
    /// context and the tail keeps the error that usually trails.
    #[test]
    fn test_truncate_output_keeps_head_and_tail_with_marker() {
        let input = format!(
            "{}{}{}",
            "H".repeat(512),
            "M".repeat(4000),
            "T".repeat(1024)
        );

        let out = truncate_output(input);

        assert!(out.starts_with(&"H".repeat(512)));
        assert!(out.ends_with(&"T".repeat(1024)));
        assert!(
            out.contains("... (4000 bytes omitted) ..."),
            "marker must name the omitted byte count, got: {out}"
        );
    }

    /// Truncation must not split a multi-byte character. Cutting mid-codepoint
    /// would panic on the slice; the boundary floor prevents it.
    #[test]
    fn test_truncate_output_does_not_split_multibyte_characters() {
        // "é" is 2 bytes, so every candidate cut index lands mid-character.
        let input = "é".repeat(4000);

        let out = truncate_output(input);

        assert!(out.contains("bytes omitted"), "input must exceed the cap");
        assert!(!out.contains('\u{fffd}'), "no replacement chars: {out}");
    }
}
