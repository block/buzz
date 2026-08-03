//! Host-aware harness discovery.
//!
//! Local discovery (`discover_acp_runtimes_from`) answers "which harnesses are
//! on *this* machine?" This module answers it for any host in the user's
//! `~/.ssh/config`, so an agent that already runs on another machine can be
//! found rather than described by hand.
//!
//! # Design constraints, learned the hard way
//!
//! * **The probe script is a constant.** No user input is interpolated into it,
//!   so single-quoting it into the `ssh` argv is safe by construction rather
//!   than by careful escaping. Host and port reach `ssh` as separate argv
//!   entries, never through the shell.
//! * **It runs under `exec $SHELL -lc` — login, but NOT interactive.** Harness
//!   binaries live in npm-global, homebrew, pyenv, and venv prefixes that a
//!   *login* shell puts on `PATH`, so `-l` is required. `-i` is not, and is
//!   actively harmful: an interactive shell sources `.zshrc`/`.bashrc`, which is
//!   where prompt frameworks, completion init, and autosuggestion plugins live.
//!   Several of those block forever without a TTY. Verified against a real macOS
//!   `/bin/zsh` host: `-lic` hung indefinitely and had to be killed, while
//!   `-lc` returned the complete binary set including a Python venv prefix.
//!   A probe that hangs is worse than one that misses a path, because it turns a
//!   healthy host into a timeout.
//! * **The `for` list is a flat set of binary names.** Harness identity is
//!   reattached afterwards, in Rust, by matching resolved binaries back to the
//!   probe targets. Encoding `harness=binary` pairs in the shell loop instead
//!   would put a delimiter inside a `for … in` list, and the obvious choice
//!   (`|`) is a parse error in both bash and zsh that kills the loop before it
//!   runs. Keeping the shell dumb avoids the question entirely.
//! * **`BatchMode=yes`, and a password wall is a status, not a prompt.** Buzz
//!   never collects or stores an SSH password. A host that offers only
//!   interactive auth is reported as such, with the fix (install a key) in the
//!   message.
//! * **Local and remote return the same shape.** `probe_localhost` runs the
//!   identical script, so nothing downstream needs a special case for "this
//!   machine".

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::managed_agents::discovery::{harness_probe_targets, HarnessProbeTarget};
use crate::managed_agents::ssh_config::{resolve_ssh_binary, SshHost};
use crate::managed_agents::HarnessSource;

/// Sentinel that brackets the probe's own output.
///
/// A login shell may print motd banners, shell-init chatter, or warnings before
/// and after our commands. Without a delimiter those lines get parsed as
/// results; with one, everything outside the markers is discarded.
const PROBE_START: &str = "---BUZZ-PROBE-START---";
const PROBE_END: &str = "---BUZZ-PROBE-END---";

/// Wall-clock ceiling for a single host probe. A wedged host must not be able
/// to hold the caller open — the UI renders one row per host and a single
/// unresponsive machine would otherwise stall the whole list.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// `ssh` connect timeout, kept well under [`PROBE_TIMEOUT`] so an unreachable
/// host fails through ssh's own error path (which yields a useful message)
/// rather than our blunt kill path.
const SSH_CONNECT_TIMEOUT_SECS: u32 = 6;

/// Per-binary ceiling for a `--version` call on the probed host.
///
/// A version string is informational; a hung `--version` is not. Observed on a
/// real host: `claude --version` never returned, which truncated the probe and
/// silently hid every harness later in the loop. Bounding each call means a
/// broken or first-run binary costs one `unknown` version instead of the whole
/// result.
///
/// Kept small because it multiplies: worst case is roughly this value times the
/// number of harnesses that both exist and hang, and it must stay well inside
/// [`PROBE_TIMEOUT`].
const VERSION_TIMEOUT_SECS: u32 = 3;

/// Why a probe failed, when the cause is actionable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProbeErrorKind {
    /// The host offered only password / keyboard-interactive auth, which a
    /// `BatchMode` probe cannot satisfy and Buzz will not collect.
    PasswordRequired,
    /// The host key is unknown or changed — a trust decision the user must make
    /// outside Buzz.
    HostKeyProblem,
    /// Name resolution or the TCP connection failed.
    Unreachable,
    /// The probe exceeded [`PROBE_TIMEOUT`].
    TimedOut,
    /// The probe started but its output stopped before the closing marker, so
    /// the facts gathered are an unknown fraction of the real ones.
    Truncated,
}

/// One harness found on a probed host.
///
/// Deliberately narrower than the local `AcpRuntimeCatalogEntry`. That type
/// carries `can_auto_install`, `node_required`, and `auth_status`, all of which
/// describe actions Buzz performs on the local machine. Buzz does not install
/// software on, or authenticate CLIs on, someone else's host — reusing the local
/// shape would mean fabricating those fields, and the UI would then offer
/// buttons that cannot work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHarness {
    pub id: String,
    pub label: String,
    pub source: HarnessSource,
    /// Resolved absolute path of the ACP command on the remote host.
    pub acp_command_path: Option<String>,
    /// The ACP command basename that resolved, for building a run command.
    pub acp_command: Option<String>,
    /// Version string the ACP command reported, when it reported one.
    pub version: Option<String>,
    /// Resolved path of the vendor CLI this harness wraps, when it wraps one.
    pub underlying_cli_path: Option<String>,
    /// True when the harness is usable on this host: its ACP command resolved,
    /// and any vendor CLI it wraps also resolved.
    pub ready: bool,
    pub install_hint: String,
    pub install_instructions_url: String,
}

/// Result of probing one host.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostProbeResult {
    /// The `ssh` alias probed, or [`LOCALHOST_ID`] for this machine.
    pub host: String,
    pub ok: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<HostProbeErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Path of the `buzz` CLI on the host. A connected agent needs it to reach
    /// the relay, so its absence is the single most useful thing to surface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buzz_cli_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buzz_cli_version: Option<String>,
    pub harnesses: Vec<RemoteHarness>,
}

/// Host id used for the local machine, so it can sit in the same list as ssh
/// aliases without colliding with one (`localhost` is a legal alias, this is
/// not).
pub const LOCALHOST_ID: &str = "__localhost__";

/// Build the probe script for a target set.
///
/// Returns a script containing only literals derived from the compiled-in
/// harness tables — never user input. Callers must not append anything to it.
fn build_probe_script(targets: &[HarnessProbeTarget]) -> String {
    // Every ACP command basename across all harnesses, plus every vendor CLI,
    // plus `buzz`. Sorted and deduped so the emitted script is deterministic
    // (which makes it cacheable and makes test assertions stable).
    let mut binaries: BTreeSet<&str> = BTreeSet::new();
    for target in targets {
        for command in target.acp_commands {
            binaries.insert(command);
        }
        if let Some(cli) = target.underlying_cli {
            binaries.insert(cli);
        }
    }
    binaries.insert("buzz");

    let binary_list = binaries.into_iter().collect::<Vec<_>>().join(" ");

    // `command -v` rather than `which`: it is a POSIX shell builtin, present
    // even on minimal images, and does not depend on an external binary that
    // may itself be missing.
    //
    // Each `--version` call is individually time-bounded. This is not
    // defensive padding — a real harness binary was observed hanging forever on
    // `--version` on a real host (a `claude` install on macOS), which truncated
    // the whole probe: every harness after it in the loop went unreported and
    // the trailing sentinel never printed, so the result looked like a
    // half-provisioned machine rather than a stuck command.
    //
    // The bound is hand-rolled because `timeout(1)` is not portable — it is
    // absent from a stock macOS, which is precisely where the hang was found.
    // Shape: run the version command in the background, run a killer in the
    // background, then `wait` for the version command. The killer's stdout is
    // closed, which matters — otherwise it holds the command substitution's
    // pipe open for the full sleep and every binary would cost
    // `VERSION_TIMEOUT` even when it answered instantly.
    //
    // `</dev/null` on the version call is a second, independent guard: an ACP
    // harness that does not implement `--version` may instead start its
    // JSON-RPC server and block reading stdin. EOF makes it exit rather than
    // wait for the killer.
    //
    // Quotes are stripped from the captured version because several harnesses
    // print quoted strings, and a stray quote would corrupt the `:`-delimited
    // record. `\047` is a literal single quote — it cannot appear as itself,
    // since the whole script is single-quoted into the ssh argv.
    format!(
        r#"exec $SHELL -lc '
echo "{PROBE_START}"
for tool in {binary_list}; do
  bin=$(command -v "$tool" 2>/dev/null)
  if [ -n "$bin" ]; then
    ver=$( {{ "$tool" --version </dev/null 2>/dev/null & vp=$!; {{ sleep {version_timeout}; kill -9 $vp; }} >/dev/null 2>&1 & kp=$!; wait $vp; kill -9 $kp; }} 2>/dev/null | head -1 | tr -d "\"\047" | tr -d "\r" )
    echo "BIN:$tool:$bin:${{ver:-unknown}}"
  fi
done
echo "USER:$USER"
echo "HOST:$(hostname -s 2>/dev/null)"
echo "OS:$(uname -s 2>/dev/null)"
echo "{PROBE_END}"
'"#,
        version_timeout = VERSION_TIMEOUT_SECS
    )
}

/// Facts a single probe run recovered from the host.
#[derive(Debug, Default)]
struct ProbeFacts {
    /// binary basename → (resolved path, version)
    binaries: BTreeMap<String, (String, Option<String>)>,
    user: Option<String>,
    hostname: Option<String>,
    os: Option<String>,
}

/// Parse probe stdout, ignoring everything outside the sentinels.
fn parse_probe_output(raw: &str) -> ProbeFacts {
    let mut facts = ProbeFacts::default();
    let mut inside = false;

    for line in raw.lines() {
        if line.contains(PROBE_START) {
            inside = true;
            continue;
        }
        if line.contains(PROBE_END) {
            inside = false;
            continue;
        }
        if !inside {
            continue;
        }

        if let Some(rest) = line.strip_prefix("BIN:") {
            // `BIN:<tool>:<path>:<version>` — the version may itself contain
            // colons, so split into at most 3 pieces and keep the remainder
            // whole. The path may not contain a colon, which holds for every
            // real install prefix.
            let mut parts = rest.splitn(3, ':');
            let (Some(tool), Some(path)) = (parts.next(), parts.next()) else {
                continue;
            };
            let version = parts
                .next()
                .map(str::trim)
                .filter(|v| !v.is_empty() && *v != "unknown")
                .map(str::to_string);
            let tool = tool.trim();
            let path = path.trim();
            if tool.is_empty() || path.is_empty() {
                continue;
            }
            facts
                .binaries
                .insert(tool.to_string(), (path.to_string(), version));
        } else if let Some(rest) = line.strip_prefix("USER:") {
            facts.user = non_empty(rest);
        } else if let Some(rest) = line.strip_prefix("HOST:") {
            facts.hostname = non_empty(rest);
        } else if let Some(rest) = line.strip_prefix("OS:") {
            facts.os = non_empty(rest);
        }
    }

    facts
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Assemble harness entries from probe facts.
///
/// Shared by the ssh and localhost paths so both produce identical shapes.
fn assemble_harnesses(facts: &ProbeFacts, targets: &[HarnessProbeTarget]) -> Vec<RemoteHarness> {
    targets
        .iter()
        .map(|target| {
            // First listed ACP command that resolved wins, matching the local
            // catalog's preference-order semantics.
            let found = target
                .acp_commands
                .iter()
                .find_map(|cmd| facts.binaries.get(*cmd).map(|hit| (*cmd, hit)));

            let underlying_cli_path = target
                .underlying_cli
                .and_then(|cli| facts.binaries.get(cli))
                .map(|(path, _)| path.clone());

            // A harness is ready only if its ACP command exists AND, when it is
            // an adapter, the vendor CLI it wraps exists too. An adapter without
            // its CLI starts and then fails at first use, so reporting it as
            // ready would be worse than reporting it missing.
            let ready = found.is_some()
                && (target.underlying_cli.is_none() || underlying_cli_path.is_some());

            RemoteHarness {
                id: target.id.to_string(),
                label: target.label.to_string(),
                source: target.source.clone(),
                acp_command: found.map(|(cmd, _)| cmd.to_string()),
                acp_command_path: found.map(|(_, (path, _))| path.clone()),
                version: found.and_then(|(_, (_, version))| version.clone()),
                underlying_cli_path,
                ready,
                install_hint: target.install_hint.to_string(),
                install_instructions_url: target.install_instructions_url.to_string(),
            }
        })
        .collect()
}

/// Classify ssh's stderr into an actionable cause.
///
/// Raw ssh stderr is accurate but unhelpful in a UI; these are the cases where
/// naming the cause tells the user what to actually do.
pub fn classify_ssh_failure(stderr: &str) -> Option<HostProbeErrorKind> {
    let lower = stderr.to_ascii_lowercase();

    // A denial listing password or keyboard-interactive means the host wants
    // interactive auth. A bare `(publickey)` denial is NOT this case — that is a
    // missing or rejected key, where the raw message is the more honest report.
    if let Some(start) = lower.find("permission denied") {
        let tail = &lower[start..];
        if let (Some(open), Some(close)) = (tail.find('('), tail.find(')')) {
            if open < close {
                let methods = &tail[open + 1..close];
                if methods.contains("password") || methods.contains("keyboard-interactive") {
                    return Some(HostProbeErrorKind::PasswordRequired);
                }
            }
        }
    }

    if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
        // Emitted by `StrictHostKeyChecking=yes` for a first-seen host. Matched
        // in its own right because it is the line that names the actual cause;
        // relying only on the generic "verification failed" that follows it
        // would leave an unknown key indistinguishable from a changed one.
        || lower.contains("you have requested strict checking")
    {
        return Some(HostProbeErrorKind::HostKeyProblem);
    }

    if lower.contains("could not resolve hostname")
        || lower.contains("name or service not known")
        || lower.contains("connection refused")
        || lower.contains("connection timed out")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
        || lower.contains("operation timed out")
    {
        return Some(HostProbeErrorKind::Unreachable);
    }

    None
}

/// Human-facing message for a classified failure, including the remedy.
fn failure_message(kind: &HostProbeErrorKind, host: &str, stderr: &str) -> String {
    match kind {
        HostProbeErrorKind::PasswordRequired => format!(
            "'{host}' accepts only password login. Buzz never stores SSH passwords — \
             set up key-based access instead (for example `ssh-copy-id {host}`), or add \
             an IdentityFile for this host in ~/.ssh/config."
        ),
        // A changed key and a first-seen key are both refused, but they are not
        // the same news: one is routine setup, the other is the warning ssh
        // exists to give. Reporting them identically would train the user to
        // dismiss the serious one.
        HostProbeErrorKind::HostKeyProblem
            if stderr
                .to_ascii_lowercase()
                .contains("remote host identification has changed") =>
        {
            format!(
                "The host key for '{host}' has CHANGED since it was last trusted. This can mean \
                 the host was rebuilt — or that the connection is being intercepted. Buzz will \
                 not probe it. Verify the new key out of band before touching known_hosts."
            )
        }
        HostProbeErrorKind::HostKeyProblem => format!(
            "The host key for '{host}' is not yet trusted on this machine. Buzz does not accept \
             host keys on your behalf — connect once with `ssh {host}`, check the fingerprint, \
             then probe again."
        ),
        HostProbeErrorKind::Unreachable => {
            format!("'{host}' is not reachable: {}", first_line(stderr))
        }
        HostProbeErrorKind::TimedOut => format!(
            "Probing '{host}' exceeded {}s and was cancelled.",
            PROBE_TIMEOUT.as_secs()
        ),
        HostProbeErrorKind::Truncated => format!(
            "The probe of '{host}' was cut off before it finished. What it found is incomplete, \
             so it is not being reported. Check the connection to '{host}' and probe again."
        ),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no error output")
        .to_string()
}

/// Probe one ssh host for harnesses and the `buzz` CLI.
///
/// Never returns `Err` for a *host-side* problem: an unreachable or
/// unauthenticated host is a normal, reportable outcome, and the caller renders
/// one row per host regardless. `Err` is reserved for a failure to run `ssh` at
/// all.
pub fn probe_ssh_host(host: &SshHost) -> HostProbeResult {
    let started = Instant::now();
    let targets = harness_probe_targets();
    let script = build_probe_script(&targets);

    let mut command = Command::new(resolve_ssh_binary());
    command.args(ssh_probe_args(host)).arg(&script);

    run_probe(command, &host.host, &targets, started)
}

/// The `ssh` arguments preceding the probe script, ending with the host alias.
///
/// Split out so the trust-affecting options are assertable: nothing else in this
/// module consults `known_hosts`, so whether Buzz can alter the user's trust
/// state is decided entirely by this list.
fn ssh_probe_args(host: &SshHost) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"),
        // Never prompt. A probe that blocks on a password prompt would hang the
        // UI with no way for the user to see or answer it.
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        // Reject an unknown key as well as a changed one. `accept-new` would
        // write a first-seen key into the user's `known_hosts` as a side effect
        // of opening a dialog and clicking Probe — Buzz would be making a trust
        // decision, and persisting it, on their behalf. Both cases are a
        // reportable status here; the user grants trust with `ssh <host>`, where
        // they see the fingerprint and answer for themselves.
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        // Suppress banners so parsing has less to discard.
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
    ];
    if let Some(port) = &host.port {
        args.push("-p".to_string());
        args.push(port.clone());
    }
    // The alias, not `user@hostname`: the alias is what carries the user's own
    // ssh config (User, IdentityFile, ProxyJump, and anything else we do not
    // model). Rebuilding a user@host string would discard all of it.
    args.push(host.host.clone());
    args
}

/// Probe the machine Buzz is running on, using the identical script.
pub fn probe_localhost() -> HostProbeResult {
    let started = Instant::now();
    let targets = harness_probe_targets();
    let script = build_probe_script(&targets);

    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(&script);

    run_probe(command, LOCALHOST_ID, &targets, started)
}

/// Execute a prepared probe command and shape its outcome.
fn run_probe(
    mut command: Command,
    host: &str,
    targets: &[HarnessProbeTarget],
    started: Instant,
) -> HostProbeResult {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let base = |ok: bool| HostProbeResult {
        host: host.to_string(),
        ok,
        duration_ms: started.elapsed().as_millis() as u64,
        error: None,
        error_kind: None,
        user: None,
        hostname: None,
        os: None,
        buzz_cli_path: None,
        buzz_cli_version: None,
        harnesses: Vec::new(),
    };

    let output = match wait_with_timeout(command, PROBE_TIMEOUT) {
        Ok(Some(output)) => output,
        Ok(None) => {
            let kind = HostProbeErrorKind::TimedOut;
            return HostProbeResult {
                error: Some(failure_message(&kind, host, "")),
                error_kind: Some(kind),
                ..base(false)
            };
        }
        Err(err) => {
            return HostProbeResult {
                error: Some(format!("could not run probe for '{host}': {err}")),
                error_kind: None,
                ..base(false)
            };
        }
    };

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Success is "the probe produced its own output", not "exit code 0". A login
    // shell can exit non-zero because of an unrelated rc-file quirk while still
    // having run every command we asked for; discarding that would report a
    // healthy host as broken.
    if !stdout.contains(PROBE_START) {
        let kind = classify_ssh_failure(&stderr);
        let message = match &kind {
            Some(kind) => failure_message(kind, host, &stderr),
            None => {
                let detail = first_line(&stderr);
                format!("probe of '{host}' produced no output: {detail}")
            }
        };
        return HostProbeResult {
            error: Some(message),
            error_kind: kind,
            ..base(false)
        };
    }

    // Both markers, not just the opening one. The script emits PROBE_END as its
    // last statement, so its absence means the session died partway through the
    // harness loop — and `parse_probe_output` cannot tell that from a host that
    // genuinely has no `openclaw` installed. Reporting `ok: true` there would
    // present "this harness is missing" and "we never got to look" as the same
    // answer, and the connect dialog would offer a harness list that is missing
    // entries for no visible reason.
    if !stdout.contains(PROBE_END) {
        let kind = HostProbeErrorKind::Truncated;
        return HostProbeResult {
            error: Some(failure_message(&kind, host, &stderr)),
            error_kind: Some(kind),
            ..base(false)
        };
    }

    let facts = parse_probe_output(&stdout);
    let harnesses = assemble_harnesses(&facts, targets);
    let buzz = facts.binaries.get("buzz");

    HostProbeResult {
        user: facts.user.clone(),
        hostname: facts.hostname.clone(),
        os: facts.os.clone(),
        buzz_cli_path: buzz.map(|(path, _)| path.clone()),
        buzz_cli_version: buzz.and_then(|(_, version)| version.clone()),
        harnesses,
        ..base(true)
    }
}

/// Wait for a child with a wall-clock ceiling.
///
/// Returns `Ok(None)` on timeout, having killed the child. `Command::output()`
/// has no timeout, and an ssh that connects but then stalls (a wedged login
/// shell, a hung NFS mount in a profile script) would otherwise block forever.
fn wait_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<Option<std::process::Output>> {
    let mut child = command.spawn()?;

    // Reading the pipes must not be deferred until after the wait: a child that
    // fills its stdout pipe buffer blocks on write while we block on wait.
    // Draining on threads keeps both sides moving.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr_reader = std::thread::spawn(move || read_all(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    Ok(status.map(|status| std::process::Output {
        status,
        stdout,
        stderr,
    }))
}

fn read_all<R: std::io::Read>(source: Option<R>) -> Vec<u8> {
    let mut buffer = Vec::new();
    if let Some(mut source) = source {
        let _ = std::io::Read::read_to_end(&mut source, &mut buffer);
    }
    buffer
}

#[cfg(test)]
#[path = "remote_probe_tests.rs"]
mod tests;
