//! The model-call seam: a trait plus the proven subprocess implementation.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::Error;

/// Executes one fold run's model call. Implementations receive the exact
/// priced `model_input` string and return raw model output text.
///
/// This is the only impurity seam in the crate. A future implementation can
/// route through `buzz-agent`'s provider client; v1 ships the subprocess
/// runner below.
pub trait FoldRunner {
    /// Run the model over `model_input` and return its output text.
    fn run(&self, model_input: &str, model: &str) -> Result<String, Error>;
}

/// Default max thinking-token cap passed to the CLI. 1024 keeps citation
/// accuracy — fully disabling thinking made models drop citations; unbounded
/// thinking dominated run latency.
const DEFAULT_MAX_THINKING_TOKENS: &str = "1024";

/// Phrases an unauthenticated CLI prints (with exit 0) instead of output.
const SENTINELS: &[&str] = &["not logged in", "please run /login", "invalid api key"];

/// Shells out to `claude -p --system-prompt <minimal>` with input on stdin.
///
/// A fold run is a pure text transform, but a plain agent CLI call is a full
/// harness: its own large system prompt, memory/context injection, and
/// unbounded extended thinking. So we pass a minimal transform-engine system
/// prompt and cap thinking (override with `BUZZ_FOLD_MAX_THINKING_TOKENS`;
/// set it empty to keep the CLI default). The binary defaults to `claude`,
/// overridable with `BUZZ_FOLD_RUNNER_BIN`.
///
/// Hardening (each gate is load-bearing, learned on real runs):
/// - exit != 0 is invalid even with partial stdout (mid-stream crash);
/// - exit 0 is not trusted: empty output, or a short output that is
///   essentially just an auth sentinel ("Not logged in"), is refused —
///   a real digest may legitimately *discuss* login issues, so the sentinel
///   check applies only to outputs ≤ 80 chars;
/// - a hard timeout kills the child rather than waiting forever — on unix the
///   whole process group, so a wrapper script cannot leave a billed model
///   call running as an orphan;
/// - pipe drains are joined with a deadline, so a grandchild inheriting the
///   pipes cannot wedge the run after the child exits (the group is killed
///   in that case too).
pub struct SubprocessRunner {
    binary: String,
    timeout: Duration,
}

/// How long after child exit the pipe drains may take to finish.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Kill the child — on unix the entire process group it leads (see
/// `process_group(0)` below), elsewhere the direct child only.
fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    // SAFETY: plain syscall; a negative pid signals the process group.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Default for SubprocessRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SubprocessRunner {
    /// Runner with the environment-configured binary and a 600s timeout.
    pub fn new() -> Self {
        Self {
            binary: std::env::var("BUZZ_FOLD_RUNNER_BIN").unwrap_or_else(|_| "claude".to_string()),
            timeout: Duration::from_secs(600),
        }
    }

    /// Runner with an explicit binary and timeout (tests, pinned installs).
    pub fn with_binary(binary: impl Into<String>, timeout: Duration) -> Self {
        Self {
            binary: binary.into(),
            timeout,
        }
    }

    /// The minimal system prompt that turns an agent CLI into a transform engine.
    pub const SYSTEM_PROMPT: &'static str = "You are a headless text-transform engine. There is \
        no filesystem, no tools, no memory. Read the task and input, output ONLY the requested \
        document — no preamble, no questions.";

    fn command(&self, model: &str) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg("--system-prompt")
            .arg(Self::SYSTEM_PROMPT);
        if !model.is_empty() {
            cmd.arg("--model").arg(model);
        }
        let thinking = std::env::var("BUZZ_FOLD_MAX_THINKING_TOKENS")
            .unwrap_or_else(|_| DEFAULT_MAX_THINKING_TOKENS.to_string());
        if !thinking.is_empty() {
            cmd.env("MAX_THINKING_TOKENS", thinking);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Lead a fresh process group so a timeout kill reaches every
        // descendant (the model CLI may itself be a wrapper script).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd
    }

    fn wait_with_deadline(&self, child: &mut Child) -> Result<std::process::ExitStatus, Error> {
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {
                    if start.elapsed() > self.timeout {
                        kill_group(child);
                        return Err(Error::Runner(format!(
                            "fold runner timed out after {}s",
                            self.timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(Error::Runner(format!("failed to wait on fold runner: {e}"))),
            }
        }
    }
}

/// First 200 chars, for error snippets.
fn snip(s: &str) -> String {
    s.chars().take(200).collect()
}

/// Drain a child pipe to a string on a background thread (avoids pipe-buffer
/// deadlock against the concurrent stdin write). The result arrives on a
/// channel so the caller can bound how long it waits: a grandchild holding
/// the pipe open must not wedge the run after the child exits.
fn drain<R: Read + Send + 'static>(
    reader: Option<R>,
    label: &'static str,
) -> std::sync::mpsc::Receiver<Result<String, Error>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let result = match reader {
            Some(mut r) => r
                .read_to_string(&mut text)
                .map(|_| text)
                .map_err(|e| Error::Runner(format!("failed reading runner {label}: {e}"))),
            None => Ok(text),
        };
        let _ = tx.send(result);
    });
    rx
}

impl FoldRunner for SubprocessRunner {
    fn run(&self, model_input: &str, model: &str) -> Result<String, Error> {
        let mut child = self.command(model).spawn().map_err(|e| {
            Error::Runner(format!(
                "failed to launch fold runner {:?}: {e}",
                self.binary
            ))
        })?;
        let stdin = child.stdin.take();
        let input = model_input.to_string();
        let writer = std::thread::spawn(move || {
            if let Some(mut pipe) = stdin {
                let _ = pipe.write_all(input.as_bytes());
            }
        });
        let stdout = drain(child.stdout.take(), "stdout");
        let stderr = drain(child.stderr.take(), "stderr");
        let status = self.wait_with_deadline(&mut child)?;
        let _ = writer.join();
        let mut recv = |rx: std::sync::mpsc::Receiver<Result<String, Error>>,
                        label: &str|
         -> Result<String, Error> {
            rx.recv_timeout(DRAIN_TIMEOUT).map_err(|_| {
                // Something (a grandchild) is still holding the pipe open
                // after exit — kill the group so nothing billed lives on.
                kill_group(&mut child);
                Error::Runner(format!(
                    "runner {label} still open {}s after exit (orphaned descendant?); killed \
                     the process group and treated the run as failed",
                    DRAIN_TIMEOUT.as_secs()
                ))
            })?
        };
        let out = recv(stdout, "stdout")?;
        let err = recv(stderr, "stderr")?;
        let out = out.trim().to_string();
        if !status.success() {
            return Err(Error::Runner(format!(
                "fold runner exited {status} — treating output as invalid. stdout={:?} stderr={:?}",
                snip(&out),
                snip(&err)
            )));
        }
        let low = out.to_lowercase();
        let is_sentinel = SENTINELS.iter().any(|s| low.contains(s));
        if out.is_empty() || (is_sentinel && out.chars().count() <= 80) {
            return Err(Error::Runner(format!(
                "fold runner produced no usable output (auth or empty-response failure). \
                 exit={status} stdout={:?} stderr={:?}",
                snip(&out),
                snip(&err)
            )));
        }
        Ok(out)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// Write an executable stub script and return its path.
    fn stub(name: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("buzz-accumulator-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    fn runner(path: &std::path::Path) -> SubprocessRunner {
        SubprocessRunner::with_binary(path.to_string_lossy(), Duration::from_secs(20))
    }

    #[test]
    fn missing_binary_fails_cleanly() {
        let r =
            SubprocessRunner::with_binary("/nonexistent/buzz-fold-runner", Duration::from_secs(1));
        let err = r.run("input", "haiku");
        assert!(matches!(err, Err(Error::Runner(msg)) if msg.contains("failed to launch")));
    }

    #[test]
    fn passes_input_on_stdin_and_returns_stdout() {
        let path = stub("echo-runner.sh", "cat -");
        let out = runner(&path).run("the exact input", "haiku").expect("run");
        assert_eq!(out, "the exact input");
    }

    #[test]
    fn nonzero_exit_invalidates_partial_stdout() {
        let path = stub("crash-runner.sh", "echo partial output; exit 3");
        let err = runner(&path).run("input", "haiku");
        assert!(
            matches!(err, Err(Error::Runner(msg)) if msg.contains("treating output as invalid"))
        );
    }

    #[test]
    fn short_sentinel_output_is_rejected_even_on_exit_zero() {
        let path = stub(
            "auth-runner.sh",
            "echo 'Not logged in · Please run /login'; exit 0",
        );
        let err = runner(&path).run("input", "haiku");
        assert!(matches!(err, Err(Error::Runner(msg)) if msg.contains("no usable output")));
    }

    #[test]
    fn long_output_discussing_login_is_not_rejected() {
        let path = stub(
            "discuss-runner.sh",
            "printf '%s' 'A long digest that happens to mention someone was not logged in during the incident window, plus plenty of other prose to exceed the sentinel length gate.'",
        );
        let out = runner(&path).run("input", "haiku").expect("run");
        assert!(out.contains("not logged in"));
    }

    #[test]
    fn empty_output_is_rejected() {
        let path = stub("silent-runner.sh", "exit 0");
        let err = runner(&path).run("input", "haiku");
        assert!(matches!(err, Err(Error::Runner(msg)) if msg.contains("no usable output")));
    }

    #[test]
    fn grandchild_holding_the_pipe_cannot_wedge_the_run() {
        // The child exits immediately but leaves a background grandchild
        // holding stdout open. The drain must give up on a deadline and kill
        // the group instead of blocking until the grandchild finishes.
        let path = stub(
            "orphan-runner.sh",
            "(sleep 30; echo late) & echo now; exit 0",
        );
        let started = Instant::now();
        let err = runner(&path).run("input", "haiku");
        assert!(
            matches!(err, Err(Error::Runner(msg)) if msg.contains("still open")),
            "expected drain-deadline error"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "run should fail on the drain deadline, not the grandchild's sleep"
        );
    }
}
