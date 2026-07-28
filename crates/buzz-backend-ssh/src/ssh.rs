//! Transport: the system `ssh` binary, driven with a generated script on stdin.
//!
//! `ssh` rather than a Rust SSH library, because the alternative is
//! reimplementing `~/.ssh/config`, `ProxyJump`, agent forwarding, known-hosts
//! policy and Tailscale's `ProxyCommand` — badly, in a security-sensitive
//! place. The system client already has all of it, configured the way the user
//! configured it.
//!
//! **Every op sends its script over stdin to a remote `sh -s`.** That is what
//! makes the crate's central invariant true by construction: the remote `ps` is
//! world-readable and the desktop's redaction has no reach there, so a secret
//! on the remote argv would leak the agent identity to every user on the box.
//! With the script on stdin the remote argv is the literal string `sh -s` and
//! the local argv is the ssh options — neither ever carries a credential.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::protocol::{Failure, SshConfig};

/// Remote output is wrapped into this provider's own stdout, which the desktop
/// caps at 1 MB. Refusing to buffer more than that here makes the failure a
/// clear message instead of an OOM.
const OUTPUT_CAP: usize = 1_048_576;

/// Deliberately not `Debug`: `stderr` is raw remote output, and only
/// [`Output::failure`] runs it through the credential scrubber.
pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }

    /// A one-line failure description, credential-scrubbed.
    pub fn failure(&self) -> String {
        let detail = crate::protocol::snippet(&self.stderr);
        let code = self
            .status
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "killed by signal".to_string());
        if detail.is_empty() {
            format!("ssh failed ({code})")
        } else {
            format!("ssh failed ({code}): {detail}")
        }
    }
}

/// A configured `ssh` invocation. Holds no connection — each `run` is one
/// process, and each op is one `run`.
pub struct Session {
    binary: PathBuf,
    args: Vec<String>,
}

impl Session {
    /// `accept_new_host_key` must be true only for addresses that came from the
    /// Tailscale device list. Those are reached over an already
    /// WireGuard-authenticated transport, so trust-on-first-use adds nothing.
    /// For a manually typed host it would convert the user's own known-hosts
    /// decision into a silent default, which is a real MITM window.
    pub fn new(config: &SshConfig, accept_new_host_key: bool) -> Result<Self, String> {
        let binary = resolve_ssh().ok_or("ssh client not found on PATH")?;
        let mut args = vec![
            // Removes every interactive prompt structurally, which is what
            // makes "this provider never asks for, transmits, or stores a
            // password" a property of the code rather than a promise.
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ConnectTimeout=10".into(),
            "-o".into(),
            format!(
                "StrictHostKeyChecking={}",
                if accept_new_host_key {
                    "accept-new"
                } else {
                    "ask"
                }
            ),
            // With BatchMode, `ask` cannot prompt — it declines. Keep ssh's own
            // diagnosis on stderr and nothing else.
            "-o".into(),
            "LogLevel=ERROR".into(),
        ];
        if let Some(port) = config.port {
            args.push("-p".into());
            args.push(port.to_string());
        }
        if let Some(identity) = &config.identity_file {
            args.push("-i".into());
            args.push(identity.clone());
        }
        args.push("--".into());
        args.push(config.target());
        // The remote argv, in full. Everything else arrives on stdin.
        args.push("sh -s".into());
        Ok(Self { binary, args })
    }

    /// Feed `script` to the remote shell and collect its output.
    ///
    /// A tailnet ACL asking for a browser re-auth is classified here rather
    /// than at the five call sites: `ssh` prints the URL and then blocks, so
    /// every op would otherwise burn its whole budget — 8s to 300s — and report
    /// a bare timeout for something one click fixes. Owning it here also keeps
    /// the marker out of the callers, so no one is tempted to match on the
    /// string.
    pub fn run(&self, script: &str, timeout: Duration) -> Result<Output, Failure> {
        let mut command = Command::new(&self.binary);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_no_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("failed to run {}: {e}", self.binary.display()))?;

        // Write on a thread: a script larger than the pipe buffer would
        // otherwise deadlock against a remote that is still starting up.
        let payload = script.to_string();
        let mut stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(stdin) = stdin.as_mut() {
                let _ = stdin.write_all(payload.as_bytes());
            }
            drop(stdin);
        });

        let stdout = Drain::start(child.stdout.take());
        let stderr = Drain::start(child.stderr.take());

        // Poll to a deadline rather than blocking on `wait`, the repo's standard
        // pattern (`discovery::probe_codex_acp_major_version`).
        let deadline = Instant::now() + timeout;
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code()),
                Ok(None) => {
                    // Scanning what has arrived, rather than waiting for EOF,
                    // is what turns the whole budget into one poll interval:
                    // the auth prompt is printed and *then* ssh blocks. Checked
                    // before the deadline so the timeout path cannot discard an
                    // answer already sitting in the buffer.
                    if let Some(url) = stderr.with_bytes(crate::tailscale::auth_url_in) {
                        break Err(Failure::tailscale_auth(url));
                    }
                    if Instant::now() >= deadline {
                        break Err(stderr.with_bytes(|buffered| timed_out(timeout, buffered)));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => break Err(format!("ssh wait failed: {e}").into()),
            }
        };
        if outcome.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Joined before the error is returned either way: the writer holds the
        // stdin handle, and a live handle keeps a killed child's pipe open.
        let _ = writer.join();

        let status = outcome?;
        let stderr = stderr.finish();
        // The same classification for a host that printed the URL and then
        // exited on its own, so both shapes reach the caller as one failure.
        if let Some(url) = crate::tailscale::auth_url_in(stderr.as_bytes()) {
            return Err(Failure::tailscale_auth(url));
        }
        Ok(Output {
            status,
            stdout: stdout.finish(),
            stderr,
        })
    }
}

/// A pipe being read to EOF on its own thread, capped at [`OUTPUT_CAP`]. Both
/// pipes must be drained concurrently with the wait or a chatty remote fills
/// one and blocks.
///
/// The buffer is shared rather than returned by the thread so the poll loop can
/// read what has arrived so far without waiting on the child — which is what
/// makes the Tailscale check detectable, and is also the only safe shape on the
/// timeout path, where a descendant holding the pipe open would leave a
/// `join()` blocked forever.
struct Drain {
    buffer: Arc<Mutex<Vec<u8>>>,
    thread: std::thread::JoinHandle<()>,
}

impl Drain {
    fn start<R: Read + Send + 'static>(pipe: Option<R>) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let thread = std::thread::spawn(move || {
            let Some(mut pipe) = pipe else { return };
            let mut chunk = [0u8; 8192];
            loop {
                // Read outside the lock: holding it across a blocking read
                // would stall every peek for as long as the remote is quiet.
                let Ok(read @ 1..) = pipe.read(&mut chunk) else {
                    return;
                };
                let mut buffer = lock(&sink);
                if buffer.len() >= OUTPUT_CAP {
                    return;
                }
                buffer.extend_from_slice(&chunk[..read]);
                buffer.truncate(OUTPUT_CAP);
            }
        });
        Self { buffer, thread }
    }

    /// Read what has arrived so far, in place. Never waits on the child, so it
    /// is safe on the timeout path.
    fn with_bytes<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(&lock(&self.buffer))
    }

    /// Everything, once the pipe closes.
    fn finish(self) -> String {
        let _ = self.thread.join();
        String::from_utf8_lossy(&lock(&self.buffer)).into_owned()
    }
}

/// A timeout that still reports whatever the child managed to say. The drained
/// stderr used to be dropped on this path — exactly when it is most wanted, on
/// a host that printed a diagnosis and then hung. Scrubbed through the same
/// `snippet` as [`Output::failure`], because it is raw remote output.
fn timed_out(timeout: Duration, buffered: &[u8]) -> Failure {
    let detail = crate::protocol::snippet(&String::from_utf8_lossy(buffered));
    let seconds = timeout.as_secs();
    if detail.is_empty() {
        format!("ssh timed out after {seconds}s").into()
    } else {
        format!("ssh timed out after {seconds}s: {detail}").into()
    }
}

/// The drain thread cannot panic while holding the buffer, so poisoning is
/// unreachable; recovering rather than unwrapping keeps that from ever becoming
/// a way to take the provider down.
fn lock(buffer: &Mutex<Vec<u8>>) -> std::sync::MutexGuard<'_, Vec<u8>> {
    buffer.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Windows ships OpenSSH in System32 but does not always put it on a GUI
/// process's PATH, so look there first.
fn resolve_ssh() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    let mut candidates = Vec::new();
    if cfg!(windows) {
        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        candidates.push(system_root.join("System32").join("OpenSSH").join(exe));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(exe)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// The desktop's `util::configure_no_window`, transcribed. The desktop applies
/// it to its own spawn of this provider, but `CREATE_NO_WINDOW` does not
/// inherit, so every child spawned here must set it again or Windows flashes a
/// console window per op.
pub fn configure_no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Quote a value for POSIX `sh`. Single quotes suppress every expansion; the
/// only character needing care is `'` itself.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(host: &str) -> SshConfig {
        SshConfig {
            host: host.into(),
            user: Some("ubuntu".into()),
            port: Some(2222),
            identity_file: Some("/home/me/.ssh/id_ed25519".into()),
            buzz_acp_path: None,
        }
    }

    #[test]
    fn every_invocation_is_batch_mode_with_the_script_on_stdin() {
        let Ok(session) = Session::new(&config("vps"), false) else {
            return; // no ssh client in this environment
        };
        assert!(session
            .args
            .windows(2)
            .any(|w| w == ["-o", "BatchMode=yes"]));
        // The remote argv is exactly `sh -s`; nothing op-specific, and so
        // nothing secret, is ever visible in the remote process table.
        assert_eq!(session.args.last().unwrap(), "sh -s");
        assert!(session.args.contains(&"--".to_string()));
        assert!(session.args.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(session
            .args
            .windows(2)
            .any(|w| w == ["-i", "/home/me/.ssh/id_ed25519"]));
    }

    #[test]
    fn accept_new_host_key_is_scoped_to_tailnet_addresses() {
        let Ok(manual) = Session::new(&config("vps.example.com"), false) else {
            return;
        };
        assert!(manual
            .args
            .contains(&"StrictHostKeyChecking=ask".to_string()));
        assert!(!manual
            .args
            .contains(&"StrictHostKeyChecking=accept-new".to_string()));

        let tailnet = Session::new(&config("vps.tailcfd703.ts.net"), true).unwrap();
        assert!(tailnet
            .args
            .contains(&"StrictHostKeyChecking=accept-new".to_string()));
    }

    #[test]
    fn quote_neutralizes_shell_metacharacters() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("$(touch /tmp/pwn)"), "'$(touch /tmp/pwn)'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    // The two tests below stand a local `/bin/sh` in for the remote host to
    // exercise the write/drain/wait loop without a network. Everything else in
    // this file is platform-neutral and runs on the Windows CI job too.
    #[cfg(unix)]
    #[test]
    fn run_reports_the_command_output() {
        let Ok(session) = Session::new(&config("vps"), false) else {
            return;
        };
        // Point at a shell instead of a real host: `run` is transport
        // plumbing, and this exercises the write/drain/wait loop without a
        // network. `sh` ignores the ssh options it does not know.
        let local = Session {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-s".into()],
        };
        let _ = session;
        let out = local
            .run(
                "printf hello; printf oops >&2; exit 3",
                Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(out.stdout, "hello");
        assert_eq!(out.stderr, "oops");
        assert_eq!(out.status, Some(3));
        assert!(!out.ok());
        assert!(out.failure().contains("exit 3"));
    }

    #[cfg(unix)]
    #[test]
    fn run_kills_a_command_that_outlives_its_budget() {
        let session = Session {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-s".into()],
        };
        // `Output` is intentionally not `Debug` — it carries raw remote stderr,
        // which only `failure()` scrubs — so the error comes out by hand.
        let Err(error) = session.run("sleep 30", Duration::from_millis(300)) else {
            panic!("a command past its deadline must be killed, not awaited");
        };
        assert!(error.message.contains("timed out"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_still_reports_what_the_host_managed_to_say() {
        // The stderr of a host that diagnoses itself and *then* hangs is
        // exactly the output worth keeping, and it used to be dropped.
        let session = Session {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-s".into()],
        };
        let Err(error) = session.run(
            "printf 'disk is full\\n' >&2; sleep 30",
            Duration::from_millis(300),
        ) else {
            panic!("a command past its deadline must be killed, not awaited");
        };
        assert!(error.message.contains("disk is full"), "{error}");
        assert!(error.auth_url.is_none(), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_tailscale_auth_prompt_fails_fast_instead_of_burning_the_budget() {
        let session = Session {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-s".into()],
        };
        let started = Instant::now();
        // The shape ssh prints under a `check`-action tailnet ACL: the URL,
        // then a wait for a human who is not there.
        let Err(error) = session.run(
            "printf '# To authenticate, visit: https://login.tailscale.com/a/abc123\\n' >&2\nsleep 30\n",
            Duration::from_secs(10),
        ) else {
            panic!("a host asking for browser auth must fail, not hang");
        };
        assert_eq!(
            error.auth_url.as_deref(),
            Some("https://login.tailscale.com/a/abc123")
        );
        // The fail-fast IS the feature: waiting out the budget is the bug.
        assert!(started.elapsed() < Duration::from_secs(5), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_tailscale_auth_prompt_on_a_failing_exit_is_classified_the_same_way() {
        // ssh can also print the URL and give up on its own, which lands past
        // the poll loop entirely.
        let session = Session {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-s".into()],
        };
        let Err(error) = session.run(
            "printf 'visit: https://login.tailscale.com/a/xyz789\\n' >&2; exit 255",
            Duration::from_secs(10),
        ) else {
            panic!("a host asking for browser auth must fail, not succeed");
        };
        assert_eq!(
            error.auth_url.as_deref(),
            Some("https://login.tailscale.com/a/xyz789")
        );
    }

    #[test]
    fn failure_text_scrubs_credentials_from_remote_stderr() {
        let output = Output {
            status: Some(1),
            stdout: String::new(),
            stderr: "refused key nsec1leakedleaked".into(),
        };
        assert!(!output.failure().contains("nsec1leakedleaked"));
        assert!(output.failure().contains("[REDACTED]"));
    }
}
